param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$SessionPrefix = "frontend-cli-acceptance"
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
  $logPath = Join-Path $runtimeDir ("frontend-cli-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("frontend-cli-acceptance-" + $stamp + ".json")

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
  $results += Invoke-Step -Name "frontend-chat-stream-tool-permission-attach-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq12_frontend_cli_chat_stream_tool_permission_attach_e2e")
  $results += Invoke-Step -Name "transport-session-event-contract-v2" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq6_transport_session_event_contract_v2")
  $results += Invoke-Step -Name "query-plane-cli-event-chain-visible" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "observability::query_plane::tests::query_plane_includes_cli_event_chain_records")

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
  Write-Output ("FRONTEND_CLI_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("FRONTEND_CLI_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
