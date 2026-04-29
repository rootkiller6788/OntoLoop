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
    root.push(format!("autoloop-pq5-{}-{}", std::process::id(), id));
    root
}

#[tokio::test]
async fn crash_resume_and_history_compaction_alignment() {
    let checkpoint_root = unique_checkpoint_root();
    let memory_window = 3;
    let session_id = "pq5-session";

    let store_a = SessionStore::with_checkpoint_root(memory_window, checkpoint_root.clone());
    store_a.append_user_message(session_id, "u1").await;
    store_a.append_assistant_message(session_id, "a1").await;
    store_a
        .append_tool_message(session_id, "mcp::x", "t1")
        .await;
    store_a.append_user_message(session_id, "u2").await;
    store_a.append_assistant_message(session_id, "a2").await;

    let history_before_crash = store_a.history(session_id).await;
    assert_eq!(history_before_crash.len(), memory_window);
    assert_eq!(history_before_crash[0].content, "mcp::x: t1");
    assert_eq!(history_before_crash[1].content, "u2");
    assert_eq!(history_before_crash[2].content, "a2");

    drop(store_a);

    let store_b = SessionStore::with_checkpoint_root(memory_window, checkpoint_root.clone());
    let restored = store_b.load_from_checkpoint(session_id).await;
    assert!(
        restored,
        "session should be restored from checkpoint after crash"
    );

    let history_after_resume = store_b.history(session_id).await;
    assert_eq!(
        message_pairs(&history_after_resume),
        message_pairs(&history_before_crash)
    );

    let snapshot = store_b
        .resume_snapshot(session_id)
        .await
        .expect("resume snapshot should exist");
    assert_eq!(snapshot.recovered_messages, 5);
    assert_eq!(
        message_pairs(&snapshot.compacted_history),
        message_pairs(&history_after_resume)
    );

    let compacted = store_b.compacted_history(session_id).await;
    assert_eq!(
        message_pairs(&compacted),
        message_pairs(&history_after_resume)
    );

    let checkpoint = store_b
        .checkpoint(session_id)
        .await
        .expect("checkpoint should be persisted");
    assert_eq!(checkpoint.compaction.window_size, memory_window);
    assert_eq!(
        message_pairs(&checkpoint.compaction.compacted_history),
        message_pairs(&history_after_resume)
    );

    let _ = fs::remove_dir_all(checkpoint_root);
}
fn message_pairs(messages: &[autoloop::providers::ChatMessage]) -> Vec<(String, String)> {
    messages
        .iter()
        .map(|msg| (msg.role.clone(), msg.content.clone()))
        .collect()
}



