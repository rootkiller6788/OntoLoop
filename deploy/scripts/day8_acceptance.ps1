param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$SessionId = "day8-core"
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
  $logPath = Join-Path $runtimeDir ("day8-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("day8-acceptance-" + $stamp + ".json")

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
  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  $results += Invoke-Step -Name "e2e-plugin-rollout-hotupdate" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq7_plugin_lifecycle_signed_e2e")
  $results += Invoke-Step -Name "lib-query-replay-plugin-trace" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "query_plane_aggregates_mismatch_explanations")
  $results += Invoke-Step -Name "lib-query-policy-routing" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "query_plane_policy_controls_graph_and_routing_surface")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    session_id = $SessionId
    all_passed = $true
    focus = @(
      "plugin shadow/canary/full/quick-rollback lifecycle",
      "query/replay plugin execution trace visibility",
      "mismatch explainer plugin cause surface"
    )
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("DAY8_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("DAY8_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
