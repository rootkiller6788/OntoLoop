param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "soak-stability",
  [int]$DurationHours = 6,
  [int]$CaseTimeoutSec = 90,
  [int]$MaxRetriesPerCase = 1
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
$logPath = Join-Path $runtimeDir ("soak-stability-" + $stamp + ".log")
$jsonPath = Join-Path $runtimeDir ("soak-stability-" + $stamp + ".json")
$endAt = (Get-Date).AddHours([Math]::Max(1, [Math]::Min(8, $DurationHours)))
$caseIdx = 0
$recoveries = 0
$failures = 0
$steps = @()
$trackedPids = New-Object System.Collections.Generic.List[int]

function Get-P95 {
  param([double[]]$Values)
  if ($null -eq $Values -or $Values.Count -eq 0) { return 0.0 }
  $sorted = $Values | Sort-Object
  $idx = [Math]::Ceiling($sorted.Count * 0.95) - 1
  if ($idx -lt 0) { $idx = 0 }
  if ($idx -ge $sorted.Count) { $idx = $sorted.Count - 1 }
  return [double]$sorted[$idx]
}

function Invoke-SoakCase {
  param(
    [string]$SessionId,
    [int]$Attempt
  )
  $prompt = "Create and write a production-style HTML business page to deploy/runtime/soak-artifact-$SessionId.html. Must use tools and produce verifiable output."
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $outFile = Join-Path $runtimeDir ("soak-case-" + $SessionId + "-out.log")
  $errFile = Join-Path $runtimeDir ("soak-case-" + $SessionId + "-err.log")
  $argList = @(
    "run",
    "--manifest-path", $ManifestPath,
    "--",
    "--config", $ProdConfigPath,
    "--session", $SessionId,
    "--message", $prompt
  )
  $proc = Start-Process -FilePath "cargo" -ArgumentList $argList -RedirectStandardOutput $outFile -RedirectStandardError $errFile -PassThru -NoNewWindow
  $trackedPids.Add([int]$proc.Id) | Out-Null
  $timedOut = $false
  try {
    Wait-Process -Id $proc.Id -Timeout $CaseTimeoutSec -ErrorAction Stop
  } catch {
    $timedOut = $true
    try { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue } catch {}
    try {
      $orphans = Get-CimInstance Win32_Process | Where-Object {
        $_.Name -eq "ontoloop.exe" -and $_.CommandLine -like ("*" + $SessionId + "*")
      }
      foreach ($o in $orphans) {
        try { Stop-Process -Id $o.ProcessId -Force -ErrorAction SilentlyContinue } catch {}
      }
    } catch {}
  }
  $proc.Refresh()
  $exitCode = if ($timedOut) { 124 } else { $proc.ExitCode }
  $sw.Stop()
  $output = @()
  if (Test-Path $outFile) { $output += (Get-Content -Path $outFile -ErrorAction SilentlyContinue) }
  if (Test-Path $errFile) { $output += (Get-Content -Path $errFile -ErrorAction SilentlyContinue) }
  if ($output.Count -gt 0) {
    $output | Out-File -FilePath $logPath -Append -Encoding utf8
  }
  return [pscustomobject]@{
    attempt = $Attempt
    passed = ($exitCode -eq 0)
    exit_code = $exitCode
    duration_ms = [int64]$sw.ElapsedMilliseconds
    timed_out = $timedOut
  }
}
try {
  $targetDir = Join-Path $runtimeDir ("target-soak-" + $stamp)
  New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
  $env:CARGO_TARGET_DIR = $targetDir
  $localBaseUri = [Environment]::GetEnvironmentVariable("AUTOLOOP_LOCAL_POSTGRES_URI")
  if ([string]::IsNullOrWhiteSpace($localBaseUri)) {
    $localBaseUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod"
  }
  $schema = ("ol_soak_" + $stamp.Replace("-", "_"))
  $env:AUTOLOOP_LOCAL_POSTGRES_URI = $localBaseUri + "?options=-csearch_path%3D" + $schema + "%2Cpublic"
  $psql = Get-Command psql -ErrorAction SilentlyContinue
  if ($null -ne $psql) {
    $createSql = "CREATE SCHEMA IF NOT EXISTS " + $schema + ";"
    & $psql.Source -d $localBaseUri -v ON_ERROR_STOP=1 -c $createSql | Out-Null
  }

  while ((Get-Date) -lt $endAt) {
  $caseIdx++
  $sessionId = "$SessionPrefix-$stamp-$caseIdx"
  $casePassed = $false
  $attempts = @()
  $attempt = 0

  while ($attempt -le $MaxRetriesPerCase -and -not $casePassed) {
    $attempt++
    $result = Invoke-SoakCase -SessionId $sessionId -Attempt $attempt
    $attempts += $result
    if ($result.passed) {
      $casePassed = $true
      if ($attempt -gt 1) { $recoveries++ }
    } elseif ($attempt -le $MaxRetriesPerCase) {
      Start-Sleep -Seconds ([Math]::Min(5, [Math]::Max(1, [int]($CaseTimeoutSec / 30))))
    }
  }

  if (-not $casePassed) { $failures++ }
  $steps += [pscustomobject]@{
    case_id = $sessionId
    passed = $casePassed
    attempts = $attempts
  }
  }

  $durations = @($steps | ForEach-Object {
  $ok = $_.attempts | Where-Object { $_.passed } | Select-Object -First 1
  if ($null -ne $ok) { [double]$ok.duration_ms }
  })
  $total = [double]$steps.Count
  $errorRate = if ($total -gt 0) { [double]$failures / $total } else { 1.0 }
  $p95 = Get-P95 -Values $durations

  $summary = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  duration_hours = $DurationHours
  case_timeout_sec = $CaseTimeoutSec
  max_retries_per_case = $MaxRetriesPerCase
  totals = [pscustomobject]@{
    cases = [int]$steps.Count
    failures = [int]$failures
    recoveries = [int]$recoveries
  }
  stability = [pscustomobject]@{
    p95_latency_ms = [Math]::Round($p95, 2)
    error_rate = [Math]::Round($errorRate, 4)
    recovery_count = [int]$recoveries
  }
  steps = $steps
  log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  $scriptSucceeded = $true
  Write-Output ("SOAK_STABILITY_LOG=" + $logPath)
  Write-Output ("SOAK_STABILITY_JSON=" + $jsonPath)
} finally {
  foreach ($pid in $trackedPids) {
    try { Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue } catch {}
  }
  try {
    $orphans = Get-CimInstance Win32_Process | Where-Object {
      $_.Name -in @("ontoloop.exe","cargo.exe") -and $_.CommandLine -like ("*" + $SessionPrefix + "-" + $stamp + "*")
    }
    foreach ($o in $orphans) {
      try { Stop-Process -Id $o.ProcessId -Force -ErrorAction SilentlyContinue } catch {}
    }
  } catch {}
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
  if ($scriptSucceeded) {
    if ($null -ne $targetDir -and (Test-Path $targetDir)) {
      Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    Get-ChildItem -Path $runtimeDir -File -Filter "soak-stability-*.log" -ErrorAction SilentlyContinue |
      Where-Object { $_.LastWriteTime -lt (Get-Date).AddDays(-7) } |
      Remove-Item -Force -ErrorAction SilentlyContinue
  } else {
    $cutoff = (Get-Date).AddDays(-7)
    $diagFiles = @(
      Get-ChildItem -Path $runtimeDir -File -Filter "soak-stability-*.json" -ErrorAction SilentlyContinue
      Get-ChildItem -Path $runtimeDir -File -Filter "soak-stability-*.log" -ErrorAction SilentlyContinue
    ) | Sort-Object LastWriteTime -Descending
    $keepDiag = $diagFiles | Select-Object -First 1
    foreach ($f in $diagFiles) {
      if ($null -ne $keepDiag -and $f.FullName -eq $keepDiag.FullName) {
        if ($f.LastWriteTime -lt $cutoff) {
          Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue
        }
        continue
      }
      Remove-Item -LiteralPath $f.FullName -Force -ErrorAction SilentlyContinue
    }
    $targets = Get-ChildItem -Path $runtimeDir -Directory -Filter "target-soak-*" -ErrorAction SilentlyContinue |
      Sort-Object LastWriteTime -Descending
    $keepOne = $targets | Select-Object -First 1
    foreach ($d in $targets) {
      if ($null -ne $keepOne -and $d.FullName -eq $keepOne.FullName) {
        if ($d.LastWriteTime -lt $cutoff) {
          Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
        continue
      }
      Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
  }
}
