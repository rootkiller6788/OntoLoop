$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot

Write-Host "Starting one-command local deploy..."
Write-Host "Config root: $Root"

docker compose -f (Join-Path $Root "..\\docker-compose.yml") up -d --build

Write-Host "AutoLoop local stack requested."
Write-Host "Services: autoloop, state_store, browserless, prometheus, alertmanager"

