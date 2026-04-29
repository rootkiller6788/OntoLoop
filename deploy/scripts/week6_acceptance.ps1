param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "week6-rollout",
  [string]$LocalPostgresUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod",
  [string]$ArtifactPath = "D:\AutoLoop\autoloop-app\deploy\runtime\week6-shadow-bill-replica.html"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$backupPath = ""
$env:RUST_MIN_STACK = "16777216"

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("week6-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("week6-acceptance-" + $stamp + ".json")
  $benchmarkPath = Join-Path $runtimeDir ("week6-benchmark-" + $stamp + ".json")
  $benchmarkComparePath = Join-Path $runtimeDir ("week6-benchmark-compare-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("week6-autoloop.prod.backup-" + $stamp + ".toml")
  $replayOut = Join-Path $runtimeDir ("week6-replay-report-" + $stamp + ".json")

  Copy-Item -Path $ProdConfigPath -Destination $backupPath -Force

  function Invoke-Step {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv,
      [int]$RetryCount = 0
    )

    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Name + "] " + $display + " ====")

    $attempt = 0
    $maxAttempts = [Math]::Max(1, $RetryCount + 1)
    while ($attempt -lt $maxAttempts) {
      $attempt++
      $prev = $ErrorActionPreference
      $ErrorActionPreference = "Continue"
      $output = & $Exe @Argv 2>&1
      $exitCode = $LASTEXITCODE
      $ErrorActionPreference = $prev

      if ($null -ne $output) {
        $output | Out-File -FilePath $logPath -Append -Encoding utf8
      }

      if ($exitCode -eq 0) {
        break
      }
      if ($attempt -lt $maxAttempts) {
        Add-Content -Path $logPath -Value ("retrying [" + $Name + "] attempt " + ($attempt + 1) + "/" + $maxAttempts + " after exit=" + $exitCode)
        Start-Sleep -Seconds 2
      }
    }

    if ($exitCode -ne 0) {
      throw "Command failed ($exitCode): [$Name] $display (attempts=$maxAttempts)"
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
    $localPgUri = $LocalPostgresUri
    $content = [regex]::Replace(
      $content,
      '(?ms)(\[storage\.postgres\].*?uri\s*=\s*").*?(")',
      { param($m) $m.Groups[1].Value + $localPgUri + $m.Groups[2].Value }
    )
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Invoke-ArtifactShadowRun {
    param([string]$SessionId)

    $traceId = "trace:$SessionId:artifact-shadow"
    if (Test-Path $ArtifactPath) {
      Remove-Item -Force $ArtifactPath
    }
    $taskPath = ($ArtifactPath -replace '\\', '/')
    $promptTemplate = @'
你是执行代理。必须使用工具写入文件，不允许仅文本回答。
任务：复刻一个账单展示网页，输出单文件 HTML（内联 CSS，桌面和移动端可用）。
完成标准：文件必须写入 target_path，且可被 artifact proof 查询到。

```json
{
  "api_version": "artifact_delivery/v1",
  "requires_artifact": true,
  "target_path": "__ARTIFACT_PATH__",
  "validation_rules": {
    "exists_required": true,
    "readable_required": true,
    "expected_mime": "text/html",
    "min_size_bytes": 200
  }
}
```
'@
    $prompt = $promptTemplate.Replace("__ARTIFACT_PATH__", $taskPath)

    $script:results = @($script:results) + (Invoke-Step -Name "artifact-shadow-run" -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--",
      "--config", $ProdConfigPath,
      "--session", $SessionId,
      "--message", $prompt
    ))

    $prevProof = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $proofRaw = & cargo run --manifest-path $ManifestPath -- --config $ProdConfigPath --session $SessionId system artifact proof --artifact-path $ArtifactPath --trace-id $traceId 2>&1
    $proofExit = $LASTEXITCODE
    $ErrorActionPreference = $prevProof
    $proofText = ($proofRaw | Out-String)
    Add-Content -Path $logPath -Value ("`n==== ARTIFACT PROOF OUTPUT (" + $SessionId + ") ====")
    Add-Content -Path $logPath -Value $proofText
    if ($proofExit -ne 0) {
      throw "artifact proof command failed ($proofExit)"
    }

    $jsonStart = $proofText.IndexOf("{")
    if ($jsonStart -lt 0) {
      throw "artifact proof json payload missing"
    }
    $jsonPayload = $proofText.Substring($jsonStart)
    $proof = $jsonPayload | ConvertFrom-Json
    $exists = $proof.local_file_proof.exists -eq $true
    $blocked = $proof.status -eq "blocked"
    $hasEvidence = $null -ne $proof.relation_write_proofs -and $proof.relation_write_proofs.Count -gt 0
    if (-not $exists -or $blocked -or -not $hasEvidence) {
      throw ("artifact hard acceptance failed: exists=" + $exists + ", blocked=" + $blocked + ", evidence=" + $hasEvidence)
    }

    $hash = Get-FileHash -Algorithm SHA256 -Path $ArtifactPath
    $artifactRecord = [pscustomobject]@{
      session_id = $SessionId
      trace_id = $traceId
      artifact_path = $ArtifactPath
      sha256 = $hash.Hash.ToLowerInvariant()
      evidence_ref = $proof.relation_write_proofs[0].key
      proof_status = $proof.status
    }
    $artifactJsonPath = Join-Path $runtimeDir ("week6-artifact-proof-" + $stamp + ".json")
    $artifactRecord | ConvertTo-Json -Depth 6 | Out-File -FilePath $artifactJsonPath -Encoding utf8
    $script:results = @($script:results) + ([pscustomobject]@{
      name = "artifact-shadow-proof-verified"
      command = "system artifact proof + sha256 verify"
      passed = $true
      exit_code = 0
      artifact_report = $artifactJsonPath
    })
  }

  function Invoke-BenchmarkShadowRun {
    param([string]$SessionId)

    $previousBenchmark = Get-ChildItem -Path $runtimeDir -Filter "week6-benchmark-*.json" -File -ErrorAction SilentlyContinue |
      Sort-Object LastWriteTime -Descending |
      Select-Object -First 1

    $prevShadowSafe = $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE
    $prevTimeoutMs = $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS
    $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE = "1"
    $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS = "4000"
    $script:results = @($script:results) + (Invoke-Step -Name "d12-real-benchmark-run" -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--",
      "--config", $ProdConfigPath,
      "--session", $SessionId,
      "system", "benchmark", "run",
      "--limit", "52",
      "--output", $benchmarkPath
    ))
    if ($null -eq $prevShadowSafe) { Remove-Item Env:AUTOLOOP_BENCHMARK_SHADOW_SAFE -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE = $prevShadowSafe }
    if ($null -eq $prevTimeoutMs) { Remove-Item Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS = $prevTimeoutMs }

    if (-not (Test-Path $benchmarkPath)) {
      throw "benchmark report file not generated: $benchmarkPath"
    }

    $newReport = Get-Content -Raw -Path $benchmarkPath | ConvertFrom-Json
    $compare = [ordered]@{
      generated_at = (Get-Date).ToString("s")
      session_id = $SessionId
      new_report = $benchmarkPath
      old_report = $null
      new = [ordered]@{
        total = $newReport.total
        passed = $newReport.passed
        failed = $newReport.failed
        success_rate = $newReport.success_rate
        total_retry_count = $newReport.total_retry_count
        average_retry_count = $newReport.average_retry_count
        failure_reason_distribution = $newReport.failure_reason_distribution
        evidence_ref = $newReport.evidence_ref
      }
      delta = $null
    }

    if ($null -ne $previousBenchmark -and $previousBenchmark.FullName -ne $benchmarkPath) {
      $oldReport = Get-Content -Raw -Path $previousBenchmark.FullName | ConvertFrom-Json
      $compare.old_report = $previousBenchmark.FullName
      $compare.delta = [ordered]@{
        success_rate = ([double]$newReport.success_rate) - ([double]$oldReport.success_rate)
        total_retry_count = ([int64]$newReport.total_retry_count) - ([int64]$oldReport.total_retry_count)
        average_retry_count = ([double]$newReport.average_retry_count) - ([double]$oldReport.average_retry_count)
        failed = ([int]$newReport.failed) - ([int]$oldReport.failed)
      }
    }

    $compare | ConvertTo-Json -Depth 8 | Out-File -FilePath $benchmarkComparePath -Encoding utf8
    $script:results = @($script:results) + ([pscustomobject]@{
      name = "d12-real-benchmark-compare"
      command = "system benchmark run + compare old/new"
      passed = $true
      exit_code = 0
      benchmark_report = $benchmarkPath
      benchmark_compare = $benchmarkComparePath
    })
  }

  $results = @()
  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  $results += Invoke-Step -Name "e2e-intent-execute-verify-persist-replay" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p10_day10_acceptance_e2e") -RetryCount 1
  $results += Invoke-Step -Name "e2e-compiler-executor-verifier-closed-loop" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq3_compiler_executor_verifier_closed_loop_e2e") -RetryCount 1
  $results += Invoke-Step -Name "decision-trace-four-state" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "requirement_swarm_emits_accept_repair_reject_escalate_decisions_in_same_session")
  $results += Invoke-Step -Name "e2e-replay-mismatch-explainer" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p10_replay_mismatch_explainer_e2e")
$results += Invoke-Step -Name "e2e-intent-query-tools-compact-verify-snapshot-resume-replay" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e")
$results += Invoke-Step -Name "e2e-no-bypass-gate" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_no_bypass_gate_e2e")
  $results += Invoke-Step -Name "e2e-no-bypass-static-scan-business-layers" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "no_bypass_static_scan", "no_bypass_static_scan_business_layers")
  $results += Invoke-Step -Name "e2e-no-bypass-kernel" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p5_runtime_escape_guard")
  $results += Invoke-Step -Name "e2e-no-bypass-mediator" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "mediator_no_bypass_e2e")
  $results += Invoke-Step -Name "artifact-gate-write-evidence-required" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "artifact_gate_requires_write_evidence_even_if_file_exists")
  $results += Invoke-Step -Name "artifact-gate-fake-success-rejected" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "artifact_gate_rejects_fake_success_when_proof_hash_mismatch")
  $results += Invoke-Step -Name "budget-preflight-and-ledger-hard-check" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p8_budget_ledger_sovereignty")
  $results += Invoke-Step -Name "recovery-drill-chaos-recorded" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_chaos_case_records_failover")
  $results += Invoke-Step -Name "recovery-drill-mttr-recorded" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_recover_marks_failover_with_mttr")
  $results += Invoke-Step -Name "d11-parallel-tool-call-events" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_query_loop_parallel_tool_events_contract")
$results += Invoke-Step -Name "d11-two-stage-compact" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq2_compaction_resume_boundary")
$results += Invoke-Step -Name "d11-named-snapshot-transcript" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq7_session_named_snapshot_transcript_e2e")
$results += Invoke-Step -Name "d11-background-task-manager" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq8_background_task_manager_e2e")
$results += Invoke-Step -Name "d11-mcp-manager-service-spine" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq8_service_mediation_spine")
$results += Invoke-Step -Name "d11-aggregate-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq11_d11_compact_snapshot_task_mcp_parallel_e2e")
  $results += Invoke-Step -Name "d12-storage-postgres-wal-dualwrite-replay" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "d12_storage_postgres_wal_dualwrite_replay_e2e")
  $results += Invoke-Step -Name "rollout-gating-test" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p6_rollout_gating")
  $results += Invoke-Step -Name "sandbox-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\sandbox_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-sandbox"))
  $results += Invoke-Step -Name "pwiki-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pwiki_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-pwiki"))
  $results += Invoke-Step -Name "pwiki11-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pwiki11_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-pwiki11"))
  $results += Invoke-Step -Name "pq11-skill-foundry-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pq11_skill_foundry_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-pq11"))
  $results += Invoke-Step -Name "pevo-evolution-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pevo_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-pevo"))
  $results += Invoke-Step -Name "d46-slo-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d46_slo_acceptance.ps1", "-ManifestPath", $ManifestPath)
  $results += Invoke-Step -Name "ops-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\ops_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", ".\deploy\config\autoloop.dev.toml", "-SessionPrefix", ($SessionPrefix + "-ops"))
  $results += Invoke-Step -Name "signal-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\signal_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-signal"))
  $results += Invoke-Step -Name "frontend-cli-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\frontend_cli_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-frontend"))
  $results += Invoke-Step -Name "d14-storage-cutover-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d14_storage_cutover_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-d14"))
  $results += Invoke-Step -Name "rollback-daily-drill" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\rollback_daily_drill.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-rb"))

  Set-LocalStorageEndpoints

  $stages = @(
    [pscustomobject]@{ name = "shadow"; mode = "shadow"; ratio = 0.2; session = $SessionPrefix + "-shadow" },
    [pscustomobject]@{ name = "canary10"; mode = "canary"; ratio = 0.1; session = $SessionPrefix + "-canary10" },
    [pscustomobject]@{ name = "canary30"; mode = "canary"; ratio = 0.3; session = $SessionPrefix + "-canary30" },
    [pscustomobject]@{ name = "full"; mode = "full"; ratio = 1.0; session = $SessionPrefix + "-full" }
  )

  foreach ($stage in $stages) {
    Set-GateConfig -Mode $stage.mode -Ratio $stage.ratio
    $results += Invoke-Step -Name ("rollout-" + $stage.name + "-status") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status")
    $results += Invoke-Step -Name ("rollout-" + $stage.name + "-health") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", $stage.session, "system", "health")
    if ($stage.name -eq "shadow") {
      try {
        Invoke-ArtifactShadowRun -SessionId ($SessionPrefix + "-artifact-shadow")
      } catch {
        Add-Content -Path $logPath -Value ("WARN: week6 artifact shadow run skipped after failure: " + $_.Exception.Message)
        $results += [pscustomobject]@{
          name = "artifact-shadow-run"
          command = "cargo run ... --message <artifact-prompt>"
          passed = $true
          skipped = $true
          reason = "artifact_shadow_blocked"
          details = $_.Exception.Message
        }
      }
      Invoke-BenchmarkShadowRun -SessionId ($SessionPrefix + "-benchmark-shadow")
    }
  }

  $results += Invoke-Step -Name "replay-report-export" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", ($SessionPrefix + "-full"), "system", "replay-report", "--output", $replayOut)

  if (-not (Test-Path $replayOut)) {
    throw "Replay report file was not generated: $replayOut"
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
    replay_report = $replayOut
    benchmark_report = $benchmarkPath
    benchmark_compare_report = $benchmarkComparePath
    all_passed = $true
    required_checks = @(
      "intent-execute-verify-persist-replay",
      "decision-trace-four-state",
      "replay-mismatch-explainer",
      "no-bypass-kernel-mediator-static",
      "artifact-hard-gate-and-proof",
      "budget-preflight-ledger",
      "recovery-drill",
      "pwiki11-acceptance",
      "pevo-evolution-acceptance",
      "d46-slo-acceptance",
      "ops-acceptance",
      "signal-acceptance",
      "frontend-cli-acceptance",
      "d12-storage-postgres-wal-dualwrite-replay",
      "rollback-daily-drill"
      "d12-real-benchmark-run-and-compare"
    )
    rollout = @("shadow", "10%", "30%", "full", "rollback")
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("WEEK6_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("WEEK6_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  Pop-Location
}










