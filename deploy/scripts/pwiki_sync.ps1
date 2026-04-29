param(
  [string]$RepoRoot = "D:\AutoLoop\autoloop-app",
  [string]$Session = "sync:pwiki",
  [string]$Mode = "dry-run",
  [int]$BatchNo = 1,
  [string]$TargetsJson = "[]"
)

$ErrorActionPreference = "Stop"

$targets = @()
try {
  $parsed = $TargetsJson | ConvertFrom-Json
  if ($parsed -is [System.Array]) {
    $targets = @($parsed)
  } elseif ($null -ne $parsed) {
    $targets = @($parsed.ToString())
  }
} catch {
  $targets = @()
}

$result = [ordered]@{
  script = "pwiki_sync.ps1"
  repo_root = $RepoRoot
  session = $Session
  mode = $Mode
  batch_no = $BatchNo
  target_count = $targets.Count
  targets = $targets
  operations = @(
    "batch_ingest_precheck",
    "batch_heal_queue_probe",
    "batch_refresh_plan"
  )
  executed = $false
  steps = @()
}

if ($Mode -ieq "dry-run") {
  $result.steps += [ordered]@{
    step = "dry_run_only"
    detail = "No runtime command executed"
  }
  $result | ConvertTo-Json -Depth 8
  exit 0
}

$manifestPath = Join-Path $RepoRoot "Cargo.toml"
$step1 = & cargo run --manifest-path $manifestPath -- --session $Session memory patch queue 2>&1
$result.steps += [ordered]@{
  step = "memory_patch_queue"
  detail = ($step1 | Out-String).Trim()
}

$step2 = & cargo run --manifest-path $manifestPath -- --session $Session memory compiler status --repo-root $RepoRoot 2>&1
$result.steps += [ordered]@{
  step = "memory_compiler_status"
  detail = ($step2 | Out-String).Trim()
}

$step3 = & cargo run --manifest-path $manifestPath -- --session $Session memory graph export --repo-root $RepoRoot --clean --report 2>&1
$result.steps += [ordered]@{
  step = "memory_graph_export"
  detail = ($step3 | Out-String).Trim()
}

$result.executed = $true
$result | ConvertTo-Json -Depth 8
