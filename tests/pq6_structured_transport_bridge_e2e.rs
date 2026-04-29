use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::transport::{TransportKind, TransportMessageKind},
    transport::{StructuredIoIngress, TransportIngressSource},
};

#[tokio::test]
async fn structured_transport_bridge_binds_session_runtime() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq6-bridge-session";

    app.ensure_session_identity(
        session_id,
        "tenant:pq6",
        "principal:pq6",
        "policy:pq6",
        3_600_000,
    )
    .await
    .expect("bind identity");

    let started = app
        .transport
        .start(
            session_id,
            TransportKind::Cli,
            "bridge:test",
            "tenant:pq6",
            3_600_000,
        )
        .await
        .expect("bridge start");
    assert!(started.running);
    assert_eq!(
        started
            .descriptor
            .as_ref()
            .expect("bridge descriptor")
            .transport_kind,
        TransportKind::Cli
    );

    let cli_envelope =
        StructuredIoIngress::cli_user_input(session_id, "trace:pq6:cli", "hello from cli");
    app.transport
        .ingest_envelope(&cli_envelope)
        .await
        .expect("ingest cli");

    let ws_envelope = StructuredIoIngress::build(
        TransportIngressSource::WebSocket,
        TransportMessageKind::EventStream,
        session_id,
        "trace:pq6:ws",
        serde_json::json!({"event":"tick","seq":1}),
    );
    app.transport
        .ingest_envelope(&ws_envelope)
        .await
        .expect("ingest websocket event");

    let history = app.sessions.history(session_id).await;
    assert!(
        history.iter().any(|msg| msg.content == "hello from cli"),
        "expected user message in session runtime history"
    );
    assert!(
        history
            .iter()
            .any(|msg| msg.content.contains("\"event\":\"tick\"")),
        "expected event stream payload in session runtime history"
    );

    let status = app.transport.status(session_id).await.expect("status");
    assert!(status.running);
    assert!(status.buffered_messages >= 2);

    app.transport.stop(session_id).await.expect("stop");
    let stopped_status = app
        .transport
        .status(session_id)
        .await
        .expect("status after stop");
    assert!(!stopped_status.running);

    let reject_after_stop = app.transport.ingest_envelope(&cli_envelope).await;
    assert!(
        reject_after_stop.is_err(),
        "bridge should reject ingest when stopped"
    );
}



