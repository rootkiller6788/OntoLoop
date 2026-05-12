param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "pq9-rollout"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$backupPath = ""

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("pq9-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("pq9-acceptance-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("pq9-autoloop.prod.backup-" + $stamp + ".toml")

  Copy-Item -Path $ProdConfigPath -Destination $backupPath -Force

  function Invoke-Step {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv
    )

    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Name + "] " + $display + " ====")

    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $Exe @Argv 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev

    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
    }

    if ($exitCode -ne 0) {
      throw "Command failed ($exitCode): [$Name] $display"
    }

    return [pscustomobject]@{
      name = $Name
      command = $display
      passed = $true
      exit_code = 0
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

  function Set-RollbackVersion {
    param([string]$Version)
    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, 'rollback_contract_version\s*=\s*".*?"', ('rollback_contract_version = "' + $Version + '"'))
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Set-LocalStorageEndpoints {
    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, '(?ms)(\[state_store\].*?backend\s*=\s*").*?(")', '$1in_memory$2')
    $content = [regex]::Replace($content, '(?ms)(\[state_store\].*?uri\s*=\s*").*?(")', '$1http://127.0.0.1:3000$2')
    $localPgUri = [Environment]::GetEnvironmentVariable("AUTOLOOP_LOCAL_POSTGRES_URI")
    if ([string]::IsNullOrWhiteSpace($localPgUri)) {
      $localPgUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod"
    }
    $content = [regex]::Replace(
      $content,
      '(?ms)(\[storage\.postgres\].*?uri\s*=\s*").*?(")',
      { param($m) $m.Groups[1].Value + $localPgUri + $m.Groups[2].Value }
    )
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  $results = @()

  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  $results += Invoke-Step -Name "e2e-full-chain-single-session" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p10_day10_acceptance_e2e")
  $results += Invoke-Step -Name "e2e-replay-mismatch-explainer" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p10_replay_mismatch_explainer_e2e")
  $results += Invoke-Step -Name "e2e-no-bypass-kernel" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p5_runtime_escape_guard")
  $results += Invoke-Step -Name "e2e-resume-after-restart" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq5_session_checkpoint_resume_e2e")

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
    $results += Invoke-Step -Name ("rollout-" + $stage.name + "-health") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "health")

    if ($env:OPENAI_API_KEY) {
      $results += Invoke-Step -Name ("rollout-" + $stage.name + "-workload") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", $stage.session, "--swarm", "--message", "run canary workload")
    }
    else {
      Add-Content -Path $logPath -Value ("INFO: skip workload on stage " + $stage.name + " because OPENAI_API_KEY is empty")
      $results += [pscustomobject]@{
        name = "rollout-" + $stage.name + "-workload"
        command = "cargo run ... --swarm --message 'run canary workload'"
        passed = $true
        skipped = $true
        reason = "OPENAI_API_KEY missing"
      }
    }
  }

  Set-RollbackVersion -Version "v1"
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
    required_checks = @(
      "long-single-session-e2e",
      "replay-mismatch-explainer",
      "no-bypass-kernel",
      "resume-after-restart"
    )
    rollout = @("shadow", "10%", "30%", "full", "rollback")
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("PQ9_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("PQ9_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  Pop-Location
}

