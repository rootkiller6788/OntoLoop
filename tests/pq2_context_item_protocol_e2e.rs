use autoloop::{AutoLoopApp, config::AppConfig, contracts::context::KnowledgeContext};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn context_item_protocol_tracks_source_scope_priority_and_budget() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq2-context-item-protocol";

    app.state_store()
        .grant_permissions(
            session_id,
            vec![
                PermissionAction::Read,
                PermissionAction::Write,
                PermissionAction::Dispatch,
            ],
        )
        .await
        .expect("grant permissions");
    app.ensure_session_identity(
        session_id,
        "tenant:pq2",
        "principal:pq2",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");

    app.state_store()
        .upsert_knowledge(
            "kb:tenant:pq2:seed-doc".to_string(),
            "{\"title\":\"seed knowledge context\"}".to_string(),
            "test-seed".to_string(),
        )
        .await
        .expect("seed kb");
    app.state_store()
        .upsert_knowledge(
            format!("memory:supermemory:atomic:{session_id}:seed"),
            "{\"memory_id\":\"seed-memory\",\"chunk_id\":\"seed-chunk\"}".to_string(),
            "test-seed".to_string(),
        )
        .await
        .expect("seed supermemory atomic");

    app.sessions
        .append_user_message(session_id, "Need traceable context item protocol")
        .await;
    app.sessions
        .append_tool_message(session_id, "context-probe", "tool-state:ready")
        .await;

    let checkpoint = app
        .sessions
        .checkpoint(session_id)
        .await
        .expect("checkpoint should exist after appending messages");
    assert!(
        checkpoint
            .context_items
            .iter()
            .any(|item| item.kind == "session"),
        "checkpoint should persist session context items"
    );
    assert!(
        checkpoint
            .context_items
            .iter()
            .any(|item| item.kind == "tool_state"),
        "checkpoint should persist tool-state context items"
    );

    let _ = app
        .process_requirement_swarm(
            session_id,
            "Compile one run and persist unified context items for traceability.",
        )
        .await
        .expect("process requirement swarm");

    let context_record = app.state_store()
        .get_knowledge(&format!("knowledge-context:{session_id}:latest"))
        .await
        .expect("load knowledge context")
        .expect("knowledge context latest exists");
    let context: KnowledgeContext =
        serde_json::from_str(&context_record.value).expect("deserialize knowledge context");

    assert!(
        context
            .context_items
            .iter()
            .any(|item| item.kind == "knowledge"),
        "knowledge context items should include knowledge sources"
    );
    assert!(
        context
            .context_items
            .iter()
            .any(|item| item.kind == "supermemory"),
        "knowledge context items should include supermemory sources"
    );
    assert!(
        context
            .context_items
            .iter()
            .any(|item| item.kind == "session"),
        "knowledge context items should include session sources"
    );
    assert!(
        context
            .context_items
            .iter()
            .any(|item| item.kind == "tool_state"),
        "knowledge context items should include tool-state sources"
    );

    for item in &context.context_items {
        assert!(
            !item.source_ref.trim().is_empty(),
            "context item source_ref should be populated"
        );
        assert!(
            !item.permission_scope.trim().is_empty(),
            "context item permission_scope should be populated"
        );
        assert!(
            item.priority > 0.0,
            "context item priority should be positive"
        );
        assert!(
            item.budget_micros > 0,
            "context item budget_micros should be positive"
        );
    }

    let total_count = context
        .metadata
        .get("context_item_total_count")
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(
        total_count >= context.context_items.len().saturating_sub(1),
        "context metadata should track context item totals"
    );
    let budget_total = context
        .metadata
        .get("context_item_budget_micros_total")
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(0);
    assert!(
        budget_total > 0,
        "context metadata should track total budget for context items"
    );
}




