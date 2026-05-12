# Contract Migration Matrix v1 (A-phase)

Status: Frozen for current batch  
Version: v1  
Effective Date: 2026-05-01

## Purpose

Migrate legacy scripts/tests/config behavior to the production contract without relaxing hard gates.

Contract baseline (must stay unchanged): `docs/production_contract.md`

## Scope Boundary (Locked)

Allowed change surface:

1. `deploy/scripts/*`
2. `src/command_dispatch.rs`
3. Related E2E assertions and config entry points

Disallowed in this batch:

1. Cross-module refactor
2. New feature architecture
3. Unrelated cleanup

## Drift Matrix (Old -> New)

| ID | Legacy Behavior | Contract Requirement | Impacted Surface | Migration Action | Acceptance Evidence |
|---|---|---|---|---|---|
| M01 | Script/profile implicit defaults (`AppConfig::default` style behavior in E2E/script paths) | Explicit production profile required | `deploy/scripts/*`, E2E config entry | All production acceptance paths must pass explicit `--profile production-e2e` and/or `AUTOLOOP_PROFILE=production-e2e` | L0 `profile.alignment` pass in diagnostic/full JSON |
| M02 | `production-e2e` may skip when storage readiness fails | Production mirror readiness must hard-fail | Storage E2E + week6 scripts | Replace skip/soft handling with hard fail for production profile; missing Postgres mirror blocks | L0/L1 failure is blocker; no skip-green in report |
| M03 | Timeout/retry policy mixed with permission and other failure classes | Retry classification: timeout-only retry; permission/policy must not blind retry | `week6_acceptance.ps1/.sh`, benchmark routing in `src/command_dispatch.rs` | Keep tiered timeout (direct/swarm) and enforce timeout-only retry branch | Logs + check results show no permission retries |
| M04 | Cross-shell path handling inconsistent (`/d/...` passed directly to PowerShell) | Deterministic path contract across shell boundaries | `week6_diagnostic.sh`, `week6_pipeline.sh` | Convert bash paths to Windows paths before PowerShell file IO (`cygpath -w`) | Diagnostic/full canonical JSON exists and is loadable |
| M05 | Script parsing/runtime depends on unstable local shim (`python3` on WindowsApps) | No hidden runtime dependency for gate scripts | `week6_diagnostic.sh`, `week6_pipeline.sh` | Remove Python hard dependency from gate-critical path; use native shell/PowerShell JSON parsing | Gate scripts run with predictable host deps |
| M06 | Admission expectations still partly binary in old assertions | Admission must be tri-state | Related E2E assertions + gate reports | Normalize assertions/report handling to `Allow/RequiresApproval/Blocked` semantics | Admission contract checks pass under L1 |
| M07 | Artifact success can be declared without full proof chain in edge paths | Artifact completion requires `write_proof+hash+evidence_ref` | Artifact checks in scripts + E2E | Enforce proof triad in acceptance and reject text-only or missing-proof success | Artifact gate tests + proof reports pass |
| M08 | NoBypass can be verified mainly at runtime in some flows | NoBypass requires static+compile+runtime | Static scan test + compile gate + runtime suites | Keep static scan in hard gate set and block progression on failure | L1 no-bypass checks all pass |
| M09 | Week6 execution diagnostics incomplete under fail-fast only | Layered acceptance required: diagnostic then full then release gate | `week6_diagnostic.*`, `week6_acceptance.*`, `week6_pipeline.*` | Keep non-fail-fast diagnostic, then conditional full, then single release decision | Canonical reports: diagnostic/full/release_gate |
| M10 | Release decision may consume mixed or implicit sources | Single release decision from explicit source set | `release_gate_report.ps1/.sh`, pipeline | Enforce explicit input merge (week6 + d13 + d14 + version-a + d46) | `release_gate.json` with `allow_release` only decision source |

## Migration Completion Criteria (A-phase DoD)

1. `production_contract.md` remains unchanged in hard standards (only governance/scope metadata updates allowed).
2. All production acceptance scripts run with explicit production profile semantics.
3. Any production readiness/storage mirror mismatch is hard-fail, not skip.
4. Retry policy is timeout-only for automated retries in gate scripts and benchmark repair loop.
5. Diagnostic/full/release reports are generated with canonical paths and consistent fields.
6. Release decision depends only on `release_gate.json` (`allow_release=true|false`).

## Notes

This matrix is a migration control artifact, not a place to redefine standards.  
If any matrix entry conflicts with `production_contract.md`, contract wins.
