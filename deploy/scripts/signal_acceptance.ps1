param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$SessionPrefix = "signal-acceptance"
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
  $logPath = Join-Path $runtimeDir ("signal-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("signal-acceptance-" + $stamp + ".json")

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
  $results += Invoke-Step -Name "signal-contract-order-reject-replay-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq11_signal_pipeline_contract_order_reject_replay_e2e")
  $results += Invoke-Step -Name "signal-no-bypass-static-scan" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "observability::signal_facade::tests::signal_write_path_is_no_bypass")
  $results += Invoke-Step -Name "signal-cli-whitebox-command-surface" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--bin", "ontoloop", "system_signal_status_and_explain_views_are_available")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    session_prefix = $SessionPrefix
    all_passed = $true
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("SIGNAL_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("SIGNAL_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
