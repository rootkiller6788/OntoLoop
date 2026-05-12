# OntoLoop Sovereign OS (Engineering Runtime)
OntoLoop is an AI sovereign runtime oriented to **Governance + Execution + Learning + Replay**.
The current version has entered a production-ready engineering phase, featuring closed-loop capabilities for governance, execution, trust verification and continuous learning.

## Core Capability Modules
- Governance Pipeline：Policy / Tenant / Approval / Risk / Budget / Rollout
- Execution Pipeline：Capability Admission → Trust Admission Kernel → Runtime Guard → Layered Tool Stack → Execution Fabric
- Trust Pipeline：Evidence Tagger → Flow State Engine → Trust Evidence Ledger (Hash-chain)
- Learning Pipeline：Learning Proposal → Promotion Gate → Private Memory / Org Knowledge
- Observability Pipeline：Telemetry Collector → Policy Signal Aggregator → Query/Replay/Explanation Plane
- TrustKernel (NoumenonCore) Integration：Identity Authentication, Attestation, Supply Chain Validation, Strong Isolation, Replayable Audit

## Architecture Pipeline (Compact Version)
```text
[User/Trigger]
    -> [Structured Transport + Session Bridge]
    -> [Query Turn State Machine + Context Compiler/Compactor]
    -> [Requirement Clarification]
    -> [Policy & Governance Context]
    -> [Knowledge Context + SuperMemory]
    -> [QueryEngine + Orchestrator + Capability Router]
    -> [Capability Admission]
    -> [Trust Admission Kernel]
    -> [Runtime Guard + Permission Mode]
    -> [Layered Tool Execution + Execution Fabric + Hook Runtime]
    -> [Evidence Tagger]
    -> [Flow State Engine]
    -> [Trust Evidence Ledger]
    -> [Verifier & Audit]
    -> [Learning + Promotion]
    -> [Memory / Org Knowledge Update]
    -> [Unified Query / Replay / Explanation]
    -> [Observability + Reports]
    -> [Next Iteration]
```

## Quick Start
### 1) Environment Requirements
- Rust toolchain
- Optional: StateStore CLI
- Optional: Docker / Docker Compose

### 2) Local Execution
```powershell
cargo run --manifest-path .\Cargo.toml -- --message "Build a governed autonomous loop" --swarm
```

### 3) Local Validation
```powershell
cargo check --workspace --manifest-path .\Cargo.toml
cargo test --workspace --manifest-path .\Cargo.toml
```

## Common CLI Commands
```powershell
cargo run --manifest-path .\Cargo.toml -- system health
cargo run --manifest-path .\Cargo.toml -- --session demo system replay-report
cargo run --manifest-path .\Cargo.toml -- trigger list
cargo run --manifest-path .\Cargo.toml -- --session demo focus board
cargo run --manifest-path .\Cargo.toml -- --session demo org context
cargo run --manifest-path .\Cargo.toml -- bridge status
cargo run --manifest-path .\Cargo.toml -- knowledge batch-export --anchor-list .\deploy\anchors.txt --type graph
cargo run --manifest-path .\Cargo.toml -- trigger webhook --anchor-id cli:focus --topic order.created --payload "{\"order_id\": \"A-1001\"}" --run-now
cargo run --manifest-path .\Cargo.toml -- --session cli:focus system export
cargo run --manifest-path .\Cargo.toml -- --session cli:focus frontend status
cargo run --manifest-path .\Cargo.toml -- --session cli:focus frontend events --format pretty --limit 20
```

The CLI frontend is designed additively. Existing frontend directories such as `dashboard-ui/` are fully reserved for subsequent application interface expansion.

## Acceptance Scripts
```powershell
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\p95_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\pq9_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\week6_acceptance.ps1
powershell -ExecutionPolicy Bypass -File .\deploy\scripts\trigger_supermemory_acceptance.ps1
```

Linux Environment:
```bash
bash ./deploy/scripts/week6_acceptance.sh
```

Output Artifacts:
- `deploy/runtime/p95-acceptance.log`
- `deploy/runtime/p95-acceptance.json`

## Directory Structure
- `src/`: Core runtime & governance logic
- `src/query_engine/`: Turn state / Continuation / Context Compactor / Loop Scheduler
- `src/runtime/`: Runtime Guard / Admission Control / Execution / Evidence Collection / Flow Engine
- `src/security/`: Policy Control / Permission Mode / Capability Admission System
- `src/session/`: Checkpoint / Resume / Runtime Session Management
- `src/memory/`: Private Memory + SuperMemory Pipeline
- `src/observability/`: Telemetry Collector / Query Plane / Replay & Trace Explanation
- `src/transport/`: Structured Transport / Cross-environment Session Bridge
- `src/plugins/`: Plugin Lifecycle Management
- `src/skills/`: Skill Registry / Capability Build Pipeline
- `src/services/`: Service Mediation & Orchestration Spine
- `tests/`: E2E Testing & Regression Test Suite
- `deploy/scripts/`: Automated Acceptance & Operational Scripts
- `docs/`: Protocol Specifications / Architecture Design / Acceptance Standards

## Current Status
The current release is in a high-maturity engineering stage: **core pipeline runnable, data traceable, audit log explainable**.
It is recommended to complete full regression verification via `deploy/scripts/` first, then gradually scale production workloads.

CLI Specification Document:
- docs/CLI_SPECIFICATION.md

## Signal Whitebox Commands (D9/D10/D11)
The internal signal pipeline is exposed via white-box CLI commands for debugging and inspection:
```powershell
cargo run --manifest-path .\Cargo.toml -- system signal status
cargo run --manifest-path .\Cargo.toml -- system signal explain --trace-id <trace-id>
cargo run --manifest-path .\Cargo.toml -- system signal drain
```

Implementation Governance Rules:
- All business-side signal writes must pass through the unified `SignalFacade` layer.
- Direct bypass write operations are blocked by static scan tests.
- Signal pipeline acceptance is integrated into `week6_acceptance` full regression scripts.

## Rule Reference Policy
Third-party reference materials under the `rule/` directory are only used for architecture benchmarking and theoretical abstraction.
These files are excluded from build dependencies and runtime execution.
