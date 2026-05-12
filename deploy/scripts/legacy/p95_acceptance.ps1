param(
  [string]$ManifestPath = "D:\AutoLoop\autoloop-app\Cargo.toml"
)

$ErrorActionPreference = "Stop"
$logDir = "D:\AutoLoop\autoloop-app\deploy\runtime"
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir | Out-Null }
$logPath = Join-Path $logDir "p95-acceptance.log"
$jsonPath = Join-Path $logDir "p95-acceptance.json"
if (Test-Path $logPath) { Remove-Item $logPath -Force }

$commands = @(
  "cargo check --manifest-path $ManifestPath",
  "cargo test --manifest-path $ManifestPath --test p7_trigger_wake_plan_execute_e2e",
  "cargo test --manifest-path $ManifestPath --test p7_capability_admission_reject_e2e",
  "cargo test --manifest-path $ManifestPath --test p10_evidence_six_segments_e2e",
  "cargo test --manifest-path $ManifestPath --test p10_replay_mismatch_explainer_e2e",
  "cargo test --manifest-path $ManifestPath --lib runtime::trust_bridge::tests::attestation_ttl_expired_is_rejected",
  "cargo test --manifest-path $ManifestPath --test p12_promotion_rollback_memory_guard_e2e",
  "cargo test --manifest-path $ManifestPath --test permission_mode_runtime_enforced_e2e",
  "cargo test --manifest-path $ManifestPath --test layered_tool_stack_pipeline_e2e",
  "cargo test --manifest-path $ManifestPath --test session_continuation_resume_e2e",
  "cargo test --manifest-path $ManifestPath --test skill_plugin_router_integration_e2e",
  "cargo test --manifest-path $ManifestPath --test mediator_no_bypass_e2e"
)

$results = @()
foreach ($cmd in $commands) {
  Add-Content -Path $logPath -Value ("`n==== RUN: " + $cmd + " ====")
  $prev = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $output = & cmd /c $cmd 2>&1
  $ErrorActionPreference = $prev
  $output | Out-File -FilePath $logPath -Append -Encoding utf8

  if ($LASTEXITCODE -ne 0) {
    $results += [pscustomobject]@{ command = $cmd; passed = $false; exit_code = $LASTEXITCODE }
    throw "Command failed: $cmd"
  }

  $results += [pscustomobject]@{ command = $cmd; passed = $true; exit_code = 0 }
}

$summary = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  manifest = $ManifestPath
  all_passed = $true
  commands = $results
  log_path = $logPath
}
$summary | ConvertTo-Json -Depth 6 | Out-File -FilePath $jsonPath -Encoding utf8
Write-Output ("ACCEPTANCE_OK log=" + $logPath)
Write-Output ("ACCEPTANCE_JSON=" + $jsonPath)
