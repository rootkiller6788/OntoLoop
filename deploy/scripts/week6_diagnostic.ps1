param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "week6-diagnostic"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$previousAutoLoopProfile = if (Test-Path Env:AUTOLOOP_PROFILE) { $env:AUTOLOOP_PROFILE } else { $null }
$env:AUTOLOOP_PROFILE = "production-e2e"
$env:RUST_MIN_STACK = "16777216"

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("week6-diagnostic-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("week6-diagnostic-report-" + $stamp + ".json")
  $canonicalJsonPath = Join-Path $runtimeDir "week6_diagnostic_report.json"
  $results = @()

  function Resolve-Stage {
    param([string]$CheckId)
    if ($CheckId -like "config-doctor*" -or $CheckId -like "cargo-check") { return "L0" }
    if ($CheckId -like "*no-bypass*" -or $CheckId -like "*artifact-gate*" -or $CheckId -like "*waltx*") { return "L1" }
    if ($CheckId -like "*sandbox*" -or $CheckId -like "*frontend*" -or $CheckId -like "*signal*" -or $CheckId -like "*pevo*" -or $CheckId -like "*d14*" -or $CheckId -like "*d12-storage*") { return "L2" }
    return "L2"
  }

  function Invoke-DiagnosticStep {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv,
      [int]$RetryCount = 1
    )

    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Name + "] " + $display + " ====")
    $start = Get-Date
    $errorMessage = $null
    $exitCode = 0

    $maxAttempts = [Math]::Max(1, $RetryCount + 1)
    $attempt = 0
    while ($attempt -lt $maxAttempts) {
      $attempt++
      try {
        $prev = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        $output = & $Exe @Argv 2>&1
        $exitCode = $LASTEXITCODE
        $ErrorActionPreference = $prev
        if ($null -ne $output) {
          $output | Out-File -FilePath $logPath -Append -Encoding utf8
        }
        if ($exitCode -ne 0) {
          $errorMessage = "exit_code=$exitCode"
          if ($attempt -lt $maxAttempts) {
            Add-Content -Path $logPath -Value ("retrying [" + $Name + "] attempt " + ($attempt + 1) + "/" + $maxAttempts + " after " + $errorMessage)
            Start-Sleep -Seconds 2
            continue
          }
        }
        break
      } catch {
        $exitCode = -1
        $errorMessage = $_.Exception.Message
        Add-Content -Path $logPath -Value ("EXCEPTION: " + $errorMessage)
        if ($attempt -lt $maxAttempts) {
          Add-Content -Path $logPath -Value ("retrying [" + $Name + "] attempt " + ($attempt + 1) + "/" + $maxAttempts + " after exception")
          Start-Sleep -Seconds 2
          continue
        }
        break
      }
    }

    $durationMs = [int]((Get-Date) - $start).TotalMilliseconds
    return [pscustomobject]@{
      stage = Resolve-Stage -CheckId $Name
      check_id = $Name
      passed = ($exitCode -eq 0)
      severity = if ($exitCode -eq 0) { "info" } else { "blocker" }
      deny_reason = if ($exitCode -eq 0) { $null } else { $errorMessage }
      evidence_ref = $null
      replay_fp = $null
      duration_ms = $durationMs
    }
  }

  $steps = @(
    @{ Name = "cargo-check"; Exe = "cargo"; Argv = @("check", "--workspace", "--manifest-path", $ManifestPath) },
    @{ Name = "p10-day10-acceptance"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "p10_day10_acceptance_e2e") },
    @{ Name = "pq3-closed-loop-e2e"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "pq3_compiler_executor_verifier_closed_loop_e2e") },
    @{ Name = "p10-replay-mismatch-e2e"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "p10_replay_mismatch_explainer_e2e") },
    @{ Name = "pq10-intent-query-chain-e2e"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e") },
    @{ Name = "pq10-no-bypass-gate-e2e"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "pq10_no_bypass_gate_e2e") },
    @{ Name = "pq3-permission-mode-tristate-matrix"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "pq3_permission_mode_matrix") },
    @{ Name = "no-bypass-static-scan"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "no_bypass_static_scan") },
    @{ Name = "d10-d11-security-governance"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "d10_d11_security_governance_gate_suite") },
    @{ Name = "p8-budget-ledger"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "p8_budget_ledger_sovereignty"); RetryCount = 1 },
    @{ Name = "d12-storage-postgres-wal"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "d12_storage_postgres_wal_dualwrite_replay_e2e"); RetryCount = 1 },
    @{ Name = "waltx-production-write-min"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--test", "waltx_production_write_minimal_e2e"); RetryCount = 1 },
    @{ Name = "artifact-gate-write-evidence-required"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--lib", "artifact_gate_requires_write_evidence_even_if_file_exists") },
    @{ Name = "artifact-gate-fake-success-rejected"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--lib", "artifact_gate_rejects_fake_success_when_proof_hash_mismatch") },
    @{ Name = "config-doctor-bad-config-blocked"; Exe = "cargo"; Argv = @("test", "--manifest-path", $ManifestPath, "--bins", "system_config_doctor_blocks_intentionally_bad_config") },
    @{ Name = "sandbox-acceptance"; Exe = "powershell"; Argv = @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\sandbox_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-sandbox")); RetryCount = 1 },
    @{ Name = "signal-acceptance"; Exe = "powershell"; Argv = @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\signal_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-signal")); RetryCount = 1 },
    @{ Name = "frontend-cli-acceptance"; Exe = "powershell"; Argv = @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\frontend_cli_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-frontend")); RetryCount = 1 },
    @{ Name = "pevo-acceptance"; Exe = "powershell"; Argv = @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pevo_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-pevo")); RetryCount = 1 },
    @{ Name = "version-a-acceptance"; Exe = "powershell"; Argv = @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\version_a_acceptance.ps1", "-ManifestPath", $ManifestPath); RetryCount = 1 },
    @{ Name = "d14-rollout-acceptance"; Exe = "powershell"; Argv = @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d14_rollout.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-d14")); RetryCount = 1 },
    @{ Name = "d46-slo-acceptance"; Exe = "powershell"; Argv = @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d46_slo_acceptance.ps1", "-ManifestPath", $ManifestPath); RetryCount = 1 }
  )

  foreach ($step in $steps) {
    $retryCount = 1
    if ($step.ContainsKey("RetryCount")) {
      $retryCount = [int]$step.RetryCount
    }
    $result = Invoke-DiagnosticStep -Name $step.Name -Exe $step.Exe -Argv $step.Argv -RetryCount $retryCount
    $results += $result
  }

  $failed = @($results | Where-Object { -not $_.passed })
  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    profile = "production-e2e"
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    all_passed = ($failed.Count -eq 0)
    total = $results.Count
    passed = @($results | Where-Object { $_.passed }).Count
    failed = $failed.Count
    blockers = @($failed | Select-Object -ExpandProperty check_id)
    checks = $results
    log_path = $logPath
  }
  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $canonicalJsonPath -Encoding utf8

  Write-Output ("WEEK6_DIAGNOSTIC_JSON=" + $jsonPath)
  Write-Output ("WEEK6_DIAGNOSTIC_CANONICAL_JSON=" + $canonicalJsonPath)
  Write-Output ("WEEK6_DIAGNOSTIC_LOG=" + $logPath)
} finally {
  if ($null -eq $previousAutoLoopProfile) {
    Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_PROFILE = $previousAutoLoopProfile
  }
  Pop-Location
}
