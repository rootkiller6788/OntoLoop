use std::sync::Arc;

use autoloop::session::{
    audit::NoopAuditSink, machine::WorkflowMachine, signal::WorkflowSignal, state::WorkflowState,
};

#[tokio::test]
async fn policy_rejected_loops_back_to_policy_review() {
    let mut machine = WorkflowMachine::new("session-policy-loop", Arc::new(NoopAuditSink));
    machine
        .apply(WorkflowSignal::IntentReceived, Some("intake".into()))
        .await
        .expect("intent -> policy review");
    let transition = machine
        .apply(
            WorkflowSignal::PolicyRejected,
            Some("policy check failed".into()),
        )
        .await
        .expect("policy reject must loop");
    assert_eq!(transition.to, WorkflowState::PolicyReview);
}

#[tokio::test]
async fn verify_rejected_loops_back_to_planned() {
    let mut machine = WorkflowMachine::new("session-verify-loop", Arc::new(NoopAuditSink));
    machine
        .apply(WorkflowSignal::IntentReceived, Some("intake".into()))
        .await
        .expect("intent");
    machine
        .apply(
            WorkflowSignal::PolicyApproved,
            Some("policy approved".into()),
        )
        .await
        .expect("policy approved");
    machine
        .apply(WorkflowSignal::PlanCommitted, Some("plan".into()))
        .await
        .expect("planned->scheduled");
    machine
        .apply(WorkflowSignal::TaskScheduled, Some("scheduled".into()))
        .await
        .expect("scheduled->executing");
    let transition = machine
        .apply(
            WorkflowSignal::VerifyRejected,
            Some("verifier asked iteration".into()),
        )
        .await
        .expect("verify rejected should route back to planned");
    assert_eq!(transition.to, WorkflowState::Planned);
}



