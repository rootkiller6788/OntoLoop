# Day10 E2E Acceptance

This document is the Day10 acceptance checklist for the AutoLoop governed runtime chain.

## Scope

Day10 verifies three things together:

1. E2E test coverage for requirement -> orchestration -> trigger/runtime -> query/replay artifact production.
2. Command-level acceptance path that can be replayed locally.
3. Evidence artifact outputs in `deploy/runtime` for audit.

## Test Matrix

Run from repository root (`autoloop-app`):

```powershell
cargo test --manifest-path .\Cargo.toml --test p10_day10_acceptance_e2e
cargo test --manifest-path .\Cargo.toml --test p7_trigger_wake_plan_execute_e2e
cargo test --manifest-path .\Cargo.toml --test p10_evidence_six_segments_e2e
cargo test --manifest-path .\Cargo.toml --test p10_replay_mismatch_explainer_e2e
```

Expected result: all tests pass.

## Command-Level Acceptance

Use the Day10 script:

```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\day10_acceptance.ps1
```

The script runs:

1. `cargo check`
2. Day10-related tests
3. Command-level checks:

```powershell
cargo run --manifest-path .\Cargo.toml -- --session day10-cli --tenant tenant:day10 --principal principal:day10 --policy policy:default --swarm --message "Run day10 acceptance chain"
cargo run --manifest-path .\Cargo.toml -- --session day10-cli system query --trace-id trace:day10 --output .\deploy\runtime\day10-query.json
cargo run --manifest-path .\Cargo.toml -- --session day10-cli system replay-report --output .\deploy\runtime\day10-replay-report.json
cargo run --manifest-path .\Cargo.toml -- --session day10-cli org context --output .\deploy\runtime\day10-org-context.json
cargo run --manifest-path .\Cargo.toml -- --session day10-cli trigger list --output .\deploy\runtime\day10-trigger-list.json
```

## Acceptance Artifacts

After a successful run:

- Log: `deploy/runtime/day10-acceptance.log`
- Summary JSON: `deploy/runtime/day10-acceptance.json`
- Query artifact: `deploy/runtime/day10-query.json`
- Replay report: `deploy/runtime/day10-replay-report.json`
- Org context snapshot: `deploy/runtime/day10-org-context.json`
- Trigger list snapshot: `deploy/runtime/day10-trigger-list.json`

## Pass Criteria

1. `day10-acceptance.json` has `all_passed = true`.
2. `day10-query.json` contains keys: `metrics`, `traces`, `events`, `ledger`, `graph`, `replay`.
3. `day10-replay-report.json` contains `session_id` and `reports`.
4. No command in the script exits non-zero.
