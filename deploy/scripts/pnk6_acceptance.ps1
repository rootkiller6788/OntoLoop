param(
  [string]$ManifestPath = "Cargo.toml",
  [string]$SessionId = "pnk6-e2e"
)

$ErrorActionPreference = "Stop"

$runtimeDir = Join-Path "deploy" "runtime"
if (-not (Test-Path $runtimeDir)) {
  New-Item -ItemType Directory -Path $runtimeDir | Out-Null
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logPath = Join-Path $runtimeDir ("pnk6-acceptance-" + $stamp + ".log")
$jsonPath = Join-Path $runtimeDir ("pnk6-acceptance-" + $stamp + ".json")
$replayOut = Join-Path $runtimeDir ("pnk6-replay-report-" + $stamp + ".json")

$commands = @(
  [pscustomobject]@{ Name = "check"; Exe = "cargo"; Args = @("check", "--manifest-path", $ManifestPath) },
  [pscustomobject]@{ Name = "pnk1-bypass-kernel-rejected"; Exe = "cargo"; Args = @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::pnk1_bypass_kernel_path_is_rejected") },
  [pscustomobject]@{ Name = "pnk2-sigstore-supply-chain-admission"; Exe = "cargo"; Args = @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::pnk2_supply_chain_manifest_injected_from_tool_manifest") },
  [pscustomobject]@{ Name = "pnk3-attestation-evidence-chain"; Exe = "cargo"; Args = @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::pnk3_attestation_verdict_is_forced_into_evidence_chain") },
  [pscustomobject]@{ Name = "ptee7-attestation-ttl-expired-reject"; Exe = "cargo"; Args = @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::trust_bridge::tests::attestation_ttl_expired_is_rejected") },
  [pscustomobject]@{ Name = "pnk5-strong-consistency-ledger"; Exe = "cargo"; Args = @("test", "--manifest-path", $ManifestPath, "--lib", "runtime::tests::pnk5_strong_consistency_fails_admission_when_evidence_write_fails") },
  [pscustomobject]@{ Name = "full-chain-intent-to-replay"; Exe = "cargo"; Args = @("test", "--manifest-path", $ManifestPath, "--test", "p10_full_chain_uniform_tracking_e2e") },
  [pscustomobject]@{ Name = "replay-report-attestation-column"; Exe = "cargo"; Args = @("test", "--manifest-path", $ManifestPath, "--test", "ptee6_replay_report_attestation_e2e") },
  [pscustomobject]@{ Name = "export-replay-report-cli"; Exe = "cargo"; Args = @("run", "--manifest-path", $ManifestPath, "--", "--session", $SessionId, "system", "replay-report", "--output", $replayOut) }
)

$results = @()

foreach ($cmd in $commands) {
  $line = "`n==== RUN: $($cmd.Name) :: $($cmd.Exe) $($cmd.Args -join ' ') ===="
  Add-Content -Path $logPath -Value $line

  $commandLine = $cmd.Exe + " " + ($cmd.Args -join " ")
  $prev = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  $output = & cmd /c $commandLine 2>&1
  $exit = $LASTEXITCODE
  $ErrorActionPreference = $prev

  $output | Out-File -FilePath $logPath -Append -Encoding utf8
  if ($exit -ne 0) {
    $results += [pscustomobject]@{
      name = $cmd.Name
      passed = $false
      exit_code = $exit
      command = $commandLine
    }
    throw "PNK6 command failed: $($cmd.Name)"
  }

  $results += [pscustomobject]@{
    name = $cmd.Name
    passed = $true
    exit_code = 0
    command = $commandLine
  }
}

if (-not (Test-Path $replayOut)) {
  throw "Replay report file was not generated: $replayOut"
}

$replay = Get-Content -Raw -Path $replayOut | ConvertFrom-Json
if (-not ($replay.PSObject.Properties.Name -contains "session_id")) {
  throw "Replay report missing session_id"
}
if (-not ($replay.PSObject.Properties.Name -contains "reports")) {
  throw "Replay report missing reports"
}

$summary = [pscustomobject]@{
  generated_at = (Get-Date).ToString("s")
  manifest = $ManifestPath
  session_id = $SessionId
  all_passed = $true
  replay_report = $replayOut
  log_path = $logPath
  commands = $results
  chain = @("intent", "admit(sigstore+attestation)", "execute", "record", "verify", "replay")
}

$summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8

Write-Output ("PNK6_ACCEPTANCE_OK log=" + $logPath)
Write-Output ("PNK6_ACCEPTANCE_JSON=" + $jsonPath)
