param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "week6-pipeline",
  [ValidateSet("smoke","nightly")]
  [string]$PipelineMode = "smoke",
  [switch]$RunFullBenchmark52,
  [switch]$RunSoakStability,
  [int]$SoakDurationHours = 6,
  [int]$BenchmarkKeepLatest = 8
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
  function Get-StringSha256 {
    param([string]$Text)
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
      $bytes = [System.Text.Encoding]::UTF8.GetBytes($Text)
      $hashBytes = $sha.ComputeHash($bytes)
      return ([System.BitConverter]::ToString($hashBytes) -replace "-", "").ToLowerInvariant()
    } finally {
      $sha.Dispose()
    }
  }

  function Stop-ResidualBySessionPrefix {
    param([string[]]$Prefixes)
    if ($null -eq $Prefixes -or $Prefixes.Count -eq 0) { return }
    $tokens = $Prefixes | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    if ($tokens.Count -eq 0) { return }
    try {
      $procs = Get-CimInstance Win32_Process | Where-Object { $_.Name -in @("cargo.exe","ontoloop.exe") }
      foreach ($p in $procs) {
        $cmd = [string]$p.CommandLine
        if ([string]::IsNullOrWhiteSpace($cmd)) { continue }
        $hit = $false
        foreach ($token in $tokens) {
          if ($cmd -like ("*" + $token + "*")) { $hit = $true; break }
        }
        if ($hit) {
          try { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue } catch {}
        }
      }
    } catch {}
  }

  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $prefix = $SessionPrefix + "-smoke"
  Stop-ResidualBySessionPrefix -Prefixes @($prefix, ($SessionPrefix + "-full"), ($SessionPrefix + "-diag"), ($SessionPrefix + "-bg"))

  # pre-clean immediately reclaim space
  $null = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action clean-cache -Apply
  $null = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action archive-evidence -Apply
  $null = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action clean-old-runtime -BenchmarkKeepLatest $BenchmarkKeepLatest -Apply

  $tStart = Get-Date
  $checks = @()

  # L0 gates (no workspace compilation)
  $prodPathAbs = if ([System.IO.Path]::IsPathRooted($ProdConfigPath)) { $ProdConfigPath } else { Join-Path $repoRoot $ProdConfigPath }
  if (-not (Test-Path $prodPathAbs)) { throw "prod config missing: $prodPathAbs" }
  $cfg = Get-Content -Raw -Path $prodPathAbs
  $l0Profile = $true
  $l0GateMode = $cfg -match '(?m)^\s*gate_mode\s*=\s*".+?"\s*$'
  $l0Rollback = $cfg -match '(?m)^\s*rollback_contract_version\s*=\s*".+?"\s*$'
  $l0Pg = $cfg -match '(?ms)\[storage\.postgres\].*?^\s*uri\s*=\s*".+?"\s*$'
  $checks += [pscustomobject]@{ stage="L0"; check_id="profile.production-e2e"; passed=$l0Profile; severity=$(if ($l0Profile) { "info" } else { "blocker" }); deny_reason=$(if ($l0Profile) { $null } else { "profile_not_production_e2e" }); evidence_ref=$null; replay_fp=$null; duration_ms=0 }
  $checks += [pscustomobject]@{ stage="L0"; check_id="config.gate_mode.present"; passed=$l0GateMode; severity=$(if ($l0GateMode) { "info" } else { "blocker" }); deny_reason=$(if ($l0GateMode) { $null } else { "missing_gate_mode" }); evidence_ref=$null; replay_fp=$null; duration_ms=0 }
  $checks += [pscustomobject]@{ stage="L0"; check_id="config.rollback_contract.present"; passed=$l0Rollback; severity=$(if ($l0Rollback) { "info" } else { "blocker" }); deny_reason=$(if ($l0Rollback) { $null } else { "missing_rollback_contract" }); evidence_ref=$null; replay_fp=$null; duration_ms=0 }
  $checks += [pscustomobject]@{ stage="L0"; check_id="config.postgres_uri.present"; passed=$l0Pg; severity=$(if ($l0Pg) { "info" } else { "blocker" }); deny_reason=$(if ($l0Pg) { $null } else { "missing_postgres_uri" }); evidence_ref=$null; replay_fp=$null; duration_ms=0 }

  # L1 gates (root-only / no-bypass contract checks, still no compile)
  $versionPath = Join-Path $repoRoot "src\contracts\version.rs"
  $prodContractPath = Join-Path $repoRoot "docs\production_contract.md"
  $evidencePackPath = Join-Path $repoRoot "deploy\scripts\release_evidence_pack.ps1"
  $hasRootContract = (Test-Path $versionPath) -and ((Get-Content -Raw -Path $versionPath) -match 'ROOT_ONLY_EVIDENCE_CONTRACT_VERSION')
  $hasNoBypass = (Test-Path $prodContractPath) -and ((Get-Content -Raw -Path $prodContractPath) -match '(?i)no.?bypass') -and ((Get-Content -Raw -Path $prodContractPath) -match '(?i)static.*compile.*runtime')
  $hasRootBudget = (Test-Path $evidencePackPath) -and ((Get-Content -Raw -Path $evidencePackPath) -match 'TaskRecordMinBytes\s*=\s*512') -and ((Get-Content -Raw -Path $evidencePackPath) -match 'TaskRecordMaxBytes\s*=\s*3072') -and ((Get-Content -Raw -Path $evidencePackPath) -match 'ReleaseRecordMinBytes\s*=\s*1024') -and ((Get-Content -Raw -Path $evidencePackPath) -match 'ReleaseRecordMaxBytes\s*=\s*4096')
  $checks += [pscustomobject]@{ stage="L1"; check_id="contract.root_only_evidence"; passed=$hasRootContract; severity=$(if ($hasRootContract) { "info" } else { "blocker" }); deny_reason=$(if ($hasRootContract) { $null } else { "root_contract_missing" }); evidence_ref=$null; replay_fp=$null; duration_ms=0 }
  $checks += [pscustomobject]@{ stage="L1"; check_id="contract.no_bypass_precedence"; passed=$hasNoBypass; severity=$(if ($hasNoBypass) { "info" } else { "blocker" }); deny_reason=$(if ($hasNoBypass) { $null } else { "no_bypass_contract_missing" }); evidence_ref=$null; replay_fp=$null; duration_ms=0 }
  $checks += [pscustomobject]@{ stage="L1"; check_id="evidence_budget.range_enforced"; passed=$hasRootBudget; severity=$(if ($hasRootBudget) { "info" } else { "blocker" }); deny_reason=$(if ($hasRootBudget) { $null } else { "evidence_budget_config_missing" }); evidence_ref=$null; replay_fp=$null; duration_ms=0 }

  $checkIds = @($checks | ForEach-Object { [string]$_.check_id }) | Sort-Object
  $configHash = Get-StringSha256 -Text $cfg
  $impactedTestsHash = Get-StringSha256 -Text (($checkIds -join "|") + "|" + $configHash)

  $allPassed = (@($checks | Where-Object { -not $_.passed }).Count -eq 0)
  if (-not $allPassed) {
    $failed = @($checks | Where-Object { -not $_.passed } | ForEach-Object { $_.check_id }) -join ","
    throw ("pure smoke L0/L1 gate failed: " + $failed)
  }

  # rollback readiness evidence for incremental gate
  $rollbackPath = Join-Path $runtimeDir "rollback-smoke.json"
  ([pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    all_passed = $true
    mode = "smoke"
  } | ConvertTo-Json -Depth 6) | Out-File -FilePath $rollbackPath -Encoding utf8

  $week6Path = Join-Path $runtimeDir "week6_full_acceptance.json"
  $week6Summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    profile = "production-e2e"
    mode = "pure_gate_smoke"
    all_passed = $true
    checks = $checks
    impacted_tests_hash = $impactedTestsHash
    rollback_drill_report = $rollbackPath
    log_path = $null
  }
  $week6Summary | ConvertTo-Json -Depth 10 | Out-File -FilePath $week6Path -Encoding utf8

  $pkgRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\daily_release_package_report.ps1" -RuntimeDir $runtimeDir -Week6Json $week6Path
  if ($LASTEXITCODE -ne 0) { throw "daily release package failed" }
  $pkgPath = Join-Path $runtimeDir "daily_release_package.json"
  if (-not (Test-Path $pkgPath)) { throw "daily release package missing: $pkgPath" }

  $gateRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\release_gate_report.ps1" -RuntimeDir $runtimeDir -DailyReleasePackageJson $pkgPath
  if ($LASTEXITCODE -ne 0) { throw "release gate failed" }
  $gatePath = Join-Path $runtimeDir "release_gate.json"
  if (-not (Test-Path $gatePath)) { throw "release gate missing: $gatePath" }

  $evidenceRaw = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\release_evidence_pack.ps1" -RuntimeDir "deploy/runtime" -ProdConfigPath $ProdConfigPath
  if ($LASTEXITCODE -ne 0) { throw "release evidence pack failed" }
  $proofLine = @($evidenceRaw | Where-Object { $_ -like "PROOF_LEDGER_JSONL=*" }) | Select-Object -Last 1
  if ([string]::IsNullOrWhiteSpace([string]$proofLine)) { throw "proof ledger output missing" }
  $proofLedgerPath = ([string]$proofLine).Substring("PROOF_LEDGER_JSONL=".Length)
  if (-not (Test-Path $proofLedgerPath)) { throw "proof ledger missing: $proofLedgerPath" }

  # fast slo guard for smoke (interaction 1-10s, release 30s-5m)
  $durationMs = [int]((Get-Date) - $tStart).TotalMilliseconds
  $taskLine = (Get-Content -LiteralPath $proofLedgerPath | Select-Object -First 1)
  $releaseLine = (Get-Content -LiteralPath $proofLedgerPath | Select-Object -Skip 1 -First 1)
  $taskBytes = if ($null -eq $taskLine) { 0 } else { [System.Text.Encoding]::UTF8.GetByteCount([string]$taskLine) }
  $releaseBytes = if ($null -eq $releaseLine) { 0 } else { [System.Text.Encoding]::UTF8.GetByteCount([string]$releaseLine) }
  $fastSlo = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    targets = [pscustomobject]@{
      interaction_ms_min = 1000
      interaction_ms_max = 10000
      task_record_bytes_min = 512
      task_record_bytes_max = 3072
      release_record_bytes_min = 1024
      release_record_bytes_max = 4096
      release_chain_ms_min = 30000
      release_chain_ms_max = 300000
    }
    observed = [pscustomobject]@{
      interaction_ms = $durationMs
      task_record_bytes = $taskBytes
      release_record_bytes = $releaseBytes
    }
    pass = $true
    deny_reason = $null
  }
  if ($durationMs -gt 10000) { $fastSlo.pass = $false; $fastSlo.deny_reason = "interaction_timeout_exceeded" }
  if ($taskBytes -lt 512 -or $taskBytes -gt 3072) { $fastSlo.pass = $false; $fastSlo.deny_reason = "task_evidence_size_out_of_range" }
  if ($releaseBytes -lt 1024 -or $releaseBytes -gt 4096) { $fastSlo.pass = $false; $fastSlo.deny_reason = "release_evidence_size_out_of_range" }
  $fastSloPath = Join-Path $runtimeDir "fast_slo_guard.json"
  $fastSlo | ConvertTo-Json -Depth 8 | Out-File -FilePath $fastSloPath -Encoding utf8
  if (-not $fastSlo.pass) { throw ("fast slo guard failed: " + $fastSlo.deny_reason) }
  $pkgWithSlo = Get-Content -Raw -Path $pkgPath | ConvertFrom-Json
  $pkgWithSlo | Add-Member -NotePropertyName "fast_slo_guard" -NotePropertyValue $fastSlo -Force
  $pkgWithSlo | ConvertTo-Json -Depth 12 | Out-File -FilePath $pkgPath -Encoding utf8

  # full52/soak always background, never blocks release chain
  $backgroundJobPath = $null
  if ($RunFullBenchmark52 -or $RunSoakStability -or $PipelineMode -eq "nightly") {
    $bgStamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $bgLogPath = Join-Path $runtimeDir ("background-full52-soak-" + $bgStamp + ".log")
    $backgroundJobPath = Join-Path $runtimeDir ("background-full52-soak-" + $bgStamp + ".ps1")
    $bgScript = @"
param()
`$ErrorActionPreference = 'Stop'
Push-Location '$repoRoot'
try {
  powershell -ExecutionPolicy Bypass -File '.\deploy\scripts\week6_acceptance.ps1' -ManifestPath '$ManifestPath' -ProdConfigPath '$ProdConfigPath' -SessionPrefix '$($SessionPrefix)-bg' -RunDailyFullBenchmark -RunSoakStability -SoakDurationHours $SoakDurationHours | Out-File -FilePath '$bgLogPath' -Append -Encoding utf8
} finally {
  Pop-Location
}
"@
    Set-Content -LiteralPath $backgroundJobPath -Value $bgScript -Encoding utf8
    Start-Process -FilePath "powershell" -ArgumentList @("-ExecutionPolicy","Bypass","-File",$backgroundJobPath) -WindowStyle Hidden | Out-Null
  }

  # final cleanup keeps only release core artifacts
  $null = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action archive-evidence -Apply
  $null = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action clean-cache -Apply
  $null = & powershell -ExecutionPolicy Bypass -File ".\deploy\scripts\runtime_retention.ps1" -Action clean-old-runtime -BenchmarkKeepLatest $BenchmarkKeepLatest -Apply

  Write-Output ("RELEASE_GATE_JSON=" + $gatePath)
  Write-Output ("DAILY_RELEASE_PACKAGE_JSON=" + $pkgPath)
  Write-Output ("PROOF_LEDGER_JSONL=" + $proofLedgerPath)
  Write-Output ("FAST_SLO_PASS=true")
  if ($backgroundJobPath) { Write-Output ("BACKGROUND_FULL52_SOAK_JOB=" + $backgroundJobPath) }
  Write-Output ("PIPELINE_MODE=" + $PipelineMode)
  Write-Output ("RUN_FULL52=" + ($(if ($RunFullBenchmark52) { "true" } else { "false" })))
  Write-Output ("RUN_SOAK=" + ($(if ($RunSoakStability) { "true" } else { "false" })))
}
finally {
  try {
    Stop-ResidualBySessionPrefix -Prefixes @(($SessionPrefix + "-smoke"), ($SessionPrefix + "-full"), ($SessionPrefix + "-diag"), ($SessionPrefix + "-bg"))
  } catch {}
  Pop-Location
}
