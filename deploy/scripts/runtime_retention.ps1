param(
  [ValidateSet("clean-cache", "archive-evidence", "clean-old-runtime", "all")]
  [string]$Action = "all",
  [int]$RetentionDays = 7,
  [int]$BenchmarkKeepLatest = 8,
  [switch]$Apply,
  [switch]$PersistReport
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..\..")).Path
$runtimeDir = Join-Path $repoRoot "deploy\runtime"

if (-not (Test-Path $runtimeDir)) {
  New-Item -ItemType Directory -Path $runtimeDir | Out-Null
}

function Invoke-Delete {
  param([string]$Path)
  if (-not (Test-Path $Path)) { return $false }
  if ($Apply) {
    try {
      Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
    } catch {
      return $false
    }
    if (Test-Path $Path) {
      return $false
    }
  }
  return $true
}

function Collect-Files {
  param([string[]]$Patterns)
  $files = @()
  foreach ($pattern in $Patterns) {
    $files += Get-ChildItem -Path $runtimeDir -File -Filter $pattern -ErrorAction SilentlyContinue
  }
  return @($files | Sort-Object FullName -Unique)
}

function Prune-LatestByPattern {
  param(
    [string]$Pattern,
    [int]$KeepLatest,
    [string]$Type
  )
  $items = Get-ChildItem -Path $runtimeDir -File -Filter $Pattern -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending
  if ($null -eq $items -or $items.Count -le $KeepLatest) { return }
  $toDelete = $items | Select-Object -Skip $KeepLatest
  foreach ($file in $toDelete) {
    if (Invoke-Delete -Path $file.FullName) {
      $report.deleted += [ordered]@{
        type = $Type
        path = $file.FullName
        last_write_time = $file.LastWriteTime.ToString("s")
      }
    }
  }
}

$report = [ordered]@{
  generated_at = (Get-Date).ToString("s")
  repo_root = $repoRoot
  runtime_dir = $runtimeDir
  action = $Action
  retention_days = $RetentionDays
  apply = [bool]$Apply
  deleted = @()
  kept = @()
  summary_files = @()
  errors = @()
}

try {
  if ($Action -in @("clean-cache", "all")) {
    $cacheTargets = @(
      (Join-Path $repoRoot "target"),
      (Join-Path $repoRoot "autoloop-app\target")
    )
    foreach ($dir in $cacheTargets) {
      if (Invoke-Delete -Path $dir) {
        $report.deleted += [ordered]@{ type = "cache_dir"; path = $dir }
      }
    }

    $runtimeTargetDirs = Get-ChildItem -Path $runtimeDir -Directory -Filter "target-*" -ErrorAction SilentlyContinue
    foreach ($dir in $runtimeTargetDirs) {
      if (Invoke-Delete -Path $dir.FullName) {
        $report.deleted += [ordered]@{ type = "runtime_target"; path = $dir.FullName }
      }
    }
    $namedRuntimeTargets = @(
      (Join-Path $runtimeDir "target-final-check"),
      (Join-Path $runtimeDir "target-benchmark-shared"),
      (Join-Path $runtimeDir "target-benchmark-compare")
    )
    foreach ($dir in $namedRuntimeTargets) {
      if (Invoke-Delete -Path $dir) {
        $report.deleted += [ordered]@{ type = "runtime_target_named"; path = $dir }
      }
    }

    $incrementalDirs = Get-ChildItem -Path (Join-Path $repoRoot "autoloop-app") -Directory -Recurse -Filter "incremental" -ErrorAction SilentlyContinue
    foreach ($dir in $incrementalDirs) {
      if (Invoke-Delete -Path $dir.FullName) {
        $report.deleted += [ordered]@{ type = "incremental_dir"; path = $dir.FullName }
      }
    }

    $pdbFiles = Get-ChildItem -Path (Join-Path $repoRoot "autoloop-app") -File -Recurse -Filter "*.pdb" -ErrorAction SilentlyContinue
    foreach ($file in $pdbFiles) {
      if (Invoke-Delete -Path $file.FullName) {
        $report.deleted += [ordered]@{ type = "pdb_file"; path = $file.FullName }
      }
    }

    $tempRoot = [System.IO.Path]::GetTempPath()
    if (-not [string]::IsNullOrWhiteSpace($tempRoot) -and (Test-Path $tempRoot)) {
      $tempTargets = Get-ChildItem -Path $tempRoot -Directory -Filter "autoloop-target-*" -ErrorAction SilentlyContinue
      foreach ($dir in $tempTargets) {
        if (Invoke-Delete -Path $dir.FullName) {
          $report.deleted += [ordered]@{ type = "temp_target"; path = $dir.FullName }
        }
      }
    }
  }

  if ($Action -in @("archive-evidence", "all")) {
    $keepCandidates = @()
    $releaseCore = @(
      "release_gate.json",
      "daily_release_package.json",
      "proof_ledger.jsonl"
    )
    foreach ($name in $releaseCore) {
      $path = Join-Path $runtimeDir $name
      if (Test-Path $path) {
        $keepCandidates += Get-Item -LiteralPath $path
      }
    }

    $keepFiles = @($keepCandidates | Sort-Object FullName -Unique)
    $keepMap = @{}
    foreach ($file in $keepFiles) { $keepMap[$file.FullName] = $true }

    foreach ($file in $keepFiles) {
      $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant()
      $record = [ordered]@{
        name = $file.Name
        path = $file.FullName
        size_bytes = [int64]$file.Length
        sha256 = $hash
        last_write_time = $file.LastWriteTime.ToString("s")
      }
      $report.kept += $record
    }

    $allRuntimeFiles = Get-ChildItem -Path $runtimeDir -File -ErrorAction SilentlyContinue
    foreach ($file in $allRuntimeFiles) {
      if ($keepMap.ContainsKey($file.FullName)) { continue }
      if (Invoke-Delete -Path $file.FullName) {
        $report.deleted += [ordered]@{ type = "runtime_non_release_file"; path = $file.FullName }
      }
    }

    # Also clear runtime subdirectories to prevent long-term log/cache accumulation.
    $runtimeDirs = Get-ChildItem -Path $runtimeDir -Directory -ErrorAction SilentlyContinue
    foreach ($dir in $runtimeDirs) {
      if (Invoke-Delete -Path $dir.FullName) {
        $report.deleted += [ordered]@{ type = "runtime_subdir"; path = $dir.FullName }
      }
    }
  }

  if ($Action -in @("clean-old-runtime", "all")) {
    $cutoff = (Get-Date).AddDays(-1 * $RetentionDays)
    $detailPatterns = @(
      "week6-acceptance-*.log",
      "week6-acceptance-*.json",
      "week6-diagnostic-*.json",
      "d13-benchmark-raw-*.json",
      "d13-relation-status-*.json",
      "d13-relation-collect-*.log",
      "fault-injection-daily-*.json",
      "rollback-daily-drill-*.json"
    )
    $detailFiles = Collect-Files -Patterns $detailPatterns
    foreach ($file in $detailFiles) {
      if ($file.LastWriteTime -lt $cutoff) {
        if (Invoke-Delete -Path $file.FullName) {
          $report.deleted += [ordered]@{
            type = "runtime_old_detail"
            path = $file.FullName
            last_write_time = $file.LastWriteTime.ToString("s")
          }
        }
      }
    }

    # Keep runtime benchmark intermediates bounded by count, not only by age.
    $keepLatest = [Math]::Max(1, $BenchmarkKeepLatest)
    Prune-LatestByPattern -Pattern "d13-realbiz-benchmark-*.json" -KeepLatest $keepLatest -Type "runtime_benchmark_overflow"
    Prune-LatestByPattern -Pattern "d13-benchmark-raw-*.json" -KeepLatest $keepLatest -Type "runtime_benchmark_overflow"
    Prune-LatestByPattern -Pattern "benchmark_v1_eval_*.json" -KeepLatest $keepLatest -Type "runtime_benchmark_overflow"
    Prune-LatestByPattern -Pattern "benchmark_v1_compare_*.json" -KeepLatest $keepLatest -Type "runtime_benchmark_overflow"
  }
}
catch {
  $report.errors += $_.Exception.Message
  throw
}
finally {
  if ($PersistReport) {
    $retentionDir = Join-Path $runtimeDir "retention"
    if (-not (Test-Path $retentionDir)) {
      New-Item -ItemType Directory -Path $retentionDir | Out-Null
    }
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $reportPath = Join-Path $retentionDir ("runtime-retention-report-" + $stamp + ".json")
    ($report | ConvertTo-Json -Depth 8) | Out-File -FilePath $reportPath -Encoding utf8
    Write-Output ("RUNTIME_RETENTION_REPORT=" + $reportPath)
  } else {
    Write-Output "RUNTIME_RETENTION_REPORT=inline"
    Write-Output (($report | ConvertTo-Json -Depth 8))
  }
}
