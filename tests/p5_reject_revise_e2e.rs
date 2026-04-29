use std::sync::Arc;

use autoloop::session::{
    audit::InMemoryAuditSink, machine::WorkflowMachine, signal::WorkflowSignal,
    state::WorkflowState,
};

#[tokio::test]
async fn e2e_policy_reject_revise_path() {
    let sink = Arc::new(InMemoryAuditSink::default());
    let mut machine = WorkflowMachine::new("e2e-policy-reject", sink.clone());

    machine
        .apply(WorkflowSignal::IntentReceived, Some("intake".into()))
        .await
        .expect("intake");
    machine
        .apply(WorkflowSignal::PolicyRejected, Some("policy failed".into()))
        .await
        .expect("policy reject loop");
    machine
        .apply(WorkflowSignal::PolicyApproved, Some("policy fixed".into()))
        .await
        .expect("policy approve");
    machine
        .apply(WorkflowSignal::PlanCommitted, Some("plan".into()))
        .await
        .expect("plan");
    machine
        .apply(WorkflowSignal::TaskScheduled, Some("scheduled".into()))
        .await
        .expect("scheduled");
    machine
        .apply(WorkflowSignal::VerifyPassed, Some("final pass".into()))
        .await
        .expect("verify pass");

    assert_eq!(machine.state(), WorkflowState::Closed);
    let records = sink.records().await;
    assert!(records.iter().any(|record| {
        record.signal == WorkflowSignal::PolicyRejected && record.to == WorkflowState::PolicyReview
    }));
}

#[tokio::test]
async fn e2e_verify_reject_revise_path() {
    let sink = Arc::new(InMemoryAuditSink::default());
    let mut machine = WorkflowMachine::new("e2e-verify-reject", sink.clone());

    machine
        .apply(WorkflowSignal::IntentReceived, Some("intake".into()))
        .await
        .expect("intake");
    machine
        .apply(
            WorkflowSignal::PolicyApproved,
            Some("policy approved".into()),
        )
        .await
        .expect("policy");
    machine
        .apply(WorkflowSignal::PlanCommitted, Some("plan".into()))
        .await
        .expect("plan");
    machine
        .apply(WorkflowSignal::TaskScheduled, Some("scheduled".into()))
        .await
        .expect("scheduled");
    machine
        .apply(WorkflowSignal::VerifyRejected, Some("needs revise".into()))
        .await
        .expect("verify reject");
    machine
        .apply(WorkflowSignal::PlanCommitted, Some("replan".into()))
        .await
        .expect("replan");
    machine
        .apply(WorkflowSignal::TaskScheduled, Some("rescheduled".into()))
        .await
        .expect("rescheduled");
    machine
        .apply(WorkflowSignal::VerifyPassed, Some("pass".into()))
        .await
        .expect("verify pass");

    assert_eq!(machine.state(), WorkflowState::Closed);
    let records = sink.records().await;
    assert!(records.iter().any(|record| {
        record.signal == WorkflowSignal::VerifyRejected && record.to == WorkflowState::Planned
    }));
}



