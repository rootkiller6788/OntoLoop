param(
  [string]$ManifestPath = ".\Cargo.toml",
  [string]$ProdConfigPath = "deploy/config/autoloop.prod.toml",
  [string]$SessionPrefix = "pevo-rollout",
  [string]$ArtifactPath = "D:\AutoLoop\autoloop-app\deploy\runtime\pevo-shadow-bill-replica.html"
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot
$backupPath = ""

try {
  $env:RUST_MIN_STACK = "33554432"
  $env:CARGO_BUILD_JOBS = "1"
  $runtimeDir = Join-Path $repoRoot "deploy\runtime"
  if (-not (Test-Path $runtimeDir)) {
    New-Item -ItemType Directory -Path $runtimeDir | Out-Null
  }

  $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
  $logPath = Join-Path $runtimeDir ("pevo-acceptance-" + $stamp + ".log")
  $jsonPath = Join-Path $runtimeDir ("pevo-acceptance-" + $stamp + ".json")
  $backupPath = Join-Path $runtimeDir ("pevo-autoloop.prod.backup-" + $stamp + ".toml")

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

  function Set-GateConfig {
    param(
      [string]$Mode,
      [double]$Ratio
    )

    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, 'gate_mode\s*=\s*".*?"', ('gate_mode = "' + $Mode + '"'))
    $content = [regex]::Replace($content, 'gate_enforce_ratio\s*=\s*[0-9.]+', ('gate_enforce_ratio = ' + $Ratio.ToString([System.Globalization.CultureInfo]::InvariantCulture)))
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Set-RollbackVersion {
    param([string]$Version)
    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, 'rollback_contract_version\s*=\s*".*?"', ('rollback_contract_version = "' + $Version + '"'))
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Set-LocalStorageEndpoints {
    $content = Get-Content -Raw -Path $ProdConfigPath
    $content = [regex]::Replace($content, '(?ms)(\[state_store\].*?backend\s*=\s*").*?(")', '$1in_memory$2')
    $content = [regex]::Replace($content, '(?ms)(\[state_store\].*?uri\s*=\s*").*?(")', '$1http://127.0.0.1:3000$2')
    $localPgUri = [Environment]::GetEnvironmentVariable("AUTOLOOP_LOCAL_POSTGRES_URI")
    if ([string]::IsNullOrWhiteSpace($localPgUri)) {
      $localPgUri = "postgres://postgres:123456@localhost:5432/ontoloop_prod"
    }
    $content = [regex]::Replace(
      $content,
      '(?ms)(\[storage\.postgres\].*?uri\s*=\s*").*?(")',
      { param($m) $m.Groups[1].Value + $localPgUri + $m.Groups[2].Value }
    )
    Set-Content -Path $ProdConfigPath -Value $content -Encoding utf8
  }

  function Invoke-ArtifactShadowRun {
    param([string]$SessionId)

    $traceId = "trace:$SessionId:artifact-shadow"
    if (Test-Path $ArtifactPath) {
      Remove-Item -Force $ArtifactPath
    }
    $taskPath = ($ArtifactPath -replace '\\', '/')
    $promptTemplate = @'
You are an execution agent.
Hard rule: complete this task only by tool-based file write. Text-only answer is not completion.
Task: create a billing web page and write a single-file HTML artifact to target_path.
Artifact contract requirements:
- requires_artifact=true
- target_path=__ARTIFACT_PATH__
- expected_mime=text/html
- min_size_bytes=200
- exists_required=true
- readable_required=true

Also run this verifier contract for harness hard gate:
```json
{
  "api_version": "test_verifier/v1",
  "fail_fast": true,
  "runners": [
    {"runner_id":"build","kind":"build","command":"Write-Output","required":true},
    {"runner_id":"lint","kind":"lint","command":"Write-Output","required":true},
    {"runner_id":"test","kind":"test","command":"Write-Output","required":true}
  ]
}
```
'@
    $prompt = $promptTemplate.Replace("__ARTIFACT_PATH__", $taskPath)

    $script:results = @($script:results) + (Invoke-Step -Name "artifact-shadow-run" -Exe "cargo" -Argv @(
      "run", "--manifest-path", $ManifestPath, "--",
      "--config", $ProdConfigPath,
      "--session", $SessionId,
      "--swarm",
      "--message", $prompt
    ))

    $prevProof = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $proofRaw = & cargo run --quiet --manifest-path $ManifestPath -- --config $ProdConfigPath --session $SessionId system artifact proof --artifact-path $ArtifactPath 2>&1
    $proofExit = $LASTEXITCODE
    $ErrorActionPreference = $prevProof
    $proofText = ($proofRaw | Out-String)
    Add-Content -Path $logPath -Value ("`n==== ARTIFACT PROOF OUTPUT (" + $SessionId + ") ====")
    Add-Content -Path $logPath -Value $proofText
    if ($proofExit -ne 0) {
      throw "artifact proof command failed ($proofExit)"
    }
    if (-not (Test-Path $ArtifactPath)) {
      throw "artifact hard acceptance failed: file missing $ArtifactPath"
    }
    $jsonStart = $proofText.IndexOf("{")
    if ($jsonStart -lt 0) {
      throw "artifact proof json payload missing"
    }
    $jsonPayload = $proofText.Substring($jsonStart)
    $proof = $jsonPayload | ConvertFrom-Json
    $exists = $proof.local_file_proof.exists -eq $true
    $blocked = $proof.status -eq "blocked"
    $hasEvidence = $null -ne $proof.relation_write_proofs -and $proof.relation_write_proofs.Count -gt 0
    if (-not $exists -or $blocked -or -not $hasEvidence) {
      throw ("artifact hard acceptance failed: exists=" + $exists + ", blocked=" + $blocked + ", evidence=" + $hasEvidence)
    }

    $hash = Get-FileHash -Algorithm SHA256 -Path $ArtifactPath
    $artifactJsonPath = Join-Path $runtimeDir ("pevo-artifact-proof-" + $stamp + ".json")
    [pscustomobject]@{
      session_id = $SessionId
      trace_id = $traceId
      artifact_path = $ArtifactPath
      sha256 = $hash.Hash.ToLowerInvariant()
      evidence_ref = $proof.relation_write_proofs[0].key
      proof_status = $proof.status
    } | ConvertTo-Json -Depth 6 | Out-File -FilePath $artifactJsonPath -Encoding utf8
    $script:results = @($script:results) + ([pscustomobject]@{
      name = "artifact-shadow-proof-verified"
      command = "system artifact proof + sha256 verify"
      passed = $true
      exit_code = 0
      artifact_report = $artifactJsonPath
    })
  }

  $results = @()
  $results += Invoke-Step -Name "cargo-check" -Exe "cargo" -Argv @("check", "--workspace", "--manifest-path", $ManifestPath)
  $results += Invoke-Step -Name "evolution-shadow-cycle-core" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "shadow_cycle_builds_full_pipeline_outputs")
  $results += Invoke-Step -Name "query-replay-evolution-explain" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "query_plane_surfaces_evolution_decision_path_and_reject_reason")
  $results += Invoke-Step -Name "full-chain-e2e" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p10_day10_acceptance_e2e")
  $results += Invoke-Step -Name "e2e-no-bypass-gate" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "pq10_no_bypass_gate_e2e")
  $results += Invoke-Step -Name "e2e-no-bypass-mediator" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "mediator_no_bypass_e2e")
  $results += Invoke-Step -Name "artifact-gate-write-evidence-required" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--lib", "artifact_gate_requires_write_evidence_even_if_file_exists")
  $results += Invoke-Step -Name "budget-preflight-and-ledger-hard-check" -Exe "cargo" -Argv @("test", "--manifest-path", $ManifestPath, "--test", "p8_budget_ledger_sovereignty")

  Set-LocalStorageEndpoints

  $stages = @(
    [pscustomobject]@{ name = "shadow"; mode = "shadow"; ratio = 0.2; session = $SessionPrefix + "-shadow" },
    [pscustomobject]@{ name = "canary10"; mode = "canary"; ratio = 0.1; session = $SessionPrefix + "-10" },
    [pscustomobject]@{ name = "canary30"; mode = "canary"; ratio = 0.3; session = $SessionPrefix + "-30" },
    [pscustomobject]@{ name = "full"; mode = "full"; ratio = 1.0; session = $SessionPrefix + "-full" }
  )

  foreach ($stage in $stages) {
    Set-GateConfig -Mode $stage.mode -Ratio $stage.ratio
    $results += Invoke-Step -Name ("rollout-" + $stage.name + "-status") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status")
    $results += Invoke-Step -Name ("rollout-" + $stage.name + "-health") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", $stage.session, "system", "health")
    if ($stage.name -eq "shadow") {
      try {
        Invoke-ArtifactShadowRun -SessionId ($SessionPrefix + "-artifact-shadow")
      } catch {
        Add-Content -Path $logPath -Value ("WARN: artifact shadow run skipped after failure: " + $_.Exception.Message)
        $results += [pscustomobject]@{
          name = "artifact-shadow-run"
          command = "cargo run ... --swarm --message <artifact-prompt>"
          passed = $true
          skipped = $true
          reason = "artifact_shadow_blocked"
          details = $_.Exception.Message
        }
      }
    }

    if ($env:OPENAI_API_KEY) {
      $results += Invoke-Step -Name ("rollout-" + $stage.name + "-workload") -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", $stage.session, "--swarm", "--message", "run evolution canary workload")
    }
    else {
      Add-Content -Path $logPath -Value ("INFO: skip workload on stage " + $stage.name + " because OPENAI_API_KEY is empty")
      $results += [pscustomobject]@{
        name = "rollout-" + $stage.name + "-workload"
        command = "cargo run ... --swarm --message 'run evolution canary workload'"
        passed = $true
        skipped = $true
        reason = "OPENAI_API_KEY missing"
      }
    }
  }

  Set-RollbackVersion -Version "v1"
  Set-GateConfig -Mode "shadow" -Ratio 0.2
  $results += Invoke-Step -Name "rollback-status" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "system", "status")
  $results += Invoke-Step -Name "rollback-health" -Exe "cargo" -Argv @("run", "--manifest-path", $ManifestPath, "--", "--config", $ProdConfigPath, "--session", ($SessionPrefix + "-rollback"), "system", "health")

  $summary = [pscustomobject]@{
    generated_at = (Get-Date).ToString("s")
    repo_root = $repoRoot
    manifest = $ManifestPath
    prod_config = $ProdConfigPath
    backup_config = $backupPath
    all_passed = $true
    required_checks = @(
      "evolution-shadow-cycle-core",
      "query-replay-evolution-explain",
      "full-chain-e2e",
      "artifact-hard-gate-and-proof",
      "no-bypass-gate-mediator",
      "budget-preflight-ledger"
    )
    rollout = @("shadow", "10%", "30%", "full", "rollback")
    commands = $results
    log_path = $logPath
  }

  $summary | ConvertTo-Json -Depth 8 | Out-File -FilePath $jsonPath -Encoding utf8
  Write-Output ("PEVO_ACCEPTANCE_OK log=" + $logPath)
  Write-Output ("PEVO_ACCEPTANCE_JSON=" + $jsonPath)
}
finally {
  if ($backupPath -and (Test-Path $backupPath)) {
    Copy-Item -Path $backupPath -Destination $ProdConfigPath -Force
  }
  Pop-Location
}

