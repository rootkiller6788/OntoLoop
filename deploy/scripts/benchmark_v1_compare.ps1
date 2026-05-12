param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ControlConfigPath = "deploy/config/autoloop.opencode_like.toml",
  [string]$ExperimentConfigPath = "deploy/config/autoloop.baseline_v0.toml",
  [string]$OntoloopExePath = "",
  [ValidateSet("dev","heldout","stress","all")]
  [string]$Split = "all",
  [int]$Limit = 0,
  [int]$CaseTimeoutMs = 60000,
  [int]$RetryCount = 0,
  [string]$SessionPrefix = "benchmark-v1-compare",
  [bool]$CleanCacheAfterRun = $true,
  [bool]$UseTempTargetDir = $true,
  [int]$BenchmarkKeepLatest = 8
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$previousCargoTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
$tempTargetDirs = New-Object System.Collections.Generic.List[string]

try {
  function Resolve-AbsPath {
    param([string]$PathText)
    if ([System.IO.Path]::IsPathRooted($PathText)) { return $PathText }
    return (Join-Path $repoRoot $PathText)
  }

  function Read-TomlScalar {
    param(
      [string]$TomlRaw,
      [string]$Pattern
    )
    $m = [regex]::Match($TomlRaw, $Pattern)
    if (-not $m.Success) { return $null }
    return $m.Groups[1].Value
  }

  function Invoke-Eval {
    param(
      [string]$Label,
      [string]$ConfigPath,
      [string]$SplitName,
      [string]$RunnerExePath
    )
    $raw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\benchmark_v1_eval.ps1" `
      -ManifestPath $ManifestPath `
      -ProdConfigPath $ConfigPath `
      -OntoloopExePath $RunnerExePath `
      -Split $SplitName `
      -Limit $Limit `
      -CaseTimeoutMs $CaseTimeoutMs `
      -RetryCount $RetryCount `
      -SessionPrefix ($SessionPrefix + "-" + $Label)
    if ($LASTEXITCODE -ne 0) {
      throw ("benchmark_v1_eval failed for " + $Label)
    }
    $line = @($raw | Where-Object { $_ -like "BENCHMARK_V1_EVAL_JSON=*" }) | Select-Object -Last 1
    if ([string]::IsNullOrWhiteSpace([string]$line)) {
      throw ("missing BENCHMARK_V1_EVAL_JSON output for " + $Label)
    }
    $path = ([string]$line).Substring("BENCHMARK_V1_EVAL_JSON=".Length)
    if (-not (Test-Path $path)) {
      throw ("eval report missing for " + $Label + ": " + $path)
    }
    return Get-Content -Raw -Path $path | ConvertFrom-Json
  }

  function Merge-FailureDistribution {
    param([object[]]$Runs)
    $agg = @{}
    foreach ($run in $Runs) {
      $dist = $run.failure_reason_distribution
      if ($null -eq $dist) { continue }
      foreach ($prop in $dist.PSObject.Properties) {
        $key = [string]$prop.Name
        $value = 0
        if ($null -ne $prop.Value) {
          $value = [int]$prop.Value
        }
        if ($agg.ContainsKey($key)) {
          $agg[$key] = [int]$agg[$key] + $value
        } else {
          $agg[$key] = $value
        }
      }
    }
    return [pscustomobject]$agg
  }

  function Winner {
    param(
      [double]$Control,
      [double]$Experiment,
      [switch]$HigherIsBetter
    )
    if ($HigherIsBetter.IsPresent) {
      if ($Experiment -gt $Control) { return "experiment" }
      if ($Experiment -lt $Control) { return "control" }
      return "tie"
    }
    if ($Experiment -lt $Control) { return "experiment" }
    if ($Experiment -gt $Control) { return "control" }
    return "tie"
  }

  $controlConfigAbs = Resolve-AbsPath -PathText $ControlConfigPath
  $experimentConfigAbs = Resolve-AbsPath -PathText $ExperimentConfigPath
  if (-not (Test-Path $controlConfigAbs)) { throw ("control config missing: " + $controlConfigAbs) }
  if (-not (Test-Path $experimentConfigAbs)) { throw ("experiment config missing: " + $experimentConfigAbs) }

  $controlConfigRaw = Get-Content -Raw -Path $controlConfigAbs
  $experimentConfigRaw = Get-Content -Raw -Path $experimentConfigAbs
  $controlModel = Read-TomlScalar -TomlRaw $controlConfigRaw -Pattern '(?m)^\s*default_model\s*=\s*"([^"]+)"\s*$'
  $experimentModel = Read-TomlScalar -TomlRaw $experimentConfigRaw -Pattern '(?m)^\s*default_model\s*=\s*"([^"]+)"\s*$'
  $controlBudget = Read-TomlScalar -TomlRaw $controlConfigRaw -Pattern '(?m)^\s*default_budget_micros\s*=\s*(\d+)\s*$'
  $experimentBudget = Read-TomlScalar -TomlRaw $experimentConfigRaw -Pattern '(?m)^\s*default_budget_micros\s*=\s*(\d+)\s*$'
  if ([string]::IsNullOrWhiteSpace($controlModel) -or [string]::IsNullOrWhiteSpace($experimentModel)) {
    throw "failed to parse default_model from one or both configs"
  }
  if ($controlModel -ne $experimentModel) {
    throw ("model mismatch: control=" + $controlModel + " experiment=" + $experimentModel)
  }
  if ([string]::IsNullOrWhiteSpace($controlBudget) -or [string]::IsNullOrWhiteSpace($experimentBudget)) {
    throw "failed to parse default_budget_micros from one or both configs"
  }
  if ($controlBudget -ne $experimentBudget) {
    throw ("budget mismatch: control=" + $controlBudget + " experiment=" + $experimentBudget)
  }

  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) { New-Item -ItemType Directory -Path $runtimeDir | Out-Null }
  $runnerExePath = $OntoloopExePath
  if ([string]::IsNullOrWhiteSpace($runnerExePath)) {
    if ($UseTempTargetDir) {
      $benchTargetDir = Join-Path $env:TEMP ("autoloop-target-benchmark-compare-" + [guid]::NewGuid().ToString("N"))
      New-Item -ItemType Directory -Path $benchTargetDir -Force | Out-Null
      $tempTargetDirs.Add($benchTargetDir) | Out-Null
    } else {
      $benchTargetDir = Join-Path $runtimeDir "target-benchmark-compare"
    }
    $env:CARGO_TARGET_DIR = $benchTargetDir
    $prevErr = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $buildRaw = & cargo build --manifest-path $ManifestPath 2>&1
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevErr
    if ($buildExit -ne 0) {
      # Retry once with a fresh target dir after cache cleanup to avoid lock conflicts.
      & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action "clean-cache" -Apply | Out-Null
      $secondTarget = if ($UseTempTargetDir) {
        Join-Path $env:TEMP ("autoloop-target-benchmark-compare-" + [guid]::NewGuid().ToString("N"))
      } else {
        Join-Path $runtimeDir ("target-benchmark-compare-" + [guid]::NewGuid().ToString("N"))
      }
      New-Item -ItemType Directory -Path $secondTarget -Force | Out-Null
      if ($UseTempTargetDir) { $tempTargetDirs.Add($secondTarget) | Out-Null }
      $env:CARGO_TARGET_DIR = $secondTarget
      $prevErr = $ErrorActionPreference
      $ErrorActionPreference = "Continue"
      $buildRaw = & cargo build --manifest-path $ManifestPath 2>&1
      $buildExit = $LASTEXITCODE
      $ErrorActionPreference = $prevErr
      if ($buildExit -ne 0) {
        throw ("benchmark runner build failed exit_code=" + $buildExit + " detail=" + ($buildRaw | Out-String))
      }
      $runnerExePath = Join-Path $secondTarget "debug\\ontoloop.exe"
    } else {
      $runnerExePath = Join-Path $benchTargetDir "debug\\ontoloop.exe"
    }
    if (-not (Test-Path $runnerExePath)) {
      throw ("benchmark runner binary missing after build: " + $runnerExePath)
    }
  } elseif (-not [System.IO.Path]::IsPathRooted($runnerExePath)) {
    $runnerExePath = Join-Path $repoRoot $runnerExePath
  }
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }

  $controlEval = Invoke-Eval -Label "control" -ConfigPath $controlConfigAbs -SplitName $Split -RunnerExePath $runnerExePath
  $experimentEval = Invoke-Eval -Label "experiment" -ConfigPath $experimentConfigAbs -SplitName $Split -RunnerExePath $runnerExePath

  $controlAgg = $controlEval.aggregate
  $experimentAgg = $experimentEval.aggregate

  $wSuccess = Winner -Control ([double]$controlAgg.success_rate_percent) -Experiment ([double]$experimentAgg.success_rate_percent) -HigherIsBetter
  $wP50 = Winner -Control ([double]$controlAgg.latency_p50_ms) -Experiment ([double]$experimentAgg.latency_p50_ms)
  $wP95 = Winner -Control ([double]$controlAgg.latency_p95_ms) -Experiment ([double]$experimentAgg.latency_p95_ms)
  $wCost = Winner -Control ([double]$controlAgg.average_cost_micros) -Experiment ([double]$experimentAgg.average_cost_micros)
  $wRetry = Winner -Control ([double]$controlAgg.average_retry_count) -Experiment ([double]$experimentAgg.average_retry_count)

  $outPath = Join-Path $runtimeDir ("benchmark_v1_compare_" + $Split + "_" + (Get-Date -Format "yyyyMMdd-HHmmss") + ".json")

  $report = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    benchmark = "benchmark_v1"
    split = $Split
    fairness = [pscustomobject]@{
      same_machine = $true
      model = $controlModel
      budget_micros = [int64]$controlBudget
      case_timeout_ms = $CaseTimeoutMs
      retry_count = $RetryCount
    }
    control = [pscustomobject]@{
      label = "opencode_like_baseline"
      config = $controlConfigAbs
      eval = $controlEval
      failure_distribution = Merge-FailureDistribution -Runs @($controlEval.runs)
    }
    experiment = [pscustomobject]@{
      label = "ontoloop_baseline_v0"
      config = $experimentConfigAbs
      eval = $experimentEval
      failure_distribution = Merge-FailureDistribution -Runs @($experimentEval.runs)
    }
    metric_winner = [pscustomobject]@{
      success_rate = $wSuccess
      latency_p50 = $wP50
      latency_p95 = $wP95
      average_cost = $wCost
      average_retry_count = $wRetry
    }
  }

  $report | ConvertTo-Json -Depth 12 | Out-File -FilePath $outPath -Encoding utf8
  Write-Output ("BENCHMARK_V1_COMPARE_JSON=" + $outPath)
}
finally {
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  if ($CleanCacheAfterRun) {
    try {
      & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action "clean-cache" -Apply | Out-Null
      & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action "clean-old-runtime" -BenchmarkKeepLatest $BenchmarkKeepLatest -Apply | Out-Null
    } catch {
      Write-Warning ("benchmark_v1_compare clean-cache failed: " + $_.Exception.Message)
    }
  }
  if ($UseTempTargetDir) {
    foreach ($dir in $tempTargetDirs) {
      if ([string]::IsNullOrWhiteSpace([string]$dir)) { continue }
      if (-not (Test-Path $dir)) { continue }
      try { Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue } catch {}
    }
  }
  Pop-Location
}
