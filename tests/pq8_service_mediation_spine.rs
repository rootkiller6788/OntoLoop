use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::services::{
        SERVICE_GATE_TOKEN_FIELD, ServiceCall, ServiceDomain, build_service_gate_token,
    },
    plugins::compute_plugin_signature,
};

fn service_call(
    session_id: &str,
    trace_id: &str,
    domain: ServiceDomain,
    service_name: &str,
    operation: &str,
    input: serde_json::Value,
) -> ServiceCall {
    let mut secured_input = input;
    if domain.requires_gate_token() {
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
    }

    ServiceCall {
        session_id: session_id.into(),
        trace_id: trace_id.into(),
        service_domain: domain,
        service_name: service_name.to_string(),
        operation: operation.to_string(),
        input: secured_input,
        budget_scope: "default".into(),
        requested_at_ms: 0,
    }
}

#[tokio::test]
async fn service_mediation_spine_routes_core_domains() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq8-service";
    let tenant_id = "tenant:pq8";
    let principal = "principal:pq8";

    app.ensure_session_identity(session_id, tenant_id, principal, "policy:pq8", 3_600_000)
        .await
        .expect("identity");

    let provider = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:provider",
            ServiceDomain::Provider,
            "provider",
            "chat",
            serde_json::json!({
                "messages": [{"role":"user","content":"hello mediation"}],
                "model": "gpt-4.1-mini"
            }),
        ))
        .await
        .expect("provider mediation");
    assert!(provider.success);

    let tool = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:tool",
            ServiceDomain::Tool,
            "read_file",
            "execute",
            serde_json::json!({
                "name": "read_file",
                "arguments": "{\"path\":\"README.md\"}"
            }),
        ))
        .await
        .expect("tool mediation");
    assert!(tool.success);

    let mcp_status = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:mcp-status",
            ServiceDomain::Tool,
            "mcp",
            "mcp_status",
            serde_json::json!({}),
        ))
        .await
        .expect("mcp status mediation");
    assert!(mcp_status.success);
    assert!(
        mcp_status
            .output
            .get("total_tools")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            >= 1
    );

    let mcp_resource = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:mcp-resource",
            ServiceDomain::Tool,
            "mcp",
            "mcp_register_resource",
            serde_json::json!({
                "server": "local-mcp",
                "resource_id": "resource://catalog",
                "kind": "catalog",
                "capability_id": "mcp::local-mcp::invoke"
            }),
        ))
        .await
        .expect("mcp register resource mediation");
    assert!(mcp_resource.success);

    let plugin_id = "plugin:pq8";
    let source = "https://plugins.example.com/pq8/v1";
    let signature = compute_plugin_signature(plugin_id, source, tenant_id, principal);
    let plugin_install = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:plugin-install",
            ServiceDomain::Plugin,
            plugin_id,
            "install",
            serde_json::json!({
                "plugin_id": plugin_id,
                "source": format!("{source}#sig={signature}"),
                "requested_by": principal,
                "tenant_id": tenant_id,
                "verify_signature": true
            }),
        ))
        .await
        .expect("plugin install mediation");
    assert!(plugin_install.success);

    let plugin_verify = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:plugin-verify",
            ServiceDomain::Plugin,
            plugin_id,
            "verify",
            serde_json::json!({
                "plugin_id": plugin_id
            }),
        ))
        .await
        .expect("plugin verify mediation");
    assert!(plugin_verify.success);
    assert_eq!(
        plugin_verify
            .output
            .get("verified")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    let policy = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:policy",
            ServiceDomain::Policy,
            "policy",
            "status",
            serde_json::json!({}),
        ))
        .await
        .expect("policy mediation");
    assert!(policy.success);
    assert!(policy.output.get("runtime").is_some());

    let memory = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:memory",
            ServiceDomain::Memory,
            "memory",
            "build_context",
            serde_json::json!({
                "user_input": "How should anchor memory be stored?",
                "session_history": [{"role":"user","content":"store anchors in graph memory"}]
            }),
        ))
        .await
        .expect("memory mediation");
    assert!(memory.success);
    assert!(memory.output.get("context").is_some());

    let telemetry = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:telemetry",
            ServiceDomain::Telemetry,
            "telemetry",
            "snapshot",
            serde_json::json!({}),
        ))
        .await
        .expect("telemetry mediation");
    assert!(telemetry.success);

    let settings = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq8:settings",
            ServiceDomain::SettingsSync,
            "settings_sync",
            "upsert",
            serde_json::json!({
                "tenant_id": tenant_id,
                "scope": "provider-routing",
                "version": "v1",
                "payload": {"judge_model":"gpt-5"}
            }),
        ))
        .await
        .expect("settings sync mediation");
    assert!(settings.success);
    assert!(
        app.state_store()
            .get_knowledge("settings-sync:tenant:pq8:provider-routing:v1")
            .await
            .expect("settings key read")
            .is_some()
    );

    let health = app.services.health_snapshot().await.expect("health");
    assert!(health.iter().any(|item| item.service_name == "provider"));
    assert!(health.iter().any(|item| item.service_name == "tool"));
    assert!(health.iter().any(|item| item.service_name == "policy"));
    assert!(health.iter().any(|item| item.service_name == "plugin"));
    assert!(health.iter().any(|item| item.service_name == "mcp"));
    assert!(health.iter().any(|item| item.service_name == "memory"));
    assert!(health.iter().any(|item| item.service_name == "telemetry"));
    assert!(
        health
            .iter()
            .any(|item| item.service_name == "settings_sync")
    );
}




