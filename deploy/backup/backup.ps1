$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$OutDir = Join-Path $PSScriptRoot "snapshots"
$OutFile = Join-Path $OutDir "autoloop-backup-$Timestamp.json"

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$Payload = [ordered]@{
    timestamp = $Timestamp
    note = "Application-level backup placeholder for StateStore exports and operational snapshots."
    config_files = Get-ChildItem -Path (Join-Path $Root "config") -File | Select-Object -ExpandProperty FullName
}

$Payload | ConvertTo-Json -Depth 6 | Set-Content -Path $OutFile -Encoding UTF8
Write-Host "Backup written to $OutFile"

