param(
  [string]$ManifestPath = ".\Cargo.toml",
  [double]$P95LatencyThresholdMs = 300000,
  [double]$ErrorRateThreshold = 0.05,
  [double]$MttrThresholdMs = 60000,
  [int]$StepRetryCount = 1
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$runtimeDir = Join-Path $repoRoot "deploy\runtime"
if (-not (Test-Path $runtimeDir)) {
  New-Item -ItemType Directory -Path $runtimeDir | Out-Null
}
$previousCargoTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
$previousLocalPgUri = if (Test-Path Env:AUTOLOOP_LOCAL_POSTGRES_URI) { $env:AUTOLOOP_LOCAL_POSTGRES_URI } else { $null }
$scriptSucceeded = $false
$targetDir = $null

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logPath = Join-Path $runtimeDir ("d46-slo-acceptance-" + $stamp + ".log")
$jsonPath = Join-Path $runtimeDir ("d46-slo-acceptance-" + $stamp + ".json")
try {
  $targetDir = Join-Path $runtimeDir ("target-d46-" + $stamp)
  New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
  $env:CARGO_TARGET_DIR = $targetDir
  $localBaseUri = [Environment]::GetEnvironmentVariable("AUTOLOOP_LOCAL_POSTGRES_URI")
  if ([string]::IsNullOrWhiteSpace($localBaseUri)) {
    $localBaseUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod"
  }
  $schema = ("ol_d46_" + $stamp.Replace("-", "_"))
  $env:AUTOLOOP_LOCAL_POSTGRES_URI = $localBaseUri + "?options=-csearch_path%3D" + $schema + "%2Cpublic"
  $psql = Get-Command psql -ErrorAction SilentlyContinue
  if ($null -ne $psql) {
    $createSql = "CREATE SCHEMA IF NOT EXISTS " + $schema + ";"
    & $psql.Source -d $localBaseUri -v ON_ERROR_STOP=1 -c $createSql | Out-Null
  }

$env:RUST_MIN_STACK = "33554432"
$env:CARGO_BUILD_JOBS = "1"

function Invoke-WarmupStep {
  param(
    [string]$Display,
    [string[]]$Argv
  )
  $maxAttempts = [Math]::Max(1, $StepRetryCount + 1)
  for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
    Add-Content -Path $logPath -Value ("`n==== WARMUP: attempt " + $attempt + "/" + $maxAttempts + " " + $Display + " ====")
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & cargo @Argv 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev
    $outputText = ""
    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
      $outputText = ($output | Out-String)
    }
    if ($exitCode -eq 0) { return }
    $isTransientLock = ($outputText -match "os error 5") -or ($outputText -match "The process cannot access the file")
    if ($attempt -lt $maxAttempts -and $isTransientLock) {
      Start-Sleep -Milliseconds (500 * $attempt)
      continue
    }
    throw "warmup failed ($exitCode): $Display"
  }
}

function Invoke-TimedStep {
  param(
    [string]$Name,
    [string]$Category,
    [string]$Exe,
    [string[]]$Argv
  )

  $maxAttempts = [Math]::Max(1, $StepRetryCount + 1)
  for ($attempt = 1; $attempt -le $maxAttempts; $attempt++) {
    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Category + "] [" + $Name + "] attempt " + $attempt + "/" + $maxAttempts + " " + $display + " ====")

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $Exe @Argv 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev
    $sw.Stop()

    $outputText = ""
    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
      $outputText = ($output | Out-String)
    }

    if ($exitCode -eq 0) {
      return [pscustomobject]@{
        name = $Name
        category = $Category
        command = $display
        passed = $true
        exit_code = 0
        duration_ms = [int64]$sw.ElapsedMilliseconds
        attempts = $attempt
      }
    }

    $isTransientLock = ($outputText -match "os error 5") -or ($outputText -match "The process cannot access the file")
    if ($attempt -lt $maxAttempts -and $isTransientLock) {
      Start-Sleep -Milliseconds (500 * $attempt)
      continue
    }

    return [pscustomobject]@{
      name = $Name
      category = $Category
      command = $display
      passed = $false
      exit_code = $exitCode
      duration_ms = [int64]$sw.ElapsedMilliseconds
      attempts = $attempt
    }
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

# warm-up compile outside SLO timing window to avoid counting one-time build cost as runtime latency
$warmupTestTargets = @(
  "p10_day10_acceptance_e2e",
  "p5_perf_stability",
  "pq10_query_loop_parallel_tool_events_contract"
)
foreach ($target in $warmupTestTargets) {
  Invoke-WarmupStep -Display ("cargo test --manifest-path " + $ManifestPath + " --test " + $target + " --no-run") -Argv @(
    "test", "--manifest-path", $ManifestPath, "--test", $target, "--no-run"
  )
}
Invoke-WarmupStep -Display ("cargo test --manifest-path " + $ManifestPath + " --lib --no-run") -Argv @(
  "test", "--manifest-path", $ManifestPath, "--lib", "--no-run"
)

# D4: pressure tests
$steps += Invoke-TimedStep -Name "single_session_long_chain" -Category "pressure" -Exe "cargo" -Argv @(
  "test", "--manifest-path", $ManifestPath, "--test", "p10_day10_acceptance_e2e", "day10_swarm_query_and_replay_artifacts_end_to_end"
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
  "test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::p11_chaos_case_records_failover"
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
$scriptSucceeded = $true
} finally {
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
  if (Test-Path $runtimeDir) {
    $cutoff = (Get-Date).AddDays(-7)
    if ($scriptSucceeded) {
      if ($targetDir -and (Test-Path $targetDir)) {
        Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction SilentlyContinue
      }
      Get-ChildItem -Path $runtimeDir -File -Filter "d46-slo-acceptance-*.log" -ErrorAction SilentlyContinue |
        Where-Object { $_.LastWriteTime -lt $cutoff } |
        Remove-Item -Force -ErrorAction SilentlyContinue
    } else {
      $targets = Get-ChildItem -Path $runtimeDir -Directory -Filter "target-d46-*" -ErrorAction SilentlyContinue |
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
}
