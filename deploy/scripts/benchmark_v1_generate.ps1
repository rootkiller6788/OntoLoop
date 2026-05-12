param(
  [string]$OutputDir = "deploy/benchmarks",
  [switch]$Overwrite
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Push-Location $repoRoot

try {
  $outDirAbs = if ([System.IO.Path]::IsPathRooted($OutputDir)) { $OutputDir } else { Join-Path $repoRoot $OutputDir }
  if (-not (Test-Path $outDirAbs)) {
    New-Item -ItemType Directory -Path $outDirAbs | Out-Null
  }

  $masterPath = Join-Path $outDirAbs "benchmark_v1_master.json"
  $aliasPath = Join-Path $outDirAbs "benchmark_v1.json"
  $devPath = Join-Path $outDirAbs "benchmark_v1_dev.json"
  $heldoutPath = Join-Path $outDirAbs "benchmark_v1_heldout.json"
  $stressPath = Join-Path $outDirAbs "benchmark_v1_stress.json"
  $manifestPath = Join-Path $outDirAbs "benchmark_v1_manifest.json"

  $targets = @($masterPath, $aliasPath, $devPath, $heldoutPath, $stressPath, $manifestPath)
  if (-not $Overwrite.IsPresent) {
    foreach ($file in $targets) {
      if (Test-Path $file) {
        throw ("target exists; rerun with -Overwrite: " + $file)
      }
    }
  }

  $categories = @(
    @{
      id = "frontend_replica"; prefix = "fe"; ext = "html"; mode = "direct"; prompt = "Build a production-style frontend replica page";
      scenarios = @("billing dashboard","crypto exchange landing","incident timeline board","release gate report console","tenant permission matrix page","pipeline observability panel","rollback drill report page","artifact evidence explorer","multi-session command center","benchmark leaderboard")
    },
    @{
      id = "backend_feature"; prefix = "be"; ext = "md"; mode = "direct"; prompt = "Implement backend feature behavior spec with API contract notes";
      scenarios = @("quota window settlement","session lease renewal","policy decision cache","relation edge pagination","write proof hashing","query replay lookup","signal ingest batching","review gate transition","rollback marker persistence","budget preflight response")
    },
    @{
      id = "bug_fix"; prefix = "bug"; ext = "md"; mode = "direct"; prompt = "Reproduce and fix a real bug, then summarize root cause and fix steps";
      scenarios = @("path traversal guard break","session id filename collision","double write proof emission","stale relation snapshot","timeout retry misclassification","schema mismatch in query plane","gate decision shadow drift","wal append ordering issue","invalid evidence hash parse","tool event orphan chain")
    },
    @{
      id = "test_repair"; prefix = "test"; ext = "md"; mode = "direct"; prompt = "Repair failing tests and document verifier plan";
      scenarios = @("flaky relation assertion","missing artifact proof check","wrong policy deny reason","non-deterministic replay fingerprint","unstable timeout threshold","bridge event ordering","shadow/full profile drift","waltx partial failure coverage","no-bypass scanner false pass","permission callback race")
    },
    @{
      id = "refactor"; prefix = "ref"; ext = "md"; mode = "direct"; prompt = "Perform safe refactor plan with behavior-preserving checks";
      scenarios = @("command dispatch split","query plane aggregation cleanup","config loader normalization","relation facade extraction","evidence writer simplification","session runtime boundary","tool adapter unification","state transition reducer","signal pipeline segmentation","benchmark reporter isolation")
    },
    @{
      id = "deploy_script"; prefix = "dep"; ext = "sh"; mode = "direct"; prompt = "Create deployment or rollback script with safety checks";
      scenarios = @("smoke release gate","canary rollout with rollback","nightly full benchmark","soak monitor runner","fault injection daily drill","config doctor preflight","postgres schema isolate","runtime retention cleanup","evidence pack export","release summary publisher")
    },
    @{
      id = "multi_tool_orchestration"; prefix = "mto"; ext = "json"; mode = "swarm"; prompt = "Run multi-tool orchestration and provide execution graph";
      scenarios = @("read->patch->test->verify flow","benchmark->relation->replay chain","policy->approval->tool execution","frontend->artifact->evidence close loop","dual-lane plan/execute split","parallel verifier and report","shadow compare and explain","batch signal ingest and drain","recovery drill orchestration","cross-session resume pipeline")
    },
    @{
      id = "permission_reject"; prefix = "perm"; ext = "md"; mode = "swarm"; prompt = "Execute permission-sensitive flow and prove correct reject/allow decision";
      scenarios = @("unauthorized production write","missing evidence_ref commit","forbidden direct provider call","expired session lease action","high-risk without approval","fallback write in full mode","cross-tenant read attempt","tool scope overreach","bypass static scan violation","rollback window expired")
    }
  )

  $all = New-Object System.Collections.Generic.List[object]
  foreach ($cat in $categories) {
    for ($i = 1; $i -le 30; $i++) {
      $split = if ($i -le 15) { "dev" } elseif ($i -le 23) { "heldout" } else { "stress" }
      $taskNo = "{0:000}" -f $i
      $taskId = "bv1-" + $cat.prefix + "-" + $taskNo
      $artifactPath = "output/benchmark_v1/" + $split + "/" + $cat.id + "/" + $taskId + "." + $cat.ext
      $mode = if ($split -eq "stress" -and $cat.mode -eq "direct") { "swarm" } else { $cat.mode }
      $scenario = [string]$cat.scenarios[(($i - 1) % $cat.scenarios.Count)]
      $prompt = @(
        "[BenchmarkV1]"
        "Category: " + $cat.id
        "Split: " + $split
        "TaskId: " + $taskId
        "Scenario: " + $scenario
        "Goal: " + $cat.prompt + " for this scenario."
        "Must write the final artifact to path: " + $artifactPath
        "Hard contract: artifact_delivery/v1 requires_artifact artifact contract."
        "Verifier requirements: file exists, non-empty, sha256 present, evidence_ref present, relation trace present."
        "Output style: first complete the file write using tools, then return a concise summary with assumptions."
      ) -join "`n"

      $record = [ordered]@{
        task_id = $taskId
        mode = $mode
        category = $cat.id
        split = $split
        target_artifact_path = $artifactPath
        auto_verifier = "artifact_exists_nonempty_sha256_evidence_relation"
        success_definition = "artifact_written_and_verified_with_write_proof_hash_evidence_ref_and_relation_trace"
        prompt = $prompt
      }
      $all.Add([pscustomobject]$record)
    }
  }

  $allArray = @($all.ToArray())
  $dev = @($allArray | Where-Object { $_.split -eq "dev" })
  $heldout = @($allArray | Where-Object { $_.split -eq "heldout" })
  $stress = @($allArray | Where-Object { $_.split -eq "stress" })

  $manifest = [ordered]@{
    generated_at = (Get-Date).ToString("s")
    benchmark = "benchmark_v1"
    total = $allArray.Count
    required_categories = @($categories | ForEach-Object { $_.id })
    split_counts = [ordered]@{
      dev = $dev.Count
      heldout = $heldout.Count
      stress = $stress.Count
    }
    category_counts = [ordered]@{}
  }
  foreach ($cat in $categories) {
    $manifest.category_counts[$cat.id] = (@($allArray | Where-Object { $_.category -eq $cat.id })).Count
  }

  $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
  [System.IO.File]::WriteAllText($masterPath, ($allArray | ConvertTo-Json -Depth 8), $utf8NoBom)
  [System.IO.File]::WriteAllText($aliasPath, ($allArray | ConvertTo-Json -Depth 8), $utf8NoBom)
  [System.IO.File]::WriteAllText($devPath, ($dev | ConvertTo-Json -Depth 8), $utf8NoBom)
  [System.IO.File]::WriteAllText($heldoutPath, ($heldout | ConvertTo-Json -Depth 8), $utf8NoBom)
  [System.IO.File]::WriteAllText($stressPath, ($stress | ConvertTo-Json -Depth 8), $utf8NoBom)
  [System.IO.File]::WriteAllText($manifestPath, (([pscustomobject]$manifest) | ConvertTo-Json -Depth 8), $utf8NoBom)

  Write-Output ("BENCHMARK_V1_MASTER=" + $masterPath)
  Write-Output ("BENCHMARK_V1_ALIAS=" + $aliasPath)
  Write-Output ("BENCHMARK_V1_DEV=" + $devPath)
  Write-Output ("BENCHMARK_V1_HELDOUT=" + $heldoutPath)
  Write-Output ("BENCHMARK_V1_STRESS=" + $stressPath)
  Write-Output ("BENCHMARK_V1_MANIFEST=" + $manifestPath)
}
finally {
  Pop-Location
}
