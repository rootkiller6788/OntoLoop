use autoloop::{AutoLoopApp, config::AppConfig, plugins::compute_plugin_signature};

#[tokio::test]
async fn plugin_lifecycle_signed_end_to_end() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq7-plugin-session";
    let tenant_id = "tenant:pq7";
    let operator = "principal:pq7";
    let plugin_id = "plugin:pq7";

    app.ensure_session_identity(session_id, tenant_id, operator, "policy:pq7", 3_600_000)
        .await
        .expect("identity");

    let base_source_v1 = "https://plugins.example.com/pq7/v1";
    let sig_v1 = compute_plugin_signature(plugin_id, base_source_v1, tenant_id, operator);
    let source_v1 = format!("{base_source_v1}#sig={sig_v1}");

    let install = app
        .plugin_install(plugin_id, &source_v1, operator, tenant_id, true)
        .await
        .expect("install");
    let install_json: serde_json::Value = serde_json::from_str(&install).expect("install json");
    assert_eq!(
        install_json
            .get("plugin_id")
            .and_then(serde_json::Value::as_str),
        Some(plugin_id)
    );
    let installed_version = install_json
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(matches!(installed_version, "v1" | "v2"));

    let verify = app.plugin_verify(plugin_id).await.expect("verify");
    let verify_json: serde_json::Value = serde_json::from_str(&verify).expect("verify json");
    assert_eq!(
        verify_json
            .get("verified")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    app.plugin_enable(plugin_id, operator, "enable for traffic")
        .await
        .expect("enable");
    app.plugin_disable(plugin_id, operator, "temporary disable")
        .await
        .expect("disable");
    app.plugin_enable(plugin_id, operator, "enable after validation")
        .await
        .expect("re-enable");

    app.plugin_rollout(plugin_id, "shadow", Some(0), operator, Some("shadow soak"))
        .await
        .expect("shadow rollout");
    app.plugin_rollout(
        plugin_id,
        "canary",
        Some(15),
        operator,
        Some("canary traffic"),
    )
    .await
    .expect("canary rollout");
    app.plugin_rollout(plugin_id, "full", Some(100), operator, Some("full traffic"))
        .await
        .expect("full rollout");

    let status_after_full = app
        .plugin_status(plugin_id)
        .await
        .expect("status full rollout");
    let status_full_json: serde_json::Value =
        serde_json::from_str(&status_after_full).expect("status full json");
    assert_eq!(
        status_full_json
            .get("rollout_mode")
            .and_then(serde_json::Value::as_str),
        Some("full")
    );
    assert_eq!(
        status_full_json
            .get("rollout_traffic_percent")
            .and_then(serde_json::Value::as_u64),
        Some(100)
    );

    let base_source_v2 = "https://plugins.example.com/pq7/v2";
    app.plugin_update(plugin_id, Some(base_source_v2), operator)
        .await
        .expect("update");

    let status_after_update = app.plugin_status(plugin_id).await.expect("status update");
    let status_update_json: serde_json::Value =
        serde_json::from_str(&status_after_update).expect("status update json");
    let updated_version = status_update_json
        .get("current_manifest")
        .and_then(|v| v.get("version"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(!updated_version.is_empty());

    app.plugin_quick_rollback(plugin_id, operator)
        .await
        .expect("quick rollback");
    let status_after_rollback = app.plugin_status(plugin_id).await.expect("status rollback");
    let status_rollback_json: serde_json::Value =
        serde_json::from_str(&status_after_rollback).expect("status rollback json");
    assert_eq!(
        status_rollback_json
            .get("state")
            .and_then(serde_json::Value::as_str),
        Some("rolled_back")
    );
    let rollback_version = status_rollback_json
        .get("current_manifest")
        .and_then(|v| v.get("version"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(matches!(rollback_version, "v1" | "v2"));

    let hook_records = app.state_store()
        .list_knowledge_by_prefix(&format!("plugin:hook:{plugin_id}:"))
        .await
        .expect("hook records");
    assert!(
        !hook_records.is_empty(),
        "expected lifecycle hook records to be persisted"
    );

    let list = app.plugin_list().await.expect("list");
    let list_json: serde_json::Value = serde_json::from_str(&list).expect("list json");
    let contains_plugin = list_json
        .as_array()
        .map(|items| {
            items.iter().any(|item| {
                item.get("plugin_id").and_then(serde_json::Value::as_str) == Some(plugin_id)
            })
        })
        .unwrap_or(false);
    assert!(contains_plugin, "expected plugin in lifecycle list");
}




