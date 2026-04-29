use std::sync::Mutex;

use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::transport::SessionEventType,
    observability::query_plane::build_unified_query_view,
};

static FRONTEND_PERMISSION_ENV_LOCK: Mutex<()> = Mutex::new(());

struct ScopedFrontendPermissionMode {
    prior: Option<String>,
}

impl ScopedFrontendPermissionMode {
    fn set_ask() -> Self {
        let prior = std::env::var("AUTOLOOP_FRONTEND_PERMISSION_MODE").ok();
        // SAFETY: this test holds a process-wide mutex and restores the prior value in Drop.
        unsafe { std::env::set_var("AUTOLOOP_FRONTEND_PERMISSION_MODE", "ask") };
        Self { prior }
    }

}

impl Drop for ScopedFrontendPermissionMode {
    fn drop(&mut self) {
        if let Some(value) = self.prior.as_ref() {
            // SAFETY: this test holds a process-wide mutex and restores the prior value in Drop.
            unsafe { std::env::set_var("AUTOLOOP_FRONTEND_PERMISSION_MODE", value) };
        } else {
            // SAFETY: this test holds a process-wide mutex and restores the prior value in Drop.
            unsafe { std::env::remove_var("AUTOLOOP_FRONTEND_PERMISSION_MODE") };
        }
    }
}

#[tokio::test]
async fn frontend_cli_chat_stream_tool_permission_attach_e2e() {
    let _env_guard = FRONTEND_PERMISSION_ENV_LOCK
        .lock()
        .expect("frontend env lock");

    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq12-frontend-cli";
    let tenant_id = "tenant:pq12";

    app.ensure_session_identity(
        session_id,
        tenant_id,
        "principal:pq12",
        "policy:pq12",
        3_600_000,
    )
    .await
    .expect("bind identity");

    let attach = app
        .frontend_attach(
            session_id,
            "cli",
            None,
            Some("bridge:pq12"),
            Some(tenant_id),
            3_600_000,
        )
        .await
        .expect("frontend attach");
    let attach_json =
        serde_json::from_str::<serde_json::Value>(&attach).expect("parse attach response");
    assert_eq!(
        attach_json
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("attached")
    );

    let ask_trace = "trace:pq12:ask";
    {
        let _permission_mode = ScopedFrontendPermissionMode::set_ask();
        let ask_response = app
            .frontend_bridge_prompt(session_id, Some(ask_trace), "run with permission gate")
            .await
            .expect("frontend prompt ask");
        let ask_json =
            serde_json::from_str::<serde_json::Value>(&ask_response).expect("parse ask response");
        assert_eq!(
            ask_json.get("status").and_then(serde_json::Value::as_str),
            Some("requires_approval")
        );
        let request_id = ask_json
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            .expect("request id")
            .to_string();

        let reject_response = app
            .frontend_permission_decide(session_id, &request_id, "reject", Some("operator rejects"))
            .await
            .expect("permission reject");
        let reject_json =
            serde_json::from_str::<serde_json::Value>(&reject_response).expect("parse reject");
        assert_eq!(
            reject_json
                .get("status")
                .and_then(serde_json::Value::as_str),
            Some("rejected")
        );
    }
    let ok_trace = "trace:pq12:ok";
    let ok_response = app
        .frontend_bridge_prompt(session_id, Some(ok_trace), "hello stream tool events")
        .await
        .expect("frontend prompt ok");
    let ok_json = serde_json::from_str::<serde_json::Value>(&ok_response).expect("parse ok");
    assert_eq!(
        ok_json.get("status").and_then(serde_json::Value::as_str),
        Some("ok")
    );

    let events = app
        .transport
        .replay_session_events_v2(session_id)
        .await
        .expect("replay v2 events");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == SessionEventType::ToolStarted),
        "expected tool_started event"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == SessionEventType::ToolCompleted),
        "expected tool_completed event"
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == SessionEventType::AssistantDelta),
        "expected assistant_delta event"
    );

    let query_view = build_unified_query_view(&app.state_store(), session_id, Some(ask_trace))
        .await
        .expect("query plane");
    let query_events = query_view
        .events
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        query_events.iter().any(|event| {
            event
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "frontend.permission.requires_approval")
                || event
                    .get("value")
                    .and_then(|value| value.get("kind"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kind| kind == "cli.frontend.permission.request")
        }),
        "query plane should include permission-related cli/frontend event chain"
    );
}
