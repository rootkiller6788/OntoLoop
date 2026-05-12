# Baseline V0 (Phase A)

Status: Frozen benchmark baseline  
Version: `baseline_v0`

## Goal

Provide a stable, minimal comparison target (closest to fast coding harness behavior) before adding governance layers one-by-one in ablation.

## Included Modules

1. `Fast Harness`
- session loop
- streaming events
- command interaction
- adapter surface

2. `Code Harness`
- repo context bundle
- patch/apply path
- shell execution loop
- test verifier

## Hard Constraints Kept

1. Artifact must land:
- `write_proof + hash + evidence_ref`

2. Root-only evidence:
- `ROOT_ONLY_EVIDENCE_CONTRACT_VERSION`

3. Release gate wal root:
- release gate must include `decision_root + wal_root + impacted_tests_hash + rollback_ready`

## Config Profile

- `deploy/config/autoloop.baseline_v0.toml`
- load via profile alias:
  - `baseline-v0`
  - `baseline_v0`
  - `baseline`

## Freeze Script

PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\baseline_v0_freeze.ps1
```

Bash:

```bash
bash ./deploy/scripts/baseline_v0_freeze.sh
```

Output:

- `deploy/runtime/baseline_v0.json`
- Contains baseline id, config hash, module fingerprints, and pass/fail checks.
