use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use autoloop::session::SessionStore;

fn unique_checkpoint_root() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut root = std::env::temp_dir();
    root.push(format!("autoloop-d7-{}-{}", std::process::id(), id));
    root
}

#[tokio::test]
async fn session_resume_snapshot_is_redacted_and_evidence_bound() {
    let checkpoint_root = unique_checkpoint_root();
    let session_id = "d7-session";
    let store = SessionStore::with_checkpoint_root(8, checkpoint_root.clone());

    store
        .append_user_message(
            session_id,
            "token=sk-ABCDEFGHIJKLMNOPQRSTUVWXYZ123456; owner=alice@example.com",
        )
        .await;
    store.append_assistant_message(session_id, "ack").await;

    let checkpoint = store
        .checkpoint(session_id)
        .await
        .expect("checkpoint should exist");
    assert!(checkpoint.evidence_ref.is_some());
    assert!(checkpoint.redaction_summary.redacted_fields >= 1);

    let snapshot = store
        .resume_snapshot(session_id)
        .await
        .expect("resume snapshot should exist");
    assert!(snapshot.evidence_bound);
    assert!(snapshot.evidence_ref.is_some());

    let compacted_blob = serde_json::to_string(&snapshot.compacted_history).unwrap_or_default();
    assert!(compacted_blob.contains("[REDACTED_API_KEY]"));
    assert!(compacted_blob.contains("[REDACTED_EMAIL]"));

    let _ = fs::remove_dir_all(checkpoint_root);
}

#[tokio::test]
async fn checkpoint_restore_fails_when_evidence_sidecar_is_missing() {
    let checkpoint_root = unique_checkpoint_root();
    let session_id = "d7-missing-evidence";
    let store = SessionStore::with_checkpoint_root(4, checkpoint_root.clone());

    store
        .append_user_message(session_id, "Bearer secret-token-value")
        .await;

    let evidence_file = checkpoint_root
        .join("evidence")
        .join(format!("{}.json", session_id.replace(':', "")));
    if evidence_file.exists() {
        let _ = fs::remove_file(&evidence_file);
    }

    let restored = store.load_from_checkpoint(session_id).await;
    assert!(!restored, "restore must fail without evidence sidecar");

    let _ = fs::remove_dir_all(checkpoint_root);
}



