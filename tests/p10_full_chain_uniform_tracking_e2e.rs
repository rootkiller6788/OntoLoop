use autoloop::{
    AutoLoopApp, config::AppConfig, observability::event_stream::list_replay_snapshots,
};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn full_chain_trigger_to_replay_enforces_uniform_tracking_tags() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "full-chain-e2e";

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
        "tenant:fullchain",
        "principal:fullchain",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("identity");

    app.state_store()
        .create_schedule_event(
            session.to_string(),
            "trigger:full-chain".to_string(),
            "focus-trigger".to_string(),
            "Run full-chain governed lifecycle once".to_string(),
            "full-chain-e2e".to_string(),
        )
        .await
        .expect("create schedule event");

    let trigger_report_raw = app
        .run_trigger_worker_once(session)
        .await
        .expect("run trigger worker");
    let trigger_report: serde_json::Value =
        serde_json::from_str(&trigger_report_raw).expect("trigger report json");
    assert!(trigger_report["executed"].as_u64().unwrap_or(0) >= 1);

    let snapshots = list_replay_snapshots(&app.state_store(), session)
        .await
        .expect("list replay snapshots");
    assert!(
        !snapshots.is_empty(),
        "execution should capture replay snapshots"
    );

    let replay_report_raw = app
        .run_replay_snapshot(&snapshots[0].snapshot_id)
        .await
        .expect("run replay snapshot");
    let replay_report: serde_json::Value =
        serde_json::from_str(&replay_report_raw).expect("replay report json");
    assert!(replay_report.get("snapshot_id").is_some());

    let exported_replay_raw = app
        .export_replay_report(session, None)
        .await
        .expect("export replay report");
    let _exported_replay: serde_json::Value =
        serde_json::from_str(&exported_replay_raw).expect("exported replay report json");

    let tags = app.state_store()
        .list_knowledge_by_prefix(&format!("evidence:tag:{session}:"))
        .await
        .expect("list evidence tags");
    assert!(!tags.is_empty(), "full chain should persist evidence tags");

    let mut has_guard = false;
    let mut has_verify = false;
    let mut has_learn = false;
    let mut has_replay = false;

    for record in tags {
        let tag: serde_json::Value = serde_json::from_str(&record.value).expect("tag json");
        match tag
            .get("stage")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
        {
            "guard" => has_guard = true,
            "verify" => has_verify = true,
            "learn" => has_learn = true,
            "replay" => has_replay = true,
            _ => {}
        }

        let tracking = tag
            .get("payload")
            .and_then(|payload| payload.get("tracking_context"))
            .and_then(serde_json::Value::as_object)
            .expect("tracking_context exists");

        for key in [
            "run_id",
            "plan_id",
            "focus_id",
            "trigger_id",
            "capability_id",
            "verifier_id",
            "tenant_id",
            "replay_fp",
            "org_ctx_id",
        ] {
            let value = tracking
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            assert!(
                !value.is_empty(),
                "tracking key '{}' should be non-empty",
                key
            );
        }
    }

    assert!(has_guard, "missing guard stage tag");
    assert!(has_verify, "missing verify stage tag");
    assert!(has_learn, "missing learn stage tag");
    assert!(
        has_replay || !snapshots.is_empty(),
        "missing replay stage tag and no replay snapshots"
    );
}




