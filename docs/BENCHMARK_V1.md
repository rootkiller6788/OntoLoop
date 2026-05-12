# Benchmark V1 (Phase B)

Status: active  
Version: `benchmark_v1`

## Objective

Create a real-task benchmark suite for ablation and product evaluation without overfitting to a single workflow.

## Coverage

- Total tasks: `240`
- Categories: `8`
  - `frontend_replica`
  - `backend_feature`
  - `bug_fix`
  - `test_repair`
  - `refactor`
  - `deploy_script`
  - `multi_tool_orchestration`
  - `permission_reject`
- Splits:
  - `dev`: `120`
  - `heldout`: `80`
  - `stress`: `40`

## Required Per-Task Fields

Every task record must include:

1. `task_id`
2. `mode`
3. `category`
4. `split`
5. `target_artifact_path`
6. `auto_verifier`
7. `success_definition`
8. `prompt`

## Dataset Files

- [benchmark_v1 master](D:/AutoLoop/autoloop-app/deploy/benchmarks/benchmark_v1_master.json)
- [benchmark_v1 alias](D:/AutoLoop/autoloop-app/deploy/benchmarks/benchmark_v1.json)
- [benchmark_v1 dev](D:/AutoLoop/autoloop-app/deploy/benchmarks/benchmark_v1_dev.json)
- [benchmark_v1 heldout](D:/AutoLoop/autoloop-app/deploy/benchmarks/benchmark_v1_heldout.json)
- [benchmark_v1 stress](D:/AutoLoop/autoloop-app/deploy/benchmarks/benchmark_v1_stress.json)
- [benchmark_v1 manifest](D:/AutoLoop/autoloop-app/deploy/benchmarks/benchmark_v1_manifest.json)

## Generation

PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\benchmark_v1_generate.ps1 -Overwrite
```

Bash:

```bash
bash ./deploy/scripts/benchmark_v1_generate.sh deploy/benchmarks true
```

## Fixed Evaluation

PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\benchmark_v1_eval.ps1 -Split dev
```

Bash:

```bash
bash ./deploy/scripts/benchmark_v1_eval.sh ./Cargo.toml deploy/config/autoloop.prod.toml dev
```

Notes:

- `Split` supports `dev | heldout | stress | all`.
- Evaluation uses explicit dataset path and writes summary JSON to `deploy/runtime`.

## Phase C Compare (Control vs Experiment)

Control:

- `deploy/config/autoloop.opencode_like.toml`

Experiment:

- `deploy/config/autoloop.baseline_v0.toml`

PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\benchmark_v1_compare.ps1 -Split all
```

Bash:

```bash
bash ./deploy/scripts/benchmark_v1_compare.sh ./Cargo.toml deploy/config/autoloop.opencode_like.toml deploy/config/autoloop.baseline_v0.toml all
```

Output:

- `deploy/runtime/benchmark_v1_compare_<split>_<timestamp>.json`

Metrics in report:

- success rate
- p50/p95 latency
- cost (micros, when emitted by runtime)
- retry counts
- failure classification
