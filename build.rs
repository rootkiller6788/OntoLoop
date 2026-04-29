use std::{collections::BTreeSet, fs, path::Path};

fn main() {
    if std::env::var("AUTOLOOP_SKIP_COMPILE_NOBYPASS_SCAN")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        println!("cargo:warning=compile no-bypass scan skipped by AUTOLOOP_SKIP_COMPILE_NOBYPASS_SCAN=1");
        return;
    }

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing");
    let src_root = Path::new(&manifest_dir).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);

    let backend_forbidden = [
        ".state_store.",
        ".state_store()",
        ".providers.",
        ".providers()",
        ".tools.",
        ".tools()",
    ];
    let backend_allow_prefixes: BTreeSet<&str> = BTreeSet::from([
        "src/lib.rs",
        "src/main.rs",
        "src/command_dispatch.rs",
        "src/dashboard_server.rs",
        "src/agent/mod.rs",
        "src/cli_runtime/command_registry.rs",
        "src/orchestration/mod.rs",
        "src/orchestration/knowledge_context.rs",
        "src/orchestration/org_context.rs",
        "src/plugins/lifecycle.rs",
        "src/runtime/mod.rs",
        "src/security/capability_admission.rs",
        "src/services/mediator.rs",
        "src/services/relation_facade.rs",
        "src/tools/mod.rs",
        "src/providers/mod.rs",
    ]);
    let mcp_allow_prefixes: BTreeSet<&str> = BTreeSet::from([
        "src/lib.rs",
        "src/services/mod.rs",
        "src/services/mediator.rs",
        "src/services/mcp_manager.rs",
    ]);

    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", manifest_dir.replace('\\', "/")))
            .unwrap_or(&file)
            .to_string();
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };

        if !backend_allow_prefixes.contains(rel.as_str()) {
            for (idx, line) in body.lines().enumerate() {
                if backend_forbidden.iter().any(|token| line.contains(token)) {
                    violations.push(format!(
                        "{rel}:{}: direct backend access forbidden (`{}`)",
                        idx + 1,
                        line.trim()
                    ));
                }
            }
        }

        if !mcp_allow_prefixes.contains(rel.as_str()) {
            for (idx, line) in body.lines().enumerate() {
                if line.contains("McpManager")
                    || line.contains("services::mcp_manager")
                    || line.contains("::mcp_manager::")
                {
                    violations.push(format!(
                        "{rel}:{}: mcp manager bypass forbidden (`{}`)",
                        idx + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    if !violations.is_empty() {
        panic!(
            "compile-time no-bypass gate failed:\n{}",
            violations.join("\n")
        );
    }
}

fn collect_rs_files(dir: &Path, out: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs_files(&path, out);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                if let Some(value) = path.to_str() {
                    out.push(value.replace('\\', "/"));
                }
            }
        }
    }
}
