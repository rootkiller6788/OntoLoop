param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "d14-rollout",
  [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$previousAutoLoopProfile = if (Test-Path Env:AUTOLOOP_PROFILE) { $env:AUTOLOOP_PROFILE } else { $null }
$previousCargoTargetDir = if (Test-Path Env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { $null }
$env:AUTOLOOP_PROFILE = "production-e2e"
$scriptSucceeded = $false
$targetDir = $null

$backupPath = ""
$summaryPath = ""
$logPath = ""
$results = @()
$allPassed = $true
$failureMessage = $null
$rollbackRecord = $null

try {
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("d14-rollout-" + $stamp + ".log")
  $summaryPath = Join-Path $runtimeDir ("d14-rollout-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("d14-rollout-config-backup-" + $stamp + ".toml")
  $targetDir = Join-Path $runtimeDir ("target-d14-" + $stamp)
  New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
  $env:CARGO_TARGET_DIR = $targetDir

  Copy-Item -Path $ProdConfigPath -Destination $backupPath -Force

  function Invoke-Step {
    param(
      [string]$Name,
      [string]$Exe,
      [string[]]$Argv,
      [switch]$AllowFailure
    )

    $display = "$Exe $($Argv -join ' ')"
    Add-Content -Path $logPath -Value ("`n==== RUN: [" + $Name + "] " + $display + " ====")
    if ($DryRun) {
      Add-Content -Path $logPath -Value "DRY_RUN=true (skipped execution)"
      $record = [pscustomobject]@{
        name = $Name
        command = $display
        passed = $true
        skipped = $true
        exit_code = 0
      }
      $script:results += $record
      return $record
    }

    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $output = & $Exe @Argv 2>&1
    $exitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev
    if ($null -ne $output) {
      $output | Out-File -FilePath $logPath -Append -Encoding utf8
    }

    $passed = ($exitCode -eq 0)
    $record = [pscustomobject]@{
      name = $Name
      command = $display
      passed = $passed
      skipped = $false
      exit_code = $exitCode
    }
    $script:results += $record
    if (-not $passed -and -not $AllowFailure) {
      throw "Command failed ($exitCode): [$Name] $display"
    }
    return $record
  }

  function Set-GateConfig {
    param(
      [string]$Mode,
      [double]$Ratio
    )
    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, 'gate_mode\s*=\s*".*?"', ('gate_mode = "' + $Mode + '"'))
    $content = [regex]::Replace(
      $content,
      'gate_enforce_ratio\s*=\s*[0-9.]+',
      ('gate_enforce_ratio = ' + $Ratio.ToString([System.Globalization.CultureInfo]::InvariantCulture))
    )
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Normalize-ConfigForLocalRollout {
    $lines = Get-Content -Path $ProdConfigPath
    $section = ""
    for ($i = 0; $i -lt $lines.Length; $i++) {
      $trimmed = $lines[$i].Trim()
      if ($trimmed -match '^\[(.+)\]$') {
        $section = $Matches[1]
        continue
      }

      switch ($section) {
        "state_store" {
          if ($trimmed -match '^backend\s*=') {
            $lines[$i] = 'backend = "in_memory"'
          }
        }
        "storage" {
          if ($trimmed -match '^backend\s*=') {
            $lines[$i] = 'backend = "postgres"'
          } elseif ($trimmed -match '^shadow_read_preference\s*=') {
            $lines[$i] = 'shadow_read_preference = "postgres"'
          }
        }
        "storage.postgres" {
          if ($trimmed -match '^enabled\s*=') {
            $lines[$i] = 'enabled = true'
          } elseif ($trimmed -match '^uri\s*=') {
            $lines[$i] = 'uri = "postgres://postgres:123456@localhost:5432/ontoloop"'
          }
        }
      }
    }
    Set-Content -Path $ProdConfigPath -Value $lines -Encoding utf8
  }

  function Invoke-ConfigDoctorGate {
    param(
      [string]$SessionId,
      [string]$Profile = "production-e2e"
    )
    $out = Join-Path $runtimeDir ("d14-config-doctor-" + $stamp + ".json")
    Invoke-Step -Name "pre-rollout-config-doctor-gate" -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--",
      "--config", $ProdConfigPath,
      "--session", $SessionId,
      "system", "config", "doctor",
      "--profile", $Profile,
      "--output", $out
    ) | Out-Null
    if (-not (Test-Path $out)) {
      throw "config doctor output missing: $out"
    }
    $doctor = Get-Content -Raw -Path $out | ConvertFrom-Json
    if ($doctor.status -ne "pass") {
      throw "config doctor hard gate failed: status=$($doctor.status)"
    }
    $requiredIds = @(
      "profile.alignment",
      "runtime.gate_mode",
      "runtime.rollback_window",
      "storage.postgres.enabled_uri",
      "storage.backend_consistency"
    )
    foreach ($id in $requiredIds) {
      $check = $doctor.checks | Where-Object { $_.id -eq $id } | Select-Object -First 1
      if ($null -eq $check) {
        throw "config doctor hard gate missing required check: $id"
      }
      if (-not $check.passed) {
        throw "config doctor hard gate check failed: $id => $($check.message)"
      }
    }
  }

  function Invoke-Rollback {
    param([string]$Reason)
    try {
      Set-GateConfig -Mode "shadow" -Ratio 0.2
      $status = Invoke-Step -Name "rollback-status" -Exe "cargo" -Argv @(
        "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status"
      ) -AllowFailure
      $health = Invoke-Step -Name "rollback-health" -Exe "cargo" -Argv @(
        "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath,
        "--session", ($SessionPrefix + "-rollback"), "system", "health"
      ) -AllowFailure
      $script:rollbackRecord = [pscustomobject]@{
        triggered = $true
        reason = $Reason
        status_ok = $status.passed
        health_ok = $health.passed
        executed_at = (Get-Date).ToString("s")
      }
    } catch {
      $script:rollbackRecord = [pscustomobject]@{
        triggered = $true
        reason = $Reason
        status_ok = $false
        health_ok = $false
        executed_at = (Get-Date).ToString("s")
        error = $_.Exception.Message
      }
    }
  }

  Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  Invoke-Step -Name "rollout-gating-test" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p6_rollout_gating")
  Normalize-ConfigForLocalRollout
  Invoke-ConfigDoctorGate -SessionId ($SessionPrefix + "-config-doctor") -Profile "production-e2e"

  $stages = @(
    [pscustomobject]@{ name = "shadow"; mode = "shadow"; ratio = 0.2; session = $SessionPrefix + "-shadow" },
    [pscustomobject]@{ name = "canary10"; mode = "canary"; ratio = 0.1; session = $SessionPrefix + "-canary10" },
    [pscustomobject]@{ name = "canary30"; mode = "canary"; ratio = 0.3; session = $SessionPrefix + "-canary30" },
    [pscustomobject]@{ name = "full"; mode = "full"; ratio = 1.0; session = $SessionPrefix + "-full" }
  )

  foreach ($stage in $stages) {
    Set-GateConfig -Mode $stage.mode -Ratio $stage.ratio
    Invoke-Step -Name ("rollout-" + $stage.name + "-status") -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status"
    )
    Invoke-Step -Name ("rollout-" + $stage.name + "-health") -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath,
      "--session", $stage.session, "system", "health"
    )
  }

  Invoke-Rollback -Reason "post_full_drill"
  $scriptSucceeded = $true
}
catch {
  $allPassed = $false
  $failureMessage = $_.Exception.Message
  Add-Content -Path $logPath -Value ("`n==== FAILURE ====")
  Add-Content -Path $logPath -Value $failureMessage
  Invoke-Rollback -Reason "auto_rollback_on_failure"
}
finally {
  if ($null -eq $previousAutoLoopProfile) {
    Remove-Item Env:AUTOLOOP_PROFILE -ErrorAction SilentlyContinue
  } else {
    $env:AUTOLOOP_PROFILE = $previousAutoLoopProfile
  }
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    backup_config = $backupPath
    all_passed = $allPassed
    dry_run = [bool]$DryRun
    rollout = @("shadow", "10%", "30%", "full", "rollback")
    failure = $failureMessage
    rollback = $rollbackRecord
    commands = $results
    log_path = $logPath
  }

  if ($summaryPath) {
    $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $summaryPath -Encoding utf8
    if ($allPassed) {
      Write-Output ("D14_ROLLOUT_OK log=" + $logPath)
      Write-Output ("D14_ROLLOUT_JSON=" + $summaryPath)
    } else {
      Write-Output ("D14_ROLLOUT_FAILED log=" + $logPath)
      Write-Output ("D14_ROLLOUT_JSON=" + $summaryPath)
    }
  }

  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  if ($scriptSucceeded) {
    if ($null -ne $targetDir -and (Test-Path $targetDir)) {
      Remove-Item -LiteralPath $targetDir -Recurse -Force -ErrorAction SilentlyContinue
    }
  } else {
    $cutoff = (Get-Date).AddDays(-7)
    $targets = Get-ChildItem -Path (Join-Path $repoRoot "deploy\runtime") -Directory -Filter "target-d14-*" -ErrorAction SilentlyContinue |
      Sort-Object LastWriteTime -Descending
    $keepOne = $targets | Select-Object -First 1
    foreach ($d in $targets) {
      if ($null -ne $keepOne -and $d.FullName -eq $keepOne.FullName) {
        if ($d.LastWriteTime -lt $cutoff) {
          Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction SilentlyContinue
        }
        continue
      }
      Remove-Item -LiteralPath $d.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
  }
  Pop-Location
}

if (-not $allPassed) {
  exit 1
}
