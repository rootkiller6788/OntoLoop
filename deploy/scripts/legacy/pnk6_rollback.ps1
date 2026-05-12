param(
  [string]$ConfigPath = "deploy/config/autoloop.dev.toml",
  [string]$RollbackContractVersion = "v1",
  [ValidateSet("shadow", "canary", "full")]
  [string]$GateMode = "shadow",
  [double]$GateEnforceRatio = 0.0,
  [switch]$NoWrite,
  [switch]$SkipVerify
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path $ConfigPath)) {
  throw "Config file not found: $ConfigPath"
}

$content = Get-Content -Raw -Path $ConfigPath
$new = $content
$new = [regex]::Replace($new, 'rollback_contract_version\s*=\s*"[^"]*"', "rollback_contract_version = `"$RollbackContractVersion`"")
$new = [regex]::Replace($new, 'gate_mode\s*=\s*"[^"]*"', "gate_mode = `"$GateMode`"")
$new = [regex]::Replace($new, 'gate_enforce_ratio\s*=\s*[0-9.]+', ("gate_enforce_ratio = " + [string]::Format("{0:0.0}", $GateEnforceRatio)))

if (-not $NoWrite) {
  Set-Content -Path $ConfigPath -Value $new -Encoding UTF8
}

if (-not $SkipVerify) {
  $manifest = "Cargo.toml"
  $commands = @(
    @("run", "--manifest-path", $manifest, "--", "--config", $ConfigPath, "system", "status"),
    @("run", "--manifest-path", $manifest, "--", "--config", $ConfigPath, "system", "health")
  )

  foreach ($args in $commands) {
    & cargo @args
    if ($LASTEXITCODE -ne 0) {
      throw "Rollback verification command failed: cargo $($args -join ' ')"
    }
  }
}

Write-Output ("PNK6_ROLLBACK_OK config=" + $ConfigPath)
Write-Output ("PNK6_ROLLBACK_MODE=" + $GateMode)
Write-Output ("PNK6_ROLLBACK_RATIO=" + [string]::Format("{0:0.0}", $GateEnforceRatio))
Write-Output ("PNK6_ROLLBACK_CONTRACT=" + $RollbackContractVersion)
Write-Output ("PNK6_ROLLBACK_VERIFY_SKIPPED=" + [string]$SkipVerify)
