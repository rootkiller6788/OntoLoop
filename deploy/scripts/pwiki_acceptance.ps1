param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$SessionPrefix = "pwiki-acceptance"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("pwiki-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("pwiki-acceptance-" + $stamp + ".json")

  function Invoke-Step {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv
    )

    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Name + "] " + $display + " ====")

    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $Exe @Argv 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev

    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
    }

    if ($exitCode -ne 0) {
      throw "Command failed ($exitCode): [$Name] $display"
    }

    return [pscustomobject]@{
      name = $Name
      command = $display
      passed = $true
      exit_code = 0
    }
  }

  $results = @()
  $results += Invoke-Step -Name "ingest-compile" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "day78_incremental_compiler_rebuilds_changed_files_only")
  $results += Invoke-Step -Name "infer-resume" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "semantic_resume_only_backfills_unfinished_sources")
  $results += Invoke-Step -Name "graph-health" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "view_plane_persists_graph_health_record_and_latest_ref")
  $results += Invoke-Step -Name "recall-expansion-enable" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "graph_enabled_expands_neighbors_with_confidence")
  $results += Invoke-Step -Name "recall-expansion-disable" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "route_disables_graph_sources_when_project_policy_disables_graph")
  $results += Invoke-Step -Name "heal-proposal-gate" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "heal_proposal_requires_approval_before_canonical_write")
  $results += Invoke-Step -Name "query-plane-summary" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "query_plane_surfaces_graph_health_summary_and_refs")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    session_prefix = $SessionPrefix
    all_passed = $true
    chain = @(
      "ingest/compile",
      "infer(resume)",
      "graph health",
      "recall expansion",
      "heal proposal",
      "approve",
      "recompile",
      "query-plane"
    )
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("PWIKI_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("PWIKI_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
