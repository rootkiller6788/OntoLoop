use autoloop::{AutoLoopApp, config::AppConfig, runtime::McpDispatchRequest};
use autoloop_state_adapter::PermissionAction;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = AutoLoopApp::new(AppConfig::default());
    app.bootstrap().await?;

    app.state_store
        .grant_permissions(
            "scheduler",
            vec![
                PermissionAction::Read,
                PermissionAction::Write,
                PermissionAction::Dispatch,
            ],
        )
        .await?;

    let event = app
        .runtime
        .dispatch_mcp_event(
            &app.state_store,
            McpDispatchRequest {
                session_id: "example-session".into(),
                tool_name: "mcp::local-mcp::invoke".into(),
                payload: "{\"action\":\"sync-memory\"}".into(),
                actor_id: "scheduler".into(),
            },
        )
        .await?;

    let reply = app
        .process_direct(
            "example-session",
            "Summarize how StateStore stores scheduler state.",
        )
        .await?;

    app.state_store
        .upsert_agent_state(
            "example-session".into(),
            "Summarize how StateStore stores scheduler state.".into(),
            Some(reply.clone()),
        )
        .await?;

    app.state_store
        .upsert_knowledge("scheduler:summary".into(), reply, "example".into())
        .await?;
    app.state_store
        .update_schedule_status(event.id, "completed")
        .await?;

    println!("StateStore SDK example completed for event {}", event.id);
    Ok(())
}

