param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "d12-ops"
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
  $jsonPath = Join-Path $runtimeDir ("d12-ops-productized-" + $stamp + ".json")

  $opsRaw = & powershell -ExecutionPolicy Bypass -File .\deploy\scripts\ops_acceptance.ps1 -ManifestPath $ManifestPath -ProdConfigPath $ProdConfigPath -SessionPrefix ($SessionPrefix + "-ops")
  if ($LASTEXITCODE -ne 0) { throw "ops_acceptance failed" }
  $opsJsonPath = ($opsRaw | Select-String -Pattern "OPS_ACCEPTANCE_JSON=" | Select-Object -Last 1).ToString().Split("=",2)[1].Trim()

  $sloRaw = & powershell -ExecutionPolicy Bypass -File .\deploy\scripts\d46_slo_acceptance.ps1 -ManifestPath $ManifestPath
  if ($LASTEXITCODE -ne 0) { throw "d46_slo_acceptance failed" }
  $sloJsonPath = ($sloRaw | Select-String -Pattern "D46_SLO_JSON=" | Select-Object -Last 1).ToString().Split("=",2)[1].Trim()

  $opsJson = Get-Content -Raw -Path $opsJsonPath | ConvertFrom-Json
  $sloJson = Get-Content -Raw -Path $sloJsonPath | ConvertFrom-Json

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    all_passed = ($opsJson.all_passed -eq $true) -and ($sloJson.slo_passed -eq $true)
    required_checks = @(
      "config-doctor",
      "startup-preflight-config-check",
      "health-check",
      "alert-drill",
      "self-heal-drill",
      "slo-thresholds"
    )
    artifacts = [pscustomobject]@{
      ops_acceptance_json = $opsJsonPath
      slo_acceptance_json = $sloJsonPath
    }
    results = [pscustomobject]@{
      config_doctor = $opsJson.artifacts.config_doctor
      health = $opsJson.artifacts.health
      alert_drill = $opsJson.artifacts.alert_drill
      self_heal = $opsJson.artifacts.self_heal
      slo = $sloJson.slo
      slo_breaches = $sloJson.breaches
    }
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  if (-not $summary.all_passed) {
    throw "D12 ops productized acceptance failed"
  }
  Write-Output ("D12_OPS_PRODUCTIZED_OK")
  Write-Output ("D12_OPS_PRODUCTIZED_JSON=" + $jsonPath)
}
finally {
  Pop-Location
}
