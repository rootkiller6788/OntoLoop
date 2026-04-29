use autoloop::{AutoLoopApp, config::AppConfig};

fn extract_token(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| {
            value
                .get("token")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .expect("token in json")
}

#[tokio::test]
async fn pq6_remote_bridge_replay_token_rejected() {
    unsafe {
        std::env::set_var("AUTOLOOP_BRIDGE_JWT_ALG", "HS256");
        std::env::set_var("AUTOLOOP_BRIDGE_JWT_SECRET", "hs256-replay-secret");
    }

    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq6-remote-replay-session";

    app.ensure_session_identity(
        session_id,
        "tenant:pq6",
        "principal:pq6",
        "policy:pq6",
        3_600_000,
    )
    .await
    .expect("identity");

    let token_json = app
        .bridge_issue_jwt(session_id, "bridge:replay", "tenant:pq6", 120_000)
        .await
        .expect("issue jwt");
    let token = extract_token(&token_json);

    app.bridge_remote_start(session_id, "websocket", &token, 120_000)
        .await
        .expect("first start");
    let replay = app
        .bridge_remote_start(session_id, "websocket", &token, 120_000)
        .await;
    assert!(replay.is_err(), "replayed token should be rejected");

    let err_text = replay.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(
        err_text.contains("replay") || err_text.contains("jti"),
        "unexpected replay error: {}",
        err_text
    );
}



