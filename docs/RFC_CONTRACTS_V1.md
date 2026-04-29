# RFC: Contracts v1 Freeze

Status: Accepted  
Version: `v1`  
Owner: AutoLoop Core

## Scope

This RFC freezes the P0 interface contract in `src/contracts/` and defines compatibility policy for all future changes.

## Frozen Interfaces

The following traits are locked for `v1`:

1. `OperatorControlPlane`
2. `PolicyRuleEngine`
3. `OrchestratorScheduler`
4. `ExecutionPool`
5. `RuntimeKernel`
6. `VerifierAuditPipeline`
7. `LearningGraphEngine`
8. `ReportingObservability`

## Frozen Core DTO

The following DTO are locked for `v1`:

1. `Intent`
2. `PolicyDecision`
3. `ExecutionPlan`
4. `TaskEnvelope`
5. `RunReceipt`
6. `VerificationVerdict`
7. `LearningDelta`
8. `ReportArtifact`

## Versioning Policy

1. Additive fields in DTO are allowed only when marked optional and backward compatible.
2. Trait method signature changes are forbidden in-place.
3. Breaking changes require a new version namespace (`v2`) and adapter bridge.
4. `CONTRACT_VERSION` in `src/contracts/version.rs` is the canonical runtime contract marker.

## Migration Rule

1. Existing `v1` callers must keep working until explicitly deprecated.
2. Deprecation requires one release cycle overlap.
3. Removal is allowed only after compatibility adapters are available.

## Runtime Gate Rule

Starting in P2, tool/MCP execution paths must go through `RuntimeKernel::execute(TaskEnvelope)` to enforce:

1. Budget constraints (CPU, memory, timeout, token)
2. I/O policy (allow/deny paths)
3. Guard decisions and circuit breaker states
4. Auditable execution evidence

## D1 Freeze Addendum (2026-04-21)

This addendum freezes two additional contracts under `v1`:

1. `ArtifactDeliveryContract` (`artifact-delivery/v1`)
2. `RelationContract` (`relation/v1`)

Canonical version markers are defined in:

- `src/contracts/version.rs`
- `src/contracts/artifact_delivery.rs`
- `src/contracts/relation.rs`

### Compatibility Rules (ArtifactDeliveryContract)

The `v1` parser accepts both canonical and legacy field names.

- `requires_artifact` <= `must_write_artifact`
- `target_path` <= `artifact_path`
- `validation_rules` <= `checks`
- `write_proof` <= `proof`
- `validation_rules.min_size_bytes` <= `min_bytes`
- `validation_rules.max_size_bytes` <= `max_bytes`
- `validation_rules.expected_mime` <= `mime`
- `write_proof.hash` <= `sha256`
- `write_proof.path` <= `artifact_path`

### Compatibility Rules (RelationContract)

The `v1` parser accepts both canonical and legacy field names.

- `nodes` <= `node_list`
- `edges` <= `edge_list`
- `events` <= `event_list`
- `RelationNode.node_id` <= `id`
- `RelationNode.node_type` <= `kind`
- `RelationNode.display_name` <= `name`
- `RelationNode.metadata` <= `attrs`
- `RelationEdge.edge_id` <= `id`
- `RelationEdge.from_node_id` <= `from`
- `RelationEdge.to_node_id` <= `to`
- `RelationEdge.edge_type` <= `kind`
- `RelationEdge.reason` <= `why`
- `RelationEdge.metadata` <= `attrs`
- `RelationEvent.event_id` <= `id`
- `RelationEvent.event_type` <= `kind`
- `RelationEvent.reason` <= `decision_reason`
- `RelationEvent.metadata` <= `attrs`

### Non-breaking Change Policy

For both contracts, `v1.x` only allows:

1. Additive optional fields.
2. New enum values only with tolerant handling in callers.
3. Alias additions for backward compatibility.

Forbidden in `v1.x`:

1. Required field removals.
2. Type changes for existing fields.
3. In-place semantic repurposing of existing field names.
