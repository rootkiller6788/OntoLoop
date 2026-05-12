param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "fault-injection-daily",
  [ValidateSet("light","full")]
  [string]$DrillMode = "light"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$runtimeDir = Join-Path $repoRoot "deploy\runtime"
if (-not (Test-Path $runtimeDir)) {
  New-Item -ItemType Directory -Path $runtimeDir | Out-Null
}
$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$logPath = Join-Path $runtimeDir ("fault-injection-daily-" + $stamp + ".log")
$jsonPath = Join-Path $runtimeDir ("fault-injection-daily-" + $stamp + ".json")

Push-Location $repoRoot
try {
  function Invoke-CheckedStep {
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
      throw "Fault injection step failed ($exitCode): [$Name] $display"
    }
    return [pscustomobject]@{
      name = $Name
      command = $display
      passed = $true
      exit_code = 0
    }
  }

  $steps = @()
  $steps += Invoke-CheckedStep -Name "inject-timeout-retry" -Exe "cargo" -Argv @(
    "test", "--manifest-path", $ManifestPath, "--lib",
    "runtime::tests::p11_chaos_case_records_failover"
  )
  $steps += Invoke-CheckedStep -Name "inject-budget-over-compact-replan" -Exe "cargo" -Argv @(
    "test", "--manifest-path", $ManifestPath, "--test", "p8_budget_ledger_sovereignty"
  )
  if ($DrillMode -eq "full") {
    $steps += Invoke-CheckedStep -Name "inject-tool-fail-rollback" -Exe "cargo" -Argv @(
      "test", "--manifest-path", $ManifestPath, "--lib",
      "runtime::tests::p11_recover_marks_failover_with_mttr"
    )
    $steps += Invoke-CheckedStep -Name "inject-budget-over-swarm-preflight" -Exe "cargo" -Argv @(
      "test", "--manifest-path", $ManifestPath, "--lib",
      "swarm_budget_preflight_compacts_when_budget_overflows"
    )
  }

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    drill_mode = $DrillMode
    cadence = if ($DrillMode -eq "full") { "weekly" } else { "daily" }
    session_prefix = $SessionPrefix
    prod_config = $ProdConfigPath
    all_passed = $true
    injections = @(
      [pscustomobject]@{
        fault = "timeout"
        expected_paths = @("retry", "rollback")
        verified_by = @("inject-timeout-retry", "inject-tool-fail-rollback")
      },
      [pscustomobject]@{
        fault = "tool_fail"
        expected_paths = @("retry", "rollback")
        verified_by = @("inject-tool-fail-rollback")
      },
      [pscustomobject]@{
        fault = "budget_over"
        expected_paths = @("compact", "replan")
        verified_by = @("inject-budget-over-compact-replan", "inject-budget-over-swarm-preflight")
      }
    )
    commands = $steps
    log_path = $logPath
  }
  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8

  Write-Output ("FAULT_INJECTION_DAILY_OK log=" + $logPath)
  Write-Output ("FAULT_INJECTION_DAILY_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
