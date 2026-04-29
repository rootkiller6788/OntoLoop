use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::transport::{SessionEventType, SessionEventV2, TransportKind},
};

#[test]
fn transport_session_event_v2_schema_validation() {
    let valid = SessionEventV2::assistant_delta(
        "session-v2",
        "trace-v2",
        "bridge:cli",
        1,
        1_700_000_000_000,
        "turn-1",
        "hello",
    );
    assert!(valid.validate().is_ok());

    let invalid = SessionEventV2 {
        schema_version: SessionEventV2::SCHEMA_VERSION.to_string(),
        event_type: SessionEventType::AssistantDelta,
        session_id: "session-v2".into(),
        trace_id: "trace-v2".into(),
        transport_id: "bridge:cli".into(),
        sequence: 2,
        emitted_at_ms: 1_700_000_000_001,
        payload: serde_json::json!({"turn_id":"turn-2"}),
    };
    assert!(invalid.validate().is_err());
}

#[tokio::test]
async fn transport_session_event_v2_replay_roundtrip() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq6-v2-replay";

    app.ensure_session_identity(
        session_id,
        "tenant:pq6-v2",
        "principal:pq6-v2",
        "policy:pq6-v2",
        3_600_000,
    )
    .await
    .expect("identity");

    app.transport
        .start(
            session_id,
            TransportKind::Cli,
            "bridge:v2",
            "tenant:pq6-v2",
            3_600_000,
        )
        .await
        .expect("start bridge");

    let base_ms = 1_700_000_000_000_u64;
    let events = vec![
        SessionEventV2::ready(
            session_id,
            "trace:v2",
            "bridge:cli",
            1,
            base_ms,
            serde_json::json!({"mode":"ready"}),
        ),
        SessionEventV2::state_snapshot(
            session_id,
            "trace:v2",
            "bridge:cli",
            2,
            base_ms + 1,
            serde_json::json!({"queue_depth":1}),
        ),
        SessionEventV2::assistant_delta(
            session_id,
            "trace:v2",
            "bridge:cli",
            3,
            base_ms + 2,
            "turn-1",
            "delta",
        ),
        SessionEventV2::tool_started(
            session_id,
            "trace:v2",
            "bridge:cli",
            4,
            base_ms + 3,
            "search",
            "call-1",
            serde_json::json!({"q":"autoloop"}),
        ),
        SessionEventV2::tool_completed(
            session_id,
            "trace:v2",
            "bridge:cli",
            5,
            base_ms + 4,
            "search",
            "call-1",
            serde_json::json!({"hits":3}),
            false,
        ),
    ];

    for event in &events {
        app.transport
            .emit_session_event_v2(event)
            .await
            .expect("emit event");
    }

    let replayed = app
        .transport
        .replay_session_events_v2(session_id)
        .await
        .expect("replay events");

    assert_eq!(replayed.len(), 5);
    assert_eq!(replayed[0].event_type, SessionEventType::Ready);
    assert_eq!(replayed[1].event_type, SessionEventType::StateSnapshot);
    assert_eq!(replayed[2].event_type, SessionEventType::AssistantDelta);
    assert_eq!(replayed[3].event_type, SessionEventType::ToolStarted);
    assert_eq!(replayed[4].event_type, SessionEventType::ToolCompleted);
    assert_eq!(
        replayed.iter().map(|event| event.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
}



