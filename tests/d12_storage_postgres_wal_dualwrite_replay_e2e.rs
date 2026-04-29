use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use autoloop::observability::query_plane::build_unified_query_view;
use autoloop_postgres_adapter::{
    AtomicRelationWriteInput, PostgresDb, PostgresDbConfig, RelationEdgeCurrentWrite,
    RelationEventAppendWrite, RelationHotIndexWrite,
};
use autoloop_state_adapter::{
    CostAttribution, KnowledgeReadPreference, KnowledgeMirrorMode, SessionLease, SpendLedger,
    SpendLedgerKind, StateStore, StateStoreBackend, StateStoreConfig,
};

static TEST_SCHEMA_SEQ: AtomicU64 = AtomicU64::new(1);

fn test_pg_uri() -> String {
    std::env::var("AUTOLOOP_PG_TEST_URI")
        .unwrap_or_else(|_| "postgres://postgres:123456@localhost:5432/postgres".to_string())
}

fn next_schema(prefix: &str) -> String {
    let seq = TEST_SCHEMA_SEQ.fetch_add(1, Ordering::Relaxed);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}_{now_ms}_{seq}")
}

#[tokio::test]
async fn d12_atomic_state_event_evidence_are_transactional() -> Result<()> {
    let uri = test_pg_uri();
    let schema = next_schema("ontoloop_d12_atomic");
    let db = PostgresDb::new(PostgresDbConfig {
        enabled: true,
        uri: uri.clone(),
        schema: schema.clone(),
        pool_size: 4,
        auto_migrate: true,
    });
    if let Err(error) = db.ensure_ready().await {
        eprintln!("skip d12_atomic_state_event_evidence_are_transactional: {error}");
        return Ok(());
    }

    let session_id = "d12-session";
    let trace_id = "d12-trace";
    let state_key = format!("relation:state:{session_id}:{trace_id}:1");
    let event_key = format!("relation:event:{session_id}:{trace_id}:1");
    let evidence_key = format!("relation:evidence:{session_id}:{trace_id}:1");
    let proof_key = format!("relation:write_proof:{session_id}:{trace_id}:1");

    let evidence_ref = db
        .atomic_write_relation_bundle(AtomicRelationWriteInput {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            state_key: state_key.clone(),
            state_payload: serde_json::json!({"stage":"state"}),
            relation_event_key: event_key.clone(),
            relation_event_payload: serde_json::json!({"stage":"event"}),
            evidence_key: evidence_key.clone(),
            evidence_payload: serde_json::json!({"stage":"evidence"}),
            write_proof_key: proof_key.clone(),
            write_proof_payload: serde_json::json!({"checksum":"sha256:abc"}),
            source: "d12-e2e".to_string(),
            edge_current: Some(RelationEdgeCurrentWrite {
                edge_id: "edge:d12:1".to_string(),
                from_node: "task:d12".to_string(),
                to_node: "artifact:d12".to_string(),
                edge_type: "produced_by".to_string(),
                payload: serde_json::json!({"confidence":1.0}),
            }),
            event_append: Some(RelationEventAppendWrite {
                event_id: "evt:d12:1".to_string(),
                event_type: "relation.atomic_write".to_string(),
                payload: serde_json::json!({"trace_id":trace_id}),
            }),
            hot_index_entries: vec![RelationHotIndexWrite {
                hot_key: "hot:d12:1".to_string(),
                relation_kind: "trace".to_string(),
                relation_ref: trace_id.to_string(),
                score: 1.0,
                payload: serde_json::json!({"source":"d12"}),
            }],
        })
        .await?;

    assert!(db.get_knowledge(&state_key).await?.is_some());
    assert!(db.get_knowledge(&event_key).await?.is_some());
    assert!(db.get_knowledge(&evidence_key).await?.is_some());
    assert!(db.get_knowledge(&proof_key).await?.is_some());

    let relation_events = db.list_relation_events(session_id, 10).await?;
    assert!(
        relation_events.iter().any(|item| item.evidence_ref == evidence_ref),
        "relation event should keep same evidence_ref from atomic transaction"
    );

    let relation_edges = db.list_relation_edges(session_id, 10).await?;
    assert!(
        relation_edges.iter().any(|item| item.evidence_ref == evidence_ref),
        "relation edge should keep same evidence_ref from atomic transaction"
    );

    let relation_hot = db.list_relation_hot_index(session_id, 10).await?;
    assert!(
        relation_hot.iter().any(|item| item.evidence_ref == evidence_ref),
        "relation hot-index should keep same evidence_ref from atomic transaction"
    );

    Ok(())
}

#[tokio::test]
async fn d12_shadow_dualwrite_diff_and_replay_view_visible() -> Result<()> {
    let uri = test_pg_uri();
    let schema = next_schema("ontoloop_d12_shadow");
    let postgres = PostgresDb::new(PostgresDbConfig {
        enabled: true,
        uri: uri.clone(),
        schema: schema.clone(),
        pool_size: 4,
        auto_migrate: true,
    });
    if let Err(error) = postgres.ensure_ready().await {
        eprintln!("skip d12_shadow_dualwrite_diff_and_replay_view_visible: {error}");
        return Ok(());
    }

    let store = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "memory://d12".to_string(),
        module_name: "ontoloop".to_string(),
        namespace: "d12".to_string(),
        pool_size: 2,
    });
    store.configure_knowledge_mirror(
        postgres.clone(),
        KnowledgeMirrorMode::Shadow,
        KnowledgeReadPreference::PrimaryStore,
        0,
    )?;

    let session_id = "d12-shadow-session";
    store
        .create_schedule_event(
            session_id.to_string(),
            "wake.plan.execute".to_string(),
            "planner".to_string(),
            "{\"intent\":\"d12\"}".to_string(),
            "agent-d12".to_string(),
        )
        .await?;
    store
        .upsert_agent_state(
            session_id.to_string(),
            "user message".to_string(),
            Some("assistant message".to_string()),
        )
        .await?;
    store
        .upsert_session_lease(SessionLease {
            lease_token: "lease-d12".to_string(),
            session_id: session_id.to_string(),
            tenant_id: "tenant-d12".to_string(),
            principal_id: "principal-d12".to_string(),
            policy_id: "policy-d12".to_string(),
            expires_at_ms: 9_999_999_999,
            issued_at_ms: 1,
        })
        .await?;
    store
        .upsert_cost_attribution(CostAttribution {
            attribution_id: "attr-d12".to_string(),
            tenant_id: "tenant-d12".to_string(),
            principal_id: "principal-d12".to_string(),
            policy_id: "policy-d12".to_string(),
            session_id: session_id.to_string(),
            trace_id: "trace-d12".to_string(),
            task_id: "task-d12".to_string(),
            capability_id: "provider:default".to_string(),
            provider_tokens: 10,
            tool_invocations: 1,
            duration_ms: 20,
            token_cost_micros: 10,
            tool_cost_micros: 5,
            duration_cost_micros: 2,
            total_cost_micros: 17,
            settled_at_ms: 2,
        })
        .await?;
    store
        .append_spend_ledger(SpendLedger {
            ledger_id: "ledger-d12".to_string(),
            tenant_id: "tenant-d12".to_string(),
            account_id: "account-d12".to_string(),
            session_id: session_id.to_string(),
            trace_id: "trace-d12".to_string(),
            task_id: "task-d12".to_string(),
            capability_id: "provider:default".to_string(),
            kind: SpendLedgerKind::Settle,
            amount_micros: 17,
            token_cost_micros: 10,
            tool_cost_micros: 5,
            duration_cost_micros: 2,
            reason: "d12".to_string(),
            created_at_ms: 2,
        })
        .await?;
    store
        .upsert_json_knowledge(
            format!("memory:{session_id}:consolidation"),
            &serde_json::json!({"ok": true}),
            "d12",
        )
        .await?;

    let view = build_unified_query_view(&store, session_id, None).await?;
    let storage_shadow = view
        .metrics
        .get("storage_shadow_diff")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let reports = storage_shadow
        .get("reports")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !reports.is_empty(),
        "shadow mode should emit dual-write diff reports"
    );

    let domains = reports
        .iter()
        .filter_map(|item| item.get("domain").and_then(serde_json::Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(domains.contains("knowledge"));
    assert!(domains.contains("scheduler"));
    assert!(domains.contains("identity"));
    assert!(domains.contains("billing"));

    for item in &reports {
        let evidence_ref = item
            .get("evidence_ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        assert!(
            !evidence_ref.trim().is_empty(),
            "every shadow diff report should carry evidence_ref"
        );
    }

    Ok(())
}
