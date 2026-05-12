param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "week6-rollout",
  [string]$LocalPostgresUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod",
  [string]$ArtifactPath = "D:\AutoLoop\autoloop-app\deploy\runtime\week6-shadow-bill-replica.html",
  [switch]$RunDailyFullBenchmark,
  [int]$BenchmarkSmokeLimit = 12,
  [int]$BenchmarkFullLimit = 52,
  [switch]$RunSoakStability,
  [int]$SoakDurationHours = 6,
  [switch]$ForceWeeklyFullDrill,
  [string]$ChangedFilesPath = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$backupPath = ""
$env:RUST_MIN_STACK = "16777216"
$previousCargoTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
$previousAutoLoopProfile = if (Test-Path Env:AUTOLOOP_PROFILE) { $env:AUTOLOOP_PROFILE } else { $null }
$env:AUTOLOOP_PROFILE = "production-e2e"
$runtimeDir = $null
$jsonPath = $null
$canonicalJsonPath = $null
$logPath = $null
$targetDir = $null
$scriptSucceeded = $false
$d13SmokeReportPath = $null
$d13FullReportPath = $null
$d13FullEnabled = $false
$faultInjectionReportPath = $null
$d14RolloutReportPath = $null
$versionAReportPath = $null
$d46ReportPath = $null
$soakStabilityReportPath = $null
$rollbackDrillReportPath = $null
$faultDrillMode = "light"
$rollbackDrillMode = "light"
$results = @()
$lastStepName = $null
$impactSelectorPath = $null
$impactSelectedChecks = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
$impactSelection = $null
$impactTestsHash = ""

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $isWeeklyDrillDay = (Get-Date).DayOfWeek -eq [System.DayOfWeek]::Sunday
  if ($ForceWeeklyFullDrill -or $isWeeklyDrillDay) {
    $rollbackDrillMode = "full"
    $faultDrillMode = "full"
  }
  $logPath = Join-Path $runtimeDir ("week6-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("week6-acceptance-" + $stamp + ".json")
  $canonicalJsonPath = Join-Path $runtimeDir "week6_full_acceptance.json"
  $benchmarkPath = Join-Path $runtimeDir ("week6-benchmark-" + $stamp + ".json")
  $benchmarkComparePath = Join-Path $runtimeDir ("week6-benchmark-compare-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("week6-autoloop.prod.backup-" + $stamp + ".toml")
  $replayOut = Join-Path $runtimeDir ("week6-replay-report-" + $stamp + ".json")
  $targetDir = Join-Path $runtimeDir ("target-week6-full-" + $stamp)
  New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
  $env:CARGO_TARGET_DIR = $targetDir

  Copy-Item -Path $ProdConfigPath -Destination $backupPath -Force

  function Invoke-Step {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv,
      [int]$RetryCount = 0
    )

    $display = "$Exe $($Argv -join ' ')"
    $script:lastStepName = $Name
    if ($script:impactSelectedChecks.Count -gt 0 -and -not $script:impactSelectedChecks.Contains($Name)) {
      Add-Content -Path $logPath -Value ("`n==== SKIP: [" + $Name + "] not in impacted scope ====")
      return [pscustomobject]@{
        name = $Name
        command = $display
        passed = $true
        skipped = $true
        skip_reason = "not_impacted"
        exit_code = 0
      }
    }
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
      $combinedOutput = if ($null -eq $output) { "" } else { ($output | Out-String) }
      $isPermissionFailure = $combinedOutput -match "(?i)(permission|approval required|requires approval|blocked by policy|denied)"
      $isTimeoutFailure = $combinedOutput -match "(?i)(timeout|timed out|deadline exceeded|operation timed out)"
      $isTransientBuildLockFailure = $combinedOutput -match "(?i)(failed to remove file .*ontoloop\.exe|拒绝访问|access is denied|os error 5|blocking waiting for file lock)"
      $allowRetry = (-not $isPermissionFailure) -and ($isTimeoutFailure -or $isTransientBuildLockFailure)
      if ($attempt -lt $maxAttempts -and $allowRetry) {
        $retryReason = if ($isTimeoutFailure) { "timeout" } else { "transient_build_lock" }
        Add-Content -Path $logPath -Value ("retrying [" + $Name + "] attempt " + ($attempt + 1) + "/" + $maxAttempts + " after exit=" + $exitCode + " (reason=" + $retryReason + ")")
        Start-Sleep -Seconds 2
      } else {
        if ($attempt -lt $maxAttempts -and -not $allowRetry) {
          Add-Content -Path $logPath -Value ("retry skipped for [" + $Name + "] exit=" + $exitCode + " (reason=non-timeout-or-permission)")
        }
        break
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

  function Initialize-ImpactSelection {
    $selectorOut = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\impact_test_selector.ps1" -RepoRoot $repoRoot -ChangedFilesPath $ChangedFilesPath
    if ($LASTEXITCODE -ne 0) {
      throw "impact_test_selector failed"
    }
    $selectorLine = @($selectorOut | Where-Object { $_ -like "IMPACT_SELECTOR_JSON=*" }) | Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace([string]$selectorLine)) {
      throw "impact selector report path missing"
    }
    $script:impactSelectorPath = ([string]$selectorLine).Substring("IMPACT_SELECTOR_JSON=".Length)
    if (-not (Test-Path $script:impactSelectorPath)) {
      throw "impact selector report missing: $script:impactSelectorPath"
    }
    $script:impactSelection = Get-Content -Raw -Path $script:impactSelectorPath | ConvertFrom-Json
    $script:impactTestsHash = [string]$script:impactSelection.impacted_tests_hash
    foreach ($id in @($script:impactSelection.impacted_checks)) {
      if (-not [string]::IsNullOrWhiteSpace([string]$id)) {
        [void]$script:impactSelectedChecks.Add([string]$id)
      }
    }
    foreach ($rid in @("replay-report-export", "rollback-status", "rollback-health")) {
      [void]$script:impactSelectedChecks.Add($rid)
    }
    if ($script:impactSelectedChecks.Contains("__RUN_ROLLOUT__")) {
      foreach ($sid in @(
        "rollout-shadow-status","rollout-shadow-health",
        "rollout-canary10-status","rollout-canary10-health",
        "rollout-canary30-status","rollout-canary30-health",
        "rollout-full-status","rollout-full-health"
      )) {
        [void]$script:impactSelectedChecks.Add($sid)
      }
    }
    $script:results = @($script:results) + ([pscustomobject]@{
      name = "impact-test-selector"
      command = "impact_test_selector.ps1"
      passed = $true
      exit_code = 0
      report = $script:impactSelectorPath
    })
  }

  function Invoke-ConfigDoctorGate {
    param(
      [string]$SessionId,
      [string]$Profile = "production-e2e"
    )
    $out = Join-Path $runtimeDir ("week6-config-doctor-" + $stamp + ".json")
    $step = Invoke-Step -Name "pre-rollout-config-doctor-gate" -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--",
      "--config", $ProdConfigPath,
      "--session", $SessionId,
      "system", "config", "doctor",
      "--profile", $Profile,
      "--output", $out
    )
    if (-not (Test-Path $out)) {
      throw "config doctor output missing: $out"
    }
    $doctor = Get-Content -Raw -Path $out | ConvertFrom-Json
    if ($doctor.status -ne "pass") {
      throw "config doctor hard gate failed: status=$($doctor.status)"
    }

    $requiredIds = @(
      "profile.alignment",
      "runtime.gate_mode",
      "runtime.rollback_window",
      "storage.postgres.enabled_uri",
      "storage.backend_consistency"
    )
    foreach ($id in $requiredIds) {
      $check = $doctor.checks | Where-Object { $_.id -eq $id } | Select-Object -First 1
      if ($null -eq $check) {
        throw "config doctor hard gate missing required check: $id"
      }
      if (-not $check.passed) {
        throw "config doctor hard gate check failed: $id => $($check.message)"
      }
    }

    return [pscustomobject]@{
      step = $step
      output = $out
      status = $doctor.status
    }
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
请把最终文件写到这个绝对路径：__ARTIFACT_PATH__
必须保证文件可读，且内容长度不少于 200 字节。
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
    $hasWriteProof = $null -ne $proof.relation_write_proofs -and $proof.relation_write_proofs.Count -gt 0
    $latestWriteProof = $null
    if ($hasWriteProof) {
      $latestWriteProof = $proof.relation_write_proofs[0]
    }
    $writeProofValue = $latestWriteProof.value
    $proofHash = if ($null -ne $writeProofValue.sha256 -and -not [string]::IsNullOrWhiteSpace([string]$writeProofValue.sha256)) {
      [string]$writeProofValue.sha256
    } elseif ($null -ne $writeProofValue.state_hash -and -not [string]::IsNullOrWhiteSpace([string]$writeProofValue.state_hash)) {
      [string]$writeProofValue.state_hash
    } else {
      ""
    }
    $proofEvidenceRef = if ($null -ne $writeProofValue.evidence_ref) {
      [string]$writeProofValue.evidence_ref
    } else {
      ""
    }
    $hasHash = -not [string]::IsNullOrWhiteSpace($proofHash)
    $hasEvidenceRef = -not [string]::IsNullOrWhiteSpace($proofEvidenceRef)
    if (-not $exists -or $blocked -or -not $hasWriteProof -or -not $hasHash -or -not $hasEvidenceRef) {
      throw ("artifact hard acceptance failed: exists=" + $exists + ", blocked=" + $blocked + ", has_write_proof=" + $hasWriteProof + ", has_hash=" + $hasHash + ", has_evidence_ref=" + $hasEvidenceRef)
    }

    $hash = Get-FileHash -Algorithm SHA256 -Path $ArtifactPath
    $artifactRecord = [pscustomobject]@{
      session_id = $SessionId
      trace_id = $traceId
      artifact_path = $ArtifactPath
      sha256 = $hash.Hash.ToLowerInvariant()
      write_proof_ref = if ($null -ne $latestWriteProof.key) { [string]$latestWriteProof.key } else { "" }
      write_proof_hash = $proofHash
      evidence_ref = $proofEvidenceRef
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

  function Invoke-D13BenchmarkCadence {
    param([string]$SessionId)

    function Test-D13ThresholdPass {
      param(
        [string]$ReportPath,
        [string]$Label
      )
      if (-not (Test-Path $ReportPath)) {
        throw "$Label benchmark report missing: $ReportPath"
      }
      $report = Get-Content -Raw -Path $ReportPath | ConvertFrom-Json
      $thresholdPass = $report.threshold_pass -eq $true
      if (-not $thresholdPass) {
        $successRate = [double]$report.success_rate_percent
        $complianceRate = [double]$report.compliance_rate_percent
        $minSuccess = [double]$report.thresholds.minimum_success_rate_percent
        $minCompliance = [double]$report.thresholds.minimum_compliance_rate_percent
        throw "$Label benchmark threshold failed: success=${successRate}% (<${minSuccess}%) or compliance=${complianceRate}% (<${minCompliance}%). rollout blocked."
      }
    }

    $smokeRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\d13_realbiz_benchmark_acceptance.ps1" `
      -ManifestPath $ManifestPath `
      -ProdConfigPath $ProdConfigPath `
      -SessionPrefix ($SessionId + "-smoke") `
      -Limit $BenchmarkSmokeLimit `
      -CaseTimeoutMs 60000
    if ($null -ne $smokeRaw) {
      $smokeRaw | Out-File -FilePath $logPath -Append -Encoding utf8
    }
    if ($LASTEXITCODE -ne 0) { throw "d13 smoke benchmark failed" }
    $smokeLine = @($smokeRaw | Where-Object { $_ -like "D13_REALBIZ_BENCHMARK_JSON=*" }) | Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace([string]$smokeLine)) { throw "d13 smoke benchmark report path missing" }
    $script:d13SmokeReportPath = ([string]$smokeLine).Substring("D13_REALBIZ_BENCHMARK_JSON=".Length)
    Test-D13ThresholdPass -ReportPath $script:d13SmokeReportPath -Label "d13-smoke"
    $script:results = @($script:results) + ([pscustomobject]@{
      name = "d13-smoke-benchmark"
      command = "d13_realbiz_benchmark_acceptance.ps1 --limit $BenchmarkSmokeLimit"
      passed = $true
      exit_code = 0
      report = $script:d13SmokeReportPath
    })

    if ($RunDailyFullBenchmark) {
      $script:d13FullEnabled = $true
      $fullRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\d13_realbiz_benchmark_acceptance.ps1" `
        -ManifestPath $ManifestPath `
        -ProdConfigPath $ProdConfigPath `
        -SessionPrefix ($SessionId + "-full") `
        -Limit $BenchmarkFullLimit `
        -CaseTimeoutMs 60000
      if ($null -ne $fullRaw) {
        $fullRaw | Out-File -FilePath $logPath -Append -Encoding utf8
      }
      if ($LASTEXITCODE -ne 0) { throw "d13 full benchmark failed" }
      $fullLine = @($fullRaw | Where-Object { $_ -like "D13_REALBIZ_BENCHMARK_JSON=*" }) | Select-Object -Last 1
      if ([string]::IsNullOrWhiteSpace([string]$fullLine)) { throw "d13 full benchmark report path missing" }
      $script:d13FullReportPath = ([string]$fullLine).Substring("D13_REALBIZ_BENCHMARK_JSON=".Length)
      Test-D13ThresholdPass -ReportPath $script:d13FullReportPath -Label "d13-full"
      $script:results = @($script:results) + ([pscustomobject]@{
        name = "d13-full-benchmark"
        command = "d13_realbiz_benchmark_acceptance.ps1 --limit $BenchmarkFullLimit"
        passed = $true
        exit_code = 0
        report = $script:d13FullReportPath
      })
    } else {
      $script:d13FullEnabled = $false
      $script:d13FullReportPath = $null
      $script:results = @($script:results) + ([pscustomobject]@{
        name = "d13-full-benchmark"
        command = "d13_realbiz_benchmark_acceptance.ps1 --limit $BenchmarkFullLimit"
        passed = $true
        skipped = $true
        reason = "RunDailyFullBenchmark disabled (smoke default)"
      })
    }
  }

  $results = @()
  Initialize-ImpactSelection
  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  $results += Invoke-Step -Name "e2e-intent-execute-verify-persist-replay" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p10_day10_acceptance_e2e") -RetryCount 1
  $results += Invoke-Step -Name "e2e-compiler-executor-verifier-closed-loop" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq3_compiler_executor_verifier_closed_loop_e2e") -RetryCount 1
  $results += Invoke-Step -Name "decision-trace-four-state" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "requirement_swarm_emits_accept_repair_reject_escalate_decisions_in_same_session")
  $results += Invoke-Step -Name "e2e-replay-mismatch-explainer" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p10_replay_mismatch_explainer_e2e")
$results += Invoke-Step -Name "e2e-intent-query-tools-compact-verify-snapshot-resume-replay" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e")
$results += Invoke-Step -Name "e2e-no-bypass-gate" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_no_bypass_gate_e2e")
  $results += Invoke-Step -Name "admission-tristate-matrix" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq3_permission_mode_matrix")
  $results += Invoke-Step -Name "e2e-no-bypass-static-scan-all-domains" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "no_bypass_static_scan")
  $results += Invoke-Step -Name "d10-d11-security-governance-gate-suite" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "d10_d11_security_governance_gate_suite")
  $results += Invoke-Step -Name "e2e-no-bypass-kernel" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p5_runtime_escape_guard")
  $results += Invoke-Step -Name "e2e-no-bypass-mediator" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "mediator_no_bypass_e2e")
  $results += Invoke-Step -Name "artifact-gate-write-evidence-required" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "artifact_gate_requires_write_evidence_even_if_file_exists")
  $results += Invoke-Step -Name "artifact-gate-fake-success-rejected" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "artifact_gate_rejects_fake_success_when_proof_hash_mismatch")
  $results += Invoke-Step -Name "budget-preflight-and-ledger-hard-check" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p8_budget_ledger_sovereignty")
  $results += Invoke-Step -Name "recovery-drill-chaos-recorded" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_chaos_case_records_failover")
  $results += Invoke-Step -Name "recovery-drill-mttr-recorded" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_recover_marks_failover_with_mttr")
$results += Invoke-Step -Name "d11-parallel-tool-call-events" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_query_loop_parallel_tool_events_contract") -RetryCount 1
$results += Invoke-Step -Name "d11-two-stage-compact" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq2_compaction_resume_boundary")
$results += Invoke-Step -Name "d11-named-snapshot-transcript" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq7_session_named_snapshot_transcript_e2e")
$results += Invoke-Step -Name "d11-background-task-manager" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq8_background_task_manager_e2e")
$results += Invoke-Step -Name "d11-mcp-manager-service-spine" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq8_service_mediation_spine")
$results += Invoke-Step -Name "d11-aggregate-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq11_d11_compact_snapshot_task_mcp_parallel_e2e") -RetryCount 1
  $results += Invoke-Step -Name "d12-storage-postgres-wal-dualwrite-replay" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "d12_storage_postgres_wal_dualwrite_replay_e2e")
  $results += Invoke-Step -Name "waltx-production-write-minimal-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "waltx_production_write_minimal_e2e")
  $results += Invoke-Step -Name "config-doctor-bad-config-blocked-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--bins", "system_config_doctor_blocks_intentionally_bad_config")
  $results += Invoke-Step -Name "rollout-gating-test" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p6_rollout_gating")
  $results += Invoke-Step -Name "sandbox-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\sandbox_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-sandbox")) -RetryCount 1
  $results += Invoke-Step -Name "pwiki-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pwiki_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-pwiki")) -RetryCount 1
  $results += Invoke-Step -Name "pwiki11-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pwiki11_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-pwiki11")) -RetryCount 1
  $results += Invoke-Step -Name "pq11-skill-foundry-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pq11_skill_foundry_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-pq11")) -RetryCount 1
  $results += Invoke-Step -Name "pevo-evolution-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\pevo_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-pevo")) -RetryCount 1
  $results += Invoke-Step -Name "d46-slo-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d46_slo_acceptance.ps1", "-ManifestPath", $ManifestPath)
  $latestD46 = Get-ChildItem -Path $runtimeDir -Filter "d46-slo-acceptance-*.json" -File -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
  if ($null -eq $latestD46) { throw "d46 report path missing" }
  $d46ReportPath = $latestD46.FullName
  if ($RunSoakStability) {
    $soakRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\soak_stability_acceptance.ps1" `
      -ManifestPath $ManifestPath `
      -ProdConfigPath $ProdConfigPath `
      -SessionPrefix ($SessionPrefix + "-soak") `
      -DurationHours $SoakDurationHours
    if ($LASTEXITCODE -ne 0) { throw "soak stability acceptance failed" }
    $soakLine = @($soakRaw | Where-Object { $_ -like "SOAK_STABILITY_JSON=*" }) | Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace([string]$soakLine)) { throw "soak stability report path missing" }
    $soakStabilityReportPath = ([string]$soakLine).Substring("SOAK_STABILITY_JSON=".Length)
    $results += [pscustomobject]@{
      name = "soak-stability-acceptance"
      command = "soak_stability_acceptance.ps1"
      passed = $true
      exit_code = 0
      report = $soakStabilityReportPath
    }
  }
  $results += Invoke-Step -Name "ops-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\ops_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-ops"))
  $results += Invoke-Step -Name "d12-ops-productized-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d12_ops_productized_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-d12ops"))
  $results += Invoke-Step -Name "signal-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\signal_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-signal"))
  $results += Invoke-Step -Name "frontend-cli-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\frontend_cli_acceptance.ps1", "-ManifestPath", $ManifestPath, "-SessionPrefix", ($SessionPrefix + "-frontend"))
  $results += Invoke-Step -Name "d14-storage-cutover-acceptance" -Exe "powershell" -Argv @("-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d14_storage_cutover_acceptance.ps1", "-ManifestPath", $ManifestPath, "-ProdConfigPath", $ProdConfigPath, "-SessionPrefix", ($SessionPrefix + "-d14"))
  $rbRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\rollback_daily_drill.ps1" `
    -ManifestPath $ManifestPath `
    -ProdConfigPath $ProdConfigPath `
    -SessionPrefix ($SessionPrefix + "-rb") `
    -DrillMode $rollbackDrillMode
  if ($LASTEXITCODE -ne 0) { throw "rollback daily drill failed" }
  $rbLine = @($rbRaw | Where-Object { $_ -like "ROLLBACK_DAILY_DRILL_JSON=*" }) | Select-Object -Last 1
  if ([string]::IsNullOrWhiteSpace([string]$rbLine)) { throw "rollback drill report path missing" }
  $rollbackDrillReportPath = ([string]$rbLine).Substring("ROLLBACK_DAILY_DRILL_JSON=".Length)
  $results += [pscustomobject]@{
    name = "rollback-daily-drill"
    command = "rollback_daily_drill.ps1 -DrillMode $rollbackDrillMode"
    passed = $true
    exit_code = 0
    report = $rollbackDrillReportPath
  }
  $faultRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\fault_injection_daily_drill.ps1" `
    -ManifestPath $ManifestPath `
    -ProdConfigPath $ProdConfigPath `
    -SessionPrefix ($SessionPrefix + "-fault") `
    -DrillMode $faultDrillMode
  if ($LASTEXITCODE -ne 0) { throw "fault injection daily drill failed" }
  $faultLine = @($faultRaw | Where-Object { $_ -like "FAULT_INJECTION_DAILY_JSON=*" }) | Select-Object -Last 1
  if ([string]::IsNullOrWhiteSpace([string]$faultLine)) { throw "fault injection report path missing" }
  $faultInjectionReportPath = ([string]$faultLine).Substring("FAULT_INJECTION_DAILY_JSON=".Length)
  $results += [pscustomobject]@{
    name = "fault-injection-daily-drill"
    command = "fault_injection_daily_drill.ps1"
    passed = $true
    exit_code = 0
    report = $faultInjectionReportPath
  }

  Set-LocalStorageEndpoints
  $doctorGate = Invoke-ConfigDoctorGate -SessionId ($SessionPrefix + "-config-doctor") -Profile "production-e2e"
  $results += [pscustomobject]@{
    name = "pre-rollout-config-doctor-gate"
    command = "system config doctor --profile production-e2e --output"
    passed = $true
    exit_code = 0
    report = $doctorGate.output
    status = $doctorGate.status
  }

  # L0-L2 gate: full-chain (L3) can only start when required preconditions passed.
  $l0l2RequiredChecks = @(
    "pre-rollout-config-doctor-gate",         # L0
    "e2e-no-bypass-static-scan-all-domains",  # L1
    "artifact-gate-write-evidence-required",  # L1
    "d12-storage-postgres-wal-dualwrite-replay", # L1/L2 bridge
    "sandbox-acceptance",                     # L2
    "signal-acceptance",                      # L2
    "frontend-cli-acceptance",                # L2
    "pevo-evolution-acceptance"               # L2
  )
  foreach ($requiredName in $l0l2RequiredChecks) {
    $matched = $results | Where-Object { $_.name -eq $requiredName } | Select-Object -First 1
    if ($null -eq $matched) {
      throw "L0-L2 gate failed: required check missing before L3: $requiredName"
    }
    if (-not $matched.passed) {
      throw "L0-L2 gate failed: required check not passed before L3: $requiredName"
    }
  }

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
      Invoke-ArtifactShadowRun -SessionId ($SessionPrefix + "-artifact-shadow")
      Invoke-BenchmarkShadowRun -SessionId ($SessionPrefix + "-benchmark-shadow")
      Invoke-D13BenchmarkCadence -SessionId ($SessionPrefix + "-d13")
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

  $versionARaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\version_a_acceptance.ps1" -ManifestPath $ManifestPath
  if ($LASTEXITCODE -ne 0) { throw "version_a_acceptance failed" }
  $versionALine = @($versionARaw | Where-Object { $_ -like "VERSION_A_ACCEPTANCE_JSON=*" }) | Select-Object -Last 1
  if ([string]::IsNullOrWhiteSpace([string]$versionALine)) { throw "version-a report path missing" }
  $versionAReportPath = ([string]$versionALine).Substring("VERSION_A_ACCEPTANCE_JSON=".Length)
  $results += [pscustomobject]@{
    name = "version-a-acceptance"
    command = "version_a_acceptance.ps1"
    passed = $true
    exit_code = 0
    report = $versionAReportPath
  }

  $d14Raw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\d14_rollout.ps1" -ManifestPath $ManifestPath -ProdConfigPath $ProdConfigPath -SessionPrefix ($SessionPrefix + "-d14")
  if ($LASTEXITCODE -ne 0) { throw "d14_rollout failed" }
  $d14Line = @($d14Raw | Where-Object { $_ -like "D14_ROLLOUT_JSON=*" }) | Select-Object -Last 1
  if ([string]::IsNullOrWhiteSpace([string]$d14Line)) { throw "d14 rollout report path missing" }
  $d14RolloutReportPath = ([string]$d14Line).Substring("D14_ROLLOUT_JSON=".Length)
  $results += [pscustomobject]@{
    name = "d14-rollout-final"
    command = "d14_rollout.ps1"
    passed = $true
    exit_code = 0
    report = $d14RolloutReportPath
  }


  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    backup_config = $backupPath
    replay_report = $replayOut
    benchmark_report = $benchmarkPath
    benchmark_compare_report = $benchmarkComparePath
    fault_injection_report = $faultInjectionReportPath
    rollback_drill_report = $rollbackDrillReportPath
    drill_modes = [pscustomobject]@{
      rollback = $rollbackDrillMode
      fault = $faultDrillMode
    }
    version_a_report = $versionAReportPath
    d14_rollout_report = $d14RolloutReportPath
    d46_slo_report = $d46ReportPath
    soak_stability_report = $soakStabilityReportPath
    d13_smoke_report = $d13SmokeReportPath
    d13_full_report = $d13FullReportPath
    d13_full_enabled = [bool]$d13FullEnabled
    all_passed = $true
    required_checks = @(
      "impact-test-selector",
      "intent-execute-verify-persist-replay",
      "decision-trace-four-state",
      "replay-mismatch-explainer",
      "no-bypass-kernel-mediator-static",
      "d10-d11-security-governance-gate-suite",
      "artifact-hard-gate-and-proof",
      "budget-preflight-ledger",
      "recovery-drill",
      "pwiki11-acceptance",
      "pevo-evolution-acceptance",
      "d46-slo-acceptance",
      "ops-acceptance",
      "d12-ops-productized-acceptance",
      "signal-acceptance",
      "frontend-cli-acceptance",
      "d12-storage-postgres-wal-dualwrite-replay",
      "waltx-production-write-minimal-e2e",
      "config-doctor-bad-config-blocked-e2e",
      "rollback-daily-drill",
      "fault-injection-daily-drill",
      "version-a-acceptance",
      "d14-rollout-final",
      "d12-real-benchmark-run-and-compare",
      "d13-smoke-benchmark-always",
      "d13-full-benchmark-daily"
    )
    rollout = @("shadow", "10%", "30%", "full", "rollback")
    checks = @(
      $results | ForEach-Object {
        [pscustomobject]@{
          stage = if ($_.name -like "pre-rollout-config-doctor-gate" -or $_.name -like "cargo-check") { "L0" } elseif ($_.name -like "*no-bypass*" -or $_.name -like "*artifact*" -or $_.name -like "*waltx*") { "L1" } elseif ($_.name -like "rollout-*" -or $_.name -like "replay-report-export") { "L3" } else { "L2" }
          check_id = [string]$_.name
          passed = [bool]$_.passed
          severity = if ([bool]$_.passed) { "info" } else { "blocker" }
          deny_reason = if ([bool]$_.passed) { $null } else { ("exit_code=" + [string]$_.exit_code) }
          evidence_ref = $null
          replay_fp = $null
          duration_ms = if ($null -ne $_.duration_ms) { [int]$_.duration_ms } else { 0 }
          skipped = if ($null -ne $_.skipped) { [bool]$_.skipped } else { $false }
        }
      }
    )
    impact_selector_report = $impactSelectorPath
    impacted_tests_hash = $impactTestsHash
    log_path = $logPath
    release_gate_report = $null
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $canonicalJsonPath -Encoding utf8

  $scriptSucceeded = $true
  Write-Output ("WEEK6_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("WEEK6_ACCEPTANCE_JSON=" + $jsonPath)
  Write-Output ("WEEK6_FULL_ACCEPTANCE_CANONICAL_JSON=" + $canonicalJsonPath)
}
catch {
  $failureMessage = $_.Exception.Message
  if ($logPath) {
    Add-Content -Path $logPath -Value ("`n==== FAILURE ====")
    Add-Content -Path $logPath -Value $failureMessage
  }
  if ($runtimeDir -and -not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }
  if (-not $jsonPath -and $runtimeDir) {
    $fallbackStamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $jsonPath = Join-Path $runtimeDir ("week6-acceptance-" + $fallbackStamp + ".json")
  }
  if (-not $canonicalJsonPath -and $runtimeDir) {
    $canonicalJsonPath = Join-Path $runtimeDir "week6_full_acceptance.json"
  }
  if ($jsonPath -and $canonicalJsonPath) {
    $summary = [pscustomobject]@{
      generated_at = (Get-Date).ToString("s")
      repo_root = $repoRoot
      manifest = $ManifestPath
      prod_config = $ProdConfigPath
      backup_config = $backupPath
      benchmark_report = $null
      benchmark_compare_report = $null
      fault_injection_report = $faultInjectionReportPath
      rollback_drill_report = $rollbackDrillReportPath
      drill_modes = [pscustomobject]@{
        rollback = $rollbackDrillMode
        fault = $faultDrillMode
      }
      version_a_report = $versionAReportPath
      d14_rollout_report = $d14RolloutReportPath
      d46_slo_report = $d46ReportPath
      soak_stability_report = $soakStabilityReportPath
      d13_smoke_report = $d13SmokeReportPath
      d13_full_report = $d13FullReportPath
      d13_full_enabled = [bool]$d13FullEnabled
      all_passed = $false
      failure = $failureMessage
      failed_step = $lastStepName
      rollout = @("shadow", "10%", "30%", "full", "rollback")
      checks = @(
        $results | ForEach-Object {
          [pscustomobject]@{
            stage = if ($_.name -like "pre-rollout-config-doctor-gate" -or $_.name -like "cargo-check") { "L0" } elseif ($_.name -like "*no-bypass*" -or $_.name -like "*artifact*" -or $_.name -like "*waltx*") { "L1" } elseif ($_.name -like "rollout-*" -or $_.name -like "replay-report-export") { "L3" } else { "L2" }
            check_id = [string]$_.name
            passed = [bool]$_.passed
            severity = if ([bool]$_.passed) { "info" } else { "blocker" }
            deny_reason = if ([bool]$_.passed) { $null } else { ("exit_code=" + [string]$_.exit_code) }
            evidence_ref = $null
            replay_fp = $null
            duration_ms = if ($null -ne $_.duration_ms) { [int]$_.duration_ms } else { 0 }
            skipped = if ($null -ne $_.skipped) { [bool]$_.skipped } else { $false }
          }
        }
      )
      impact_selector_report = $impactSelectorPath
      impacted_tests_hash = $impactTestsHash
      log_path = $logPath
      release_gate_report = $null
    }
    $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
    $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $canonicalJsonPath -Encoding utf8
    Write-Output ("WEEK6_ACCEPTANCE_FAILED log=" + $logPath)
    Write-Output ("WEEK6_ACCEPTANCE_JSON=" + $jsonPath)
    Write-Output ("WEEK6_FULL_ACCEPTANCE_CANONICAL_JSON=" + $canonicalJsonPath)
  }
  throw
}
finally {
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  if ($null -eq $previousAutoLoopProfile) {
    Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_PROFILE = $previousAutoLoopProfile
  }
  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  if ($runtimeDir -and (Test-Path $runtimeDir)) {
    $cutoff = (Get-Date).AddDays(-7)
    if ($scriptSucceeded) {
      if ($targetDir -and (Test-Path $targetDir)) {
        Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction SilentlyContinue
      }
      Get-ChildItem -Path $runtimeDir -File -Filter "week6-acceptance-*.log" -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -lt $cutoff } |
        Remove-Item -Force -ErrorAction SilentlyContinue
    } else {
      $targets = Get-ChildItem -Path $runtimeDir -Directory -Filter "target-week6-full-*" -ErrorAction SilentlyContinue |
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










