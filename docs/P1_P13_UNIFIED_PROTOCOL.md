# P1-P13 Unified Specification Protocol (v1)

This protocol standardizes how AutoLoop phases `P1` through `P13` are described, reviewed, implemented, and reported by AI agents.

## 1. Scope

- Applies to phase plans and status outputs for `P1..P13`.
- Applies to both design docs and execution reports.
- Uses neutral software language only.

## 2. Required Output Structure (Contract)

Every AI output for a phase MUST follow this structure:

1. `phase_meta`
2. `objective`
3. `invariants`
4. `module_scope`
5. `data_models`
6. `flow`
7. `guardrails`
8. `tests`
9. `acceptance`
10. `rollback`
11. `dependency_graph`
12. `status`

## 3. Canonical JSON Shape

```json
{
  "phase_meta": {
    "phase_id": "P7",
    "title": "Identity and Tenant Sovereignty",
    "priority": "highest",
    "version": "v1",
    "owner": "autoloop-core"
  },
  "objective": {
    "problem": "what gap is closed",
    "outcome": "observable engineering outcome"
  },
  "invariants": [
    "hard rule 1",
    "hard rule 2"
  ],
  "module_scope": {
    "write_modules": ["src/security", "src/runtime"],
    "read_modules": ["src/session", "src/contracts"],
    "external_surfaces": ["CLI", "dashboard", "state_store adapter"]
  },
  "data_models": [
    {
      "name": "SessionLease",
      "kind": "table|dto|event",
      "keys": ["session_id", "tenant_id"],
      "append_only": true
    }
  ],
  "flow": {
    "trigger": "entry trigger",
    "steps": [
      "step 1",
      "step 2",
      "step 3"
    ],
    "exit_conditions": ["condition A", "condition B"]
  },
  "guardrails": {
    "runtime_gate": ["budget", "timeout", "sandbox", "token"],
    "policy_gate": ["approval", "permission", "tenant boundary"],
    "audit_trace": ["trace_id", "session_id", "task_id", "capability_id", "version"]
  },
  "tests": {
    "unit": ["unit case 1"],
    "integration": ["integration case 1"],
    "e2e": ["e2e case 1"]
  },
  "acceptance": [
    "measurable criterion 1",
    "measurable criterion 2"
  ],
  "rollback": {
    "trigger": "when to rollback",
    "action": "how to rollback",
    "target_version": "v1"
  },
  "dependency_graph": {
    "depends_on": ["P6"],
    "unblocks": ["P8", "P9"]
  },
  "status": {
    "stage": "planned|in_progress|done|blocked",
    "completion": 0.0,
    "risks": ["risk 1"],
    "next_actions": ["next action 1"]
  }
}
```

## 4. AI Prompt Contract

Use this prompt for any model:

```text
You are producing an AutoLoop phase report under P1-P13 Unified Specification Protocol v1.
Output valid JSON only.
Follow the required keys exactly:
phase_meta, objective, invariants, module_scope, data_models, flow, guardrails, tests, acceptance, rollback, dependency_graph, status.
Use measurable acceptance criteria.
Use explicit module paths.
Use append-only traceability language where applicable.
Do not add political metaphors or non-technical labels.
```

## 5. Phase Registry (P1-P13)

| Phase | Core Target | Primary Modules | Core Acceptance |
|---|---|---|---|
| P1 | State machine lock | `src/session`, `src/orchestration` | legal transitions only + reject/revise loops |
| P2 | Runtime guard mandatory | `src/runtime`, `src/tools`, `src/providers` | all execution through unified gate |
| P3 | Traceable append-only events | `src/observability`, `adapter`, `state_store` | replayable chain with full IDs |
| P4 | 8-layer implementation alignment | `src/lib.rs` + layer modules | each layer has explicit interface mapping |
| P5 | Integration and escape tests | `tests/` | no runtime bypass + stable matrix |
| P6 | Gray rollout | `deploy/config`, `cli system` | shadow/canary/full/rollback operable |
| P7 | Identity and tenant sovereignty | `src/security`, `src/session`, `src/runtime` | cross-tenant access denied |
| P8 | Cost and budget ledger | `src/runtime`, `src/observability`, `adapter` | task-level cost decomposition + reconciliation |
| P9 | Capability supply chain trust | `src/tools`, `src/providers`, `src/security` | only verified+trusted+active execution |
| P10 | Deterministic replay boundary | `src/observability/event_stream`, `src/runtime` | snapshot replay + explainable drift |
| P11 | Recovery/degrade/chaos | `src/runtime`, `src/orchestration`, `deploy/` | MTTR + degrade success targets met |
| P12 | Governed learning loop | `src/memory`, `src/rag`, `src/hooks`, `src/security` | learning must pass gate before promotion |
| P13 | Business loop mapping | `src/observability`, `dashboard-ui`, `src/lib.rs` | revenue-cost-profit-risk traceable to task |

## 6. Layer Internal Core Flows

## 6.1 Operator Control Plane

1. Receive approval/rejection command.
2. Bind `session_id` + reason + actor context.
3. Emit policy decision event.
4. Trigger retry/replan or continue.

## 6.2 Policy and Rule Engine

1. Load policy bindings and constraints.
2. Evaluate intent, risk class, budget class.
3. Produce `approved/rejected/revise`.
4. Persist decision artifact.

## 6.3 Orchestration and Scheduler

1. Convert approved intent into execution plan.
2. Build dependency graph and task envelopes.
3. Route to stable/adaptive pools.
4. Push execution queue with trace IDs.

## 6.4 Execution Pools

1. Pick executable capability from catalog.
2. Hand off to runtime kernel only.
3. Record outcome and raw feedback.
4. Return structured run receipt.

## 6.5 Runtime Kernel

1. Enforce identity/tenant and lease validity.
2. Enforce CPU/memory/timeout/token/I/O budgets.
3. Enforce circuit breaker state machine.
4. Execute, settle ledger, emit runtime events.

## 6.6 Verifier and Audit Pipeline

1. Validate output quality and policy compliance.
2. Mark pass/iterate/reject and attach evidence.
3. Emit audit events and blocker records.
4. Route reject paths back to planning loop.

## 6.7 Learning and GraphRAG Engine

1. Ingest verified evidence and witness logs.
2. Build/refine entity-relation memory graph.
3. Update skill/causal/session strategy memory.
4. Publish learning deltas for next routing cycle.

## 6.8 Reporting and Observability

1. Aggregate session/task/capability metrics.
2. Emit dashboard snapshots and replay streams.
3. Publish cost-risk-margin-SLA reports.
4. Support operator forensics and replay.

## 7. Validation Rules

- Any missing required key => invalid protocol output.
- Any metric without trace IDs => invalid.
- Any execution path bypassing runtime gate => invalid.
- Any cross-tenant access success => invalid.

## 8. Recommended Usage

1. Pick a phase (`P1..P13`).
2. Ask model to output JSON using Section 4 prompt.
3. Validate keys against Section 2.
4. Compare acceptance criteria against Section 5 and current tests.
5. Store report as append-only artifact.

## 9. Per-Phase Core Flows (P1-P13)

Each phase has a concise core flow for implementation and review:

- `P1 State Machine Lock`
  - Define states/signals -> enforce legal transitions only -> persist transition audit -> expose reject/revise loop.
- `P2 Runtime Guard`
  - Build task envelope -> enforce identity/budget/sandbox/token gate -> execute through single kernel path -> emit runtime decision.
- `P3 Traceable Event Model`
  - Normalize IDs (`trace/session/task/capability/version`) -> append event stream -> build read views -> verify replay chain integrity.
- `P4 8-Layer Mapping`
  - Map interfaces to layers -> wire layer boundaries in code -> enforce contracts in orchestration path -> validate no cross-layer bypass.
- `P5 Integration and Escape Tests`
  - Build transition matrix tests -> run bypass/escape guards -> run reject/revise e2e -> assert performance and stability baseline.
- `P6 Gray Rollout`
  - Shadow observe -> canary enforce (10%-30%) -> full enforce -> keep rollback switch/version target.
- `P7 Identity and Tenant Sovereignty`
  - Authenticate request -> bind tenant/principal/policy/lease -> runtime validates context every execution -> reject cross-tenant access.
- `P8 Budget and Cost Ledger`
  - Precharge budget -> execute -> settle ledger + attribution -> reconcile account and append cost reports.
- `P9 Capability Supply Chain Trust`
  - Register capability artifact -> verify signature/provenance/SBOM -> approve into active catalog -> runtime re-check before invoke.
- `P10 Deterministic Replay Boundary`
  - Snapshot inputs/routes/digests -> replay under fixed boundary -> detect deviation -> emit explainable replay report.
- `P11 Recovery and Degrade Sovereignty`
  - Detect fault -> switch degrade profile -> recover/circuit reset -> trigger manual takeover point when needed.
- `P12 Governed Learning Loop`
  - Propose learning delta -> verifier gate -> limited trial -> promote or rollback -> commit to long-term memory/graph.
- `P13 Business Loop Mapping`
  - Create work order -> aggregate cost + revenue -> produce margin/SLA/risk outputs -> trace every metric back to task.

## 10. Overall Core Loop

The full system loop is:

`Understand -> Plan -> Execute -> Verify -> Learn -> Evolve -> Repeat`

Mapped to phases:

1. `Understand`
   - P1, P7, P4
   - Intake state + identity binding + layer contract context.
2. `Plan`
   - P1, P4, P6
   - Policy review, orchestration, rollout strategy.
3. `Execute`
   - P2, P8, P9, P11
   - Guarded execution, cost controls, trusted capability path, degrade handling.
4. `Verify`
   - P3, P5, P10
   - Append-only trace, integration checks, deterministic replay and drift analysis.
5. `Learn`
   - P12
   - Evidence-gated learning proposals and promotion pipeline.
6. `Evolve`
   - P6, P9, P12, P13
   - Controlled rollout, capability governance, skill evolution, business optimization.
7. `Repeat`
   - P3 + P1 baseline
   - Persisted event history feeds the next cycle with traceable state and constraints.

## 11. End-to-End Runtime Sequence (Condensed)

1. Request enters with identity and constraints (`P1/P7`).
2. Plan and route are generated under layer contracts (`P4`).
3. Runtime kernel executes with hard gates (`P2/P8/P9`).
4. Verifier/audit evaluates outcomes and replay evidence (`P3/P5/P10`).
5. Recovery/degrade triggers when needed (`P11`).
6. Learning gate decides promotion or rollback (`P12`).
7. Business reports update margin/SLA/risk and close loop (`P13`).
8. Next cycle starts with updated memory, capability posture, and rollout policy (`P6`).

