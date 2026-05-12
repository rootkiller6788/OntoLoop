param(
  [string]$RuntimeDir = "deploy/runtime",
  [string]$Week6Json = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$resolvedRuntime = if ([System.IO.Path]::IsPathRooted($RuntimeDir)) { $RuntimeDir } else { Join-Path $repoRoot $RuntimeDir }
if (-not (Test-Path $resolvedRuntime)) {
  New-Item -ItemType Directory -Path $resolvedRuntime | Out-Null
}

function Resolve-RequiredInput {
  param(
    [string]$Name,
    [string]$PathValue
  )
  if ([string]::IsNullOrWhiteSpace($PathValue)) { throw "required input missing: $Name" }
  if (-not (Test-Path $PathValue)) { throw "required input path not found: $Name => $PathValue" }
  return (Resolve-Path $PathValue).Path
}

function Get-StringSha256 {
  param([string]$Text)
  $sha = [System.Security.Cryptography.SHA256]::Create()
  try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
    $hashBytes = $sha.ComputeHash($bytes)
    return ([System.BitConverter]::ToString($hashBytes) -replace "-", "").ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }
}

function Collect-WalIds {
  param([string[]]$Paths)
  $ids = New-Object 'System.Collections.Generic.HashSet[string]'
  foreach ($p in $Paths) {
    if ([string]::IsNullOrWhiteSpace($p) -or -not (Test-Path $p)) { continue }
    $lines = Get-Content -LiteralPath $p -ErrorAction SilentlyContinue
    foreach ($line in $lines) {
      $m1 = [System.Text.RegularExpressions.Regex]::Matches($line, '"wal_tx_id"\s*:\s*"([^"]+)"')
      foreach ($m in $m1) { [void]$ids.Add($m.Groups[1].Value) }
      $m2 = [System.Text.RegularExpressions.Regex]::Matches($line, '"wal_id"\s*:\s*"([^"]+)"')
      foreach ($m in $m2) { [void]$ids.Add($m.Groups[1].Value) }
    }
  }
  return @($ids | Sort-Object)
}

$week6Path = Resolve-RequiredInput -Name "week6" -PathValue $Week6Json
$week6 = Get-Content -Raw -Path $week6Path | ConvertFrom-Json

$d14Path = [string]$week6.d14_rollout_report
$rollbackPath = [string]$week6.rollback_drill_report
$faultPath = [string]$week6.fault_injection_report
$versionAPath = [string]$week6.version_a_report

$week6Checks = @($week6.checks)
$impactedTestsHash = ""
if ($null -ne $week6.PSObject.Properties["impacted_tests_hash"] -and -not [string]::IsNullOrWhiteSpace([string]$week6.impacted_tests_hash)) {
  $impactedTestsHash = [string]$week6.impacted_tests_hash
} else {
  $impactedChecks = @(
    $week6Checks |
      Where-Object { $_.stage -in @("L1", "L2", "L3") } |
      ForEach-Object { [string]$_.check_id }
  )
  $impactedTestsHash = Get-StringSha256 -Text (($impactedChecks | Sort-Object -Unique) -join "|")
}

$rollbackReady = $false
if (-not [string]::IsNullOrWhiteSpace($d14Path) -and (Test-Path $d14Path)) {
  $d14 = Get-Content -Raw -Path $d14Path | ConvertFrom-Json
  $rollbackReady = ($d14.rollback.triggered -eq $true) -and ($d14.rollback.status_ok -eq $true) -and ($d14.rollback.health_ok -eq $true)
}
if (-not $rollbackReady -and -not [string]::IsNullOrWhiteSpace($rollbackPath) -and (Test-Path $rollbackPath)) {
  $rb = Get-Content -Raw -Path $rollbackPath | ConvertFrom-Json
  $rollbackReady = ($rb.all_passed -eq $true)
}

$decisionSeed = "{0}|{1}|{2}|{3}" -f `
  ([string]$week6.generated_at), `
  ([string]$week6.all_passed), `
  ([string]$versionAPath), `
  $impactedTestsHash
$decisionRoot = Get-StringSha256 -Text $decisionSeed

$walIds = Collect-WalIds -Paths @($week6Path, $d14Path, $rollbackPath, $faultPath)
$walRoot = Get-StringSha256 -Text ($walIds -join "|")

$incrementalAllow = (-not [string]::IsNullOrWhiteSpace($decisionRoot)) -and `
  (-not [string]::IsNullOrWhiteSpace($walRoot)) -and `
  (-not [string]::IsNullOrWhiteSpace($impactedTestsHash)) -and `
  ($rollbackReady -eq $true)

$denyReasons = @()
if (-not $rollbackReady) { $denyReasons += "rollback_not_ready" }
if ([string]::IsNullOrWhiteSpace($decisionRoot)) { $denyReasons += "missing_decision_root" }
if ([string]::IsNullOrWhiteSpace($walRoot)) { $denyReasons += "missing_wal_root" }
if ([string]::IsNullOrWhiteSpace($impactedTestsHash)) { $denyReasons += "missing_impacted_tests_hash" }

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$outPath = Join-Path $resolvedRuntime ("daily-release-package-" + $stamp + ".json")
$canonicalPath = Join-Path $resolvedRuntime "daily_release_package.json"

$summary = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  package_version = "v3-incremental-root-gate"
  allow_release = $incrementalAllow
  deny_reasons = $denyReasons
  release_decision = if ($incrementalAllow) { "pass" } else { "fail" }
  incremental_gate = [pscustomobject]@{
    decision_root = $decisionRoot
    wal_root = $walRoot
    impacted_tests_hash = $impactedTestsHash
    rollback_ready = $rollbackReady
  }
  source_reports = [pscustomobject]@{
    week6 = $week6Path
    d14 = $d14Path
    rollback_drill = $rollbackPath
    fault_drill = $faultPath
    version_a = $versionAPath
  }
}

$summary | ConvertTo-Json -Depth 10 | Out-File -FilePath $outPath -Encoding utf8
$summary | ConvertTo-Json -Depth 10 | Out-File -FilePath $canonicalPath -Encoding utf8
Write-Output ("DAILY_RELEASE_PACKAGE_JSON=" + $outPath)
Write-Output ("DAILY_RELEASE_PACKAGE_CANONICAL_JSON=" + $canonicalPath)
