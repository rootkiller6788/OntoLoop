use autoloop::{AutoLoopApp, config::AppConfig};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn governance_policy_and_cross_kernel_chain_is_persisted() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "policy-feedback-e2e";

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
        "tenant:policy-e2e",
        "principal:policy-e2e",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("identity");

    app.state_store()
        .create_schedule_event(
            session.to_string(),
            "trigger:policy-feedback".to_string(),
            "focus-trigger".to_string(),
            "Run governance/policy/cross-kernel chain".to_string(),
            "policy-feedback-test".to_string(),
        )
        .await
        .expect("schedule");

    let _ = app
        .run_trigger_worker_once(session)
        .await
        .expect("trigger worker");

    assert!(
        app.state_store()
            .get_knowledge(&format!("governance:{session}:telemetry-scope"))
            .await
            .expect("scope read")
            .is_some()
    );

    assert!(
        app.state_store()
            .get_knowledge(&format!("policy-signals:{session}:latest"))
            .await
            .expect("policy signals")
            .is_some()
    );

    assert!(
        app.state_store()
            .get_knowledge(&format!("policy:{session}:adaptive-update"))
            .await
            .expect("policy adaptive")
            .is_some()
    );

    assert!(
        app.state_store()
            .get_knowledge(&format!("capability-admission:{session}:feedback"))
            .await
            .expect("admission feedback")
            .is_some()
    );

    assert!(
        app.state_store()
            .get_knowledge(&format!("runtime-mode:{session}:hint"))
            .await
            .expect("runtime mode hint")
            .is_some()
    );

    assert!(
        app.state_store()
            .get_knowledge(&format!("org-sharing-gate:{session}:latest"))
            .await
            .expect("org gate")
            .is_some()
    );

    let cross_kernel = app.state_store()
        .list_knowledge_by_prefix(&format!("cross-kernel:{session}:"))
        .await
        .expect("cross-kernel decisions");
    assert!(
        !cross_kernel.is_empty(),
        "cross-kernel decisions should be persisted"
    );

    let tags = app.state_store()
        .list_knowledge_by_prefix(&format!("evidence:tag:{session}:"))
        .await
        .expect("tags");
    let has_cross_kernel_tag = tags.iter().any(|record| {
        serde_json::from_str::<serde_json::Value>(&record.value)
            .ok()
            .and_then(|value| {
                value
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .map(|label| label == "cross-kernel.decision.propagated")
            .unwrap_or(false)
    });
    assert!(
        has_cross_kernel_tag,
        "cross-kernel propagation tag should exist"
    );
}




