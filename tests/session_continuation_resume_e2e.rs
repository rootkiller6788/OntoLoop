use autoloop::{AutoLoopApp, config::AppConfig};

#[tokio::test]
async fn session_continuation_resume_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq-session-resume";

    app.ensure_session_identity(
        session_id,
        "tenant:pq",
        "principal:pq",
        "policy:pq",
        3_600_000,
    )
    .await
    .expect("identity");

    let _ = app
        .process_direct(
            session_id,
            "Please produce a concise plan and continue if needed",
        )
        .await
        .expect("process");

    let checkpoint = app
        .sessions
        .checkpoint(session_id)
        .await
        .expect("checkpoint");

    assert!(
        checkpoint.continuation_turn_id.is_some(),
        "continuation turn id missing"
    );
    assert!(
        checkpoint.continuation_checkpoint_token.is_some(),
        "continuation checkpoint token missing"
    );

    let continuation_ref = app.state_store()
        .get_knowledge(&format!("query:continuation:{session_id}:latest"))
        .await
        .expect("continuation ref read");
    assert!(continuation_ref.is_some(), "query continuation ref missing");
}




