use autoloop::{AutoLoopApp, config::AppConfig};
use autoloop::contracts::services::{
    SERVICE_GATE_TOKEN_FIELD, ServiceDomain, build_service_gate_token,
};

#[tokio::test]
async fn d11_parallel_compact_snapshot_task_mcp_acceptance_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq11-d11-acceptance";

    app.ensure_session_identity(
        session_id,
        "tenant:pq11",
        "principal:pq11",
        "policy:pq11",
        3_600_000,
    )
    .await
    .expect("bind identity");

    let prompt = "Please use parallel tool-calls when possible, then summarize with compact context.";
    let run = app.process_direct(session_id, prompt).await.expect("process direct");
    assert!(!run.trim().is_empty());

    let snapshot = app
        .session_named_snapshot(session_id, "d11-final")
        .await
        .expect("named snapshot");
    let snapshot_json: serde_json::Value = serde_json::from_str(&snapshot).expect("snapshot json");
    assert_eq!(snapshot_json["status"], "ok");

    let transcript = app
        .session_export_transcript(session_id)
        .await
        .expect("transcript");
    assert!(transcript.contains("Session Transcript"));

    let task = app
        .background_task_start_shell(
            session_id,
            "d11-tail",
            "Write-Output 'd11-tail-ok'",
            0,
        )
        .await
        .expect("start background task");
    let task_json: serde_json::Value = serde_json::from_str(&task).expect("task json");
    assert_eq!(task_json["status"], "started");

    for _ in 0..30 {
        let status = app
            .background_task_status(session_id, Some("d11-tail"))
            .await
            .expect("task status");
        let status_json: serde_json::Value = serde_json::from_str(&status).expect("status json");
        let state = status_json["tasks"][0]["status"]
            .as_str()
            .unwrap_or("running");
        if state != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let tail = app
        .background_task_logs(session_id, "d11-tail", 20)
        .await
        .expect("task logs");
    let tail_json: serde_json::Value = serde_json::from_str(&tail).expect("tail json");
    let logs = tail_json["logs"]
        .as_array()
        .expect("logs array")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(logs.contains("d11-tail-ok"));

    let mcp_upsert = app
        .service_mediate(&autoloop::contracts::services::ServiceCall {
            session_id: session_id.into(),
            trace_id: "trace:pq11:mcp-upsert".into(),
            service_domain: autoloop::contracts::services::ServiceDomain::Tool,
            service_name: "mcp".into(),
            operation: "mcp_upsert_connection".into(),
            input: serde_json::json!({
                "server": "local-mcp",
                "connected": true,
                SERVICE_GATE_TOKEN_FIELD: build_service_gate_token(
                    &session_id.into(),
                    &ServiceDomain::Tool,
                    1,
                ),
            }),
            budget_scope: "default".into(),
            requested_at_ms: 0,
        })
        .await
        .expect("mcp upsert");
    let upsert_json: serde_json::Value = serde_json::from_str(&mcp_upsert).expect("upsert json");
    assert_eq!(upsert_json["success"], true);

    let mcp_status = app
        .service_mediate(&autoloop::contracts::services::ServiceCall {
            session_id: session_id.into(),
            trace_id: "trace:pq11:mcp-status".into(),
            service_domain: autoloop::contracts::services::ServiceDomain::Tool,
            service_name: "mcp".into(),
            operation: "mcp_status".into(),
            input: serde_json::json!({
                SERVICE_GATE_TOKEN_FIELD: build_service_gate_token(
                    &session_id.into(),
                    &ServiceDomain::Tool,
                    2,
                ),
            }),
            budget_scope: "default".into(),
            requested_at_ms: 0,
        })
        .await
        .expect("mcp status");
    let status_json: serde_json::Value = serde_json::from_str(&mcp_status).expect("status json");
    assert_eq!(status_json["success"], true);
    assert!(
        status_json["output"]["total_servers"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
}



