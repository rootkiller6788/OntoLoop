use std::collections::BTreeMap;

use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::contracts::flow::FlowNodeState;
use crate::runtime::flow_state_engine::{FlowRuntimeUpdate, FlowStateEngine};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionFabricTrace {
    pub session_id: String,
    pub task_id: String,
    pub trace_id: String,
    pub pool: String,
    pub route_variant: String,
    pub tool_name: Option<String>,
    pub mcp_server: Option<String>,
    pub admission_status: String,
    pub admission_reason: Option<String>,
    pub admission_evidence_ref: Option<String>,
    pub guard_decision: String,
    pub guard_reason: String,
    pub outcome_score: i32,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionFabricStep {
    pub sequence: u32,
    pub stage: String,
    pub state: FlowNodeState,
    pub reason: String,
    pub side_effect_state: String,
    pub budget_state: String,
    pub trigger_state: String,
    pub metadata: BTreeMap<String, String>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionFabricReplaySequence {
    pub session_id: String,
    pub task_id: String,
    pub trace_id: String,
    pub capability_id: String,
    pub generated_at_ms: u64,
    pub steps: Vec<ExecutionFabricStep>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionFabricRecord {
    pub trace: ExecutionFabricTrace,
    pub guard_evidence_ref: String,
    #[serde(default)]
    pub flow_sequence_ref: Option<String>,
}

fn normalized_guard_decision(raw: &str) -> &'static str {
    let decision = raw.to_ascii_lowercase();
    if decision.contains("allow") || decision.contains("pass") {
        "allow"
    } else if decision.contains("approval") || decision.contains("require") {
        "requires_approval"
    } else {
        "blocked"
    }
}

fn admission_rejected(admission_status: &str) -> bool {
    let status = admission_status.to_ascii_lowercase();
    status.contains("reject") || status.contains("deny") || status.contains("block")
}

fn capability_id_for_trace(trace: &ExecutionFabricTrace) -> String {
    if let Some(tool) = trace.tool_name.as_ref() {
        return tool.clone();
    }
    if let Some(server) = trace.mcp_server.as_ref() {
        return format!("mcp::{server}::invoke");
    }
    "provider:chat".into()
}

pub fn build_replayable_flow_sequence(
    trace: &ExecutionFabricTrace,
) -> ExecutionFabricReplaySequence {
    let mut steps = Vec::new();
    let base_ms = trace.created_at_ms;
    let capability_id = capability_id_for_trace(trace);
    let guard = normalized_guard_decision(&trace.guard_decision);
    let admission_is_rejected = admission_rejected(&trace.admission_status);

    let mut base_meta = BTreeMap::new();
    base_meta.insert("pool".into(), trace.pool.clone());
    base_meta.insert("route_variant".into(), trace.route_variant.clone());
    base_meta.insert("guard_decision".into(), trace.guard_decision.clone());
    base_meta.insert("admission_status".into(), trace.admission_status.clone());

    steps.push(ExecutionFabricStep {
        sequence: 0,
        stage: "dispatch".into(),
        state: FlowNodeState::Pending,
        reason: "execution_fabric_dispatch".into(),
        side_effect_state: "none".into(),
        budget_state: "precharge_pending".into(),
        trigger_state: "fabric.dispatch".into(),
        metadata: base_meta.clone(),
        at_ms: base_ms,
    });

    let admission_state = if admission_is_rejected {
        FlowNodeState::Blocked
    } else {
        FlowNodeState::Ready
    };
    let mut admission_meta = base_meta.clone();
    if let Some(reason) = trace.admission_reason.as_ref() {
        admission_meta.insert("admission_reason".into(), reason.clone());
    }
    if let Some(reference) = trace.admission_evidence_ref.as_ref() {
        admission_meta.insert("admission_evidence_ref".into(), reference.clone());
    }
    steps.push(ExecutionFabricStep {
        sequence: 1,
        stage: "admission".into(),
        state: admission_state,
        reason: trace
            .admission_reason
            .clone()
            .unwrap_or_else(|| format!("admission:{}", trace.admission_status)),
        side_effect_state: if admission_is_rejected {
            "blocked".into()
        } else {
            "none".into()
        },
        budget_state: if admission_is_rejected {
            "blocked".into()
        } else {
            "precharge_pending".into()
        },
        trigger_state: "fabric.admission".into(),
        metadata: admission_meta,
        at_ms: base_ms.saturating_add(1),
    });

    let guard_state = if !admission_is_rejected && guard == "allow" {
        FlowNodeState::Running
    } else {
        FlowNodeState::Blocked
    };
    let mut guard_meta = base_meta.clone();
    guard_meta.insert("guard_reason".into(), trace.guard_reason.clone());
    steps.push(ExecutionFabricStep {
        sequence: 2,
        stage: "guard".into(),
        state: guard_state,
        reason: trace.guard_reason.clone(),
        side_effect_state: if guard == "allow" {
            "pending".into()
        } else {
            "blocked".into()
        },
        budget_state: if guard == "allow" {
            "reserved".into()
        } else {
            "rollback_refund".into()
        },
        trigger_state: "fabric.guard".into(),
        metadata: guard_meta,
        at_ms: base_ms.saturating_add(2),
    });

    let (terminal_state, terminal_side_effect, terminal_budget, terminal_reason) =
        if admission_is_rejected {
            (
                FlowNodeState::Blocked,
                "blocked".to_string(),
                "blocked".to_string(),
                "admission_rejected".to_string(),
            )
        } else if guard != "allow" {
            (
                FlowNodeState::Blocked,
                "blocked".to_string(),
                "rollback_refund".to_string(),
                format!("guard_{guard}"),
            )
        } else if trace.outcome_score > 0 {
            (
                FlowNodeState::Succeeded,
                "applied".to_string(),
                "consumed".to_string(),
                "execution_succeeded".to_string(),
            )
        } else {
            (
                FlowNodeState::Failed,
                "failed".to_string(),
                "consumed".to_string(),
                "execution_failed".to_string(),
            )
        };

    let mut final_meta = base_meta;
    final_meta.insert("outcome_score".into(), trace.outcome_score.to_string());
    steps.push(ExecutionFabricStep {
        sequence: 3,
        stage: "execute.complete".into(),
        state: terminal_state,
        reason: terminal_reason,
        side_effect_state: terminal_side_effect,
        budget_state: terminal_budget,
        trigger_state: "fabric.execute.complete".into(),
        metadata: final_meta,
        at_ms: base_ms.saturating_add(3),
    });

    ExecutionFabricReplaySequence {
        session_id: trace.session_id.clone(),
        task_id: trace.task_id.clone(),
        trace_id: trace.trace_id.clone(),
        capability_id,
        generated_at_ms: trace.created_at_ms,
        steps,
    }
}

async fn persist_execution_fabric_flow_sequence(
    db: &StateStore,
    sequence: &ExecutionFabricReplaySequence,
) -> Result<String> {
    let sequence_key = format!(
        "execution-fabric-sequence:{}:{}:{}",
        sequence.session_id, sequence.task_id, sequence.generated_at_ms
    );
    db.upsert_json_knowledge(sequence_key.clone(), sequence, "execution-fabric")
        .await?;
    db.upsert_knowledge(
        format!(
            "execution-fabric-sequence:{}:{}:latest",
            sequence.session_id, sequence.task_id
        ),
        sequence_key.clone(),
        "execution-fabric".into(),
    )
    .await?;

    let flow_engine = FlowStateEngine::new(db.clone());
    for step in &sequence.steps {
        let mut metadata = step.metadata.clone();
        metadata.insert("fabric_stage".into(), step.stage.clone());
        metadata.insert("fabric_sequence".into(), step.sequence.to_string());
        metadata.insert("fabric_trace_id".into(), sequence.trace_id.clone());
        metadata.insert("fabric_sequence_key".into(), sequence_key.clone());
        metadata.insert("fabric_step_at_ms".into(), step.at_ms.to_string());
        flow_engine
            .apply_runtime_update(FlowRuntimeUpdate {
                session_id: sequence.session_id.clone(),
                trace_id: format!("{}:fabric:{}", sequence.trace_id, step.sequence),
                task_id: sequence.task_id.clone(),
                capability_id: sequence.capability_id.clone(),
                state: Some(step.state.clone()),
                reason: step.reason.clone(),
                side_effect_state: Some(step.side_effect_state.clone()),
                budget_state: Some(step.budget_state.clone()),
                trigger_state: Some(step.trigger_state.clone()),
                metadata,
            })
            .await?;
    }
    Ok(sequence_key)
}

pub async fn persist_execution_fabric_trace(
    db: &StateStore,
    trace: &ExecutionFabricTrace,
) -> Result<String> {
    let guard_ref = format!(
        "guard-evidence:{}:{}:{}",
        trace.session_id, trace.task_id, trace.created_at_ms
    );
    db.upsert_json_knowledge(
        guard_ref.clone(),
        &serde_json::json!({
            "session_id": trace.session_id,
            "task_id": trace.task_id,
            "trace_id": trace.trace_id,
            "guard_decision": trace.guard_decision,
            "guard_reason": trace.guard_reason,
            "created_at_ms": trace.created_at_ms,
        }),
        "execution-fabric",
    )
    .await?;

    let key = format!(
        "execution-fabric:{}:{}:{}",
        trace.session_id, trace.task_id, trace.created_at_ms
    );
    let flow_sequence = build_replayable_flow_sequence(trace);
    let flow_sequence_ref = persist_execution_fabric_flow_sequence(db, &flow_sequence).await?;
    let record = ExecutionFabricRecord {
        trace: trace.clone(),
        guard_evidence_ref: guard_ref,
        flow_sequence_ref: Some(flow_sequence_ref),
    };
    db.upsert_json_knowledge(key.clone(), &record, "execution-fabric")
        .await?;
    Ok(key)
}

#[derive(Clone)]
pub struct ExecutionFabricPortAdapter {
    db: StateStore,
}

impl ExecutionFabricPortAdapter {
    pub fn new(db: StateStore) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::ExecutionFabricPort for ExecutionFabricPortAdapter {
    async fn persist_execution_fabric(
        &self,
        receipt: &crate::contracts::types::RunReceipt,
        verdict: &crate::contracts::types::VerificationVerdict,
    ) -> Result<(), crate::contracts::errors::ContractError> {
        let trace = ExecutionFabricTrace {
            session_id: receipt.session_id.to_string(),
            task_id: receipt.task_id.to_string(),
            trace_id: receipt.trace_id.to_string(),
            pool: "port-adapter".into(),
            route_variant: "contract".into(),
            tool_name: None,
            mcp_server: None,
            admission_status: "unknown".into(),
            admission_reason: None,
            admission_evidence_ref: None,
            guard_decision: if receipt.success {
                "allow".into()
            } else {
                "blocked".into()
            },
            guard_reason: verdict
                .reasons
                .first()
                .cloned()
                .unwrap_or_else(|| "no-verifier-reason".into()),
            outcome_score: (verdict.score * 100.0) as i32,
            created_at_ms: crate::orchestration::current_time_ms(),
        };
        persist_execution_fabric_trace(&self.db, &trace)
            .await
            .map_err(|e| crate::contracts::errors::ContractError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn persist_execution_fabric_creates_guard_and_fabric_records() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let trace = ExecutionFabricTrace {
            session_id: "session-fabric".into(),
            task_id: "task-1".into(),
            trace_id: "trace-1".into(),
            pool: "stable".into(),
            route_variant: "control".into(),
            tool_name: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            admission_status: "admitted".into(),
            admission_reason: None,
            admission_evidence_ref: Some("admission:ref:1".into()),
            guard_decision: "allow".into(),
            guard_reason: "passed".into(),
            outcome_score: 92,
            created_at_ms: 42,
        };

        let key = persist_execution_fabric_trace(&db, &trace)
            .await
            .expect("persist");
        let record = db.get_knowledge(&key).await.expect("get").expect("some");
        assert!(
            record
                .key
                .starts_with("execution-fabric:session-fabric:task-1:")
        );

        let guard = db
            .get_knowledge("guard-evidence:session-fabric:task-1:42")
            .await
            .expect("guard")
            .expect("guard exists");
        assert!(guard.value.contains("guard_decision"));

        let sequence = db
            .get_knowledge("execution-fabric-sequence:session-fabric:task-1:42")
            .await
            .expect("sequence")
            .expect("sequence exists");
        assert!(sequence.value.contains("\"stage\":\"execute.complete\""));

        let flow_state = db
            .get_knowledge("flow:node:session-fabric:task-1:state")
            .await
            .expect("flow state")
            .expect("flow state exists");
        assert!(flow_state.value.contains("succeeded"));
    }

    #[tokio::test]
    async fn persist_execution_fabric_blocked_path_writes_blocked_terminal_state() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let trace = ExecutionFabricTrace {
            session_id: "session-fabric-blocked".into(),
            task_id: "task-b".into(),
            trace_id: format!("trace-b-{}", crate::orchestration::current_time_ms()),
            pool: "adaptive".into(),
            route_variant: "treatment".into(),
            tool_name: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            admission_status: "admitted".into(),
            admission_reason: None,
            admission_evidence_ref: None,
            guard_decision: "blocked".into(),
            guard_reason: "breaker-open".into(),
            outcome_score: -7,
            created_at_ms: 777,
        };
        persist_execution_fabric_trace(&db, &trace)
            .await
            .expect("persist blocked");

        let flow_state = db
            .get_knowledge("flow:node:session-fabric-blocked:task-b:state")
            .await
            .expect("flow state")
            .expect("flow state exists");
        assert!(flow_state.value.contains("blocked"));
    }
}

