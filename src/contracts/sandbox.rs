use std::collections::BTreeMap;

use super::{
    capability::CapabilityIntent,
    ids::{CapabilityId, SessionId, TaskId, TraceId},
    types::{ConstraintSet, ExecutionIdentity, TaskEnvelope, TrustExecutionPlan},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeClass {
    Provider,
    ToolNative,
    ToolSandboxed,
    TrustedHighRisk,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionIntent {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub task_id: TaskId,
    pub capability_id: CapabilityId,
    pub objective: String,
    pub payload: serde_json::Value,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxPlan {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub task_id: TaskId,
    pub constraints: ConstraintSet,
    pub runtime_class_hint: RuntimeClass,
    #[serde(default)]
    pub trust_plan: Option<TrustExecutionPlan>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilityRequest {
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub objective: String,
    pub required_tags: Vec<String>,
    pub preferred_servers: Vec<String>,
    #[serde(default)]
    pub capability_id_hint: Option<CapabilityId>,
    #[serde(default)]
    pub identity: Option<ExecutionIdentity>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxContractBundle {
    pub execution_intent: ExecutionIntent,
    pub sandbox_plan: SandboxPlan,
    pub capability_request: CapabilityRequest,
}

impl ExecutionIntent {
    pub fn from_legacy(
        envelope: &TaskEnvelope,
        capability_intent: Option<&CapabilityIntent>,
    ) -> Self {
        let mut metadata = BTreeMap::new();
        metadata.insert("tenant_id".into(), envelope.identity.tenant_id.clone());
        metadata.insert("principal_id".into(), envelope.identity.principal_id.clone());
        metadata.insert("policy_id".into(), envelope.identity.policy_id.clone());
        metadata.insert("legacy_contract".into(), "task_envelope".into());

        let objective = capability_intent
            .map(|intent| intent.objective.clone())
            .unwrap_or_else(|| envelope.capability_id.to_string());

        Self {
            session_id: envelope.session_id.clone(),
            trace_id: envelope.trace_id.clone(),
            task_id: envelope.task_id.clone(),
            capability_id: envelope.capability_id.clone(),
            objective,
            payload: envelope.payload.clone(),
            metadata,
        }
    }
}

impl SandboxPlan {
    pub fn from_legacy(envelope: &TaskEnvelope) -> Self {
        let runtime_class_hint = if envelope.constraints.sandbox_profile.contains("trusted")
            || envelope
                .trust_plan
                .as_ref()
                .map(|plan| plan.attestation_required)
                .unwrap_or(false)
        {
            RuntimeClass::TrustedHighRisk
        } else {
            RuntimeClass::ToolSandboxed
        };

        Self {
            session_id: envelope.session_id.clone(),
            trace_id: envelope.trace_id.clone(),
            task_id: envelope.task_id.clone(),
            constraints: envelope.constraints.clone(),
            runtime_class_hint,
            trust_plan: envelope.trust_plan.clone(),
        }
    }
}

impl CapabilityRequest {
    pub fn from_legacy(
        envelope: &TaskEnvelope,
        capability_intent: Option<&CapabilityIntent>,
    ) -> Self {
        let (objective, required_tags, preferred_servers) = match capability_intent {
            Some(intent) => (
                intent.objective.clone(),
                intent.required_tags.clone(),
                intent.preferred_servers.clone(),
            ),
            None => (envelope.capability_id.to_string(), Vec::new(), Vec::new()),
        };

        Self {
            session_id: envelope.session_id.clone(),
            task_id: envelope.task_id.clone(),
            objective,
            required_tags,
            preferred_servers,
            capability_id_hint: Some(envelope.capability_id.clone()),
            identity: Some(envelope.identity.clone()),
        }
    }
}

impl SandboxContractBundle {
    pub fn from_legacy(
        envelope: &TaskEnvelope,
        capability_intent: Option<&CapabilityIntent>,
    ) -> Self {
        Self {
            execution_intent: ExecutionIntent::from_legacy(envelope, capability_intent),
            sandbox_plan: SandboxPlan::from_legacy(envelope),
            capability_request: CapabilityRequest::from_legacy(envelope, capability_intent),
        }
    }
}

impl From<&TaskEnvelope> for ExecutionIntent {
    fn from(value: &TaskEnvelope) -> Self {
        Self::from_legacy(value, None)
    }
}

impl From<&TaskEnvelope> for SandboxPlan {
    fn from(value: &TaskEnvelope) -> Self {
        Self::from_legacy(value)
    }
}

impl From<&CapabilityIntent> for CapabilityRequest {
    fn from(value: &CapabilityIntent) -> Self {
        Self {
            session_id: value.session_id.clone().into(),
            task_id: "legacy-task".into(),
            objective: value.objective.clone(),
            required_tags: value.required_tags.clone(),
            preferred_servers: value.preferred_servers.clone(),
            capability_id_hint: None,
            identity: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::types::{ConstraintSet, ExecutionIdentity, TaskEnvelope};

    fn envelope() -> TaskEnvelope {
        TaskEnvelope {
            session_id: "session-a".into(),
            trace_id: "trace-a".into(),
            task_id: "task-a".into(),
            capability_id: "mcp::local-mcp::invoke".into(),
            identity: ExecutionIdentity {
                tenant_id: "tenant-a".into(),
                principal_id: "principal-a".into(),
                policy_id: "policy-a".into(),
                lease_token: "lease-a".into(),
            },
            payload: serde_json::json!({"input":"hello"}),
            constraints: ConstraintSet {
                max_cpu_percent: 40,
                max_memory_mb: 256,
                timeout_ms: 30_000,
                max_retries: 1,
                max_tokens: 8_000,
                io_allow_paths: vec!["./workspace".into()],
                io_deny_paths: vec!["./.git".into()],
                sandbox_profile: "runtime-sandbox-v1".into(),
                requires_human_approval: false,
            },
            trust_plan: None,
        }
    }

    #[test]
    fn legacy_mapping_preserves_identity_and_objective() {
        let env = envelope();
        let capability_intent = CapabilityIntent {
            session_id: env.session_id.to_string(),
            objective: "deploy rollout".into(),
            required_tags: vec!["deploy".into()],
            preferred_servers: vec!["local-mcp".into()],
        };

        let bundle = SandboxContractBundle::from_legacy(&env, Some(&capability_intent));
        assert_eq!(bundle.execution_intent.objective, "deploy rollout");
        assert_eq!(
            bundle
                .capability_request
                .identity
                .as_ref()
                .map(|i| i.tenant_id.as_str()),
            Some("tenant-a")
        );
        assert_eq!(
            bundle
                .capability_request
                .capability_id_hint
                .as_ref()
                .map(|id| id.to_string()),
            Some("mcp::local-mcp::invoke".to_string())
        );
    }

    #[test]
    fn high_risk_trust_plan_maps_to_trusted_runtime_class() {
        let mut env = envelope();
        env.trust_plan = Some(TrustExecutionPlan {
            trust_level: "high".into(),
            verify_identity: true,
            verify_environment: true,
            rollout_gate: "canary".into(),
            attestation_backend: "remote".into(),
            attestation_required: true,
            attestation_policy_version: Some("v2".into()),
            policy_refs: vec!["policy-a".into()],
            budget_scope: "high-risk".into(),
        });
        let plan = SandboxPlan::from_legacy(&env);
        assert!(matches!(plan.runtime_class_hint, RuntimeClass::TrustedHighRisk));
    }
}
