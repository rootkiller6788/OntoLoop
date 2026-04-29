use autoloop::runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage};
use autoloop_state_adapter::{StateStore, StateStoreBackend, StateStoreConfig};

fn test_store() -> StateStore {
    StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    })
}

#[tokio::test]
async fn evidence_worm_hardening_blocks_tamper_replay_and_rollback_attacks() {
    let db = test_store();
    let session_id = "worm-session";
    let trace_id = "trace-worm";

    let first_key = EvidenceLedgerWriter::append_stage(
        &db,
        session_id,
        trace_id,
        EvidenceStage::Admission,
        serde_json::json!({"status":"admitted"}),
        None,
    )
    .await
    .expect("first stage append");

    let first_record = db
        .get_knowledge(&first_key)
        .await
        .expect("first get")
        .expect("first exists");

    let first_json: serde_json::Value =
        serde_json::from_str(&first_record.value).expect("first json parse");
    let first_hash = first_json
        .get("record_hash")
        .and_then(serde_json::Value::as_str)
        .expect("first hash")
        .to_string();

    let second_key = EvidenceLedgerWriter::append_stage(
        &db,
        session_id,
        trace_id,
        EvidenceStage::Execution,
        serde_json::json!({"step":"run"}),
        None,
    )
    .await
    .expect("second stage append");

    let second_record = db
        .get_knowledge(&second_key)
        .await
        .expect("second get")
        .expect("second exists");
    let second_json: serde_json::Value =
        serde_json::from_str(&second_record.value).expect("second json parse");
    let second_hash = second_json
        .get("record_hash")
        .and_then(serde_json::Value::as_str)
        .expect("second hash")
        .to_string();

    // tamper attack: direct overwrite on existing evidence key should fail
    let tamper = db
        .upsert_json_knowledge(
            first_key.clone(),
            &serde_json::json!({"tampered":true}),
            "attacker",
        )
        .await;
    assert!(tamper.is_err(), "tamper update should be blocked");
    assert!(
        tamper
            .err()
            .map(|err| err.to_string())
            .unwrap_or_default()
            .contains("WORM"),
        "tamper rejection must mention WORM"
    );

    // replay attack: duplicate append payload over existing key should fail
    let replay = db
        .upsert_knowledge(
            second_key.clone(),
            second_record.value.clone(),
            "attacker-replay".into(),
        )
        .await;
    assert!(replay.is_err(), "replay should be blocked");

    // rollback attack: stale prev hash should fail and produce auditable entry
    let rollback = EvidenceLedgerWriter::append_stage(
        &db,
        session_id,
        trace_id,
        EvidenceStage::Verify,
        serde_json::json!({"attempt":"rollback-branch"}),
        Some(&first_hash),
    )
    .await;
    assert!(rollback.is_err(), "rollback branch should be blocked");
    let rollback_error = rollback
        .err()
        .map(|err| err.to_string())
        .unwrap_or_default();
    assert!(
        rollback_error.contains("audit_ref="),
        "rollback rejection should expose audit reference"
    );

    EvidenceLedgerWriter::verify_stage_chain(&db, session_id, trace_id)
        .await
        .expect("chain verify after attacks");

    let audits = db
        .list_knowledge_by_prefix(&format!("audit:evidence:worm:{session_id}:{trace_id}:"))
        .await
        .expect("audit list");
    assert!(!audits.is_empty(), "worm violation must be auditable");

    assert_ne!(first_hash, second_hash, "hash chain should advance");
}



