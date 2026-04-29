use autoloop::observability::event_stream::{
    aggregate_session_view, append_event, list_session_events, replay_trace_events,
};
use autoloop::state_store_adapter::{StateStoreBackend, StateStore, StateStoreConfig};

#[tokio::test]
async fn sampled_trace_chain_is_replayable_and_ordered() {
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    });
    let session_id = "trace-session";
    let trace_a = "trace-a";
    let trace_b = "trace-b";

    for i in 0..5 {
        let _ = append_event(
            &db,
            "task_runs",
            trace_a,
            session_id,
            Some(format!("task-{i}")),
            Some("mcp::local-mcp::invoke".into()),
            "v1",
            serde_json::json!({"step": i}),
        )
        .await
        .expect("append trace a");
    }
    for i in 0..3 {
        let _ = append_event(
            &db,
            "state_transitions",
            trace_b,
            session_id,
            Some(format!("task-b-{i}")),
            None,
            "v1",
            serde_json::json!({"step": i}),
        )
        .await
        .expect("append trace b");
    }

    let events = list_session_events(&db, session_id)
        .await
        .expect("list events");
    assert!(events.len() >= 8);
    for window in events.windows(2) {
        assert!(window[0].created_at_ms <= window[1].created_at_ms);
    }

    let sampled_trace = events
        .iter()
        .find(|event| event.trace_id == trace_a)
        .map(|event| event.trace_id.clone())
        .expect("sample trace");
    let replay = replay_trace_events(&db, session_id, &sampled_trace)
        .await
        .expect("replay trace");
    assert!(!replay.is_empty());
    assert!(replay.iter().all(|event| event.trace_id == sampled_trace));

    let view = aggregate_session_view(&db, session_id)
        .await
        .expect("aggregate");
    assert_eq!(view.total_events, events.len());
    assert!(view.by_kind.get("task_runs").copied().unwrap_or(0) >= 5);
}




