use std::{fs, path::Path};

use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::services::{ServiceCall, ServiceDomain},
};

fn mediated_call(session_id: &str) -> ServiceCall {
    ServiceCall {
        session_id: session_id.into(),
        trace_id: "trace:pq10-no-bypass".into(),
        service_domain: ServiceDomain::Tool,
        service_name: "read_file".into(),
        operation: "execute".into(),
        input: serde_json::json!({"name":"read_file","arguments":"{\"path\":\"README.md\"}"}),
        budget_scope: "default".into(),
        requested_at_ms: 0,
    }
}

#[tokio::test]
async fn pq10_no_bypass_gate_enforced_and_mediated_path_available() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq10-no-bypass";

    app.ensure_session_identity(
        session_id,
        "tenant:pq10",
        "principal:pq10",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("identity");

    let result = app
        .service_mediate(&mediated_call(session_id))
        .await
        .expect("mediate");
    let json: serde_json::Value = serde_json::from_str(&result).expect("json");
    assert_eq!(
        json.get("success").and_then(serde_json::Value::as_bool),
        Some(true),
        "mediated path should remain available"
    );

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut violations = Vec::new();
    scan_rs_files(&root, &mut |path, line_no, line| {
        let normalized = path.replace('\\', "/");
        let allow = normalized.ends_with("/providers/mod.rs")
            || normalized.ends_with("/runtime/mod.rs")
            || normalized.ends_with("/tools/mod.rs")
            || normalized.ends_with("/tools/cli_forge.rs");
        if allow {
            return;
        }

        let trimmed = line.trim();
        if trimmed.contains(".providers.chat(")
            || trimmed.contains(".providers.chat_with_policy(")
            || trimmed.contains(".tools.execute(")
        {
            violations.push(format!("{path}:{line_no}: {trimmed}"));
        }
    });

    assert!(
        violations.is_empty(),
        "no-bypass gate failed; direct provider/tool call detected:\n{}",
        violations.join("\n")
    );
}

fn scan_rs_files(dir: &Path, visitor: &mut dyn FnMut(String, usize, String)) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_rs_files(&path, visitor);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                if let Ok(content) = fs::read_to_string(&path) {
                    for (idx, line) in content.lines().enumerate() {
                        visitor(path.display().to_string(), idx + 1, line.to_string());
                    }
                }
            }
        }
    }
}



