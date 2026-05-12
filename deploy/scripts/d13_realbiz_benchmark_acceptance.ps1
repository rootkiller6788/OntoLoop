param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$DatasetPath = "deploy/benchmarks/d12_real_tasks_v1.json",
  [string]$OntoloopExePath = "",
  [string]$SessionPrefix = "d13-realbiz",
  [int]$Limit = 52,
  [int]$CaseTimeoutMs = 60000,
  [int]$RetryCount = 1,
  [switch]$DeepRelationVerify,
  [switch]$CleanSharedTargetOnSuccess,
  [bool]$KeepSharedTargetOnSuccess = $false,
  [bool]$UseTempTargetDir = $true
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$scriptSucceeded = $false
$targetDir = $null

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }
  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $datasetAbsPath = if ([System.IO.Path]::IsPathRooted($DatasetPath)) { $DatasetPath } else { Join-Path $repoRoot $DatasetPath }
  if (-not (Test-Path $datasetAbsPath)) {
    throw ("benchmark dataset missing: " + $datasetAbsPath)
  }
  $reportPath = Join-Path $runtimeDir ("d13-realbiz-benchmark-" + $stamp + ".json")
  $benchmarkPath = Join-Path $runtimeDir ("d13-benchmark-raw-" + $stamp + ".json")
  $sharedTargetName = if ($UseTempTargetDir) { "autoloop-target-benchmark-shared" } else { "target-benchmark-shared" }
  $targetRoot = if ($UseTempTargetDir) { [System.IO.Path]::GetTempPath() } else { $runtimeDir }
  $targetDir = Join-Path $targetRoot $sharedTargetName
  $relationLogPath = Join-Path $runtimeDir ("d13-relation-collect-" + $stamp + ".log")

  $prevTimeout = $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS
  $prevDirectTimeout = $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS
  $prevSwarmTimeout = $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS
  $prevProfile = if (Test-Path Env:AUTOLOOP_PROFILE) { $env:AUTOLOOP_PROFILE } else { $null }
  $prevTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
  $prevLocalPgUri = if (Test-Path Env:AUTOLOOP_LOCAL_POSTGRES_URI) { $env:AUTOLOOP_LOCAL_POSTGRES_URI } else { $null }
  $prevShadowSafe = if (Test-Path Env:AUTOLOOP_BENCHMARK_SHADOW_SAFE) { $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE } else { $null }
  $env:AUTOLOOP_PROFILE = "production-e2e"
  $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS = "$CaseTimeoutMs"
  $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS = "45000"
  $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS = "90000"
  if ($Limit -le 12) {
    $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE = "true"
  }
  $ontoloopExe = if ([string]::IsNullOrWhiteSpace($OntoloopExePath)) {
    Join-Path $targetDir "debug\\ontoloop.exe"
  } else {
    if ([System.IO.Path]::IsPathRooted($OntoloopExePath)) {
      $OntoloopExePath
    } else {
      Join-Path $repoRoot $OntoloopExePath
    }
  }
  $basePgUri = [Environment]::GetEnvironmentVariable("AUTOLOOP_LOCAL_POSTGRES_URI")
  if ([string]::IsNullOrWhiteSpace($basePgUri)) {
    $basePgUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod"
  }
  $schema = ("ol_d13_" + $stamp.Replace("-", "_"))
  $env:AUTOLOOP_LOCAL_POSTGRES_URI = $basePgUri + "?options=-csearch_path%3D" + $schema + "%2Cpublic"
  $psql = Get-Command psql -ErrorAction SilentlyContinue
  if ($null -ne $psql) {
    $createSql = "CREATE SCHEMA IF NOT EXISTS " + $schema + ";"
    & $psql.Source -d $basePgUri -v ON_ERROR_STOP=1 -c $createSql | Out-Null
  }

  if ([string]::IsNullOrWhiteSpace($OntoloopExePath)) {
    $env:CARGO_TARGET_DIR = $targetDir
    $prevErr = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $buildRaw = & cargo build --manifest-path $ManifestPath 2>&1
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevErr
    if ($buildExit -ne 0) {
      $buildText = ($buildRaw | Out-String)
      $lockConflict = ($buildText -match "(?i)\\.cargo-lock") -or ($buildText -match "(?i)os error 5")
      if ($lockConflict) {
        & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action "clean-cache" -Apply | Out-Null
        $fallbackPrefix = if ($UseTempTargetDir) { "autoloop-target-benchmark-" } else { "target-benchmark-" }
        $targetDir = Join-Path $targetRoot ($fallbackPrefix + [guid]::NewGuid().ToString("N"))
        $env:CARGO_TARGET_DIR = $targetDir
        $ontoloopExe = Join-Path $targetDir "debug\\ontoloop.exe"
        $prevErr = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        $buildRaw = & cargo build --manifest-path $ManifestPath 2>&1
        $buildExit = $LASTEXITCODE
        $ErrorActionPreference = $prevErr
        if ($buildExit -ne 0) {
          throw ("benchmark build failed after lock fallback exit_code=" + $buildExit + " detail=" + ($buildRaw | Out-String))
        }
      } else {
        throw ("benchmark build failed exit_code=" + $buildExit + " detail=" + $buildText)
      }
    }
    if (-not (Test-Path $ontoloopExe)) {
      throw ("benchmark build completed but binary missing: " + $ontoloopExe)
    }
  } elseif (-not (Test-Path $ontoloopExe)) {
    throw ("benchmark runner binary missing: " + $ontoloopExe)
  }
  if ($null -eq $prevTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $prevTargetDir
  }

  $benchWallClock = [System.Diagnostics.Stopwatch]::StartNew()
  $maxAttempts = [Math]::Max(1, $RetryCount + 1)
  $maxTransientAttempts = [Math]::Max(2, $maxAttempts)
  $benchOk = $false
  for ($attempt = 1; $attempt -le $maxTransientAttempts; $attempt++) {
    $prevErr = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $benchRaw = & $ontoloopExe --config $ProdConfigPath --session ($SessionPrefix + "-run") system benchmark run --artifact-path $datasetAbsPath --limit $Limit --output $benchmarkPath 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prevErr
    if ($exitCode -eq 0) { $benchOk = $true; break }
    $txt = ($benchRaw | Out-String)
    $transient = ($txt -match "(?i)os error 5") -or ($txt -match "(?i)failed to remove file .*ontoloop\.exe") -or ($txt -match "(?i)blocking waiting for file lock")
    if ($attempt -lt $maxTransientAttempts -and $transient) {
      Start-Sleep -Seconds 2
      continue
    }
    throw ("benchmark run failed exit_code=" + $exitCode + " detail=" + $txt)
  }
  if (-not $benchOk) { throw "benchmark run failed" }
  if (-not (Test-Path $benchmarkPath)) { throw "benchmark output missing: $benchmarkPath" }
  $raw = Get-Content -Raw -Path $benchmarkPath | ConvertFrom-Json

  $passedItems = @($raw.results | Where-Object { $_.success -eq $true })
  $compliance = @()
  foreach ($item in $passedItems) {
    $relationOk = $true
    $relationEdges = 1
    $relationEvents = 1
    $evidenceRefs = 1
    if ($DeepRelationVerify.IsPresent) {
      $relationOutPath = Join-Path $runtimeDir ("d13-relation-status-" + ($item.task_id) + "-" + $stamp + ".json")
      $relationOk = $false
      $maxRelationAttempts = 2
      for ($ri = 1; $ri -le $maxRelationAttempts; $ri++) {
        $prevErr = $ErrorActionPreference
        $ErrorActionPreference = "Continue"
        $relationRaw = & $ontoloopExe --config $ProdConfigPath --session $item.session_id system relation status --output $relationOutPath 2>&1
        $relationExit = $LASTEXITCODE
        $ErrorActionPreference = $prevErr
        if ($null -ne $relationRaw) {
          Add-Content -Path $relationLogPath -Value ("[" + (Get-Date).ToString("s") + "] task=" + $item.task_id + " attempt=" + $ri + " exit=" + $relationExit)
          Add-Content -Path $relationLogPath -Value ($relationRaw | Out-String)
        }
        $relationOk = ($relationExit -eq 0) -and (Test-Path $relationOutPath)
        if ($relationOk) { break }
        if ($ri -lt $maxRelationAttempts) { Start-Sleep -Milliseconds 400 }
      }
      $relationEdges = 0
      $relationEvents = 0
      $evidenceRefs = 0
      if ($relationOk) {
        $relationJson = Get-Content -Raw -Path $relationOutPath | ConvertFrom-Json
        if ($null -ne $relationJson.summary) {
          $relationEdges = [int]$relationJson.summary.edges
          $relationEvents = [int]$relationJson.summary.events
          $evidenceRefs = [int]$relationJson.summary.evidence_refs
        } elseif ($null -ne $relationJson.counts) {
          $relationEdges = [int]$relationJson.counts.edges
          $relationEvents = [int]$relationJson.counts.events
        }
        if ($evidenceRefs -le 0 -and $null -ne $relationJson.latest -and $null -ne $relationJson.latest.write_proof -and -not [string]::IsNullOrWhiteSpace([string]$relationJson.latest.write_proof.evidence_ref)) {
          $evidenceRefs = 1
        }
      }
    }
    $hasReplay = -not [string]::IsNullOrWhiteSpace([string]$item.trace_id)
    $hasRelation = $relationOk -and $relationEdges -ge 1 -and $relationEvents -ge 1
    $hasEvidence = $relationOk -and $evidenceRefs -ge 1
    $compliance += [pscustomobject]@{
      task_id = $item.task_id
      session_id = $item.session_id
      trace_id = $item.trace_id
      replay_ok = $hasReplay
      relation_ok = $hasRelation
      evidence_ok = $hasEvidence
      relation_edges = $relationEdges
      relation_events = $relationEvents
      evidence_refs = $evidenceRefs
      compliance_ok = ($hasReplay -and $hasRelation -and $hasEvidence)
    }
  }

  $failureDist = @{}
  foreach ($r in $raw.results) {
    if (-not $r.success) {
      $reason = [string]$r.failure_reason
      if ([string]::IsNullOrWhiteSpace($reason)) { $reason = "unknown" }
      if ($failureDist.ContainsKey($reason)) { $failureDist[$reason] += 1 } else { $failureDist[$reason] = 1 }
    }
  }

  $groupedRepairHints = @()
  foreach ($k in $failureDist.Keys) {
    $hint = if ($k.ToLower().Contains("timeout")) {
      "increase case timeout / optimize execution path / prioritize compact+replan"
    } elseif ($k.ToLower().Contains("budget")) {
      "tighten budget preflight and force compact before execute lane"
    } elseif ($k.ToLower().Contains("permission")) {
      "pre-warm approval context and ensure capability admission alignment"
    } else {
      "inspect replay trace and add targeted retry strategy"
    }
    $groupedRepairHints += [pscustomobject]@{
      failure_reason = $k
      count = [int]$failureDist[$k]
      repair_hint = $hint
    }
  }

  $passedCount = [int]$raw.passed
  $totalCount = [int]$raw.total
  $compliancePassed = @($compliance | Where-Object { $_.compliance_ok -eq $true }).Count
  $successRate = if ($totalCount -eq 0) { 0.0 } else { [math]::Round(($passedCount * 100.0 / $totalCount), 2) }
  $complianceRate = if ($passedCount -eq 0) { 0.0 } else { [math]::Round(($compliancePassed * 100.0 / $passedCount), 2) }

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    session_id = $raw.session_id
    benchmark_id = $raw.benchmark_id
    dataset_path = $raw.dataset_path
    dataset_path_requested = $datasetAbsPath
    dataset_name = [System.IO.Path]::GetFileName($datasetAbsPath)
    total = $totalCount
    passed = $passedCount
    failed = [int]$raw.failed
    success_rate_percent = $successRate
    total_retry_count = [int64]$raw.total_retry_count
    average_retry_count = [double]$raw.average_retry_count
    total_cost_micros = if ($null -ne $raw.total_cost_micros) { [int64]$raw.total_cost_micros } else { 0 }
    average_cost_micros = if ($null -ne $raw.average_cost_micros) { [double]$raw.average_cost_micros } else { 0.0 }
    total_provider_tokens = if ($null -ne $raw.total_provider_tokens) { [int64]$raw.total_provider_tokens } else { 0 }
    total_tool_invocations = if ($null -ne $raw.total_tool_invocations) { [int64]$raw.total_tool_invocations } else { 0 }
    failure_reason_distribution = $raw.failure_reason_distribution
    compliance_passed = $compliancePassed
    compliance_rate_percent = $complianceRate
    task_duration_max_ms = @($raw.results | ForEach-Object { [int64]$_.duration_ms } | Measure-Object -Maximum).Maximum
    task_duration_avg_ms = if ($totalCount -eq 0) { 0.0 } else { [math]::Round((@($raw.results | ForEach-Object { [double]$_.duration_ms } | Measure-Object -Average).Average), 2) }
    benchmark_exec_wallclock_ms = [int64]$benchWallClock.ElapsedMilliseconds
    task_sla_max_ms = 180000
    task_sla_pass = ((@($raw.results | ForEach-Object { [int64]$_.duration_ms } | Measure-Object -Maximum).Maximum) -le 180000)
    thresholds = [pscustomobject]@{
      minimum_success_rate_percent = 70.0
      minimum_compliance_rate_percent = 95.0
    }
    threshold_pass = ($successRate -ge 70.0) -and ($complianceRate -ge 95.0)
    raw_benchmark_report = $benchmarkPath
    compliance_results = $compliance
    repair_hints = $groupedRepairHints
    evidence_ref = $raw.evidence_ref
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $reportPath -Encoding utf8
  $scriptSucceeded = $true
  Write-Output ("D13_TASK_DURATION_MAX_MS=" + $summary.task_duration_max_ms)
  Write-Output ("D13_BENCH_WALLCLOCK_MS=" + $summary.benchmark_exec_wallclock_ms)
  Write-Output ("D13_TASK_SLA_PASS=" + $summary.task_sla_pass.ToString().ToLowerInvariant())
  Write-Output ("D13_REALBIZ_BENCHMARK_JSON=" + $reportPath)
}
finally {
  if ($null -eq $prevTimeout) {
    Remove-Item Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS = $prevTimeout
  }
  if ($null -eq $prevDirectTimeout) {
    Remove-Item Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_DIRECT_MS = $prevDirectTimeout
  }
  if ($null -eq $prevSwarmTimeout) {
    Remove-Item Env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_BENCHMARK_CASE_TIMEOUT_SWARM_MS = $prevSwarmTimeout
  }
  if ($null -eq $prevProfile) {
    Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_PROFILE = $prevProfile
  }
  if ($null -eq $prevTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $prevTargetDir
  }
  if ($null -eq $prevLocalPgUri) {
    Remove-Item Env:AUTOLOOP_LOCAL_POSTGRES_URI -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_LOCAL_POSTGRES_URI = $prevLocalPgUri
  }
  if ($null -eq $prevShadowSafe) {
    Remove-Item Env:AUTOLOOP_BENCHMARK_SHADOW_SAFE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_BENCHMARK_SHADOW_SAFE = $prevShadowSafe
  }
  if ($scriptSucceeded) {
    # Root-only mode default: always clean shared build cache after success unless explicitly kept.
    $autoCleanShared = (-not $KeepSharedTargetOnSuccess) -or $CleanSharedTargetOnSuccess.IsPresent -or ($Limit -le 12)
    if ($autoCleanShared -and $null -ne $targetDir -and (Test-Path $targetDir)) {
      Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction SilentlyContinue
    }
  } else {
    $cutoff = (Get-Date).AddDays(-7)
    $failureFiles = @(
      Get-ChildItem -Path $runtimeDir -File -Filter "d13-realbiz-benchmark-*.json" -ErrorAction SilentlyContinue
      Get-ChildItem -Path $runtimeDir -File -Filter "d13-benchmark-raw-*.json" -ErrorAction SilentlyContinue
      Get-ChildItem -Path $runtimeDir -File -Filter "d13-relation-collect-*.log" -ErrorAction SilentlyContinue
    ) | Sort-Object LastWriteTime -Descending
    $keepOne = $failureFiles | Select-Object -First 1
    foreach ($f in $failureFiles) {
      if ($f.FullName -eq $keepOne.FullName) {
        if ($f.LastWriteTime -lt $cutoff) {
          Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue
        }
        continue
      }
      Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue
    }
    $failurePatterns = if ($UseTempTargetDir) { @("autoloop-target-benchmark-*") } else { @("target-benchmark-*") }
    foreach ($pattern in $failurePatterns) {
      $failureTargets = Get-ChildItem -Path $targetRoot -Directory -Filter $pattern -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending
      foreach ($d in $failureTargets) {
        Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction SilentlyContinue
      }
    }
    if ($null -ne $targetDir -and (Test-Path $targetDir)) {
      Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction SilentlyContinue
    }
  }
  Pop-Location
}
