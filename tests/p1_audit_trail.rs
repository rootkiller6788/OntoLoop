use std::sync::Arc;

use autoloop::session::{
    audit::InMemoryAuditSink, machine::WorkflowMachine, signal::WorkflowSignal,
};

#[tokio::test]
async fn each_transition_writes_audit_record() {
    let sink = Arc::new(InMemoryAuditSink::default());
    let mut machine = WorkflowMachine::new("session-audit", sink.clone());

    machine
        .apply(
            WorkflowSignal::IntentReceived,
            Some("intent entered".into()),
        )
        .await
        .expect("intent transition");
    machine
        .apply(
            WorkflowSignal::PolicyApproved,
            Some("policy approved".into()),
        )
        .await
        .expect("policy transition");
    machine
        .apply(WorkflowSignal::PlanCommitted, Some("plan committed".into()))
        .await
        .expect("plan transition");

    let records = sink.records().await;
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].session_id, "session-audit");
    assert_eq!(records[0].reason.as_deref(), Some("intent entered"));
}



