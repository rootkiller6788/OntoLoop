param(
  [string]$RuntimeDir = "deploy/runtime",
  [string]$DailyReleasePackageJson = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$resolvedRuntime = if ([System.IO.Path]::IsPathRooted($RuntimeDir)) { $RuntimeDir } else { Join-Path $repoRoot $RuntimeDir }
if (-not (Test-Path $resolvedRuntime)) {
  New-Item -ItemType Directory -Path $resolvedRuntime | Out-Null
}

if ([string]::IsNullOrWhiteSpace($DailyReleasePackageJson)) {
  $DailyReleasePackageJson = Join-Path $resolvedRuntime "daily_release_package.json"
}
if (-not (Test-Path $DailyReleasePackageJson)) {
  throw "required input path not found: daily_release_package => $DailyReleasePackageJson"
}
$pkgPath = (Resolve-Path $DailyReleasePackageJson).Path
$pkg = Get-Content -Raw -Path $pkgPath | ConvertFrom-Json

$gate = $pkg.incremental_gate
if ($null -eq $gate) {
  throw "daily_release_package missing incremental_gate block"
}

$decisionRoot = [string]$gate.decision_root
$walRoot = [string]$gate.wal_root
$impactedTestsHash = [string]$gate.impacted_tests_hash
$rollbackReady = ($gate.rollback_ready -eq $true)

$allowRelease = (-not [string]::IsNullOrWhiteSpace($decisionRoot)) -and `
  (-not [string]::IsNullOrWhiteSpace($walRoot)) -and `
  (-not [string]::IsNullOrWhiteSpace($impactedTestsHash)) -and `
  $rollbackReady

$denyReasons = @()
if ([string]::IsNullOrWhiteSpace($decisionRoot)) { $denyReasons += "missing_decision_root" }
if ([string]::IsNullOrWhiteSpace($walRoot)) { $denyReasons += "missing_wal_root" }
if ([string]::IsNullOrWhiteSpace($impactedTestsHash)) { $denyReasons += "missing_impacted_tests_hash" }
if (-not $rollbackReady) { $denyReasons += "rollback_not_ready" }
if ($pkg.deny_reasons) { $denyReasons += @($pkg.deny_reasons) }
$denyReasons = @($denyReasons | Sort-Object -Unique)

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$releasePath = Join-Path $resolvedRuntime ("release-gate-" + $stamp + ".json")
$canonicalReleasePath = Join-Path $resolvedRuntime "release_gate.json"

$summary = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  release_gate_version = "v3-incremental-root-gate"
  allow_release = $allowRelease
  deny_reasons = $denyReasons
  decision = if ($allowRelease) { "pass" } else { "fail" }
  source = [pscustomobject]@{
    single_release_package = $pkgPath
  }
  incremental_gate = [pscustomobject]@{
    decision_root = $decisionRoot
    wal_root = $walRoot
    impacted_tests_hash = $impactedTestsHash
    rollback_ready = $rollbackReady
  }
  checks = @(
    [pscustomobject]@{
      stage = "L4"
      check_id = "decision_root_present"
      passed = (-not [string]::IsNullOrWhiteSpace($decisionRoot))
      severity = if (-not [string]::IsNullOrWhiteSpace($decisionRoot)) { "info" } else { "blocker" }
      deny_reason = if (-not [string]::IsNullOrWhiteSpace($decisionRoot)) { $null } else { "missing_decision_root" }
      evidence_ref = $null
      replay_fp = $null
      duration_ms = 0
    },
    [pscustomobject]@{
      stage = "L4"
      check_id = "wal_root_present"
      passed = (-not [string]::IsNullOrWhiteSpace($walRoot))
      severity = if (-not [string]::IsNullOrWhiteSpace($walRoot)) { "info" } else { "blocker" }
      deny_reason = if (-not [string]::IsNullOrWhiteSpace($walRoot)) { $null } else { "missing_wal_root" }
      evidence_ref = $null
      replay_fp = $null
      duration_ms = 0
    },
    [pscustomobject]@{
      stage = "L4"
      check_id = "impacted_tests_hash_present"
      passed = (-not [string]::IsNullOrWhiteSpace($impactedTestsHash))
      severity = if (-not [string]::IsNullOrWhiteSpace($impactedTestsHash)) { "info" } else { "blocker" }
      deny_reason = if (-not [string]::IsNullOrWhiteSpace($impactedTestsHash)) { $null } else { "missing_impacted_tests_hash" }
      evidence_ref = $null
      replay_fp = $null
      duration_ms = 0
    },
    [pscustomobject]@{
      stage = "L4"
      check_id = "rollback_ready"
      passed = $rollbackReady
      severity = if ($rollbackReady) { "info" } else { "blocker" }
      deny_reason = if ($rollbackReady) { $null } else { "rollback_not_ready" }
      evidence_ref = $null
      replay_fp = $null
      duration_ms = 0
    }
  )
}

$summary | ConvertTo-Json -Depth 10 | Out-File -FilePath $releasePath -Encoding utf8
$summary | ConvertTo-Json -Depth 10 | Out-File -FilePath $canonicalReleasePath -Encoding utf8
Write-Output ("allow_release=" + ($(if ($allowRelease) { "true" } else { "false" })))
Write-Output ("RELEASE_GATE_JSON=" + $releasePath)
Write-Output ("RELEASE_GATE_CANONICAL_JSON=" + $canonicalReleasePath)
