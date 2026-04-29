use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::services::{ServiceCall, ServiceDomain},
};

fn service_call(
    session_id: &str,
    trace_id: &str,
    operation: &str,
    builder: &str,
    source: &str,
) -> ServiceCall {
    ServiceCall {
        session_id: session_id.into(),
        trace_id: trace_id.into(),
        service_domain: ServiceDomain::SkillFoundry,
        service_name: "skill_foundry".to_string(),
        operation: operation.to_string(),
        input: serde_json::json!({
            "builder": builder,
            "source": source,
            "hint_id": source,
            "markdown": "json artifact",
            "requested_by": "principal:pq11",
        }),
        budget_scope: "skill_foundry".into(),
        requested_at_ms: 0,
    }
}

#[tokio::test]
async fn approve_promotion_updates_layer_and_affects_next_route() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq11-foundry-approval";
    let builder = format!("pq11-approval-{}", std::process::id());

    app.ensure_session_identity(
        session_id,
        "tenant:pq11",
        "principal:pq11",
        "policy:pq11",
        3_600_000,
    )
    .await
    .expect("identity");

    let _ = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq11:seed-route",
            "route",
            &builder,
            "method-steps",
        ))
        .await
        .expect("seed route");

    for idx in 0..3 {
        let _ = app
            .services
            .mediate_call(&service_call(
                session_id,
                &format!("trace:pq11:miss:{idx}"),
                "run",
                &builder,
                "method-steps",
            ))
            .await
            .expect("force miss");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let route_with_gate = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq11:route-with-gate",
            "route",
            &builder,
            "method-steps",
        ))
        .await
        .expect("route with gate");

    let hint_id_from_output = route_with_gate
        .output
        .get("promotion_gate")
        .and_then(|gate| gate.get("hint"))
        .and_then(|hint| hint.get("hint_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    let hint_id = if let Some(hint_id) = hint_id_from_output {
        hint_id
    } else {
        let pending_list = app.state_store()
            .list_knowledge_by_prefix(&format!(
                "foundry:promotion:pending:{}:{}:",
                session_id, builder
            ))
            .await
            .expect("pending list read");
        assert!(!pending_list.is_empty(), "promotion hint id available");
        let pending_raw = pending_list
            .iter()
            .find(|record| !record.key.ends_with(":latest"))
            .or_else(|| pending_list.first())
            .expect("pending record");
        let pending_hint: serde_json::Value =
            serde_json::from_str(&pending_raw.value).expect("pending json parse");
        pending_hint
            .get("hint_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .expect("promotion hint id available")
    };

    let approval = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq11:approve",
            "approve_promotion",
            &builder,
            &hint_id,
        ))
        .await
        .expect("approve promotion");
    assert!(approval.success);
    assert_eq!(
        approval
            .output
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("approved")
    );

    let routed = app
        .services
        .mediate_call(&service_call(
            session_id,
            "trace:pq11:route-after-approve",
            "route",
            &builder,
            "method-steps",
        ))
        .await
        .expect("route after approve");
    assert!(routed.success);

    let selected_layer = routed
        .output
        .get("route")
        .and_then(|route| route.get("selected_layer"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(selected_layer, Some("s2_prompt_scripts"));
}









