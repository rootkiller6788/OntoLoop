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
async fn pq6_remote_bridge_hs256_issue_and_start_with_audit() {
    unsafe {
        std::env::set_var("AUTOLOOP_BRIDGE_JWT_ALG", "HS256");
        std::env::set_var("AUTOLOOP_BRIDGE_JWT_SECRET", "hs256-test-secret");
    }

    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq6-remote-hs-session";

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
        .bridge_issue_jwt(session_id, "bridge:hs", "tenant:pq6", 120_000)
        .await
        .expect("issue jwt");
    let token = extract_token(&token_json);

    let started = app
        .bridge_remote_start(session_id, "websocket", &token, 120_000)
        .await
        .expect("remote start");
    let started_json = serde_json::from_str::<serde_json::Value>(&started).expect("json");
    assert_eq!(
        started_json
            .get("running")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let status_json = app
        .bridge_remote_status(session_id)
        .await
        .expect("remote status");
    let status = serde_json::from_str::<serde_json::Value>(&status_json).expect("status json");
    assert_eq!(
        status.get("algorithm").and_then(serde_json::Value::as_str),
        Some("HS256")
    );
    assert_eq!(
        status
            .get("token_issued")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}



