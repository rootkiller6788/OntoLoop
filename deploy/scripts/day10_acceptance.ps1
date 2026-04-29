param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$SessionId = "day10-cli"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $logPath = Join-Path $runtimeDir "day10-acceptance.log"
  $jsonPath = Join-Path $runtimeDir "day10-acceptance.json"
  $queryOut = Join-Path $runtimeDir "day10-query.json"
  $replayOut = Join-Path $runtimeDir "day10-replay-report.json"
  $orgOut = Join-Path $runtimeDir "day10-org-context.json"
  $triggerOut = Join-Path $runtimeDir "day10-trigger-list.json"

  foreach ($path in @($logPath, $jsonPath, $queryOut, $replayOut, $orgOut, $triggerOut)) {
    if (Test-Path $path) { Remove-Item $path -Force }
  }

  function Invoke-Step {
    param([pscustomobject]$Step)

    $display = "$($Step.exe) $($Step.args -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: " + $display + " ====")

    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $Step.exe @($Step.args) 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev

    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
    }

    if ($exitCode -ne 0) {
      throw "Command failed ($exitCode): $display"
    }

    return [pscustomobject]@{
      command = $display
      passed = $true
      exit_code = 0
    }
  }

  $steps = @(
    [pscustomobject]@{ exe = "cargo"; args = @("check", "--workspace", "--manifest-path", $ManifestPath) },
    [pscustomobject]@{ exe = "cargo"; args = @("test", "--manifest-path", $ManifestPath, "--test", "p10_day10_acceptance_e2e") },
    [pscustomobject]@{ exe = "cargo"; args = @("test", "--manifest-path", $ManifestPath, "--test", "p7_trigger_wake_plan_execute_e2e") },
    [pscustomobject]@{ exe = "cargo"; args = @("test", "--manifest-path", $ManifestPath, "--test", "p10_evidence_six_segments_e2e") },
    [pscustomobject]@{ exe = "cargo"; args = @("test", "--manifest-path", $ManifestPath, "--test", "p10_replay_mismatch_explainer_e2e") },
    [pscustomobject]@{ exe = "cargo"; args = @("test", "--manifest-path", $ManifestPath, "--test", "pq10_intent_query_tools_compact_verify_snapshot_resume_replay_e2e") },
    [pscustomobject]@{ exe = "cargo"; args = @("test", "--manifest-path", $ManifestPath, "--test", "pq10_no_bypass_gate_e2e") },
    [pscustomobject]@{ exe = "cargo"; args = @("run", "--manifest-path", $ManifestPath, "--", "--session", $SessionId, "--tenant", "tenant:day10", "--principal", "principal:day10", "--policy", "policy:default", "--swarm", "--message", "Run day10 acceptance chain") },
    [pscustomobject]@{ exe = "cargo"; args = @("run", "--manifest-path", $ManifestPath, "--", "--session", $SessionId, "system", "query", "--trace-id", "trace:day10", "--output", $queryOut) },
    [pscustomobject]@{ exe = "cargo"; args = @("run", "--manifest-path", $ManifestPath, "--", "--session", $SessionId, "system", "replay-report", "--output", $replayOut) },
    [pscustomobject]@{ exe = "cargo"; args = @("run", "--manifest-path", $ManifestPath, "--", "--session", $SessionId, "org", "context", "--output", $orgOut) },
    [pscustomobject]@{ exe = "cargo"; args = @("run", "--manifest-path", $ManifestPath, "--", "--session", $SessionId, "trigger", "list", "--output", $triggerOut) }
  )

  $results = @()
  foreach ($step in $steps) {
    $results += Invoke-Step -Step $step
  }

  $query = Get-Content -Raw -Path $queryOut | ConvertFrom-Json
  $requiredQueryKeys = @("metrics", "traces", "events", "ledger", "graph", "replay")
  $missingQueryKeys = @()
  foreach ($key in $requiredQueryKeys) {
    if (-not ($query.PSObject.Properties.Name -contains $key)) {
      $missingQueryKeys += $key
    }
  }
  if ($missingQueryKeys.Count -gt 0) {
    throw ("query artifact missing keys: " + ($missingQueryKeys -join ", "))
  }

  $replay = Get-Content -Raw -Path $replayOut | ConvertFrom-Json
  if (-not ($replay.PSObject.Properties.Name -contains "session_id")) {
    throw "replay artifact missing session_id"
  }
  if (-not ($replay.PSObject.Properties.Name -contains "reports")) {
    throw "replay artifact missing reports"
  }

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    session_id = $SessionId
    all_passed = $true
    commands = $results
    artifacts = [pscustomobject]@{
      log = $logPath
      summary = $jsonPath
      query = $queryOut
      replay = $replayOut
      org_context = $orgOut
      trigger_list = $triggerOut
    }
    checks = [pscustomobject]@{
      query_keys = $requiredQueryKeys
      replay_required = @("session_id", "reports")
    }
  }

  $summary | ConvertTo-Json -Depth 6 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("DAY10_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("DAY10_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}

