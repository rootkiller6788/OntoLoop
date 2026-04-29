# NoumenonCore Architecture

NoumenonCore is a mandatory trusted execution enforcement kernel.

It is not a feature center. It is a sovereignty boundary that absorbs four lines of security history into one execution core:

- Nix: reproducible closure identity, immutable store paths, generation rollback model
- seL4/l4v: minimal TCB, capability discipline, proof-oriented state machine invariants
- cosign/Sigstore: signed artifact admission, provenance verification, transparency log evidence
- hardened allocator worldview: runtime hardening for untrusted execution islands and FFI boundaries

## Core Positioning

NoumenonCore owns:

- trusted admission
- mandatory enforcement
- evidence-backed completion
- replayable verification

Any side-effectful trusted execution must pass through the NoumenonCore mandatory state machine.

## Governance Boundary

Governance inside NoumenonCore is limited to mandatory enforcement decisions and rollback gates.

Not inside kernel governance:

- policy suggestion
- route tuning
- trust decay recommendation
- soft advisory behaviors

## Replay Boundary

- `ReplayCheck` is an evidence consistency verification capability.
- Replay is not a kernel runtime mode.
- Normative freeze: `ReplayCheck != execution mode`.
- `replay_only` and `audit_only` are rejected by the mandatory execution path.

## Mandatory State Machine

Process states:

`PendingAdmission -> Admitted -> Enforcing -> Executing -> RecordingEvidence -> Verifying`

Terminal states:

`Completed | Rejected | Failed | RolledBack`

State constraints:

- Side effects are allowed only in `Executing`.
- Ledger writes are allowed only in `RecordingEvidence`.
- Rollback transition is allowed only as `Failed -> RolledBack`.

## Admission Contract (Supply-Chain Aware)

`Admitted` iff all mandatory checks pass:

- identity authenticated
- artifact signature inputs verified
- Rekor inclusion evidence present
- provenance evidence present
- policy bundle pinned
- rollout gate allowed

Trusted artifact set must bind at least:

- executor binary/container digest
- verifier digest
- policy bundle digest
- capability package/wasm digest

## Reproducible Closure Identity (Nix-inspired)

Execution identity is bound to a reproducible closure model, not host ambient state.

Required closure fields:

- `flake_lock_digest`
- `derivation_digest`
- `store_paths`
- `runtime_closure_hash`
- `generation_id`
- `config_digest`

### Execution Fingerprint

`execution_fingerprint` is computed from:

- `flake_lock_digest`
- `derivation_digest`
- `store_paths_digest`
- `runtime_closure_hash`
- `policy_bundle_digest`
- `capability_package_digest`
- `verifier_digest`
- `config_digest`
- `generation_id`
- `output_chain_digest`

Canonical formula:

`execution_fingerprint = H(flake_lock_digest + derivation_digest + store_paths_digest + runtime_closure_hash + policy_bundle_digest + capability_package_digest + verifier_digest + config_digest + generation_id + output_chain_digest)`

This upgrades replay from "compare output only" to "compare execution universe identity + outcome".

## Minimal TCB Boundary

Inside TCB:

- state machine
- capability authorization
- attestation verifier hook
- supply-chain admission verifier hook
- resource and budget reservation gates
- side-effect gate
- evidence append path
- result validator + replay check

Outside or less-trusted:

- user agents and planners
- plugin/wasm tools
- external services
- non-kernel observability/memory/evolution systems

## Kernel Invariants (Proof-Oriented Set)

- I1: Unadmitted request never reaches `Enforcing` or later.
- I2: Without successful `Enforcing`, no side effect is allowed.
- I3: Ledger writes occur only in `RecordingEvidence`.
- I4: `Failed` can only transition to `RolledBack` or terminate.
- I5: `Completed` implies mandatory validation passed.
- I6: `ReplayCheck` is never an execution mode.
- I7: Every execution is bound to explicit capability scope and tenant scope.
- I8: Every execution is bound to signed artifacts and reproducible closure identity.

## Core vs Extension Boundary

Core (kernel sovereignty):

- `ir`
- `state`
- `kernel`
- `trust`
- `governance` (mandatory gates only)
- `ledger`
- `resource`

Extensions (outside core sovereignty):

- `execution`
- `syscall`
- `routing`
- `replay` (except `ReplayCheck` contract)
- `observability`
- `evolution`
- `truth`
- `memory_evolution`
- `storage_vector`
- `tooling`

## Release Gates

Required release gates for core:

- sovereignty gate: non-mandatory modes are rejected
- admission gate: identity, supply-chain inputs, attestation, tenant/resource/budget denials reject before evidence write
- replay boundary gate: `ReplayCheck` works as capability, no replay runtime mode
- rollback gate: failures produce deterministic `Failed -> RolledBack`
- fingerprint gate: replay recomputation must match `execution_fingerprint`
- build matrix gate:
  - `cargo test`
  - `cargo test --no-default-features --features core`
  - `cargo test --features extensions`

