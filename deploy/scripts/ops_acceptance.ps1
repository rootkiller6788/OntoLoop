param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = ".\deploy\config\autoloop.prod.toml",
  [string]$SessionPrefix = "ops-acceptance"
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
  $logPath = Join-Path $runtimeDir ("ops-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("ops-acceptance-" + $stamp + ".json")

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

  function Invoke-SystemJsonStep {
    param(
      [string]$Name,
      [string]$SessionId,
      [string[]]$SystemArgs
    )
    $out = Join-Path $runtimeDir ($Name + "-" + $stamp + ".json")
    $args = @(
      "run", "--manifest-path", $ManifestPath, "--",
      "--config", $ProdConfigPath,
      "--session", $SessionId,
      "system"
    ) + $SystemArgs + @("--output", $out)
    $result = Invoke-Step -Name $Name -Exe "cargo" -Argv $args
    if (-not (Test-Path $out)) {
      throw "Expected output file missing: $out"
    }
    return [pscustomobject]@{
      step = $result
      output = $out
      json = Get-Content -Raw -Path $out | ConvertFrom-Json
    }
  }

  $results = @()

  $doctor = Invoke-SystemJsonStep -Name "ops-config-doctor" -SessionId ($SessionPrefix + "-doctor") -SystemArgs @("config", "doctor", "--profile", "production")
  $results += $doctor.step
  if ($doctor.json.status -eq "fail") {
    throw "config doctor failed"
  }

  $health = Invoke-SystemJsonStep -Name "ops-health-check" -SessionId ($SessionPrefix + "-health") -SystemArgs @("health")
  $results += $health.step

  $alertStatus = Invoke-SystemJsonStep -Name "ops-alert-status" -SessionId ($SessionPrefix + "-alert") -SystemArgs @("alert", "status")
  $results += $alertStatus.step

  $alertDrill = Invoke-SystemJsonStep -Name "ops-alert-drill" -SessionId ($SessionPrefix + "-alert") -SystemArgs @("alert", "drill", "--reason", "ops drill synthetic alert")
  $results += $alertDrill.step
  if ($alertDrill.json.status -ne "raised") {
    throw "alert drill did not raise alert"
  }

  $heal = Invoke-SystemJsonStep -Name "ops-self-heal-drill" -SessionId ($SessionPrefix + "-heal") -SystemArgs @("self-heal", "drill", "--profile", "queue_throttle", "--reason", "ops drill self-heal")
  $results += $heal.step
  if (-not $heal.json.recover.recovered) {
    throw "self-heal drill did not recover"
  }

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    all_passed = $true
    required_checks = @(
      "config-doctor",
      "health-check",
      "alert-status",
      "alert-drill",
      "self-heal-drill"
    )
    commands = $results
    artifacts = [pscustomobject]@{
      config_doctor = $doctor.output
      health = $health.output
      alert_status = $alertStatus.output
      alert_drill = $alertDrill.output
      self_heal = $heal.output
    }
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("OPS_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("OPS_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
