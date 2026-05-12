param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml"
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
  $logPath = Join-Path $runtimeDir ("phase5b-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("phase5b-acceptance-" + $stamp + ".json")
  $results = @()

  function Invoke-Step {
    param([string]$Name, [string]$Exe, [string[]]$Argv)
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
    $script:results += [pscustomobject]@{
      name = $Name
      command = $display
      passed = $true
      exit_code = 0
    }
  }

  Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--manifest-path", $ManifestPath)
  Invoke-Step -Name "evo-shadow-cycle" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "-j", "1", "shadow_cycle_builds_full_pipeline_outputs")
  Invoke-Step -Name "query-evo-explain-counterfactual" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "-j", "1", "query_plane_surfaces_evolution_decision_path_and_reject_reason")
  Invoke-Step -Name "d14-rollout-chain" -Exe "powershell" -Argv @(
    "-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d14_rollout.ps1",
    "-ManifestPath", $ManifestPath,
    "-ProdConfigPath", $ProdConfigPath
  )

  [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    all_passed = $true
    checks = @(
      "counterfactual_replay_visible",
      "org_change_proposal_visible",
      "shadow_10_30_full_rollback_automated"
    )
    commands = $results
    log_path = $logPath
  } | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8

  Write-Output ("PHASE5B_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("PHASE5B_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
