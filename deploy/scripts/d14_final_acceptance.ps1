param(
  [string]$RuntimeDir = "deploy/runtime",
  [string]$Week6Json = "",
  [string]$DailyReleasePackageJson = "",
  [string]$ReleaseGateJson = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$resolvedRuntime = if ([System.IO.Path]::IsPathRooted($RuntimeDir)) { $RuntimeDir } else { Join-Path $repoRoot $RuntimeDir }
if (-not (Test-Path $resolvedRuntime)) {
  New-Item -ItemType Directory -Path $resolvedRuntime | Out-Null
}

function Resolve-InputPath {
  param(
    [string]$Name,
    [string]$Candidate,
    [string]$Fallback
  )
  $path = if (-not [string]::IsNullOrWhiteSpace($Candidate)) { $Candidate } else { (Join-Path $resolvedRuntime $Fallback) }
  if (-not (Test-Path $path)) {
    throw ("d14 final acceptance missing required input: " + $Name + " => " + $path)
  }
  return (Resolve-Path $path).Path
}

$week6Path = Resolve-InputPath -Name "week6_full_acceptance" -Candidate $Week6Json -Fallback "week6_full_acceptance.json"
$dailyPath = Resolve-InputPath -Name "daily_release_package" -Candidate $DailyReleasePackageJson -Fallback "daily_release_package.json"
$gatePath = Resolve-InputPath -Name "release_gate" -Candidate $ReleaseGateJson -Fallback "release_gate.json"

$week6 = Get-Content -Raw -Path $week6Path | ConvertFrom-Json
$daily = Get-Content -Raw -Path $dailyPath | ConvertFrom-Json
$gate = Get-Content -Raw -Path $gatePath | ConvertFrom-Json

$versionAPath = [string]$week6.version_a_report
$d14Path = [string]$week6.d14_rollout_report
if ([string]::IsNullOrWhiteSpace($versionAPath) -or -not (Test-Path $versionAPath)) {
  throw "d14 final acceptance missing version_a_report path from week6"
}
if ([string]::IsNullOrWhiteSpace($d14Path) -or -not (Test-Path $d14Path)) {
  throw "d14 final acceptance missing d14_rollout_report path from week6"
}

$versionA = Get-Content -Raw -Path $versionAPath | ConvertFrom-Json
$d14 = Get-Content -Raw -Path $d14Path | ConvertFrom-Json
$checks = @($week6.checks)
$commands = @($d14.commands)

function Get-CheckPassed {
  param([string]$CheckId)
  $item = $checks | Where-Object { $_.check_id -eq $CheckId } | Select-Object -First 1
  if ($null -eq $item) { return $false }
  return ($item.passed -eq $true)
}

function Has-RolloutCommandPassed {
  param([string]$Name)
  $cmd = $commands | Where-Object { $_.name -eq $Name } | Select-Object -First 1
  if ($null -eq $cmd) { return $false }
  return ($cmd.passed -eq $true)
}

$contractPassed = ($versionA.all_passed -eq $true)
$gatePassed = ($daily.allow_release -eq $true) -and ($gate.allow_release -eq $true)
$leasePassed = Get-CheckPassed -CheckId "no-bypass-kernel-mediator-static"
$reviewGatePassed = Get-CheckPassed -CheckId "artifact-hard-gate-and-proof"
$walPassed = (Get-CheckPassed -CheckId "waltx-production-write-minimal-e2e") -and (Get-CheckPassed -CheckId "d12-storage-postgres-wal-dualwrite-replay")
$ontoEventPassed = (Get-CheckPassed -CheckId "signal-acceptance") -and (Get-CheckPassed -CheckId "frontend-cli-acceptance")
$releasePackagePassed = Test-Path $dailyPath

$rolloutShadow = (Has-RolloutCommandPassed -Name "rollout-shadow-status") -and (Has-RolloutCommandPassed -Name "rollout-shadow-health")
$rollout10 = (Has-RolloutCommandPassed -Name "rollout-canary10-status") -and (Has-RolloutCommandPassed -Name "rollout-canary10-health")
$rollout30 = (Has-RolloutCommandPassed -Name "rollout-canary30-status") -and (Has-RolloutCommandPassed -Name "rollout-canary30-health")
$rolloutFull = (Has-RolloutCommandPassed -Name "rollout-full-status") -and (Has-RolloutCommandPassed -Name "rollout-full-health")
$rollbackEvidence = ($d14.rollback.triggered -eq $true) -and ($d14.rollback.status_ok -eq $true) -and ($d14.rollback.health_ok -eq $true)
$rolloutChainPassed = $rolloutShadow -and $rollout10 -and $rollout30 -and $rolloutFull -and $rollbackEvidence

$denyReasons = @()
if (-not $contractPassed) { $denyReasons += "contract_failed" }
if (-not $gatePassed) { $denyReasons += "gate_failed" }
if (-not $leasePassed) { $denyReasons += "lease_guard_missing_or_failed" }
if (-not $reviewGatePassed) { $denyReasons += "reviewgate_failed" }
if (-not $walPassed) { $denyReasons += "wal_failed" }
if (-not $ontoEventPassed) { $denyReasons += "ontoevent_chain_incomplete" }
if (-not $releasePackagePassed) { $denyReasons += "release_package_missing" }
if (-not $rolloutChainPassed) { $denyReasons += "rollout_chain_incomplete" }

$allPassed = $denyReasons.Count -eq 0
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$outPath = Join-Path $resolvedRuntime ("d14-final-acceptance-" + $stamp + ".json")
$canonicalPath = Join-Path $resolvedRuntime "d14_final_acceptance.json"

$summary = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  version = "d14-final/v1"
  all_passed = $allPassed
  deny_reasons = $denyReasons
  inputs = [pscustomobject]@{
    week6 = $week6Path
    daily_release_package = $dailyPath
    release_gate = $gatePath
    version_a = $versionAPath
    d14_rollout = $d14Path
  }
  chain = [pscustomobject]@{
    contract = $contractPassed
    gate = $gatePassed
    lease = $leasePassed
    reviewgate = $reviewGatePassed
    wal = $walPassed
    ontoevent = $ontoEventPassed
    release_package = $releasePackagePassed
    rollout = [pscustomobject]@{
      shadow = $rolloutShadow
      canary10 = $rollout10
      canary30 = $rollout30
      full = $rolloutFull
      rollback = $rollbackEvidence
      all = $rolloutChainPassed
    }
  }
}

$summary | ConvertTo-Json -Depth 12 | Out-File -FilePath $outPath -Encoding utf8
$summary | ConvertTo-Json -Depth 12 | Out-File -FilePath $canonicalPath -Encoding utf8

Write-Output ("D14_FINAL_ACCEPTANCE_JSON=" + $outPath)
Write-Output ("D14_FINAL_ACCEPTANCE_CANONICAL_JSON=" + $canonicalPath)
if (-not $allPassed) {
  throw "d14 final acceptance failed"
}
