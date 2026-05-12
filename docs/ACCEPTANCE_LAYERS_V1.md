# Acceptance Layers v1 (D2)

Status: Frozen  
Version: v1  
Effective Date: 2026-05-01

## Goal

Define a layered acceptance flow that preserves hard standards while improving diagnosis speed.

Rules:
- Acceptance flow is layered diagnosis first, final hard block last.
- Acceptance standards are not downgraded.
- Any production release must pass L4.

## Layer Model

### L0 Preflight

Scope:
- profile alignment
- config doctor
- storage readiness

Checks:
- `profile.alignment`
- `runtime.gate_mode`
- `runtime.rollback_window`
- `storage.backend_consistency`

Output:
- `l0_preflight_report.json`

Block rule:
- Any failed required preflight check blocks L1-L4.

### L1 Contract

Scope:
- WAL contract
- Admission contract
- Evidence contract
- NoBypass contract

Checks:
- WalTx atomic 5-tuple:
  `state + event_log + evidence_ref + relation + write_proof`
- Admission tri-state:
  `Allow | RequiresApproval | Blocked`
- Artifact proof triad:
  `write_proof + hash + evidence_ref`
- NoBypass 3-layer enforcement:
  `static + compile + runtime`

Output:
- `l1_contract_report.json`

Block rule:
- Any contract breach is release-blocking.

### L2 Domain

Scope:
- sandbox
- frontend cli
- storage/postgres dualwrite/replay
- evolution/version-a
- signal pipeline

Output:
- `l2_domain_report.json`

Block rule:
- Any required domain suite failure blocks L3/L4.

### L3 Full Chain

Scope:
- week6 full chain acceptance (fail-fast)

Purpose:
- Validate integrated behavior after L0-L2 are already clean.

Output:
- `week6-acceptance-*.json`

Block rule:
- `all_passed != true` blocks L4.

### L4 Release Gate

Scope:
- single release decision JSON from fused evidence

Inputs:
- L3 week6 report
- benchmark report (d13)
- rollout report (d14)
- version-a report
- SLO report (d46)

Output:
- `release-gate-*.json`

Decision:
- `allow_release=true` only if all required gates pass.

## Canonical Output Contract (L4)

`release-gate-*.json` must include:
- `release_gate_version`
- `allow_release`
- `deny_reasons`
- `decision`
- `metrics.slo`
- `metrics.success_rate_percent`
- `metrics.compliance_rate_percent`
- `rollback_evidence`
- `component_gates`
- `source_reports`

## Mapping to Existing Scripts

- L0-L3 runner:
  `deploy/scripts/week6_acceptance.ps1`
  `deploy/scripts/week6_acceptance.sh`
- L4 runner:
  `deploy/scripts/release_gate_report.ps1`
  `deploy/scripts/release_gate_report.sh`

## Governance Note

This layered structure changes execution order and diagnosis clarity only.  
It does not relax any hard production standard in `docs/production_contract.md`.
