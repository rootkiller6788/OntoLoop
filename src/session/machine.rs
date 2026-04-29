use std::sync::Arc;

use super::{
    audit::{AuditSink, TransitionRecord},
    signal::WorkflowSignal,
    state::WorkflowState,
    transition::{TransitionError, next_state},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StateTransition {
    pub session_id: String,
    pub from: WorkflowState,
    pub signal: WorkflowSignal,
    pub to: WorkflowState,
    pub timestamp_ms: u64,
    pub reason: Option<String>,
}

pub struct WorkflowMachine {
    session_id: String,
    state: WorkflowState,
    audit_sink: Arc<dyn AuditSink>,
}

impl WorkflowMachine {
    pub fn new(session_id: impl Into<String>, audit_sink: Arc<dyn AuditSink>) -> Self {
        Self {
            session_id: session_id.into(),
            state: WorkflowState::Intake,
            audit_sink,
        }
    }

    pub fn state(&self) -> WorkflowState {
        self.state
    }

    pub async fn apply(
        &mut self,
        signal: WorkflowSignal,
        reason: Option<String>,
    ) -> Result<StateTransition, TransitionError> {
        // Verification is modeled as a real state hop: Executing -> Verifying -> (Closed|Planned).
        if self.state == WorkflowState::Executing
            && matches!(
                signal,
                WorkflowSignal::VerifyPassed | WorkflowSignal::VerifyRejected
            )
        {
            let verify_hop = self
                .apply_single(WorkflowState::Verifying, signal, None)
                .await?;
            let target = next_state(WorkflowState::Verifying, signal)?;
            let final_step = self.apply_single(target, signal, reason).await?;
            return Ok(StateTransition {
                session_id: final_step.session_id,
                from: verify_hop.from,
                signal: final_step.signal,
                to: final_step.to,
                timestamp_ms: final_step.timestamp_ms,
                reason: final_step.reason,
            });
        }

        let to = next_state(self.state, signal)?;
        self.apply_single(to, signal, reason).await
    }

    async fn apply_single(
        &mut self,
        to: WorkflowState,
        signal: WorkflowSignal,
        reason: Option<String>,
    ) -> Result<StateTransition, TransitionError> {
        let from = self.state;
        let timestamp_ms = current_time_ms();
        self.state = to;
        let transition = StateTransition {
            session_id: self.session_id.clone(),
            from,
            signal,
            to,
            timestamp_ms,
            reason: reason.clone(),
        };
        let record = TransitionRecord {
            session_id: self.session_id.clone(),
            from,
            signal,
            to,
            timestamp_ms,
            reason,
        };
        self.audit_sink
            .record_transition(record)
            .await
            .map_err(|_| TransitionError { from, signal })?;
        Ok(transition)
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
