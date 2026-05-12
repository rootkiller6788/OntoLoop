# Production Contract (Hard Gates)

Version: v1  
Status: Frozen (non-negotiable)  
Effective Date: 2026-05-01

## Scope

This contract defines release-blocking requirements for production write paths, admission, artifact delivery, no-bypass enforcement, and rollback behavior.

These requirements are hard gates. They cannot be downgraded for test convenience, script convenience, or temporary rollout pressure.

This file is the single source of truth for production hard standards.

## Sovereignty Precedence (Frozen)

All critical production-entry checks must preserve this stage order:

1. `Constraint Shield`
2. `ReviewGate`
3. `WAL`
4. `Harness`

Canonical chain token order:
`constraint_shield -> review_gate -> wal -> harness`

Sub-chains are allowed per entry surface, but they must keep the same relative order.

## Hard Standards (Cannot Be Relaxed)

1. WalTx atomic write contract:
   `state + event_log + evidence_ref + relation + write_proof`

2. Admission decision contract:
   `Allow | RequiresApproval | Blocked`

3. Artifact proof contract:
   `write_proof + hash + evidence_ref`

4. NoBypass enforcement contract:
   `static scan + compile-time gate + runtime gate`

5. Rollback safety contract:
   `canary failure => automatic rollback + evidence`

## Acceptance Checks

1. WalTx:
   Any production write must commit all five parts in one transaction or fully rollback.

2. Admission:
   Every decision must be one of the three states and recorded with reason/evidence linkage.

3. Artifact:
   Any artifact-completion claim is invalid without file-backed write proof, hash, and evidence ref.

4. NoBypass:
   Direct provider/tool/memory/mcp bypass must fail in static checks or compile gate before runtime.

5. Rollback:
   Canary failure must trigger rollback automatically and produce replayable audit evidence.

## Change Control

Any change to this contract requires explicit governance approval and a version bump.
No local or CI script may override these gates by default.

## Current Repair Scope Lock (A-phase)

To prevent uncontrolled drift during migration, the current repair batch is locked to:

1. `deploy/scripts/*`
2. `src/command_dispatch.rs`
3. Related E2E assertions and config entry points only

Out-of-scope for this batch:

1. Architectural refactors
2. New subsystem design
3. Non-essential module cleanup unrelated to hard-gate migration

## Acceptance Layer Binding

Execution flow is defined in `docs/ACCEPTANCE_LAYERS_V1.md`:
- L0 Preflight
- L1 Contract
- L2 Domain
- L3 Full Chain
- L4 Release Gate

Layering improves diagnosis speed, but does not relax these hard gates.
