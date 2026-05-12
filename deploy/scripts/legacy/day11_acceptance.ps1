param(
  [string]$ManifestPath = ".\Cargo.toml"
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
  $logPath = Join-Path $runtimeDir ("day11-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("day11-acceptance-" + $stamp + ".json")

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
  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--manifest-path", $ManifestPath)
  $results += Invoke-Step -Name "parallel-tool-call-events" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_query_loop_parallel_tool_events_contract")
  $results += Invoke-Step -Name "two-stage-compact" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq2_compaction_resume_boundary")
  $results += Invoke-Step -Name "named-snapshot-transcript" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq7_session_named_snapshot_transcript_e2e")
  $results += Invoke-Step -Name "background-task-manager" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq8_background_task_manager_e2e")
  $results += Invoke-Step -Name "mcp-manager-service-spine" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq8_service_mediation_spine")
  $results += Invoke-Step -Name "d11-aggregate-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq11_d11_compact_snapshot_task_mcp_parallel_e2e")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    all_passed = $true
    required_checks = @(
      "parallel_tool_call",
      "two_stage_compact",
      "named_snapshot",
      "task_manager",
      "mcp_manager"
    )
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("DAY11_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("DAY11_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
