param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$OntoloopExePath = "",
  [ValidateSet("dev","heldout","stress","all")]
  [string]$Split = "all",
  [int]$Limit = 120,
  [int]$Repeats = 3,
  [int]$CaseTimeoutMs = 60000,
  [int]$RetryCount = 0,
  [string]$SessionPrefix = "phase-d",
  [int]$BenchmarkKeepLatest = 8
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
  $previousCargoTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
  $runnerTempTargetDir = $null
  $resolvedRunnerExe = $OntoloopExePath

  function Resolve-RunnerExe {
    param([string]$Manifest)
    if (-not [string]::IsNullOrWhiteSpace($resolvedRunnerExe)) {
      if (-not [System.IO.Path]::IsPathRooted($resolvedRunnerExe)) {
        $resolvedRunnerExe = Join-Path $repoRoot $resolvedRunnerExe
      }
      if (-not (Test-Path $resolvedRunnerExe)) {
        throw ("ontoloop runner missing: " + $resolvedRunnerExe)
      }
      return
    }

    $runnerTempTargetDir = Join-Path $env:TEMP ("autoloop-target-phase-d-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $runnerTempTargetDir -Force | Out-Null
    $env:CARGO_TARGET_DIR = $runnerTempTargetDir
    $prevErr = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $buildRaw = & cargo build --manifest-path $Manifest 2>&1
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevErr
    if ($buildExit -ne 0) {
      throw ("phase_d runner build failed exit_code=" + $buildExit + " detail=" + ($buildRaw | Out-String))
    }
    $resolvedRunnerExe = Join-Path $runnerTempTargetDir "debug\\ontoloop.exe"
    if (-not (Test-Path $resolvedRunnerExe)) {
      throw ("phase_d runner missing after build: " + $resolvedRunnerExe)
    }
  }

  function Write-LayerConfig {
    param(
      [string]$BaseConfigPath,
      [string]$OutPath,
      [string]$Layer
    )
    $raw = Get-Content -Raw -Path $BaseConfigPath
    # Common identity so reports are traceable by layer.
    $raw = [regex]::Replace($raw, '(?m)^\s*name\s*=\s*".*?"\s*$', ('name = "ontoloop-phase-d-' + $Layer.ToLowerInvariant() + '"'))
    $raw = [regex]::Replace($raw, '(?m)^\s*profile\s*=\s*".*?"\s*$', ('profile = "phase-d-' + $Layer.ToLowerInvariant() + '"'))

    switch ($Layer) {
      "A0" {
        $raw = [regex]::Replace($raw, '(?m)^\s*gate_mode\s*=\s*".*?"\s*$', 'gate_mode = "shadow"')
        $raw = [regex]::Replace($raw, '(?m)^\s*gate_enforce_ratio\s*=\s*[0-9.]+\s*$', 'gate_enforce_ratio = 0.0')
        $raw = [regex]::Replace($raw, '(?m)^\s*policy_mode\s*=\s*".*?"\s*$', 'policy_mode = "off"')
      }
      "A1" {
        $raw = [regex]::Replace($raw, '(?m)^\s*gate_mode\s*=\s*".*?"\s*$', 'gate_mode = "enforced"')
        $raw = [regex]::Replace($raw, '(?m)^\s*gate_enforce_ratio\s*=\s*[0-9.]+\s*$', 'gate_enforce_ratio = 1.0')
        $raw = [regex]::Replace($raw, '(?m)^\s*policy_mode\s*=\s*".*?"\s*$', 'policy_mode = "off"')
      }
      "A2" {
        $raw = [regex]::Replace($raw, '(?m)^\s*gate_mode\s*=\s*".*?"\s*$', 'gate_mode = "enforced"')
        $raw = [regex]::Replace($raw, '(?m)^\s*gate_enforce_ratio\s*=\s*[0-9.]+\s*$', 'gate_enforce_ratio = 1.0')
        $raw = [regex]::Replace($raw, '(?m)^\s*policy_mode\s*=\s*".*?"\s*$', 'policy_mode = "off"')
        $raw = [regex]::Replace($raw, '(?m)^\s*rollback_contract_version\s*=\s*".*?"\s*$', 'rollback_contract_version = "root-only-evidence/v1"')
        $raw = [regex]::Replace($raw, '(?m)^\s*trust_ledger_consistency_mode\s*=\s*".*?"\s*$', 'trust_ledger_consistency_mode = "strong"')
      }
      default { throw "unsupported layer: $Layer" }
    }

    Set-Content -LiteralPath $OutPath -Value $raw -Encoding utf8
  }

  function Invoke-LayerRun {
    param(
      [string]$Layer,
      [string]$ConfigPath
    )
    $runReports = @()
    for ($r = 1; $r -le $Repeats; $r++) {
      $raw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\benchmark_v1_eval.ps1" `
        -ManifestPath $ManifestPath `
        -ProdConfigPath $ConfigPath `
        -OntoloopExePath $resolvedRunnerExe `
        -Split $Split `
        -Limit $Limit `
        -CaseTimeoutMs $CaseTimeoutMs `
        -RetryCount $RetryCount `
        -SessionPrefix ($SessionPrefix + "-" + $Layer.ToLowerInvariant() + "-r" + $r)
      if ($LASTEXITCODE -ne 0) {
        $detail = if ($null -eq $raw) { "<no-output>" } else { ($raw | Out-String) }
        throw ("benchmark_v1_eval failed: layer=" + $Layer + " repeat=" + $r + " detail=" + $detail)
      }
      $line = @($raw | Where-Object { $_ -like "BENCHMARK_V1_EVAL_JSON=*" }) | Select-Object -Last 1
      if ([string]::IsNullOrWhiteSpace([string]$line)) {
        throw ("missing BENCHMARK_V1_EVAL_JSON output: layer=" + $Layer + " repeat=" + $r)
      }
      $path = ([string]$line).Substring("BENCHMARK_V1_EVAL_JSON=".Length)
      if (-not (Test-Path $path)) {
        throw ("eval report missing: " + $path)
      }
      $json = Get-Content -Raw -Path $path | ConvertFrom-Json
      $agg = $json.aggregate
      $runReports += [pscustomobject]@{
        repeat = $r
        report = $path
        success_rate_percent = [double]$agg.success_rate_percent
        compliance_rate_percent = [double]$agg.compliance_rate_percent
        latency_p95_ms = [double]$agg.latency_p95_ms
        average_cost_micros = [double]$agg.average_cost_micros
        average_retry_count = [double]$agg.average_retry_count
      }
    }

    $avg = [pscustomobject]@{
      success_rate_percent = [math]::Round((@($runReports | Measure-Object -Property success_rate_percent -Average).Average), 2)
      compliance_rate_percent = [math]::Round((@($runReports | Measure-Object -Property compliance_rate_percent -Average).Average), 2)
      latency_p95_ms = [math]::Round((@($runReports | Measure-Object -Property latency_p95_ms -Average).Average), 2)
      average_cost_micros = [math]::Round((@($runReports | Measure-Object -Property average_cost_micros -Average).Average), 2)
      average_retry_count = [math]::Round((@($runReports | Measure-Object -Property average_retry_count -Average).Average), 2)
    }
    return [pscustomobject]@{ layer = $Layer; runs = $runReports; average = $avg }
  }

  function Compare-Layer {
    param(
      [object]$Prev,
      [object]$Curr
    )
    $deltaSuccess = [math]::Round(($Curr.average.success_rate_percent - $Prev.average.success_rate_percent), 2)
    $latencyDropRatio = if ([double]$Prev.average.latency_p95_ms -le 0) { 0.0 } else { ([double]$Prev.average.latency_p95_ms - [double]$Curr.average.latency_p95_ms) / [double]$Prev.average.latency_p95_ms }
    $latencyDropPercent = [math]::Round($latencyDropRatio * 100.0, 2)
    $complianceDelta = [math]::Round(($Curr.average.compliance_rate_percent - $Prev.average.compliance_rate_percent), 2)
    $complianceOk = ($complianceDelta -ge 0.0)
    $gainOk = ($deltaSuccess -ge 2.0) -or ($latencyDropPercent -ge 15.0)
    $retain = $complianceOk -and $gainOk
    return [pscustomobject]@{
      from_layer = $Prev.layer
      to_layer = $Curr.layer
      delta_success_pp = $deltaSuccess
      latency_p95_drop_percent = $latencyDropPercent
      compliance_delta_pp = $complianceDelta
      gain_ok = $gainOk
      compliance_ok = $complianceOk
      retain = $retain
      fallback = (-not $retain)
      reason = if ($retain) { "retain" } elseif (-not $complianceOk) { "compliance_regressed" } else { "insufficient_gain" }
    }
  }

  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) { New-Item -ItemType Directory -Path $runtimeDir | Out-Null }
  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $baseCfg = Join-Path $repoRoot "deploy\config\autoloop.baseline_v0.toml"
  if (-not (Test-Path $baseCfg)) { throw "missing baseline config: $baseCfg" }
  Resolve-RunnerExe -Manifest $ManifestPath

  $cfgA0 = Join-Path $runtimeDir ("phase_d_a0_" + $stamp + ".toml")
  $cfgA1 = Join-Path $runtimeDir ("phase_d_a1_" + $stamp + ".toml")
  $cfgA2 = Join-Path $runtimeDir ("phase_d_a2_" + $stamp + ".toml")
  Write-LayerConfig -BaseConfigPath $baseCfg -OutPath $cfgA0 -Layer "A0"
  Write-LayerConfig -BaseConfigPath $baseCfg -OutPath $cfgA1 -Layer "A1"
  Write-LayerConfig -BaseConfigPath $baseCfg -OutPath $cfgA2 -Layer "A2"

  $a0 = Invoke-LayerRun -Layer "A0" -ConfigPath $cfgA0
  $a1 = Invoke-LayerRun -Layer "A1" -ConfigPath $cfgA1
  $cmpA1 = Compare-Layer -Prev $a0 -Curr $a1
  $keptA1 = if ($cmpA1.retain) { $a1 } else { $a0 }
  $a2 = Invoke-LayerRun -Layer "A2" -ConfigPath $cfgA2
  $cmpA2 = Compare-Layer -Prev $keptA1 -Curr $a2
  $keptFinal = if ($cmpA2.retain) { $a2 } else { $keptA1 }

  $report = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    phase = "D"
    split = $Split
    limit = $Limit
    repeats = $Repeats
    case_timeout_ms = $CaseTimeoutMs
    retry_count = $RetryCount
    retention_rule = "success+2pp OR latency_p95_-15% with non-decreasing compliance"
    layers = @($a0, $a1, $a2)
    decisions = @($cmpA1, $cmpA2)
    kept_layer = $keptFinal.layer
    configs = [pscustomobject]@{
      a0 = $cfgA0
      a1 = $cfgA1
      a2 = $cfgA2
    }
  }

  $outPath = Join-Path $runtimeDir ("phase_d_ablation_report_" + $stamp + ".json")
  $report | ConvertTo-Json -Depth 12 | Out-File -FilePath $outPath -Encoding utf8
  Write-Output ("PHASE_D_ABLATION_JSON=" + $outPath)

  & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action clean-old-runtime -BenchmarkKeepLatest $BenchmarkKeepLatest -Apply | Out-Null
  & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action clean-cache -Apply | Out-Null
}
finally {
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  if (-not [string]::IsNullOrWhiteSpace($runnerTempTargetDir) -and (Test-Path $runnerTempTargetDir)) {
    try { Remove-Item -LiteralPath $runnerTempTargetDir -Recurse -Force -ErrorAction SilentlyContinue } catch {}
  }
  Pop-Location
}
