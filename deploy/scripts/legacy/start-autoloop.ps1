$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$RuntimeDir = Join-Path $RepoRoot "deploy\runtime"
$SettingsFile = Join-Path $RuntimeDir "operator-settings.json"
$BackendLog = Join-Path $RuntimeDir "backend-live.log"
$FrontendLog = Join-Path $RuntimeDir "frontend-live.log"
$ConfigPath = Join-Path $RepoRoot "deploy\config\autoloop.dev.toml"

New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null

if ((-not $env:OPENAI_API_KEY) -and (Test-Path $SettingsFile)) {
  $settings = Get-Content $SettingsFile -Raw | ConvertFrom-Json
  if ($settings.api_key) {
    $env:OPENAI_API_KEY = [string]$settings.api_key
  }
}

if ((Test-Path $SettingsFile)) {
  $settings = Get-Content $SettingsFile -Raw | ConvertFrom-Json
  if ($settings.api_base_url) {
    $env:AUTOLOOP_PROVIDER_BASE_URL = [string]$settings.api_base_url
  }
  if ($settings.default_model) {
    $env:AUTOLOOP_PROVIDER_MODEL = [string]$settings.default_model
  }
  if ($settings.provider_vendor) {
    $env:AUTOLOOP_PROVIDER_VENDOR = [string]$settings.provider_vendor
  }
}

$backendConn = Get-NetTCPConnection -LocalPort 8787 -ErrorAction SilentlyContinue | Select-Object -First 1
if ($backendConn) {
  Stop-Process -Id $backendConn.OwningProcess -Force -ErrorAction SilentlyContinue
}

$frontendConn = Get-NetTCPConnection -LocalPort 5174 -ErrorAction SilentlyContinue | Select-Object -First 1
if ($frontendConn) {
  Stop-Process -Id $frontendConn.OwningProcess -Force -ErrorAction SilentlyContinue
}

$backendExe = Join-Path $RepoRoot "target\debug\autoloop.exe"
if (-not (Test-Path $backendExe)) {
  cargo build --manifest-path (Join-Path $RepoRoot "Cargo.toml")
}

# startup preflight: config doctor must pass before serving
$doctorOut = & cargo run --manifest-path (Join-Path $RepoRoot "Cargo.toml") -- --config $ConfigPath --session "startup-doctor" system config doctor --profile local 2>&1
if ($LASTEXITCODE -ne 0) {
  throw ("startup preflight failed: config doctor command failed`n" + ($doctorOut | Out-String))
}
$doctorText = ($doctorOut | Out-String)
$jsonStart = $doctorText.IndexOf("{")
if ($jsonStart -lt 0) {
  throw "startup preflight failed: config doctor did not return JSON payload"
}
$doctorJson = $doctorText.Substring($jsonStart) | ConvertFrom-Json
if ($doctorJson.status -eq "fail") {
  throw ("startup preflight failed: config doctor status=fail, issues=" + (($doctorJson.checks | Where-Object { -not $_.ok } | ConvertTo-Json -Depth 6)))
}

$nodeModules = Join-Path $RepoRoot "dashboard-ui\node_modules"
if (-not (Test-Path $nodeModules)) {
  Push-Location (Join-Path $RepoRoot "dashboard-ui")
  npm install
  Pop-Location
}

Start-Process -FilePath $backendExe `
  -ArgumentList @("--config", $ConfigPath, "system", "serve", "--host", "127.0.0.1", "--port", "8787") `
  -WorkingDirectory $RepoRoot `
  -RedirectStandardOutput $BackendLog `
  -RedirectStandardError $BackendLog

Start-Process -FilePath "D:\Program Files\nodejs\npx.cmd" `
  -ArgumentList @("vite", "--host", "127.0.0.1", "--port", "5174") `
  -WorkingDirectory (Join-Path $RepoRoot "dashboard-ui") `
  -RedirectStandardOutput $FrontendLog `
  -RedirectStandardError $FrontendLog

Start-Sleep -Seconds 6
Start-Process "http://127.0.0.1:5174"

Write-Host "AutoLoop started."
Write-Host "Dashboard: http://127.0.0.1:5174"
Write-Host "Backend:   http://127.0.0.1:8787/health"
