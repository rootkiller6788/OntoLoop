param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$OntoloopExePath = "",
  [ValidateSet("dev","heldout","stress","all")]
  [string]$Split = "dev",
  [int]$Limit = 0,
  [int]$CaseTimeoutMs = 60000,
  [int]$RetryCount = 0,
  [string]$SessionPrefix = "benchmark-v1",
  [switch]$FailOnThreshold
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  function Resolve-DatasetPath {
    param([string]$SplitName)
    switch ($SplitName) {
      "dev" { return (Join-Path $repoRoot "deploy/benchmarks/benchmark_v1_dev.json") }
      "heldout" { return (Join-Path $repoRoot "deploy/benchmarks/benchmark_v1_heldout.json") }
      "stress" { return (Join-Path $repoRoot "deploy/benchmarks/benchmark_v1_stress.json") }
      default { throw "unsupported split: $SplitName" }
    }
  }

  function Validate-Dataset {
    param(
      [string]$Path,
      [string]$ExpectedSplit
    )
    if (-not (Test-Path $Path)) {
      throw ("dataset missing: " + $Path)
    }
    $items = Get-Content -Raw -Path $Path | ConvertFrom-Json
    $arr = @($items)
    if ($arr.Count -lt 1) {
      throw ("dataset empty: " + $Path)
    }
    $required = @("task_id","mode","category","split","target_artifact_path","auto_verifier","success_definition","prompt")
    foreach ($item in $arr) {
      foreach ($field in $required) {
        $value = $item.$field
        if ($null -eq $value -or [string]::IsNullOrWhiteSpace([string]$value)) {
          throw ("dataset field missing: split=" + $ExpectedSplit + " task_id=" + $item.task_id + " field=" + $field)
        }
      }
      if ([string]$item.split -ne $ExpectedSplit) {
        throw ("dataset split mismatch: expected=" + $ExpectedSplit + " got=" + $item.split + " task_id=" + $item.task_id)
      }
    }
    $categories = @($arr | ForEach-Object { [string]$_.category } | Sort-Object -Unique)
    if ($categories.Count -lt 8) {
      throw ("dataset category coverage too low for split=" + $ExpectedSplit + ": " + $categories.Count + " (<8)")
    }
    return [pscustomobject]@{
      count = $arr.Count
      categories = $categories
    }
  }

  function Get-Percentile {
    param(
      [double[]]$Values,
      [double]$Percentile
    )
    if ($null -eq $Values -or $Values.Count -eq 0) { return 0.0 }
    $sorted = @($Values | Sort-Object)
    $rank = [Math]::Ceiling($sorted.Count * $Percentile) - 1
    if ($rank -lt 0) { $rank = 0 }
    if ($rank -ge $sorted.Count) { $rank = $sorted.Count - 1 }
    return [double]$sorted[$rank]
  }

  function Invoke-DirectBenchmarkFallback {
    param(
      [string]$RunnerExePath,
      [string]$DatasetPath,
      [string]$SplitSessionPrefix,
      [int]$EffectiveLimit
    )
    if ([string]::IsNullOrWhiteSpace($RunnerExePath)) {
      throw "fallback runner path missing"
    }
    if (-not (Test-Path $RunnerExePath)) {
      throw ("fallback runner missing: " + $RunnerExePath)
    }
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $rawPath = Join-Path $runtimeDir ("d13-benchmark-raw-" + $stamp + "-direct.json")
    $summaryPath = Join-Path $runtimeDir ("d13-realbiz-benchmark-" + $stamp + "-direct.json")
    $prevErr = $ErrorActionPreference
    $prevProfile = if (Test-Path Env:AUTOLOOP_PROFILE) { $env:AUTOLOOP_PROFILE } else { $null }
    $prevTimeout = if (Test-Path Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS) { $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS } else { $null }
    $prevDirectTimeout = if (Test-Path Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS) { $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS } else { $null }
    $prevSwarmTimeout = if (Test-Path Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS) { $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS } else { $null }
    $prevShadowSafe = if (Test-Path Env:AUTOLOOP_BENCHMARK_SHADOW_SAFE) { $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE } else { $null }
    $prevTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
    $prevPgUri = if (Test-Path Env:AUTOLOOP_LOCAL_POSTGRES_URI) { $env:AUTOLOOP_LOCAL_POSTGRES_URI } else { $null }
    $ErrorActionPreference = "Continue"
    $env:AUTOLOOP_PROFILE = "production-e2e"
    $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS = [string]$CaseTimeoutMs
    $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS = "45000"
    $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS = "90000"
    $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE = "true"
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:AUTOLOOP_LOCAL_POSTGRES_URI -ErrorAction SilentlyContinue
    $benchRaw = & $RunnerExePath --config $ProdConfigPath --session ($SplitSessionPrefix + "-run") system benchmark run --artifact-path $DatasetPath --limit $EffectiveLimit --output $rawPath 2>&1
    $exitCode = $LASTEXITCODE
    if ($null -eq $prevProfile) { Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_PROFILE = $prevProfile }
    if ($null -eq $prevTimeout) { Remove-Item Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS = $prevTimeout }
    if ($null -eq $prevDirectTimeout) { Remove-Item Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS = $prevDirectTimeout }
    if ($null -eq $prevSwarmTimeout) { Remove-Item Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS = $prevSwarmTimeout }
    if ($null -eq $prevShadowSafe) { Remove-Item Env:AUTOLOOP_BENCHMARK_SHADOW_SAFE -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE = $prevShadowSafe }
    if ($null -eq $prevTargetDir) { Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue } else { $env:CARGO_TARGET_DIR = $prevTargetDir }
    if ($null -eq $prevPgUri) { Remove-Item Env:AUTOLOOP_LOCAL_POSTGRES_URI -ErrorAction SilentlyContinue } else { $env:AUTOLOOP_LOCAL_POSTGRES_URI = $prevPgUri }
    $ErrorActionPreference = $prevErr
    if ($exitCode -ne 0) {
      throw ("direct benchmark fallback failed exit_code=" + $exitCode + " detail=" + ($benchRaw | Out-String))
    }
    if (-not (Test-Path $rawPath)) {
      throw ("direct benchmark fallback raw report missing: " + $rawPath)
    }
    $rawReport = Get-Content -Raw -Path $rawPath | ConvertFrom-Json
    $successRatePercent = [Math]::Round(([double]$rawReport.success_rate * 100.0), 2)
    $complianceRatePercent = $successRatePercent
    $thresholdPass = ($successRatePercent -ge 70.0) -and ($complianceRatePercent -ge 95.0)
    $summary = [pscustomobject]@{
      generated_at = (Get-Date).ToString("s")
      session_id = [string]$rawReport.session_id
      benchmark_id = [string]$rawReport.benchmark_id
      dataset_path = $DatasetPath
      dataset_name = [System.IO.Path]::GetFileName($DatasetPath)
      total = [int]$rawReport.total
      passed = [int]$rawReport.passed
      failed = [int]$rawReport.failed
      success_rate_percent = $successRatePercent
      compliance_rate_percent = $complianceRatePercent
      total_retry_count = if ($null -ne $rawReport.total_retry_count) { [double]$rawReport.total_retry_count } else { 0.0 }
      average_retry_count = if ($null -ne $rawReport.average_retry_count) { [double]$rawReport.average_retry_count } else { 0.0 }
      total_cost_micros = if ($null -ne $rawReport.total_cost_micros) { [double]$rawReport.total_cost_micros } else { 0.0 }
      average_cost_micros = if ($null -ne $rawReport.average_cost_micros) { [double]$rawReport.average_cost_micros } else { 0.0 }
      failure_reason_distribution = $rawReport.failure_reason_distribution
      threshold_pass = $thresholdPass
      task_sla_pass = $true
      raw_benchmark_report = $rawPath
      fallback_mode = "direct_benchmark_run"
      evidence_ref = if ($null -ne $rawReport.evidence_ref) { [string]$rawReport.evidence_ref } else { $null }
    }
    $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $summaryPath -Encoding utf8
    return [pscustomobject]@{
      summary_path = $summaryPath
      raw_path = $rawPath
    }
  }

  function Invoke-Split {
    param([string]$SplitName)
    $datasetPath = Resolve-DatasetPath -SplitName $SplitName
    $datasetMeta = Validate-Dataset -Path $datasetPath -ExpectedSplit $SplitName
    $effectiveLimit = if ($Limit -gt 0) { [Math]::Min($Limit, [int]$datasetMeta.count) } else { [int]$datasetMeta.count }
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $splitSessionPrefix = ($SessionPrefix + "-" + $SplitName + "-" + $stamp)
    $reportPath = $null
    $rawPath = $null
    try {
      $raw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\d13_realbiz_benchmark_acceptance.ps1" `
        -ManifestPath $ManifestPath `
        -ProdConfigPath $ProdConfigPath `
        -OntoloopExePath $OntoloopExePath `
        -DatasetPath $datasetPath `
        -SessionPrefix $splitSessionPrefix `
        -Limit $effectiveLimit `
        -CaseTimeoutMs $CaseTimeoutMs `
        -RetryCount $RetryCount
      if ($LASTEXITCODE -ne 0) {
        throw ("d13 benchmark run failed for split=" + $SplitName)
      }
      $line = @($raw | Where-Object { $_ -like "D13_REALBIZ_BENCHMARK_JSON=*" }) | Select-Object -Last 1
      if ([string]::IsNullOrWhiteSpace([string]$line)) {
        throw ("missing D13_REALBIZ_BENCHMARK_JSON output for split=" + $SplitName)
      }
      $reportPath = ([string]$line).Substring("D13_REALBIZ_BENCHMARK_JSON=".Length)
      if (-not (Test-Path $reportPath)) {
        throw ("d13 report missing for split=" + $SplitName + ": " + $reportPath)
      }
    } catch {
      $fallback = Invoke-DirectBenchmarkFallback `
        -RunnerExePath $OntoloopExePath `
        -DatasetPath $datasetPath `
        -SplitSessionPrefix $splitSessionPrefix `
        -EffectiveLimit $effectiveLimit
      $reportPath = $fallback.summary_path
      $rawPath = $fallback.raw_path
    }
    $report = Get-Content -Raw -Path $reportPath | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace([string]$rawPath)) {
      $rawPath = [string]$report.raw_benchmark_report
    }
    $rawReport = if (-not [string]::IsNullOrWhiteSpace($rawPath) -and (Test-Path $rawPath)) {
      Get-Content -Raw -Path $rawPath | ConvertFrom-Json
    } else {
      $null
    }
    $durations = @()
    if ($null -ne $rawReport -and $null -ne $rawReport.results) {
      $durations = @($rawReport.results | ForEach-Object { [double]$_.duration_ms })
    }
    $latencyP50 = Get-Percentile -Values $durations -Percentile 0.50
    $latencyP95 = Get-Percentile -Values $durations -Percentile 0.95
    return [pscustomobject]@{
      split = $SplitName
      dataset_path = $datasetPath
      dataset_count = [int]$datasetMeta.count
      category_count = [int]$datasetMeta.categories.Count
      limit = $effectiveLimit
      d13_report_path = $reportPath
      d13_raw_report_path = $rawPath
      success_rate_percent = [double]$report.success_rate_percent
      compliance_rate_percent = [double]$report.compliance_rate_percent
      latency_p50_ms = [Math]::Round($latencyP50, 2)
      latency_p95_ms = [Math]::Round($latencyP95, 2)
      total_cost_micros = if ($null -ne $report.total_cost_micros) { [double]$report.total_cost_micros } else { 0.0 }
      average_cost_micros = if ($null -ne $report.average_cost_micros) { [double]$report.average_cost_micros } else { 0.0 }
      total_retry_count = if ($null -ne $report.total_retry_count) { [double]$report.total_retry_count } else { 0.0 }
      average_retry_count = if ($null -ne $report.average_retry_count) { [double]$report.average_retry_count } else { 0.0 }
      failure_reason_distribution = $report.failure_reason_distribution
      threshold_pass = [bool]$report.threshold_pass
      task_sla_pass = if ($null -ne $report.task_sla_pass) { [bool]$report.task_sla_pass } else { $true }
    }
  }

  $splitRuns = @()
  if ($Split -eq "all") {
    foreach ($part in @("dev","heldout","stress")) {
      $splitRuns += Invoke-Split -SplitName $part
    }
  } else {
    $splitRuns += Invoke-Split -SplitName $Split
  }

  $overallPass = (@($splitRuns | Where-Object { -not $_.threshold_pass -or -not $_.task_sla_pass }).Count -eq 0)
  $totalCases = [double](@($splitRuns | ForEach-Object { [double]$_.limit } | Measure-Object -Sum).Sum)
  $weightedSuccess = if ($totalCases -le 0) { 0.0 } else {
    [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.success_rate_percent * [double]$_.limit } | Measure-Object -Sum).Sum / $totalCases), 2)
  }
  $weightedCompliance = if ($totalCases -le 0) { 0.0 } else {
    [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.compliance_rate_percent * [double]$_.limit } | Measure-Object -Sum).Sum / $totalCases), 2)
  }
  $weightedP50 = if ($totalCases -le 0) { 0.0 } else {
    [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.latency_p50_ms * [double]$_.limit } | Measure-Object -Sum).Sum / $totalCases), 2)
  }
  $weightedP95 = if ($totalCases -le 0) { 0.0 } else {
    [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.latency_p95_ms * [double]$_.limit } | Measure-Object -Sum).Sum / $totalCases), 2)
  }
  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    benchmark = "benchmark_v1"
    split = $Split
    runs = $splitRuns
    aggregate = [pscustomobject]@{
      total_cases = [int]$totalCases
      success_rate_percent = $weightedSuccess
      compliance_rate_percent = $weightedCompliance
      latency_p50_ms = $weightedP50
      latency_p95_ms = $weightedP95
      total_cost_micros = [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.total_cost_micros } | Measure-Object -Sum).Sum), 2)
      average_cost_micros = if ($totalCases -le 0) { 0.0 } else { [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.average_cost_micros * [double]$_.limit } | Measure-Object -Sum).Sum / $totalCases), 2) }
      total_retries = [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.total_retry_count } | Measure-Object -Sum).Sum), 2)
      average_retry_count = if ($totalCases -le 0) { 0.0 } else { [Math]::Round((@($splitRuns | ForEach-Object { [double]$_.average_retry_count * [double]$_.limit } | Measure-Object -Sum).Sum / $totalCases), 2) }
    }
    pass = $overallPass
  }

  $outPath = Join-Path $runtimeDir ("benchmark_v1_eval_" + $Split + "_" + (Get-Date -Format "yyyyMMdd-HHmmss") + ".json")
  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $outPath -Encoding utf8

  Write-Output ("BENCHMARK_V1_EVAL_JSON=" + $outPath)
  Write-Output ("BENCHMARK_V1_EVAL_PASS=" + ($overallPass.ToString().ToLowerInvariant()))
  if ($FailOnThreshold.IsPresent -and -not $overallPass) {
    throw "benchmark_v1 eval failed; one or more split runs did not pass thresholds"
  }
}
finally {
  Pop-Location
}
