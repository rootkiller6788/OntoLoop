$ErrorActionPreference = "Stop"

param(
    [Parameter(Mandatory = $true)]
    [string]$BackupFile
)

if (-not (Test-Path $BackupFile)) {
    throw "Backup file not found: $BackupFile"
}

$Payload = Get-Content -Raw -Path $BackupFile | ConvertFrom-Json
Write-Host "Loaded backup timestamp: $($Payload.timestamp)"
Write-Host "This restore template is intended to coordinate StateStore import and config recovery."
Write-Host "Config files recorded in backup:"
$Payload.config_files | ForEach-Object { Write-Host " - $_" }

