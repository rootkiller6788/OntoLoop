use std::path::Path;

use autoloop::{AutoLoopApp, config::AppConfig};

#[tokio::test]
async fn session_named_snapshot_and_transcript_export_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq7-session-ops";

    app.sessions
        .append_user_message(session_id, "hello from user")
        .await;
    app.sessions
        .append_assistant_message(session_id, "hello from assistant")
        .await;

    let snapshot_body = app
        .session_named_snapshot(session_id, "release-candidate")
        .await
        .expect("named snapshot");
    let snapshot_json: serde_json::Value =
        serde_json::from_str(&snapshot_body).expect("snapshot json");

    assert_eq!(
        snapshot_json
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("ok")
    );

    let snapshot_path = snapshot_json
        .get("snapshot_path")
        .and_then(serde_json::Value::as_str)
        .expect("snapshot path");
    assert!(Path::new(snapshot_path).exists(), "snapshot file should exist");

    let transcript = app
        .session_export_transcript(session_id)
        .await
        .expect("transcript");
    assert!(transcript.contains("# Session Transcript"));
    assert!(transcript.contains("hello from user"));
    assert!(transcript.contains("hello from assistant"));
}



