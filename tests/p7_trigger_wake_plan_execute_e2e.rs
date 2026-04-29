use autoloop::{AutoLoopApp, config::AppConfig};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn trigger_wake_plan_execute_end_to_end() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "trigger-e2e";

    app.state_store()
        .grant_permissions(
            session,
            vec![
                PermissionAction::Read,
                PermissionAction::Write,
                PermissionAction::Dispatch,
            ],
        )
        .await
        .expect("grant");

    app.ensure_session_identity(
        session,
        "tenant:trigger",
        "principal:trigger",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("identity");

    app.state_store()
        .create_schedule_event(
            session.to_string(),
            "trigger:e2e".to_string(),
            "focus-trigger".to_string(),
            "Build a swarm plan and execute with MCP".to_string(),
            "trigger-e2e-test".to_string(),
        )
        .await
        .expect("event");

    let report_json = app.run_trigger_worker_once(session).await.expect("worker");
    let report: serde_json::Value = serde_json::from_str(&report_json).expect("json");
    assert!(report["executed"].as_u64().unwrap_or(0) >= 1);
    assert!(report["completed"].as_u64().unwrap_or(0) >= 1);

    let events = app.state_store()
        .list_schedule_events(session)
        .await
        .expect("events");
    assert!(
        events
            .iter()
            .any(|e| e.status.eq_ignore_ascii_case("completed"))
    );

    let brief = app.state_store()
        .get_knowledge(&format!("conversation:{session}:brief"))
        .await
        .expect("brief")
        .expect("brief exists");
    assert!(brief.value.contains("clarified_goal"));

    let feedback = app.state_store()
        .list_knowledge_by_prefix(&format!("conversation:{session}:execution-feedback:"))
        .await
        .expect("feedback");
    assert!(!feedback.is_empty());
}




