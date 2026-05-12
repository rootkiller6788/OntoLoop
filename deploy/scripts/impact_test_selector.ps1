param(
  [string]$RepoRoot = "",
  [string]$ChangedFilesPath = "",
  [string]$OutputPath = "",
  [switch]$PrintEnv
)

$ErrorActionPreference = "Stop"
$root = if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
  (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
} else {
  if ([System.IO.Path]::IsPathRooted($RepoRoot)) { $RepoRoot } else { (Resolve-Path $RepoRoot).Path }
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

function Add-ChecksForModule {
  param(
    [string]$ModuleName,
    [System.Collections.Generic.HashSet[string]]$CheckSet
  )
  $map = @{
    runtime = @(
      "cargo-check", "e2e-intent-execute-verify-persist-replay", "e2e-compiler-executor-verifier-closed-loop",
      "e2e-no-bypass-kernel", "e2e-no-bypass-mediator", "e2e-no-bypass-gate", "admission-tristate-matrix"
    )
    security = @(
      "d10-d11-security-governance-gate-suite", "e2e-no-bypass-static-scan-all-domains",
      "artifact-gate-write-evidence-required", "artifact-gate-fake-success-rejected",
      "waltx-production-write-minimal-e2e", "config-doctor-bad-config-blocked-e2e"
    )
    storage = @(
      "d12-storage-postgres-wal-dualwrite-replay", "d14-storage-cutover-acceptance",
      "waltx-production-write-minimal-e2e"
    )
    frontend = @("frontend-cli-acceptance")
    signal = @("signal-acceptance")
    sandbox = @("sandbox-acceptance")
    evolution = @("pevo-evolution-acceptance")
    ops = @("ops-acceptance", "d12-ops-productized-acceptance", "d46-slo-acceptance")
    benchmark = @("d13-smoke-benchmark-always", "d13-full-benchmark-daily")
    recovery = @("recovery-drill-chaos-recorded", "recovery-drill-mttr-recorded")
  }
  if (-not $map.ContainsKey($ModuleName)) { return }
  foreach ($checkId in $map[$ModuleName]) {
    [void]$CheckSet.Add($checkId)
  }
}

$changedFiles = @()
if (-not [string]::IsNullOrWhiteSpace($ChangedFilesPath) -and (Test-Path $ChangedFilesPath)) {
  $changedFiles = @(
    Get-Content -LiteralPath $ChangedFilesPath |
      ForEach-Object { [string]$_ } |
      Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
  )
} else {
  Push-Location $root
  try {
    $changedFiles = @(
      & git diff --name-only HEAD~1..HEAD 2>$null |
        ForEach-Object { [string]$_ } |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    )
    if ($LASTEXITCODE -ne 0 -or $changedFiles.Count -eq 0) {
      $changedFiles = @(
        & git diff --name-only 2>$null |
          ForEach-Object { [string]$_ } |
          Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
      )
    }
  } finally {
    Pop-Location
  }
}

$moduleSet = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
foreach ($file in $changedFiles) {
  if ($file -match '^src/runtime/' -or $file -match '^src/contracts/' -or $file -match '^src/agent/' -or $file -match '^src/command_dispatch\.rs$') {
    [void]$moduleSet.Add("runtime")
  }
  if ($file -match '^src/(security|governance|policy|relation|admission)/' -or $file -match '^tests/.*(no_bypass|artifact|admission|waltx|policy)') {
    [void]$moduleSet.Add("security")
  }
  if ($file -match '^src/(storage|wal|state_store)/' -or $file -match '^tests/.*(storage|wal|dualwrite|cutover)') {
    [void]$moduleSet.Add("storage")
  }
  if ($file -match '^apps/ontoloop-cli/' -or $file -match '^src/frontend_bridge\.rs$' -or $file -match '^tests/.*frontend') {
    [void]$moduleSet.Add("frontend")
  }
  if ($file -match '^src/(observability|signal)/' -or $file -match '^tests/.*signal') {
    [void]$moduleSet.Add("signal")
  }
  if ($file -match '^src/runtime/sandbox' -or $file -match '^tests/.*sandbox') {
    [void]$moduleSet.Add("sandbox")
  }
  if ($file -match '^src/(evolution|rollout|promotion)/' -or $file -match '^tests/.*(evolution|rollout|promotion)') {
    [void]$moduleSet.Add("evolution")
  }
  if ($file -match '^deploy/scripts/' -or $file -match '^src/(ops|doctor|health)/' -or $file -match '^tests/.*(ops|slo|doctor|health)') {
    [void]$moduleSet.Add("ops")
  }
  if ($file -match '^src/(benchmark|harness|loop)/' -or $file -match '^tests/.*benchmark') {
    [void]$moduleSet.Add("benchmark")
  }
}

$selectedChecks = New-Object 'System.Collections.Generic.HashSet[string]' ([System.StringComparer]::OrdinalIgnoreCase)
$alwaysRun = @(
  "pre-rollout-config-doctor-gate",
  "cargo-check",
  "e2e-no-bypass-static-scan-all-domains",
  "artifact-gate-write-evidence-required",
  "d12-storage-postgres-wal-dualwrite-replay",
  "sandbox-acceptance",
  "signal-acceptance",
  "frontend-cli-acceptance",
  "pevo-evolution-acceptance",
  "d14-rollout-final",
  "version-a-acceptance",
  "d13-smoke-benchmark-always",
  "__RUN_ROLLOUT__"
)
foreach ($id in $alwaysRun) { [void]$selectedChecks.Add($id) }
foreach ($module in $moduleSet) { Add-ChecksForModule -ModuleName $module -CheckSet $selectedChecks }

$sortedChecks = @($selectedChecks | Sort-Object)
$impactedHash = Get-StringSha256 -Text ($sortedChecks -join "|")

$report = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  mode = "impacted_only"
  changed_files = $changedFiles
  affected_modules = @($moduleSet | Sort-Object)
  impacted_checks = $sortedChecks
  impacted_tests_hash = $impactedHash
}

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
  $runtimeDir = Join-Path $root "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) { New-Item -ItemType Directory -Path $runtimeDir | Out-Null }
  $OutputPath = Join-Path $runtimeDir ("impact-test-selector-" + (Get-Date -Format "yyyyMMdd-HHmmss") + ".json")
}
$report | ConvertTo-Json -Depth 8 | Out-File -FilePath $OutputPath -Encoding utf8

Write-Output ("IMPACT_SELECTOR_JSON=" + $OutputPath)
Write-Output ("IMPACTED_TESTS_HASH=" + $impactedHash)
if ($PrintEnv) {
  Write-Output ("IMPACTED_CHECK_IDS=" + ($sortedChecks -join ","))
}
