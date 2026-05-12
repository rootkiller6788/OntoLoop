use std::collections::BTreeSet;

use autoloop::{AutoLoopApp, config::AppConfig, observability::event_stream::list_session_events};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn pq3_compiler_executor_verifier_closed_loop_decisions_have_evidence_and_queue_followups() {
    let mut config = AppConfig::default();
    config.storage.backend = autoloop::config::StorageBackend::Postgres;
    config.storage.postgres.enabled = true;
    config.storage.postgres.uri = std::env::var("ONTOLOOP_TEST_POSTGRES_URI")
        .unwrap_or_else(|_| "postgres://postgres:123456@localhost:5432/postgres".to_string());
    config.storage.shadow_read_preference = "postgres".to_string();
    let app = AutoLoopApp::new(config);
    let session_id = "pq3-closed-loop";

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
        "tenant:pq3",
        "principal:pq3",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");

    let _ = app
        .process_direct(
            session_id,
            "[decision:accept] week3 chain step 1: accept branch",
        )
        .await
        .expect("accept branch should execute");

    let _ = app
        .process_direct(
            session_id,
            "[decision:repair] week3 chain step 2: repair branch",
        )
        .await
        .expect("repair branch should execute");

    let _ = app
        .process_direct(
            session_id,
            "[decision:escalate] week3 chain step 3: escalate branch",
        )
        .await
        .expect("escalate branch should execute");

    let decision_records = app.state_store()
        .list_knowledge_by_prefix(&format!("runtime:decision:{session_id}:"))
        .await
        .expect("list decision records");
    assert!(
        decision_records.len() >= 3,
        "expected at least three decision evidence records"
    );

    let parsed = decision_records
        .iter()
        .filter_map(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
        .collect::<Vec<_>>();
    let decisions = parsed
        .iter()
        .filter_map(|value| value.get("decision").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();

    assert!(decisions.contains("accept"));
    assert!(decisions.contains("repair"));
    assert!(decisions.contains("escalate"));

    for expected in ["accept", "repair", "escalate"] {
        let hit = parsed.iter().find(|value| {
            value.get("decision").and_then(serde_json::Value::as_str) == Some(expected)
        });
        assert!(hit.is_some(), "missing decision evidence for {expected}");
        let evidence_ref = hit
            .and_then(|value| value.get("evidence_ref"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            !evidence_ref.trim().is_empty(),
            "decision evidence_ref should be present for {expected}"
        );
    }

    let scheduled = app.state_store()
        .list_schedule_events(session_id)
        .await
        .expect("list schedule events");
    assert!(
        scheduled
            .iter()
            .any(|event| event.topic == "trigger:on_message:repair"),
        "repair decision should enqueue trigger runtime follow-up"
    );
    assert!(
        scheduled
            .iter()
            .any(|event| event.topic == "trigger:on_message:escalate"),
        "escalate decision should enqueue trigger runtime follow-up"
    );

    let session_events = list_session_events(&app.state_store(), session_id)
        .await
        .expect("list session events");
    let decision_event_count = session_events
        .iter()
        .filter(|event| event.kind == "context_runtime_decision")
        .count();
    assert!(
        decision_event_count >= 3,
        "decision events should be persisted to evidence stream"
    );
}




