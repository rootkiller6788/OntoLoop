use autoloop::{
    contracts::evidence::{BudgetLedgerOp, BudgetLedgerRecord, ReplayFingerprint},
    runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
    state_store_adapter::{StateStoreBackend, StateStore, StateStoreConfig},
};

#[tokio::test]
async fn evidence_ledger_covers_all_six_segments() {
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    });
    let session = "evidence-six";
    let trace = "trace-six";
    let _ = EvidenceLedgerWriter::append_stage(
        &db,
        session,
        trace,
        EvidenceStage::Admission,
        serde_json::json!({"ok":true}),
        None,
    )
    .await
    .expect("admission");
    let _ = EvidenceLedgerWriter::append_stage(
        &db,
        session,
        trace,
        EvidenceStage::Execution,
        serde_json::json!({"ok":true}),
        Some("admission-hash"),
    )
    .await
    .expect("execution");
    let _ = EvidenceLedgerWriter::append_budget(
        &db,
        session,
        &BudgetLedgerRecord {
            trace_id: trace.into(),
            session_id: session.into(),
            task_id: "task-1".into(),
            op: BudgetLedgerOp::Reserve,
            amount_micros: 42,
            reason: "reserve".into(),
            created_at_ms: autoloop::orchestration::current_time_ms(),
        },
    )
    .await
    .expect("budget");
    let _ = EvidenceLedgerWriter::append_stage(
        &db,
        session,
        trace,
        EvidenceStage::Verify,
        serde_json::json!({"verdict":"pass"}),
        Some("execution-hash"),
    )
    .await
    .expect("verify");
    let _ = EvidenceLedgerWriter::append_stage(
        &db,
        session,
        trace,
        EvidenceStage::Promotion,
        serde_json::json!({"promotion":"accepted"}),
        Some("verify-hash"),
    )
    .await
    .expect("promotion");
    let _ = EvidenceLedgerWriter::append_replay(
        &db,
        session,
        &ReplayFingerprint {
            trace_id: trace.into(),
            boundary: "strict".into(),
            input_hash: "i".into(),
            output_hash: "o".into(),
            matched: true,
            mismatch_explanation: None,
        },
    )
    .await
    .expect("replay");
    assert!(
        db.list_knowledge_by_prefix("evidence:stage:evidence-six:trace-six:")
            .await
            .expect("stage")
            .len()
            >= 4
    );
    assert_eq!(
        db.list_knowledge_by_prefix("evidence:budget:evidence-six:")
            .await
            .expect("budget")
            .len(),
        1
    );
    assert_eq!(
        db.list_knowledge_by_prefix("evidence:replay:evidence-six:")
            .await
            .expect("replay")
            .len(),
        1
    );
}




