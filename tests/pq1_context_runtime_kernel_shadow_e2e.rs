use autoloop::{
    AutoLoopApp, config::AppConfig, observability::query_plane::persist_unified_query_view,
};

#[tokio::test]
async fn context_runtime_kernel_shadow_artifacts_are_queryable() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq-context-kernel-shadow";

    app.ensure_session_identity(
        session_id,
        "tenant:pq-context",
        "principal:pq-context",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");

    let _ = app
        .process_direct(
            session_id,
            "Return a short governed-runtime summary for context-kernel shadow verification.",
        )
        .await
        .expect("process direct");

    let shadow_records = app.state_store()
        .list_knowledge_by_prefix(&format!("context-kernel:shadow:{session_id}:"))
        .await
        .expect("list context kernel shadow records");

    assert!(
        shadow_records
            .iter()
            .any(|record| record.key.contains(":input")),
        "shadow records should include an input artifact"
    );
    assert!(
        shadow_records
            .iter()
            .any(|record| record.key.contains(":output")),
        "shadow records should include an output artifact"
    );

    let query_view = persist_unified_query_view(&app.state_store(), session_id, None)
        .await
        .expect("persist unified query view");
    let logs = query_view
        .logs
        .as_array()
        .expect("query logs should be an array");

    assert!(
        logs.iter().any(|entry| {
            entry
                .get("key")
                .and_then(serde_json::Value::as_str)
                .map(|key| key.starts_with(&format!("context-kernel:shadow:{session_id}:")))
                .unwrap_or(false)
        }),
        "query plane logs should surface context kernel shadow artifacts"
    );
}




