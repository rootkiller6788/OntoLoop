param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "d14-rollout",
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

$backupPath = ""
$summaryPath = ""
$logPath = ""
$results = @()
$allPassed = $true
$failureMessage = $null
$rollbackRecord = $null

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("d14-rollout-" + $stamp + ".log")
  $summaryPath = Join-Path $runtimeDir ("d14-rollout-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("d14-rollout-config-backup-" + $stamp + ".toml")

  Copy-Item -Path $ProdConfigPath -Destination $backupPath -Force

  function Invoke-Step {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv,
      [switch]$AllowFailure
    )

    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Name + "] " + $display + " ====")
    if ($DryRun) {
      Add-Content -Path $logPath -Value "DRY_RUN=true (skipped execution)"
      $record = [pscustomobject]@{
        name = $Name
        command = $display
        passed = $true
        skipped = $true
        exit_code = 0
      }
      $script:results += $record
      return $record
    }

    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $Exe @Argv 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
    }

    $passed = ($exitCode -eq 0)
    $record = [pscustomobject]@{
      name = $Name
      command = $display
      passed = $passed
      skipped = $false
      exit_code = $exitCode
    }
    $script:results += $record
    if (-not $passed -and -not $AllowFailure) {
      throw "Command failed ($exitCode): [$Name] $display"
    }
    return $record
  }

  function Set-GateConfig {
    param(
      [string]$Mode,
      [double]$Ratio
    )
    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, 'gate_mode\s*=\s*".*?"', ('gate_mode = "' + $Mode + '"'))
    $content = [regex]::Replace(
      $content,
      'gate_enforce_ratio\s*=\s*[0-9.]+',
      ('gate_enforce_ratio = ' + $Ratio.ToString([System.Globalization.CultureInfo]::InvariantCulture))
    )
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Normalize-ConfigForLocalRollout {
    $lines = Get-Content -Path $ProdConfigPath
    $section = ""
    for ($i = 0; $i -lt $lines.Length; $i++) {
      $trimmed = $lines[$i].Trim()
      if ($trimmed -match '^\[(.+)\]$') {
        $section = $Matches[1]
        continue
      }

      switch ($section) {
        "state_store" {
          if ($trimmed -match '^backend\s*=') {
            $lines[$i] = 'backend = "in_memory"'
          }
        }
        "storage" {
          if ($trimmed -match '^backend\s*=') {
            $lines[$i] = 'backend = "primary_store"'
          }
        }
        "storage.postgres" {
          if ($trimmed -match '^enabled\s*=') {
            $lines[$i] = 'enabled = false'
          } elseif ($trimmed -match '^uri\s*=') {
            $lines[$i] = 'uri = "postgres://postgres:123456@localhost:5432/ontoloop"'
          }
        }
      }
    }
    Set-Content -Path $ProdConfigPath -Value $lines -Encoding utf8
  }

  function Invoke-Rollback {
    param([string]$Reason)
    try {
      Set-GateConfig -Mode "shadow" -Ratio 0.2
      $status = Invoke-Step -Name "rollback-status" -Exe "cargo" -Argv @(
        "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status"
      ) -AllowFailure
      $health = Invoke-Step -Name "rollback-health" -Exe "cargo" -Argv @(
        "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath,
        "--session", ($SessionPrefix + "-rollback"), "system", "health"
      ) -AllowFailure
      $script:rollbackRecord = [pscustomobject]@{
        triggered = $true
        reason = $Reason
        status_ok = $status.passed
        health_ok = $health.passed
        executed_at = (Get-Date).ToString("s")
      }
    } catch {
      $script:rollbackRecord = [pscustomobject]@{
        triggered = $true
        reason = $Reason
        status_ok = $false
        health_ok = $false
        executed_at = (Get-Date).ToString("s")
        error = $_.Exception.Message
      }
    }
  }

  Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  Invoke-Step -Name "rollout-gating-test" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p6_rollout_gating")
  Normalize-ConfigForLocalRollout

  $stages = @(
    [pscustomobject]@{ name = "shadow"; mode = "shadow"; ratio = 0.2; session = $SessionPrefix + "-shadow" },
    [pscustomobject]@{ name = "canary10"; mode = "canary"; ratio = 0.1; session = $SessionPrefix + "-canary10" },
    [pscustomobject]@{ name = "canary30"; mode = "canary"; ratio = 0.3; session = $SessionPrefix + "-canary30" },
    [pscustomobject]@{ name = "full"; mode = "full"; ratio = 1.0; session = $SessionPrefix + "-full" }
  )

  foreach ($stage in $stages) {
    Set-GateConfig -Mode $stage.mode -Ratio $stage.ratio
    Invoke-Step -Name ("rollout-" + $stage.name + "-status") -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status"
    )
    Invoke-Step -Name ("rollout-" + $stage.name + "-health") -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath,
      "--session", $stage.session, "system", "health"
    )
  }

  Invoke-Rollback -Reason "post_full_drill"
}
catch {
  $allPassed = $false
  $failureMessage = $_.Exception.Message
  Add-Content -Path $logPath -Value ("`n==== FAILURE ====")
  Add-Content -Path $logPath -Value $failureMessage
  Invoke-Rollback -Reason "auto_rollback_on_failure"
}
finally {
  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    backup_config = $backupPath
    all_passed = $allPassed
    dry_run = [bool]$DryRun
    rollout = @("shadow", "10%", "30%", "full", "rollback")
    failure = $failureMessage
    rollback = $rollbackRecord
    commands = $results
    log_path = $logPath
  }

  if ($summaryPath) {
    $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $summaryPath -Encoding utf8
    if ($allPassed) {
      Write-Output ("D14_ROLLOUT_OK log=" + $logPath)
      Write-Output ("D14_ROLLOUT_JSON=" + $summaryPath)
    } else {
      Write-Output ("D14_ROLLOUT_FAILED log=" + $logPath)
      Write-Output ("D14_ROLLOUT_JSON=" + $summaryPath)
    }
  }

  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  Pop-Location
}

if (-not $allPassed) {
  exit 1
}
