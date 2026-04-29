use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::{capability::CapabilityIntent, ports::CapabilityIntentSelectorPort},
    plugins::compute_plugin_signature,
    security::capability_admission::CapabilityIntentSelector,
};

#[tokio::test]
async fn skill_plugin_router_integration_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let tenant = "tenant:pq";
    let principal = "principal:pq";
    let session_id = "pq-skill-plugin-router";

    app.ensure_session_identity(session_id, tenant, principal, "policy:pq", 3_600_000)
        .await
        .expect("identity");

    app.skill_register("skill:pq-router", "builtin://skill", "# skill\nrouter")
        .await
        .expect("skill register");

    let plugin_id = "plugin:pq-router";
    let source = "https://plugins.example.com/pq-router/v1";
    let sig = compute_plugin_signature(plugin_id, source, tenant, principal);
    app.plugin_install(
        plugin_id,
        &format!("{source}#sig={sig}"),
        principal,
        tenant,
        true,
    )
    .await
    .expect("plugin install");

    let selector = CapabilityIntentSelector::new(app.tools().clone());
    let candidates = selector
        .select_candidates(&CapabilityIntent {
            session_id: session_id.into(),
            objective: "route with skill and plugin aware policy".into(),
            required_tags: vec!["skill".into(), "plugin".into()],
            preferred_servers: vec![],
        })
        .await
        .expect("select");

    assert!(!candidates.is_empty(), "no capability candidates returned");
    assert!(
        candidates[0].score > 0.0,
        "candidate score should be positive"
    );
}



