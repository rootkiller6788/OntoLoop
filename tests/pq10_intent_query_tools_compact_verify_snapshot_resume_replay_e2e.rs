use autoloop::{
    AutoLoopApp, config::AppConfig, observability::event_stream::list_replay_snapshots,
    observability::query_plane::persist_unified_query_view,
};
use autoloop_state_adapter::{BudgetAccount, PermissionAction};
use std::time::{SystemTime, UNIX_EPOCH};

fn warmup_query_payload(turn: usize) -> String {
    format!(
        "[decision:repair] warmup turn {turn}: accumulate governed context for compaction boundary verification. {}",
        "evidence-token ".repeat(450)
    )
}

fn has_compaction_boundary(records: &[autoloop_state_adapter::KnowledgeRecord]) -> bool {
    records.iter().any(|record| {
        serde_json::from_str::<serde_json::Value>(&record.value)
            .ok()
            .and_then(|value| {
                value
                    .get("compile")
                    .and_then(|compile| compile.get("compaction_boundary"))
                    .cloned()
            })
            .map(|boundary| !boundary.is_null())
            .unwrap_or(false)
    })
}

#[tokio::test]
async fn pq10_full_chain_query_tools_compact_verify_snapshot_resume_replay() {
    let mut config = AppConfig::default();
    config.storage.backend = autoloop::config::StorageBackend::Postgres;
    config.storage.postgres.enabled = true;
    config.storage.postgres.uri = std::env::var("ONTOLOOP_TEST_POSTGRES_URI")
        .unwrap_or_else(|_| "postgres://postgres:123456@localhost:5432/postgres".to_string());
    config.storage.shadow_read_preference = "postgres".to_string();
    config.runtime.budget_enforced = false;
    config.runtime.default_budget_micros = 50_000_000;
    config.runtime.quota_window_budget_micros = 50_000_000;
    let app = AutoLoopApp::new(config);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis();
    let session = format!("pq10-full-chain-{now_ms}");
    let session = session.as_str();

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
        "tenant:pq10",
        "principal:pq10",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");
    app.state_store()
        .upsert_budget_account(BudgetAccount {
            account_id: "principal:pq10".to_string(),
            tenant_id: "tenant:pq10".to_string(),
            principal_id: "principal:pq10".to_string(),
            policy_id: "policy:default".to_string(),
            total_budget_micros: 50_000_000,
            reserved_micros: 0,
            spent_micros: 0,
            blocked_count: 0,
            updated_at_ms: autoloop::orchestration::current_time_ms(),
        })
        .await
        .expect("seed high budget account");

    let intent_response = app
        .process_requirement_swarm(
            session,
            "Intent stage: run one governed iteration with tool evidence and verifier output.",
        )
        .await
        .expect("process requirement swarm");
    assert!(
        !intent_response.trim().is_empty(),
        "intent response should not be empty"
    );

    let mut execution_feedback = Vec::new();
    for _ in 0..40 {
        execution_feedback = app
            .state_store()
            .list_knowledge_by_prefix(&format!("conversation:{session}:execution-feedback:"))
            .await
            .expect("list execution feedback");
        if !execution_feedback.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        !execution_feedback.is_empty(),
        "tools stage should persist execution feedback artifacts"
    );

    let verifier_report = app.state_store()
        .get_knowledge(&format!("protocol:{session}:execution-verifier-report"))
        .await
        .expect("read verifier report");
    assert!(
        verifier_report.is_some(),
        "verify stage should materialize execution verifier report"
    );

    for turn in 0..8 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let _ = app
            .process_direct(session, &warmup_query_payload(turn))
            .await
            .expect("process direct warmup for compaction");
    }

    let decision_records = app.state_store()
        .list_knowledge_by_prefix(&format!("runtime:decision:{session}:"))
        .await
        .expect("list runtime decisions");
    assert!(
        !decision_records.is_empty(),
        "query stage should emit runtime decision records"
    );
    assert!(
        has_compaction_boundary(&decision_records),
        "compact stage should emit at least one compaction boundary in decision evidence"
    );

    let checkpoint = app
        .sessions
        .checkpoint(session)
        .await
        .expect("checkpoint should exist");
    assert!(
        checkpoint.continuation_turn_id.is_some(),
        "snapshot should include continuation turn id"
    );
    assert!(
        checkpoint.continuation_checkpoint_token.is_some(),
        "snapshot should include continuation checkpoint token"
    );
    assert!(
        checkpoint.evidence_ref.is_some(),
        "snapshot should include evidence_ref"
    );

    let resumed = app
        .sessions
        .resume_snapshot(session)
        .await
        .expect("resume snapshot should exist");
    assert!(resumed.evidence_bound, "resume must remain evidence-bound");
    assert_eq!(
        resumed.checkpoint_digest, checkpoint.compaction.digest,
        "resume digest should match checkpoint digest"
    );
    assert!(
        app.sessions.load_from_checkpoint(session).await,
        "load_from_checkpoint should succeed after snapshot"
    );

    let replay_snapshots = list_replay_snapshots(&app.state_store(), session)
        .await
        .expect("list replay snapshots");
    assert!(
        !replay_snapshots.is_empty(),
        "replay stage should have at least one replay snapshot"
    );
    let replay_snapshot_id = replay_snapshots
        .last()
        .expect("snapshot exists")
        .snapshot_id
        .clone();

    let replay_run_raw = app
        .run_replay_snapshot(&replay_snapshot_id)
        .await
        .expect("run replay snapshot");
    let replay_run: serde_json::Value = serde_json::from_str(&replay_run_raw).expect("replay json");
    assert_eq!(replay_run["status"].as_str(), Some("accepted"));
    assert_eq!(
        replay_run["snapshot_id"].as_str(),
        Some(replay_snapshot_id.as_str())
    );

    let replay_report_raw = app
        .export_replay_report(session, None)
        .await
        .expect("export replay report");
    let replay_report: serde_json::Value =
        serde_json::from_str(&replay_report_raw).expect("replay report json");
    assert_eq!(replay_report["session_id"].as_str(), Some(session));
    assert!(
        replay_report["reports"].is_array(),
        "replay report should include reports array"
    );

    let query_view = persist_unified_query_view(&app.state_store(), session, None)
        .await
        .expect("persist query view");
    assert!(query_view.replay.is_object(), "query view should expose replay");
}




