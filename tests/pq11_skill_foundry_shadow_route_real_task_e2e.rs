use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::{
        services::{ServiceCall, ServiceDomain},
        skill_foundry::SkillFoundryLayer,
    },
};

fn foundry_call(
    session_id: &str,
    trace_id: &str,
    operation: &str,
    input: serde_json::Value,
) -> ServiceCall {
    ServiceCall {
        session_id: session_id.into(),
        trace_id: trace_id.into(),
        service_domain: ServiceDomain::SkillFoundry,
        service_name: "skill_foundry".to_string(),
        operation: operation.to_string(),
        input,
        budget_scope: "skill_foundry".into(),
        requested_at_ms: 0,
    }
}

#[tokio::test]
async fn skill_foundry_routes_real_tasks_in_shadow_without_mutating_main_path() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq11-foundry-shadow-route-real-task";
    let tenant = "tenant:pq11";
    let principal = "principal:pq11";
    app.ensure_session_identity(session_id, tenant, principal, "policy:pq11", 3_600_000)
        .await
        .expect("identity");

    let cases = vec![
        (
            "pq11-s1-billing-style-guide",
            "整理 billing 页面视觉规范和检查清单，输出 markdown 模板",
            SkillFoundryLayer::S1PromptOnly,
        ),
        (
            "pq11-s2-batch-report-script",
            "run_script powershell 生成日报并 validate JSON 结构",
            SkillFoundryLayer::S2PromptScripts,
        ),
        (
            "pq11-s3-exchange-sync-api",
            "调用外部交易所 API 同步账户状态并 webhook 回调",
            SkillFoundryLayer::S3PromptMcp,
        ),
    ];

    for (idx, (builder, source, expected_layer)) in cases.iter().enumerate() {
        let trace_id = format!("trace:pq11:foundry-shadow:{idx}");
        let call = foundry_call(
            session_id,
            &trace_id,
            "route",
            serde_json::json!({
                "builder": builder,
                "source": source,
                "markdown": "json artifact",
                "requested_by": principal,
            }),
        );
        let result = app.services.mediate_call(&call).await.expect("foundry route");
        assert!(result.success, "route should succeed for {builder}");

        let route = result
            .output
            .get("route")
            .cloned()
            .expect("route output exists");
        let route_suggested = result
            .output
            .get("route_suggested")
            .cloned()
            .expect("route_suggested output exists");

        let selected_layer = serde_json::from_value::<autoloop::contracts::skill_foundry::RouteDecision>(
            route.clone(),
        )
        .expect("route decode")
        .selected_layer;
        assert_eq!(
            selected_layer, *expected_layer,
            "real task should route to expected layer for {builder}"
        );

        let reasons = route
            .get("reasons")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            !reasons.is_empty(),
            "route explainability requires non-empty reasons for {builder}"
        );
        assert!(
            route
                .get("confidence")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0)
                > 0.0,
            "route confidence should be present for {builder}"
        );
        assert!(
            route.get("risk_level").and_then(serde_json::Value::as_str).is_some(),
            "route risk level should be present for {builder}"
        );

        // Shadow behavior: current effective route equals suggested route when no approved layer override exists.
        assert_eq!(route, route_suggested, "shadow route should not mutate selection for {builder}");

        // No approved promotion => no layer state update and no skill install side effect.
        let layer_state = app
            .state_store()
            .get_knowledge(&format!("foundry:skill-layer:{builder}:latest"))
            .await
            .expect("load layer state");
        assert!(
            layer_state.is_none(),
            "unapproved route must not mutate layer state for {builder}"
        );
        let installed = app
            .state_store()
            .get_knowledge(&format!("skills:manifest:{builder}"))
            .await
            .expect("load manifest");
        assert!(
            installed.is_none(),
            "unapproved route must not install skill for {builder}"
        );
    }
}
