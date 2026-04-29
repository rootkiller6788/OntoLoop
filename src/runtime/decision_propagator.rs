use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};

use crate::{
    observability::query_plane::persist_unified_query_view,
    orchestration::current_time_ms,
    runtime::{
        RuntimeKernel,
        evidence_tagger::EvidenceTagStage,
        flow_state_engine::{FlowRuntimeUpdate, FlowStateEngine},
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossKernelDecisionEnvelope {
    pub source: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub admission_id: Option<String>,
    pub policy_version: String,
    pub mode_decision: String,
    pub approval_id: Option<String>,
    pub rollback_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossKernelPropagationResult {
    pub decision_id: String,
    pub evidence_tag_ref: String,
    pub flow_state_ref: String,
    pub query_plane_generated_at_ms: u64,
}

pub async fn propagate(
    db: &StateStore,
    runtime: &RuntimeKernel,
    envelope: &CrossKernelDecisionEnvelope,
) -> Result<CrossKernelPropagationResult> {
    let now = current_time_ms();
    let decision_id = format!("cross-kernel:{}:{}", envelope.session_id, now);

    db.upsert_json_knowledge(
        decision_id.clone(),
        envelope,
        "cross-kernel-decision-propagator",
    )
    .await?;

    let tag_ref = runtime
        .tag_external_stage(
            db,
            &envelope.session_id,
            &envelope.trace_id,
            Some(&envelope.task_id),
            Some(&envelope.capability_id),
            EvidenceTagStage::Guard,
            "cross-kernel.decision.propagated",
            serde_json::json!({
                "source": envelope.source,
                "admission_id": envelope.admission_id,
                "policy_version": envelope.policy_version,
                "mode_decision": envelope.mode_decision,
                "approval_id": envelope.approval_id,
                "rollback_reason": envelope.rollback_reason,
            }),
        )
        .await?;

    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("policy_version".into(), envelope.policy_version.clone());
    metadata.insert("mode_decision".into(), envelope.mode_decision.clone());
    metadata.insert("source".into(), envelope.source.clone());
    if let Some(admission_id) = envelope.admission_id.clone() {
        metadata.insert("admission_id".into(), admission_id);
    }
    if let Some(approval_id) = envelope.approval_id.clone() {
        metadata.insert("approval_id".into(), approval_id);
    }
    if let Some(reason) = envelope.rollback_reason.clone() {
        metadata.insert("rollback_reason".into(), reason);
    }

    let flow_engine = FlowStateEngine::new(db.clone());
    flow_engine
        .apply_runtime_update(FlowRuntimeUpdate {
            session_id: envelope.session_id.clone(),
            trace_id: envelope.trace_id.clone(),
            task_id: envelope.task_id.clone(),
            capability_id: envelope.capability_id.clone(),
            state: None,
            reason: "cross-kernel-decision-propagated".into(),
            side_effect_state: Some("policy-synced".into()),
            budget_state: Some("observed".into()),
            trigger_state: Some("trigger.cross-kernel.propagated".into()),
            metadata,
        })
        .await?;

    let query =
        persist_unified_query_view(db, &envelope.session_id, Some(&envelope.trace_id)).await?;
    let flow_state_ref = format!("flow:node:{}:{}", envelope.session_id, envelope.task_id);

    Ok(CrossKernelPropagationResult {
        decision_id,
        evidence_tag_ref: tag_ref,
        flow_state_ref,
        query_plane_generated_at_ms: query.generated_at_ms,
    })
}

