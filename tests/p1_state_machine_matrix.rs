use autoloop::session::{signal::WorkflowSignal, state::WorkflowState, transition::next_state};

#[test]
fn covers_legal_and_illegal_transitions_for_all_states() {
    let states = [
        WorkflowState::Intake,
        WorkflowState::PolicyReview,
        WorkflowState::Planned,
        WorkflowState::Scheduled,
        WorkflowState::Executing,
        WorkflowState::Verifying,
        WorkflowState::Closed,
        WorkflowState::Blocked,
    ];
    let signals = [
        WorkflowSignal::IntentReceived,
        WorkflowSignal::PolicyApproved,
        WorkflowSignal::PolicyRejected,
        WorkflowSignal::PlanCommitted,
        WorkflowSignal::TaskScheduled,
        WorkflowSignal::ExecutionStarted,
        WorkflowSignal::ExecutionFailed,
        WorkflowSignal::RuntimeBlocked,
        WorkflowSignal::VerifyPassed,
        WorkflowSignal::VerifyRejected,
        WorkflowSignal::Closed,
    ];

    let mut legal_count = 0usize;
    let mut illegal_count = 0usize;
    let mut had_legal_per_state = std::collections::HashMap::new();
    let mut had_illegal_per_state = std::collections::HashMap::new();

    for state in states {
        for signal in signals {
            match next_state(state, signal) {
                Ok(_) => {
                    legal_count += 1;
                    had_legal_per_state.insert(state, true);
                }
                Err(_) => {
                    illegal_count += 1;
                    had_illegal_per_state.insert(state, true);
                }
            }
        }
    }

    assert!(legal_count > 0, "matrix must include legal transitions");
    assert!(illegal_count > 0, "matrix must include illegal transitions");
    for state in states {
        assert!(
            had_legal_per_state.get(&state).copied().unwrap_or(false),
            "state {:?} should have at least one legal transition",
            state
        );
        assert!(
            had_illegal_per_state.get(&state).copied().unwrap_or(false),
            "state {:?} should have at least one illegal transition",
            state
        );
    }
}



