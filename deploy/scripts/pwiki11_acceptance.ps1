param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "pwiki11-rollout"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$backupPath = ""
$previousAutoLoopProfile = if (Test-Path Env:AUTOLOOP_PROFILE) { $env:AUTOLOOP_PROFILE } else { $null }

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("pwiki11-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("pwiki11-acceptance-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("pwiki11-autoloop.prod.backup-" + $stamp + ".toml")

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

  $pwiki11Tests = @(
    "pwiki11_semantic_edges_contract",
    "pwiki11_inference_checkpoint_roundtrip",
    "pwiki11_graph_export_service",
    "pwiki11_hot_index_refresh_modes",
    "pwiki11_ingest_validator_validate_only",
    "pwiki11_recall_cjk_fallback",
    "pwiki11_recall_neighbor_expansion",
    "pwiki11_semantic_lint_sections",
    "pwiki11_view_plane_persist_graph_health",
    "pwiki11_heal_proposal_queue",
    "pwiki11_refresh_source_hash_stale",
    "pwiki11_memory_chain_e2e"
  )

  $env:AUTOLOOP_PROFILE = "integration"
  foreach ($testName in $pwiki11Tests) {
    $results += Invoke-Step -Name ("pwiki11-" + $testName) -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", $testName)
  }
  if ($null -eq $previousAutoLoopProfile) {
    Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_PROFILE = $previousAutoLoopProfile
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
      "pwiki11-tests",
      "graph-health-lint",
      "heal-proposal-gate",
      "e2e-memory-chain"
    )
    rollout = @("shadow", "10%", "30%", "full", "rollback")
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("PWIKI11_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("PWIKI11_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  if ($null -eq $previousAutoLoopProfile) {
    Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_PROFILE = $previousAutoLoopProfile
  }
  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  Pop-Location
}

