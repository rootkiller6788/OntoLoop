use autoloop::{AutoLoopApp, config::AppConfig};
use rand::thread_rng;
use rsa::{
    RsaPrivateKey,
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
};

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
async fn pq6_remote_bridge_rs256_issue_and_start() {
    let mut rng = thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("rsa private key");
    let public_key = private_key.to_public_key();
    let private_pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("private pem")
        .to_string();
    let public_pem = public_key
        .to_public_key_pem(LineEnding::LF)
        .expect("public pem")
        .to_string();

    unsafe {
        std::env::set_var("AUTOLOOP_BRIDGE_JWT_ALG", "RS256");
        std::env::set_var("AUTOLOOP_BRIDGE_JWT_PRIVATE_KEY_PEM", private_pem);
        std::env::set_var("AUTOLOOP_BRIDGE_JWT_PUBLIC_KEY_PEM", public_pem);
    }

    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq6-remote-rs-session";

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
        .bridge_issue_jwt(session_id, "bridge:rs", "tenant:pq6", 120_000)
        .await
        .expect("issue jwt rs256");
    let token = extract_token(&token_json);

    let started = app
        .bridge_remote_start(session_id, "websocket", &token, 120_000)
        .await
        .expect("remote start rs256");
    let started_json = serde_json::from_str::<serde_json::Value>(&started).expect("json");
    assert_eq!(
        started_json
            .get("running")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
}



