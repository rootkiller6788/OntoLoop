use anyhow::{Result, anyhow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStatus {
    PendingAdmission,
    Admitted,
    Enforcing,
    Executing,
    RecordingEvidence,
    Verifying,
    Rejected,
    Failed,
    RolledBack,
    Completed,
}

pub struct StateMachine {
    state: ExecutionStatus,
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            state: ExecutionStatus::PendingAdmission,
        }
    }

    pub fn state(&self) -> &ExecutionStatus {
        &self.state
    }

    pub fn transit(&mut self, next: ExecutionStatus) -> Result<()> {
        let valid = Self::can_transition(&self.state, &next);

        if !valid {
            return Err(anyhow!(
                "invalid state transition: {:?} -> {:?}",
                self.state,
                next
            ));
        }

        self.state = next;
        Ok(())
    }

    pub fn can_transition(from: &ExecutionStatus, next: &ExecutionStatus) -> bool {
        matches!(
            (from, next),
            (ExecutionStatus::PendingAdmission, ExecutionStatus::Admitted)
                | (ExecutionStatus::PendingAdmission, ExecutionStatus::Rejected)
                | (ExecutionStatus::Admitted, ExecutionStatus::Enforcing)
                | (ExecutionStatus::Enforcing, ExecutionStatus::Executing)
                | (ExecutionStatus::Enforcing, ExecutionStatus::Rejected)
                | (
                    ExecutionStatus::Executing,
                    ExecutionStatus::RecordingEvidence
                )
                | (ExecutionStatus::Executing, ExecutionStatus::Failed)
                | (
                    ExecutionStatus::RecordingEvidence,
                    ExecutionStatus::Verifying
                )
                | (ExecutionStatus::RecordingEvidence, ExecutionStatus::Failed)
                | (ExecutionStatus::Verifying, ExecutionStatus::Completed)
                | (ExecutionStatus::Verifying, ExecutionStatus::Failed)
                | (ExecutionStatus::Failed, ExecutionStatus::RolledBack)
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            ExecutionStatus::Completed
                | ExecutionStatus::Rejected
                | ExecutionStatus::Failed
                | ExecutionStatus::RolledBack
        )
    }

    pub fn allows_side_effect(&self) -> bool {
        matches!(self.state, ExecutionStatus::Executing)
    }

    pub fn allows_ledger_write(&self) -> bool {
        matches!(self.state, ExecutionStatus::RecordingEvidence)
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutionStatus, StateMachine};

    #[test]
    fn accepts_mandatory_happy_path() {
        let mut sm = StateMachine::new();
        sm.transit(ExecutionStatus::Admitted).expect("admit");
        sm.transit(ExecutionStatus::Enforcing).expect("enforce");
        sm.transit(ExecutionStatus::Executing).expect("execute");
        sm.transit(ExecutionStatus::RecordingEvidence)
            .expect("record");
        sm.transit(ExecutionStatus::Verifying).expect("verify");
        sm.transit(ExecutionStatus::Completed).expect("complete");
        assert!(sm.is_terminal());
    }

    #[test]
    fn rejects_invalid_transition() {
        let mut sm = StateMachine::new();
        let err = sm
            .transit(ExecutionStatus::Executing)
            .expect_err("must reject invalid jump");
        assert!(err.to_string().contains("invalid state transition"));
    }

    #[test]
    fn side_effect_allowed_only_in_executing() {
        let mut sm = StateMachine::new();
        assert!(!sm.allows_side_effect());
        sm.transit(ExecutionStatus::Admitted).expect("admit");
        sm.transit(ExecutionStatus::Enforcing).expect("enforce");
        assert!(!sm.allows_side_effect());
        sm.transit(ExecutionStatus::Executing).expect("execute");
        assert!(sm.allows_side_effect());
    }

    #[test]
    fn ledger_write_allowed_only_in_recording_evidence() {
        let mut sm = StateMachine::new();
        assert!(!sm.allows_ledger_write());
        sm.transit(ExecutionStatus::Admitted).expect("admit");
        sm.transit(ExecutionStatus::Enforcing).expect("enforce");
        sm.transit(ExecutionStatus::Executing).expect("execute");
        assert!(!sm.allows_ledger_write());
        sm.transit(ExecutionStatus::RecordingEvidence)
            .expect("record");
        assert!(sm.allows_ledger_write());
    }

    #[test]
    fn failed_transitions_only_to_rollback() {
        let mut sm = StateMachine::new();
        sm.transit(ExecutionStatus::Admitted).expect("admit");
        sm.transit(ExecutionStatus::Enforcing).expect("enforce");
        sm.transit(ExecutionStatus::Executing).expect("execute");
        sm.transit(ExecutionStatus::Failed).expect("fail");

        let invalid = sm
            .transit(ExecutionStatus::Completed)
            .expect_err("failed must not transition to completed");
        assert!(invalid.to_string().contains("invalid state transition"));

        sm.transit(ExecutionStatus::RolledBack)
            .expect("rollback from failed");
        assert!(sm.is_terminal());
    }

    #[test]
    fn can_transition_matrix_matches_contract() {
        assert!(StateMachine::can_transition(
            &ExecutionStatus::PendingAdmission,
            &ExecutionStatus::Admitted
        ));
        assert!(!StateMachine::can_transition(
            &ExecutionStatus::PendingAdmission,
            &ExecutionStatus::Executing
        ));
        assert!(StateMachine::can_transition(
            &ExecutionStatus::Failed,
            &ExecutionStatus::RolledBack
        ));
        assert!(!StateMachine::can_transition(
            &ExecutionStatus::Failed,
            &ExecutionStatus::Completed
        ));
    }
}
