param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "daily-rollback"
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
  $logPath = Join-Path $runtimeDir ("rollback-daily-drill-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("rollback-daily-drill-" + $stamp + ".json")
  $script:results = @()

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
    $script:results += [pscustomobject]@{
      name = $Name
      command = $display
      passed = $true
      exit_code = 0
    }
  }

  $env:RUST_MIN_STACK = "33554432"
  Invoke-Step -Name "high-risk-unauthorized-rejected" -Exe "cargo" -Argv @(
    "test", "--manifest-path", $ManifestPath, "--test", "permission_mode_runtime_enforced_e2e"
  )
  Invoke-Step -Name "canary-path-write-blocked-with-traceable-deny" -Exe "cargo" -Argv @(
    "test", "--manifest-path", $ManifestPath, "--lib", "tests::production_write_gate_blocks_canary_path_9c"
  )
  Invoke-Step -Name "canary-fail-auto-rollback-e2e" -Exe "cargo" -Argv @(
    "test", "--manifest-path", $ManifestPath, "--test", "pevo_r10_promote_canary_fail_rollback_e2e"
  )
  Invoke-Step -Name "rollout-drill-shadow-canary-full-rollback" -Exe "powershell" -Argv @(
    "-ExecutionPolicy", "Bypass", "-File", ".\deploy\scripts\d14_rollout.ps1",
    "-ManifestPath", $ManifestPath,
    "-ProdConfigPath", $ProdConfigPath,
    "-SessionPrefix", $SessionPrefix
  )

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    all_passed = $true
    checks = @(
      "high-risk unauthorized deny",
      "canary write gate deny with traceable reason",
      "auto rollback on canary fail",
      "daily rollout/rollback drill"
    )
    commands = $script:results
    log_path = $logPath
  }
  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8

  Write-Output ("ROLLBACK_DAILY_DRILL_OK log=" + $logPath)
  Write-Output ("ROLLBACK_DAILY_DRILL_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
