use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::services::{
        SERVICE_GATE_TOKEN_FIELD, ServiceCall, ServiceDomain, build_service_gate_token,
    },
};

fn call(session_id: &str, domain: ServiceDomain, operation: &str, input: serde_json::Value) -> ServiceCall {
    let mut secured_input = input;
    let session_id_typed: autoloop::contracts::ids::SessionId = session_id.into();
    let token = build_service_gate_token(&session_id_typed, &domain, 1);
    match &mut secured_input {
        serde_json::Value::Object(map) => {
            map.insert(
                SERVICE_GATE_TOKEN_FIELD.to_string(),
                serde_json::Value::String(token),
            );
        }
        _ => {
            secured_input = serde_json::json!({
                "payload": secured_input,
                SERVICE_GATE_TOKEN_FIELD: token
            });
        }
    }

    ServiceCall {
        session_id: session_id.into(),
        trace_id: format!("trace:pq-mediator:{operation}").into(),
        service_domain: domain,
        service_name: "mediator".into(),
        operation: operation.into(),
        input: secured_input,
        budget_scope: "default".into(),
        requested_at_ms: 0,
    }
}

#[tokio::test]
async fn mediator_no_bypass_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq-mediator";

    app.ensure_session_identity(
        session_id,
        "tenant:pq",
        "principal:pq",
        "policy:pq",
        3_600_000,
    )
    .await
    .expect("identity");

    let missing_gate = ServiceCall {
        session_id: session_id.into(),
        trace_id: "trace:pq-mediator:missing-gate".into(),
        service_domain: ServiceDomain::Tool,
        service_name: "read_file".into(),
        operation: "execute".into(),
        input: serde_json::json!({"name":"read_file","arguments":"{\"path\":\"README.md\"}"}),
        budget_scope: "default".into(),
        requested_at_ms: 0,
    };
    let no_gate = app.services.mediate_call(&missing_gate).await;
    assert!(no_gate.is_err(), "missing gate token should be hard-rejected");
    assert!(
        no_gate
            .err()
            .map(|err| err.to_string())
            .unwrap_or_default()
            .contains("gate token"),
        "missing gate token should mention gate token"
    );

    let provider = app
        .service_mediate(&call(
            session_id,
            ServiceDomain::Provider,
            "chat",
            serde_json::json!({
                "messages":[{"role":"user","content":"hello provider"}],
                "model":"gpt-4.1-mini"
            }),
        ))
        .await
        .expect("provider mediated");
    let provider_json: serde_json::Value = serde_json::from_str(&provider).expect("provider json");
    assert_eq!(
        provider_json
            .get("success")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let tool = app
        .service_mediate(&call(
            session_id,
            ServiceDomain::Tool,
            "execute",
            serde_json::json!({"name":"read_file","arguments":"{\"path\":\"README.md\"}"}),
        ))
        .await
        .expect("tool mediated");
    let tool_json: serde_json::Value = serde_json::from_str(&tool).expect("tool json");
    assert_eq!(
        tool_json.get("success").and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let mcp = app
        .service_mediate(&call(
            session_id,
            ServiceDomain::Tool,
            "mcp_status",
            serde_json::json!({}),
        ))
        .await
        .expect("mcp mediated");
    let json: serde_json::Value = serde_json::from_str(&mcp).expect("json");
    assert_eq!(
        json.get("success").and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let health = app.service_health().await.expect("health");
    let health_json: serde_json::Value = serde_json::from_str(&health).expect("health json");
    assert!(
        health_json
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false)
    );
}



