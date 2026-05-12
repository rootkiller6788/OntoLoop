# Script Entry Policy (Mainline vs Legacy)

This directory is split into:

- Mainline scripts (root of `deploy/scripts`): production acceptance/release chain only.
- Legacy scripts (`deploy/scripts/legacy`): archived historical entry points, not part of release gating.

## Mainline Entry Points (use these only)

- `week6_pipeline.ps1` / `week6_pipeline.sh`
- `week6_diagnostic.ps1` / `week6_diagnostic.sh`
- `week6_acceptance.ps1` / `week6_acceptance.sh`
- `release_gate_report.ps1` / `release_gate_report.sh`
- `d14_final_acceptance.ps1` / `d14_final_acceptance.sh`

## Benchmark Baseline Freeze (Phase A)

- `baseline_v0_freeze.ps1` / `baseline_v0_freeze.sh`

Purpose:

- Freeze the minimal benchmark baseline (`baseline_v0`) used for ablation.
- Scope is intentionally small: `Fast Harness + Code Harness`.
- Keep only hard constraints: artifact proof, root-only evidence, wal-root gate.

Output:

- `deploy/runtime/baseline_v0.json`

## Benchmark V1 (Phase B)

- `benchmark_v1_generate.ps1` / `benchmark_v1_generate.sh`
- `benchmark_v1_eval.ps1` / `benchmark_v1_eval.sh`
- `benchmark_v1_compare.ps1` / `benchmark_v1_compare.sh`

Purpose:

- Build real-task benchmark dataset with split isolation (`dev/heldout/stress`).
- Enforce required fields per task (`target_artifact_path`, `auto_verifier`, `success_definition`).
- Run fixed split evaluation via `d13_realbiz_benchmark_acceptance` with explicit dataset path.

## Release-Gate Required Inputs

Release gate now reads a single source of truth:

- `daily_release_package.json`

`daily_release_package.json` must aggregate:

- `week6_full_acceptance.json`
- `d13` report (full preferred, smoke fallback only if configured)
- `d14` rollout report
- `version-a` report
- `d46` SLO report
- rollback drill report
- fault drill report
- release gate report

`d14_final_acceptance.json` must verify:

- contract
- gate
- lease
- reviewgate
- wal
- ontoevent
- release package
- rollout chain (`shadow -> 10% -> 30% -> full -> rollback`)

## Execution Rule

- Do not run scripts from `legacy/` for release decisions.
- Production allow/block decision must come from `release_gate.json` only.
- `release_gate.json` must be derived only from `daily_release_package.json`.
