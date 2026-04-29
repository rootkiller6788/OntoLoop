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
  $logPath = Join-Path $runtimeDir ("policy-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("policy-acceptance-" + $stamp + ".json")

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
  $results += Invoke-Step -Name "bundle-signature-mismatch-reject" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "policy_acceptance_e2e", "bundle_signature_mismatch_rejected")
  $results += Invoke-Step -Name "discovery-fetch-failure-auto-rollback" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "policy_acceptance_e2e", "discovery_fetch_failure_auto_rollback_keeps_stable_current")
  $results += Invoke-Step -Name "enforced-high-risk-deny" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "policy_acceptance_e2e", "enforced_mode_high_risk_deny_effective")
  $results += Invoke-Step -Name "shadow-diff-traceable" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "policy_acceptance_e2e", "shadow_mode_diff_traceable")
  $results += Invoke-Step -Name "mask-drop-no-sensitive-leak" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "policy_acceptance_e2e", "mask_drop_logs_do_not_leak_sensitive_fields")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    all_passed = $true
    required_checks = @(
      "bundle 签名错误拒绝",
      "discovery 拉取失败自动回滚",
      "enforced 模式高风险 deny 生效",
      "shadow 模式差异可追溯",
      "mask/drop 后日志不泄露敏感字段"
    )
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("POLICY_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("POLICY_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
