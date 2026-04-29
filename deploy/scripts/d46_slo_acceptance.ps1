param(
  [string]$ManifestPath = ".\Cargo.toml",
  [double]$P95LatencyThresholdMs = 120000,
  [double]$ErrorRateThreshold = 0.05,
  [double]$MttrThresholdMs = 60000
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$runtimeDir = Join-Path $repoRoot "deploy\runtime"
if (-not (Test-Path $runtimeDir)) {
  New-Item -ItemType Directory -Path $runtimeDir | Out-Null
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logPath = Join-Path $runtimeDir ("d46-slo-acceptance-" + $stamp + ".log")
$jsonPath = Join-Path $runtimeDir ("d46-slo-acceptance-" + $stamp + ".json")

$env:RUST_MIN_STACK = "33554432"
$env:CARGO_BUILD_JOBS = "1"

function Invoke-TimedStep {
  param(
    [string]$Name,
    [string]$Category,
    [string]$Exe,
    [string[]]$Argv
  )

  $display = "$Exe $($Argv -join ' ')"
  Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Category + "] [" + $Name + "] " + $display + " ====")

  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $prev = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $output = & $Exe @Argv 2>&1
  $exitCode = $LASTEXITCODE
  $ErrorActionPreference = $prev
  $sw.Stop()

  if ($null -ne $output) {
    $output | Out-File -FilePath $logPath -Append -Encoding utf8
  }

  [pscustomobject]@{
    name = $Name
    category = $Category
    command = $display
    passed = ($exitCode -eq 0)
    exit_code = $exitCode
    duration_ms = [int64]$sw.ElapsedMilliseconds
  }
}

function Get-P95 {
  param([double[]]$Values)
  if ($null -eq $Values -or $Values.Count -eq 0) { return 0.0 }
  $sorted = $Values | Sort-Object
  $idx = [Math]::Ceiling($sorted.Count * 0.95) - 1
  if ($idx -lt 0) { $idx = 0 }
  if ($idx -ge $sorted.Count) { $idx = $sorted.Count - 1 }
  return [double]$sorted[$idx]
}

$steps = @()

# D4: pressure tests
$steps += Invoke-TimedStep -Name "single_session_long_chain" -Category "pressure" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--test", "pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e"
)
$steps += Invoke-TimedStep -Name "multi_session_concurrency" -Category "pressure" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--test", "p5_perf_stability", "baseline_concurrent_execute_is_stable"
)
$steps += Invoke-TimedStep -Name "tool_parallel_mixed_chain" -Category "pressure" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--test", "pq10_query_loop_parallel_tool_events_contract"
)

# D5: fault injection and recovery tests
$steps += Invoke-TimedStep -Name "fault_provider_timeout" -Category "fault_injection" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_provider_outage_switches_to_degrade_fallback"
)
$steps += Invoke-TimedStep -Name "fault_tool_failure" -Category "fault_injection" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_mcp_failure_switches_to_conservative_degrade"
)
$steps += Invoke-TimedStep -Name "fault_budget_overflow_compact_replan" -Category "fault_injection" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--lib", "tests::swarm_budget_preflight_compacts_when_budget_overflows"
)
$mttrStep = Invoke-TimedStep -Name "fault_recovery_mttr" -Category "fault_injection" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_recover_marks_failover_with_mttr"
)
$steps += $mttrStep

# D6: SLO calculation
$durations = @($steps | ForEach-Object { [double]$_.duration_ms })
$p95 = Get-P95 -Values $durations
$total = [double]($steps.Count)
$failed = [double](($steps | Where-Object { -not $_.passed }).Count)
$errorRate = if ($total -gt 0) { $failed / $total } else { 0.0 }
$mttrMs = [double]$mttrStep.duration_ms

$slo = [pscustomobject]@{
  p95_latency_ms = [Math]::Round($p95, 2)
  error_rate = [Math]::Round($errorRate, 4)
  mttr_ms = [Math]::Round($mttrMs, 2)
}

$thresholds = [pscustomobject]@{
  p95_latency_ms = $P95LatencyThresholdMs
  error_rate = $ErrorRateThreshold
  mttr_ms = $MttrThresholdMs
}

$breaches = @()
if ($slo.p95_latency_ms -gt $thresholds.p95_latency_ms) { $breaches += "p95_latency_ms" }
if ($slo.error_rate -gt $thresholds.error_rate) { $breaches += "error_rate" }
if ($slo.mttr_ms -gt $thresholds.mttr_ms) { $breaches += "mttr_ms" }

$summary = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  manifest = $ManifestPath
  slo = $slo
  thresholds = $thresholds
  all_steps_passed = (($steps | Where-Object { -not $_.passed }).Count -eq 0)
  slo_passed = ($breaches.Count -eq 0)
  breaches = $breaches
  steps = $steps
  log_path = $logPath
}

$summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
Write-Output ("D46_SLO_LOG=" + $logPath)
Write-Output ("D46_SLO_JSON=" + $jsonPath)

if (-not $summary.all_steps_passed) {
  throw "D4-D6 acceptance failed: one or more pressure/fault steps failed"
}
if (-not $summary.slo_passed) {
  throw ("D4-D6 SLO breached: " + ($breaches -join ", "))
}

Write-Output "D46_SLO_ACCEPTANCE_OK"
