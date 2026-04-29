use autoloop::{
    AutoLoopApp, config::AppConfig, observability::query_plane::persist_unified_query_view,
};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn day10_swarm_query_and_replay_artifacts_end_to_end() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "day10-e2e";

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
        .expect("grant permissions");
    app.ensure_session_identity(
        session,
        "tenant:day10",
        "principal:day10",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");

    let response = app
        .process_requirement_swarm(
            session,
            "Design a governed swarm execution plan and run one safe iteration",
        )
        .await
        .expect("swarm process");
    assert!(
        !response.trim().is_empty(),
        "swarm response should not be empty"
    );

    let trace_id = format!("trace:{session}:day10");
    let view = persist_unified_query_view(&app.state_store(), session, Some(&trace_id))
        .await
        .expect("persist query plane");
    assert_eq!(view.session_id, session);
    assert_eq!(view.trace_id.as_deref(), Some(trace_id.as_str()));
    assert!(view.generated_at_ms > 0);
    assert!(view.metrics.is_object(), "metrics should be object");
    assert!(view.ledger.is_object(), "ledger should be object");
    assert!(view.replay.is_object(), "replay should be object");

    let report_raw = app
        .export_replay_report(session, None)
        .await
        .expect("replay report");
    let report: serde_json::Value = serde_json::from_str(&report_raw).expect("replay report json");
    assert_eq!(report["session_id"].as_str(), Some(session));

    let org_context = app.state_store()
        .get_knowledge(&format!("org-context:{session}:latest"))
        .await
        .expect("get org context");
    assert!(
        org_context.is_some(),
        "org context should be materialized by orchestration chain",
    );
}

#[test]
fn day10_trigger_wake_report_surface_end_to_end() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(32 * 1024 * 1024)
        .build()
        .expect("build runtime");

    runtime.block_on(async {
        let app = AutoLoopApp::new(AppConfig::default());
        let session = "day10-trigger";

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
            .expect("grant permissions");
        app.ensure_session_identity(
            session,
            "tenant:day10",
            "principal:trigger",
            "policy:default",
            3_600_000,
        )
        .await
        .expect("seed identity");

        app.state_store()
            .create_schedule_event(
                session.to_string(),
                "trigger:day10".to_string(),
                "focus-trigger".to_string(),
                "Run day10 trigger acceptance".to_string(),
                "day10-trigger-test".to_string(),
            )
            .await
            .expect("create trigger event");

        let report_raw = app
            .run_trigger_worker_once(session)
            .await
            .expect("run trigger worker");
        let report: serde_json::Value =
            serde_json::from_str(&report_raw).expect("trigger report json");
        assert!(report["executed"].as_u64().unwrap_or(0) >= 1);
        assert!(report["completed"].as_u64().unwrap_or(0) >= 1);

        let events = app.state_store()
            .list_schedule_events(session)
            .await
            .expect("list schedule events");
        assert!(
            events
                .iter()
                .any(|event| event.status.eq_ignore_ascii_case("completed")),
            "at least one trigger event should be completed",
        );
    });
}




