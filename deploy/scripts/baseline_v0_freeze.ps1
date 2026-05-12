param(
  [string]$ConfigPath = "deploy/config/autoloop.baseline_v0.toml",
  [string]$RuntimeDir = "deploy/runtime"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
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

  $runtimeAbs = if ([System.IO.Path]::IsPathRooted($RuntimeDir)) { $RuntimeDir } else { Join-Path $repoRoot $RuntimeDir }
  if (-not (Test-Path $runtimeAbs)) {
    New-Item -ItemType Directory -Path $runtimeAbs | Out-Null
  }

  $configAbs = if ([System.IO.Path]::IsPathRooted($ConfigPath)) { $ConfigPath } else { Join-Path $repoRoot $ConfigPath }
  if (-not (Test-Path $configAbs)) {
    throw "baseline config missing: $configAbs"
  }

  $versionPath = Join-Path $repoRoot "src/contracts/version.rs"
  $dispatchPath = Join-Path $repoRoot "src/command_dispatch.rs"
  $prodContractPath = Join-Path $repoRoot "docs/production_contract.md"
  $releaseGatePath = Join-Path $repoRoot "deploy/scripts/release_gate_report.ps1"
  $configRaw = Get-Content -Raw -Path $configAbs
  $versionRaw = Get-Content -Raw -Path $versionPath
  $dispatchRaw = Get-Content -Raw -Path $dispatchPath
  $prodContractRaw = Get-Content -Raw -Path $prodContractPath
  $releaseGateRaw = Get-Content -Raw -Path $releaseGatePath

  $checks = @()
  $checks += [pscustomobject]@{ id = "module_a.fast_harness.frontend_dispatch"; passed = ($dispatchRaw -match "dispatch_frontend") }
  $checks += [pscustomobject]@{ id = "module_a.fast_harness.stream_events"; passed = ($dispatchRaw -match "render_session_event_pretty" -and $dispatchRaw -match "AssistantDelta") }
  $checks += [pscustomobject]@{ id = "module_a.fast_harness.command_interaction"; passed = ($dispatchRaw -match "system benchmark run") }
  $checks += [pscustomobject]@{ id = "module_b.code_harness.repo_context"; passed = ($versionRaw -match "REPO_CONTEXT_BUNDLE_CONTRACT_VERSION") }
  $checks += [pscustomobject]@{ id = "module_b.code_harness.patch_apply"; passed = ($versionRaw -match "CODE_HARNESS_CONTRACT_VERSION") }
  $checks += [pscustomobject]@{ id = "module_b.code_harness.shell_loop_verifier"; passed = ($versionRaw -match "CODE_EXECUTION_LOOP_CONTRACT_VERSION") }
  $checks += [pscustomobject]@{ id = "constraint.artifact_must_land"; passed = ($prodContractRaw -match "(?i)write_proof \+ hash \+ evidence_ref") }
  $checks += [pscustomobject]@{ id = "constraint.root_only_evidence"; passed = ($versionRaw -match "ROOT_ONLY_EVIDENCE_CONTRACT_VERSION") }
  $checks += [pscustomobject]@{ id = "constraint.wal_root_release_gate"; passed = ($releaseGateRaw -match "(?i)wal_root" -and $releaseGateRaw -match "(?i)decision_root" -and $releaseGateRaw -match "(?i)impacted_tests_hash") }
  $checks += [pscustomobject]@{ id = "profile.minimal.learning_off"; passed = ($configRaw -match "(?m)^\s*enabled\s*=\s*false\s*$" -and $configRaw -match "(?m)^\s*sidecar_enabled\s*=\s*false\s*$") }
  $checks += [pscustomobject]@{ id = "profile.minimal.research_off"; passed = ($configRaw -match "(?ms)\[research\].*?^\s*enabled\s*=\s*false\s*$") }
  $checks += [pscustomobject]@{ id = "profile.minimal.policy_off"; passed = ($configRaw -match '(?m)^\s*policy_mode\s*=\s*"off"\s*$') }

  $failed = @($checks | Where-Object { -not $_.passed })
  $passed = ($failed.Count -eq 0)

  $moduleAHash = Get-StringSha256 -Text (($checks | Where-Object { $_.id -like "module_a.*" } | ForEach-Object { $_.id + ":" + $_.passed }) -join "|")
  $moduleBHash = Get-StringSha256 -Text (($checks | Where-Object { $_.id -like "module_b.*" } | ForEach-Object { $_.id + ":" + $_.passed }) -join "|")
  $constraintHash = Get-StringSha256 -Text (($checks | Where-Object { $_.id -like "constraint.*" } | ForEach-Object { $_.id + ":" + $_.passed }) -join "|")
  $configHash = Get-StringSha256 -Text $configRaw
  $baselineId = Get-StringSha256 -Text ("baseline_v0|" + $moduleAHash + "|" + $moduleBHash + "|" + $constraintHash + "|" + $configHash)

  $report = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    baseline = "baseline_v0"
    scope = [pscustomobject]@{
      module_a = "fast_harness"
      module_b = "code_harness"
      hard_constraints = @(
        "artifact_must_land",
        "root_only_evidence",
        "wal_root_release_gate"
      )
    }
    profile = [pscustomobject]@{
      config_path = $configAbs
      config_hash = $configHash
      minimal_mode = "on"
    }
    fingerprints = [pscustomobject]@{
      baseline_id = $baselineId
      module_a_hash = $moduleAHash
      module_b_hash = $moduleBHash
      constraint_hash = $constraintHash
    }
    checks = $checks
    passed = $passed
    deny_reasons = @($failed | ForEach-Object { $_.id })
  }

  $reportPath = Join-Path $runtimeAbs "baseline_v0.json"
  $report | ConvertTo-Json -Depth 10 | Out-File -FilePath $reportPath -Encoding utf8

  Write-Output ("BASELINE_V0_JSON=" + $reportPath)
  Write-Output ("BASELINE_V0_ID=" + $baselineId)
  Write-Output ("BASELINE_V0_PASS=" + ($passed.ToString().ToLowerInvariant()))

  if (-not $passed) {
    throw ("baseline_v0 freeze failed: " + (($failed | ForEach-Object { $_.id }) -join ","))
  }
}
finally {
  Pop-Location
}
