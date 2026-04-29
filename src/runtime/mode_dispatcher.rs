use async_trait::async_trait;

use crate::{
    config::RuntimeGateMode,
    contracts::{
        errors::ContractError,
        flow::{RuntimeMode, RuntimeModeDecision},
        ports::RuntimeModeDispatcherPort,
        types::TaskEnvelope,
    },
};

#[derive(Debug, Clone)]
pub struct RuntimeModeDispatcher {
    gate_mode: RuntimeGateMode,
    gate_enforce_ratio: f32,
}

impl RuntimeModeDispatcher {
    pub fn new(gate_mode: RuntimeGateMode, gate_enforce_ratio: f32) -> Self {
        Self {
            gate_mode,
            gate_enforce_ratio,
        }
    }

    pub fn should_enforce_gate(&self, session_id: &str, task_id: &str) -> bool {
        match self.gate_mode {
            RuntimeGateMode::Shadow => false,
            RuntimeGateMode::Full => true,
            RuntimeGateMode::Canary => {
                let key = format!("{session_id}:{task_id}");
                let mut hash = 1469598103934665603u64;
                for byte in key.as_bytes() {
                    hash ^= *byte as u64;
                    hash = hash.wrapping_mul(1099511628211);
                }
                let bucket = (hash % 10_000) as f32 / 10_000.0;
                bucket < self.gate_enforce_ratio
            }
        }
    }

    fn dispatch_mode_inner(
        &self,
        envelope: &TaskEnvelope,
        has_degrade_profile: bool,
        is_replay: bool,
    ) -> RuntimeModeDecision {
        let dispatched_at_ms = now_ms();
        if is_replay {
            return RuntimeModeDecision {
                mode: RuntimeMode::Replay,
                enforce_gate: true,
                stage: "runtime.replay".into(),
                reason: "replay execution forces deterministic guard path".into(),
                dispatched_at_ms,
            };
        }

        if has_degrade_profile {
            return RuntimeModeDecision {
                mode: RuntimeMode::Degraded,
                enforce_gate: true,
                stage: "runtime.degrade".into(),
                reason: "active degrade profile detected for session".into(),
                dispatched_at_ms,
            };
        }

        let enforce_gate =
            self.should_enforce_gate(envelope.session_id.as_ref(), envelope.task_id.as_ref());
        if enforce_gate {
            let mode = match self.gate_mode {
                RuntimeGateMode::Full => RuntimeMode::Safe,
                RuntimeGateMode::Canary => RuntimeMode::Normal,
                RuntimeGateMode::Shadow => RuntimeMode::Normal,
            };
            RuntimeModeDecision {
                mode,
                enforce_gate,
                stage: "runtime.guard".into(),
                reason: "guard gate enforced for this execution".into(),
                dispatched_at_ms,
            }
        } else {
            let mode = match self.gate_mode {
                RuntimeGateMode::Canary => RuntimeMode::Mirror,
                RuntimeGateMode::Shadow => RuntimeMode::Shadow,
                RuntimeGateMode::Full => RuntimeMode::Shadow,
            };
            RuntimeModeDecision {
                mode,
                enforce_gate,
                stage: "runtime.shadow".into(),
                reason: "shadow/observe path enabled by rollout gate".into(),
                dispatched_at_ms,
            }
        }
    }
}

#[async_trait]
impl RuntimeModeDispatcherPort for RuntimeModeDispatcher {
    async fn dispatch_mode(
        &self,
        envelope: &TaskEnvelope,
        has_degrade_profile: bool,
        is_replay: bool,
    ) -> Result<RuntimeModeDecision, ContractError> {
        if !(0.0..=1.0).contains(&self.gate_enforce_ratio) {
            return Err(ContractError::Internal(
                "runtime mode dispatcher gate_enforce_ratio must be within [0.0, 1.0]".into(),
            ));
        }
        Ok(self.dispatch_mode_inner(envelope, has_degrade_profile, is_replay))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    };

    fn envelope(session_id: &str, task_id: &str) -> TaskEnvelope {
        TaskEnvelope {
            session_id: SessionId::from(session_id),
            trace_id: TraceId::from("trace-test"),
            task_id: TaskId::from(task_id),
            capability_id: CapabilityId::from("read_file"),
            identity: ExecutionIdentity {
                tenant_id: "tenant-test".into(),
                principal_id: "principal-test".into(),
                policy_id: "policy-test".into(),
                lease_token: "lease-test".into(),
            },
            payload: json!({"path":"README.md"}),
            constraints: ConstraintSet {
                max_cpu_percent: 80,
                max_memory_mb: 512,
                timeout_ms: 30_000,
                max_retries: 1,
                max_tokens: 256,
                io_allow_paths: vec![".".into()],
                io_deny_paths: vec![],
                sandbox_profile: "default".into(),
                requires_human_approval: false,
            },
            trust_plan: None,
        }
    }

    #[tokio::test]
    async fn dispatcher_marks_replay_mode() {
        let dispatcher = RuntimeModeDispatcher::new(RuntimeGateMode::Shadow, 0.2);
        let decision = dispatcher
            .dispatch_mode(&envelope("session-a", "task-a"), false, true)
            .await
            .expect("decision");
        assert!(decision.enforce_gate);
        assert!(matches!(decision.mode, RuntimeMode::Replay));
    }

    #[tokio::test]
    async fn dispatcher_marks_degraded_mode() {
        let dispatcher = RuntimeModeDispatcher::new(RuntimeGateMode::Canary, 0.2);
        let decision = dispatcher
            .dispatch_mode(&envelope("session-b", "task-b"), true, false)
            .await
            .expect("decision");
        assert!(decision.enforce_gate);
        assert!(matches!(decision.mode, RuntimeMode::Degraded));
    }
}
