param(
  [string]$RuntimeDir = "deploy/runtime",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml"
)

$ErrorActionPreference = "Stop"
$TaskRecordMinBytes = 512
$TaskRecordMaxBytes = 3072
$ReleaseRecordMinBytes = 1024
$ReleaseRecordMaxBytes = 4096

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

function Get-JsonSizeBytes {
  param([object]$Object)
  $json = $Object | ConvertTo-Json -Compress -Depth 10
  return ([System.Text.Encoding]::UTF8.GetByteCount($json))
}

function Enforce-Budget {
  param(
    [string]$Kind,
    [int]$ActualBytes,
    [int]$MinBytes,
    [int]$LimitBytes
  )
  if ($ActualBytes -lt $MinBytes) {
    throw "evidence_budget_exceeded:${Kind}_bytes=$ActualBytes min=$MinBytes"
  }
  if ($ActualBytes -gt $LimitBytes) {
    throw "evidence_budget_exceeded:${Kind}_bytes=$ActualBytes limit=$LimitBytes"
  }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$runtimeAbs = if ([System.IO.Path]::IsPathRooted($RuntimeDir)) { $RuntimeDir } else { Join-Path $repoRoot $RuntimeDir }
if (-not (Test-Path $runtimeAbs)) { New-Item -ItemType Directory -Path $runtimeAbs | Out-Null }
$proofLedgerPath = Join-Path $runtimeAbs "proof_ledger.jsonl"

try {
  $commitSha = (& git -C $repoRoot rev-parse HEAD 2>$null).Trim()
  $releaseGatePath = Join-Path $runtimeAbs "release_gate.json"
  $dailyReleasePath = Join-Path $runtimeAbs "daily_release_package.json"
  $week6Path = Join-Path $runtimeAbs "week6_full_acceptance.json"

  $gate = if (Test-Path $releaseGatePath) { Get-Content -Raw -Path $releaseGatePath | ConvertFrom-Json } else { $null }
  $pkg = if (Test-Path $dailyReleasePath) { Get-Content -Raw -Path $dailyReleasePath | ConvertFrom-Json } else { $null }
  $week6 = if (Test-Path $week6Path) { Get-Content -Raw -Path $week6Path | ConvertFrom-Json } else { $null }

  $decisionRootSeed = "{0}|{1}|{2}" -f $commitSha, (($gate.decision | Out-String).Trim()), (($pkg.release_decision | Out-String).Trim())
  $decisionRoot = Get-StringSha256 -Text $decisionRootSeed

  $walTxIds = New-Object 'System.Collections.Generic.List[string]'
  foreach ($p in @($releaseGatePath, $dailyReleasePath, $week6Path)) {
    if (-not (Test-Path $p)) { continue }
    $lines = Get-Content -LiteralPath $p -ErrorAction SilentlyContinue
    foreach ($line in $lines) {
      $matches = [System.Text.RegularExpressions.Regex]::Matches($line, '"wal_tx_id"\s*:\s*"([^"]+)"')
      foreach ($m in $matches) { [void]$walTxIds.Add($m.Groups[1].Value) }
      $matches2 = [System.Text.RegularExpressions.Regex]::Matches($line, '"wal_id"\s*:\s*"([^"]+)"')
      foreach ($m in $matches2) { [void]$walTxIds.Add($m.Groups[1].Value) }
    }
  }
  $walRoot = Get-StringSha256 -Text (($walTxIds | Sort-Object -Unique) -join "|")

  $impactTotal = "0"
  if ($pkg -and $pkg.basis -and $pkg.basis.full52) { $impactTotal = [string]$pkg.basis.full52.total }
  $week6GeneratedAt = ""
  if ($week6) { $week6GeneratedAt = [string]$week6.generated_at }
  $pkgReplayFp = ""
  if ($pkg -and $pkg.replay_fp) { $pkgReplayFp = [string]$pkg.replay_fp }
  $impactHash = Get-StringSha256 -Text ("{0}|{1}|{2}" -f $impactTotal, $week6GeneratedAt, $pkgReplayFp)

  $proofInputs = @(
    [string]$commitSha,
    [string]$decisionRoot,
    [string]$walRoot,
    [string]$impactHash,
    [string]$releaseGatePath,
    [string]$dailyReleasePath,
    [string]$week6Path
  ) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
  $proofRecord = [ordered]@{
    kind = "task_proof"
    schema_version = "root-only-evidence/v1"
    task_id = if ($week6) { "week6" } else { "unknown-task" }
    commit_sha = $commitSha
    decision_root = $decisionRoot
    wal_root = $walRoot
    impact_hash = $impactHash
    input_hash = Get-StringSha256 -Text $decisionRootSeed
    output_hash = Get-StringSha256 -Text ($impactHash + "|" + $walRoot)
    prev_root = ""
    root = Get-StringSha256 -Text ($decisionRoot + "|" + $walRoot + "|" + $impactHash)
    input_refs = $proofInputs
    budget_window = [ordered]@{
      tier = "task"
      min_bytes = $TaskRecordMinBytes
      max_bytes = $TaskRecordMaxBytes
      target_note = "single task evidence 0.5KB-3KB"
    }
    created_at_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  }
  $proofBytes = Get-JsonSizeBytes -Object $proofRecord
  Enforce-Budget -Kind "task_record" -ActualBytes $proofBytes -MinBytes $TaskRecordMinBytes -LimitBytes $TaskRecordMaxBytes

  $releaseRecord = [ordered]@{
    kind = "release_proof"
    schema_version = "root-only-evidence/v1"
    release_id = if ($pkg) { "release:" + ([string]$pkg.generated_at) } else { "release:unknown" }
    commit_sha = $commitSha
    decision_root = $decisionRoot
    wal_root = $walRoot
    impact_hash = $impactHash
    allow_release = [bool]($pkg -and $pkg.allow_release -eq $true)
    source_refs = @(
      [string]$releaseGatePath,
      [string]$dailyReleasePath,
      [string]$week6Path
    )
    gate_checks = if ($gate -and $gate.checks) {
      @($gate.checks | ForEach-Object {
        [ordered]@{
          check_id = [string]$_.check_id
          passed = [bool]$_.passed
          deny_reason = if ($null -ne $_.deny_reason) { [string]$_.deny_reason } else { "" }
        }
      })
    } else {
      @()
    }
    package_meta = if ($pkg) {
      [ordered]@{
        package_version = [string]$pkg.package_version
        release_decision = [string]$pkg.release_decision
        deny_reasons = if ($pkg.deny_reasons) { @($pkg.deny_reasons) } else { @() }
        source_reports = if ($pkg.source_reports) { $pkg.source_reports } else { $null }
      }
    } else {
      $null
    }
    gate_basis = [ordered]@{
      decision = if ($gate) { [string]$gate.decision } else { "unknown" }
      release_decision = if ($pkg) { [string]$pkg.release_decision } else { "unknown" }
      rollback_ready = if ($pkg -and $pkg.incremental_gate) { [bool]$pkg.incremental_gate.rollback_ready } else { $false }
      impacted_tests_hash = if ($pkg -and $pkg.incremental_gate) { [string]$pkg.incremental_gate.impacted_tests_hash } else { "" }
    }
    budget_window = [ordered]@{
      tier = "release"
      min_bytes = $ReleaseRecordMinBytes
      max_bytes = $ReleaseRecordMaxBytes
      target_note = "single release evidence 1KB-4KB"
    }
    created_at_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
  }
  $releaseBytes = Get-JsonSizeBytes -Object $releaseRecord
  Enforce-Budget -Kind "release_record" -ActualBytes $releaseBytes -MinBytes $ReleaseRecordMinBytes -LimitBytes $ReleaseRecordMaxBytes

  ($proofRecord | ConvertTo-Json -Compress -Depth 10) | Out-File -FilePath $proofLedgerPath -Encoding utf8 -Append
  ($releaseRecord | ConvertTo-Json -Compress -Depth 10) | Out-File -FilePath $proofLedgerPath -Encoding utf8 -Append

  Write-Output ("PROOF_LEDGER_JSONL=" + $proofLedgerPath)
  Write-Output ("PROOF_LEDGER_APPEND_COUNT=2")
}
catch {
  $msg = $_.Exception.Message
  $denyReason = if ($msg -like "evidence_budget_exceeded*") { "evidence_budget_exceeded" } else { "evidence_write_failed" }
  $failureSummary = [ordered]@{
    kind = "failure_summary"
    schema_version = "root-only-evidence/v1"
    created_at_ms = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    deny_reason = $denyReason
    summary = $msg
  }
  ($failureSummary | ConvertTo-Json -Compress -Depth 8) | Out-File -FilePath $proofLedgerPath -Encoding utf8 -Append
  Write-Output ("deny_reason=" + $denyReason)
  Write-Output ("PROOF_LEDGER_JSONL=" + $proofLedgerPath)
  throw
}
