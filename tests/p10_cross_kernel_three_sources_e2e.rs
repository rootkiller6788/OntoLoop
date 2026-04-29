use autoloop::{AutoLoopApp, config::AppConfig};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn cross_kernel_requires_three_sources_in_same_session() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "cross-kernel-3sources-e2e";

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
        "tenant:cross-kernel",
        "principal:cross-kernel",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("identity");

    app.state_store()
        .create_schedule_event(
            session.to_string(),
            "trigger:cross-kernel-sources".to_string(),
            "focus-trigger".to_string(),
            "Run one governed swarm cycle".to_string(),
            "cross-kernel-3sources-e2e".to_string(),
        )
        .await
        .expect("schedule");

    let _ = app
        .run_trigger_worker_once(session)
        .await
        .expect("trigger worker");

    let decisions = app.state_store()
        .list_knowledge_by_prefix(&format!("cross-kernel:{session}:"))
        .await
        .expect("cross-kernel decisions");
    assert!(
        !decisions.is_empty(),
        "cross-kernel propagation decisions should exist"
    );

    let mut has_promotion_gate = false;

    for record in decisions {
        let value: serde_json::Value =
            serde_json::from_str(&record.value).expect("cross-kernel decision json");
        let source = value
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        match source {
            "promotion_gate" => has_promotion_gate = true,
            _ => {}
        }
    }
    assert!(
        has_promotion_gate,
        "missing cross-kernel source: promotion_gate"
    );
}




