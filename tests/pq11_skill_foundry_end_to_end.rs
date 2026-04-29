use std::path::Path;

use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::services::{ServiceCall, ServiceDomain},
};

fn service_call(
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
        budget_scope: "default".into(),
        requested_at_ms: 0,
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[tokio::test]
async fn skill_foundry_end_to_end_pipeline_with_shadow_runtime() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq11-foundry-e2e";
    let tenant = "tenant:pq11";
    let principal = "principal:pq11";
    let builder = format!("pq11-e2e-{}", std::process::id());
    let trace = "trace:pq11:foundry";

    app.ensure_session_identity(session_id, tenant, principal, "policy:pq11", 3_600_000)
        .await
        .expect("identity");

    let base_input = serde_json::json!({
        "builder": builder,
        "source": "powershell script compile and validate",
        "markdown": "JSON output",
        "requested_by": principal,
    });

    let intake = app
        .services
        .mediate_call(&service_call(session_id, trace, "intake", base_input.clone()))
        .await
        .expect("intake");
    assert!(intake.success);
    assert!(intake.output.get("task_name").is_some());

    let extract = app
        .services
        .mediate_call(&service_call(session_id, trace, "extract", base_input.clone()))
        .await
        .expect("extract");
    assert!(extract.success);
    assert!(extract.output.get("extraction").is_some());

    let route = app
        .services
        .mediate_call(&service_call(session_id, trace, "route", base_input.clone()))
        .await
        .expect("route");
    assert!(route.success);
    assert!(route.output.get("route").is_some());

    // Shadow runtime behavior: route stage produces suggestion/decision, but does not install skill yet.
    assert!(app.state_store()
        .get_knowledge(&format!("skills:manifest:{builder}"))
        .await
        .expect("read manifest before install")
        .is_none());

    let build = app
        .services
        .mediate_call(&service_call(session_id, trace, "build", base_input.clone()))
        .await
        .expect("build");
    assert!(build.success);
    let artifact_path = build
        .output
        .get("build")
        .and_then(|v| v.get("artifact_path"))
        .and_then(serde_json::Value::as_str)
        .expect("artifact path");
    assert!(Path::new(artifact_path).exists());

    let validate = app
        .services
        .mediate_call(&service_call(session_id, trace, "validate", base_input.clone()))
        .await
        .expect("validate");
    assert!(validate.success);
    assert!(validate
        .output
        .get("validation")
        .and_then(|v| v.get("checks"))
        .and_then(serde_json::Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false));

    let package = app
        .services
        .mediate_call(&service_call(session_id, trace, "package", base_input.clone()))
        .await
        .expect("package");
    assert!(package.success);
    assert_eq!(
        package
            .output
            .get("metadata")
            .and_then(|m| m.get("packaged_by"))
            .and_then(serde_json::Value::as_str),
        Some("skill_foundry")
    );

    let install = app
        .services
        .mediate_call(&service_call(session_id, trace, "install", base_input))
        .await
        .expect("install");
    assert!(install.success);
    assert_eq!(
        install
            .output
            .get("install")
            .and_then(|v| v.get("skill_id"))
            .and_then(serde_json::Value::as_str),
        Some(builder.as_str())
    );

    let latest_feedback = app.state_store()
        .get_knowledge(&format!("foundry:feedback:{session_id}:{trace}:latest"))
        .await
        .expect("latest feedback read");
    assert!(latest_feedback.is_some());

    let foundry_evidence = app.state_store()
        .list_knowledge_by_prefix(&format!("evidence:foundry:{session_id}:"))
        .await
        .expect("foundry evidence records");
    assert!(!foundry_evidence.is_empty());

    let _ = std::fs::remove_dir_all(Path::new("skills").join(slugify(&builder)));
}





