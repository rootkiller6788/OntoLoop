param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "d14-storage-cutover",
  [string]$LocalPostgresUri = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$backupPath = ""

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("d14-storage-cutover-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("d14-storage-cutover-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("d14-autoloop.prod.backup-" + $stamp + ".toml")

  Copy-Item -Path $ProdConfigPath -Destination $backupPath -Force

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

  function Set-StorageCutover {
    param(
      [string]$Backend,
      [string]$Mode,
      [string]$ReadPreference,
      [int]$RolloutPercent
    )
    $content = Get-Content -Raw -Path $ProdConfigPath
    $storagePattern = '(?s)(\[storage\].*?)(?=\r?\n\[|\z)'
    $match = [regex]::Match($content, $storagePattern)
    if (-not $match.Success) {
      throw "Missing [storage] section in $ProdConfigPath"
    }
    $existingBlock = $match.Groups[1].Value
    $graceHours = 24
    $graceMatch = [regex]::Match($existingBlock, 'shadow_write_grace_hours\s*=\s*(\d+)')
    if ($graceMatch.Success) {
      $graceHours = [int]$graceMatch.Groups[1].Value
    }

    $storageBlock = @(
      "[storage]"
      ('backend = "' + $Backend + '"')
      ('mode = "' + $Mode + '"')
      ('shadow_read_preference = "' + $ReadPreference + '"')
      ('shadow_read_rollout_percent = ' + $RolloutPercent)
      ('shadow_write_grace_hours = ' + $graceHours)
    ) -join [Environment]::NewLine

    $content = $content.Substring(0, $match.Index) + $storageBlock + [Environment]::NewLine + $content.Substring($match.Index + $match.Length)
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Set-LegacyFallbackLocal {
    $content = Get-Content -Raw -Path $ProdConfigPath
    $legacyPattern = '(?s)(\[state_store\].*?)(?=\r?\n\[|\z)'
    $match = [regex]::Match($content, $legacyPattern)
    if (-not $match.Success) {
      return
    }
    $legacyBlock = $match.Groups[1].Value
    $legacyBlock = [regex]::Replace($legacyBlock, 'backend\s*=\s*".*?"', 'backend = "in_memory"')
    $legacyBlock = [regex]::Replace($legacyBlock, 'uri\s*=\s*".*?"', 'uri = "http://127.0.0.1:3000"')
    $content = $content.Substring(0, $match.Index) + $legacyBlock + [Environment]::NewLine + $content.Substring($match.Index + $match.Length)
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Resolve-LocalPostgresUri {
    $uri = $LocalPostgresUri
    if ([string]::IsNullOrWhiteSpace($uri)) {
      $uri = $env:AUTOLOOP_D14_PG_URI
    }
    if ([string]::IsNullOrWhiteSpace($uri)) {
      $uri = "postgres://postgres:123456@localhost:5432/postgres"
    }
    return $uri
  }

  function Set-LocalPostgresConfig {
    $uri = Resolve-LocalPostgresUri

    $content = Get-Content -Raw -Path $ProdConfigPath
    $pgPattern = '(?s)(\[storage\.postgres\].*?)(?=\r?\n\[|\z)'
    $match = [regex]::Match($content, $pgPattern)
    if (-not $match.Success) {
      return
    }
    $pgBlock = $match.Groups[1].Value
    $pgBlock = [regex]::Replace($pgBlock, 'enabled\s*=\s*.*', 'enabled = true')
    $pgBlock = [regex]::Replace($pgBlock, 'uri\s*=\s*".*?"', ('uri = "' + $uri + '"'))
    $content = $content.Substring(0, $match.Index) + $pgBlock + [Environment]::NewLine + $content.Substring($match.Index + $match.Length)
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  $results = @()
  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  $results += Invoke-Step -Name "shadow-diff-query-plane" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq11_d11_compact_snapshot_task_mcp_parallel_e2e")

  Set-StorageCutover -Backend "postgres" -Mode "direct" -ReadPreference "postgres" -RolloutPercent 100
  Set-LocalPostgresConfig
  Set-LegacyFallbackLocal
  $localPgUri = Resolve-LocalPostgresUri
  $psqlPath = "D:\Program Files\PostgreSQL\15\bin\psql.exe"
  if (-not (Test-Path $psqlPath)) {
    $psqlPath = "psql"
  }
  $results += Invoke-Step -Name "postgres-schema-bootstrap" -Exe $psqlPath -Argv @("--set", "ON_ERROR_STOP=1", "--dbname", $localPgUri, "--file", ".\\deploy\\sql\\d4_postgres_core_schema.sql")
  $results += Invoke-Step -Name "cutover-status" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status")
  $results += Invoke-Step -Name "cutover-health" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", ($SessionPrefix + "-cutover"), "system", "health")
  $results += Invoke-Step -Name "postgres-primary-read-check" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "d12_storage_postgres_wal_dualwrite_replay_e2e")

  Set-StorageCutover -Backend "postgres" -Mode "shadow" -ReadPreference "postgres" -RolloutPercent 0
  $results += Invoke-Step -Name "rollback-status" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status")
  $results += Invoke-Step -Name "rollback-health" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", ($SessionPrefix + "-rollback"), "system", "health")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    backup_config = $backupPath
    all_passed = $true
    cutover = @{
      target = "postgres-primary"
      rollback_target = "postgres-shadow-readonly-fallback"
      remove_state_store_allowed = $true
      remove_state_store_gate = "Only after this report passes and rollback drill succeeds."
    }
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("D14_STORAGE_CUTOVER_OK log=" + $logPath)
  Write-Output ("D14_STORAGE_CUTOVER_JSON=" + $jsonPath)
}
finally {
  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  Pop-Location
}

