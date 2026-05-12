param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "sandbox-rollout",
  [string]$LocalPostgresUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod",
  [int]$StepRetryCount = 1,
  [switch]$IncludeBrokerAck
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$backupPath = ""
$previousCargoTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
$previousLocalPgUri = if (Test-Path Env:AUTOLOOP_LOCAL_POSTGRES_URI) { $env:AUTOLOOP_LOCAL_POSTGRES_URI } else { $null }
$previousAutoLoopProfile = if (Test-Path Env:AUTOLOOP_PROFILE) { $env:AUTOLOOP_PROFILE } else { $null }
$env:AUTOLOOP_PROFILE = "production-e2e"
$scriptSucceeded = $false
$runtimeDir = ""
$targetDir = ""

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("sandbox-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("sandbox-acceptance-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("sandbox-autoloop.prod.backup-" + $stamp + ".toml")
  $targetDir = Join-Path $runtimeDir ("target-sandbox-" + $stamp)
  New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
  $env:CARGO_TARGET_DIR = $targetDir
  $schema = ("ol_sandbox_" + $stamp.Replace("-", "_"))
  $isolatedPgUri = $LocalPostgresUri + "?options=-csearch_path%3D" + $schema + "%2Cpublic"
  $env:AUTOLOOP_LOCAL_POSTGRES_URI = $isolatedPgUri
  $psql = Get-Command psql -ErrorAction SilentlyContinue
  if ($null -ne $psql) {
    $createSql = "CREATE SCHEMA IF NOT EXISTS " + $schema + ";"
    & $psql.Source -d $LocalPostgresUri -v ON_ERROR_STOP=1 -c $createSql | Out-Null
  }

  Copy-Item -Path $ProdConfigPath -Destination $backupPath -Force

  function Invoke-Step {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv
    )

    $maxAttempts = [Math]::Max(1, $StepRetryCount + 1)
    for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
      $display = "$Exe $($Argv -join ' ')"
      Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Name + "] attempt " + $attempt + "/" + $maxAttempts + " " + $display + " ====")

      $prev = $ErrorActionPreference
      $ErrorActionPreference = "Continue"
      $output = & $Exe @Argv 2>&1
      $exitCode = $LASTEXITCODE
      $ErrorActionPreference = $prev

      $outputText = ""
      if ($null -ne $output) {
        $output | Out-File -FilePath $logPath -Append -Encoding utf8
        $outputText = ($output | Out-String)
      }

      if ($exitCode -eq 0) {
        return [pscustomobject]@{
          name = $Name
          command = $display
          passed = $true
          exit_code = 0
          optional = $false
          attempts = $attempt
        }
      }

      $isTransientLock = ($outputText -match "os error 5") -or ($outputText -match "The process cannot access the file")
      if ($attempt -lt $maxAttempts -and $isTransientLock) {
        Start-Sleep -Milliseconds (500 * $attempt)
        continue
      }

      throw "Command failed ($exitCode): [$Name] $display"
    }
  }

  function Invoke-OptionalStep {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv
    )

    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN OPTIONAL: [" + $Name + "] " + $display + " ====")

    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $Exe @Argv 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev

    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
    }

    return [pscustomobject]@{
      name = $Name
      command = $display
      passed = ($exitCode -eq 0)
      exit_code = $exitCode
      optional = $true
    }
  }

  function Set-GateConfig {
    param(
      [string]$Mode,
      [double]$Ratio
    )

    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, 'gate_mode\s*=\s*".*?"', ('gate_mode = "' + $Mode + '"'))
    $content = [regex]::Replace($content, 'gate_enforce_ratio\s*=\s*[0-9.]+', ('gate_enforce_ratio = ' + $Ratio.ToString([System.Globalization.CultureInfo]::InvariantCulture)))
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Set-LocalStorageEndpoints {
    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, '(?ms)(\[state_store\].*?backend\s*=\s*").*?(")', '$1in_memory$2')
    $content = [regex]::Replace($content, '(?ms)(\[state_store\].*?uri\s*=\s*").*?(")', '$1http://127.0.0.1:3000$2')
    $localPgUri = $env:AUTOLOOP_LOCAL_POSTGRES_URI
    if ([string]::IsNullOrWhiteSpace($localPgUri)) {
      $localPgUri = $LocalPostgresUri
    }
    $content = [regex]::Replace(
      $content,
      '(?ms)(\[storage\.postgres\].*?uri\s*=\s*").*?(")',
      { param($m) $m.Groups[1].Value + $localPgUri + $m.Groups[2].Value }
    )
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  $results = @()
  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--manifest-path", $ManifestPath)

  # 1) contract_compat
  $results += Invoke-Step -Name "contract_compat" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "sandbox_contract_compatible")

  # 2) runtime_class_dispatch
  $results += Invoke-Step -Name "runtime_class_dispatch" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "high_risk_trust_plan_maps_to_trusted_runtime_class")

  # 3) high_risk_trustbridge_enforced
  $results += Invoke-Step -Name "high_risk_trustbridge_enforced" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::trust_bridge::tests::attestation_gate_requires_env_when_enabled")

  # 4) hook_5phase_pipeline
  $results += Invoke-Step -Name "hook_5phase_pipeline" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::hook_runtime::tests::legacy_stage_maps_to_phase")

  # 5) broker_delivery_ack (optional)
  if ($IncludeBrokerAck) {
    $results += Invoke-OptionalStep -Name "broker_delivery_ack" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "task_topics_roundtrip_and_ack")
  }

  Set-LocalStorageEndpoints

  $stages = @(
    [pscustomobject]@{ name = "shadow"; mode = "shadow"; ratio = 0.2; session = $SessionPrefix + "-shadow" },
    [pscustomobject]@{ name = "canary10"; mode = "canary"; ratio = 0.1; session = $SessionPrefix + "-10" },
    [pscustomobject]@{ name = "canary30"; mode = "canary"; ratio = 0.3; session = $SessionPrefix + "-30" },
    [pscustomobject]@{ name = "full"; mode = "full"; ratio = 1.0; session = $SessionPrefix + "-full" }
  )

  foreach ($stage in $stages) {
    Set-GateConfig -Mode $stage.mode -Ratio $stage.ratio
    $results += Invoke-Step -Name ("rollout-" + $stage.name + "-status") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status")
    $results += Invoke-Step -Name ("rollout-" + $stage.name + "-health") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", $stage.session, "system", "health")
  }

  Set-GateConfig -Mode "shadow" -Ratio 0.2
  $results += Invoke-Step -Name "rollback-status" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status")
  $results += Invoke-Step -Name "rollback-health" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", ($SessionPrefix + "-rollback"), "system", "health")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    backup_config = $backupPath
    all_passed = $true
    required_groups = @(
      "contract_compat",
      "runtime_class_dispatch",
      "high_risk_trustbridge_enforced",
      "hook_5phase_pipeline",
      "broker_delivery_ack(optional)"
    )
    rollout = @("shadow", "canary(10%)", "30%", "full", "rollback")
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  $scriptSucceeded = $true
  Write-Output ("SANDBOX_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("SANDBOX_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  if ($null -eq $previousLocalPgUri) {
    Remove-Item Env:AUTOLOOP_LOCAL_POSTGRES_URI -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_LOCAL_POSTGRES_URI = $previousLocalPgUri
  }
  if ($null -eq $previousAutoLoopProfile) {
    Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_PROFILE = $previousAutoLoopProfile
  }
  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  if (-not [string]::IsNullOrWhiteSpace($runtimeDir) -and (Test-Path $runtimeDir)) {
    $cutoff = (Get-Date).AddDays(-7)
    if ($scriptSucceeded) {
      if (-not [string]::IsNullOrWhiteSpace($targetDir) -and (Test-Path $targetDir)) {
        Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction SilentlyContinue
      }
      Get-ChildItem -Path $runtimeDir -File -Filter "sandbox-acceptance-*.log" -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -lt $cutoff } |
        Remove-Item -Force -ErrorAction SilentlyContinue
    } else {
      $diagFiles = @(
        Get-ChildItem -Path $runtimeDir -File -Filter "sandbox-acceptance-*.json" -ErrorAction SilentlyContinue
        Get-ChildItem -Path $runtimeDir -File -Filter "sandbox-acceptance-*.log" -ErrorAction SilentlyContinue
      ) | Sort-Object LastWriteTime -Descending
      $keepDiag = $diagFiles | Select-Object -First 1
      foreach ($f in $diagFiles) {
        if ($null -ne $keepDiag -and $f.FullName -eq $keepDiag.FullName) {
          if ($f.LastWriteTime -lt $cutoff) {
            Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue
          }
          continue
        }
        Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue
      }
      $targets = Get-ChildItem -Path $runtimeDir -Directory -Filter "target-sandbox-*" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending
      $keepTarget = $targets | Select-Object -First 1
      foreach ($d in $targets) {
        if ($null -ne $keepTarget -and $d.FullName -eq $keepTarget.FullName) {
          if ($d.LastWriteTime -lt $cutoff) {
            Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction SilentlyContinue
          }
          continue
        }
        Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction SilentlyContinue
      }
    }
  }
  Pop-Location
}

