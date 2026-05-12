param(
  [string]$ManifestPath = ".\Cargo.toml"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

function Invoke-TestCase {
  param(
    [string]$Name,
    [string]$Command
  )
  & powershell -NoProfile -Command $Command | Out-Null
  return [pscustomobject]@{
    name = $Name
    passed = ($LASTEXITCODE -eq 0)
    command = $Command
  }
}

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }
  $jsonPath = Join-Path $runtimeDir "version-a-acceptance.json"
  $stamp = Get-Date -Format "yyyy-MM-ddTHH:mm:ssK"

  $tests = @()
  $tests += Invoke-TestCase -Name "constraint_shield_pass" -Command "cargo test --manifest-path `"$ManifestPath`" --lib constraint_patterns_block_unsafe_payload"
  $tests += Invoke-TestCase -Name "task_tree_valid" -Command "cargo test --manifest-path `"$ManifestPath`" --lib e2r_gate_requires_dependencies_accepted_before_commit"
  $tests += Invoke-TestCase -Name "ranking_route_valid" -Command "cargo test --manifest-path `"$ManifestPath`" --lib ranking_is_stable_and_prefers_higher_score"
  $tests += Invoke-TestCase -Name "bandit_update_valid" -Command "cargo test --manifest-path `"$ManifestPath`" --lib updates_alpha_beta_posterior_counts"
  $tests += Invoke-TestCase -Name "completed_not_accepted_without_review" -Command "cargo test --manifest-path `"$ManifestPath`" --lib e2r_gate_enforces_rejected_iterate_and_accept_commit"
  $tests += Invoke-TestCase -Name "evidence_commit_valid" -Command "cargo test --manifest-path `"$ManifestPath`" --lib relation_writes_are_restricted_to_relation_facade"
  $tests += Invoke-TestCase -Name "wal_atomic_valid" -Command "cargo test --manifest-path D:\AutoLoop\autoloop-app\autoloop-postgres-adapter\Cargo.toml atomic_relation_bundle_rolls_back_on_failpoint"
  $tests += Invoke-TestCase -Name "replay_smoke_pass" -Command "cargo test --manifest-path `"$ManifestPath`" --lib wal_tx_envelope_roundtrip_is_stable"

  $index = @{}
  foreach ($item in $tests) {
    $index[$item.name] = [bool]$item.passed
  }

  $summary = [pscustomobject]@{
    generated_at = $stamp
    version = "version-a/v1"
    all_passed = ($tests | Where-Object { -not $_.passed }).Count -eq 0
    constraint_shield_pass = $index["constraint_shield_pass"]
    task_tree_valid = $index["task_tree_valid"]
    ranking_route_valid = $index["ranking_route_valid"]
    bandit_update_valid = $index["bandit_update_valid"]
    completed_not_accepted_without_review = $index["completed_not_accepted_without_review"]
    evidence_commit_valid = $index["evidence_commit_valid"]
    wal_atomic_valid = $index["wal_atomic_valid"]
    replay_smoke_pass = $index["replay_smoke_pass"]
    tests = $tests
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("VERSION_A_ACCEPTANCE_JSON=" + $jsonPath)
  if (-not $summary.all_passed) {
    throw "version-a acceptance failed"
  }
}
finally {
  Pop-Location
}
