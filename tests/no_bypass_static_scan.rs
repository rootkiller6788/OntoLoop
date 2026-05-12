use std::{collections::BTreeSet, fs, path::Path};

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

#[test]
fn no_bypass_static_scan_business_layers() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let roots = [
        repo.join("src/evolution_os"),
        repo.join("src/query_engine"),
        repo.join("src/skills/foundry"),
        repo.join("src/gitmemory_core"),
        repo.join("src/memory"),
    ];
    let forbidden = [
        ".state_store.",
        ".state_store()",
        ".providers.",
        ".providers()",
        ".tools.",
        ".tools()",
    ];

    let mut violations = Vec::new();
    for root in roots {
        let mut files = Vec::new();
        collect_rs_files(&root, &mut files);
        for file in files {
            let Ok(body) = fs::read_to_string(&file) else {
                continue;
            };
            for (idx, line) in body.lines().enumerate() {
                if forbidden.iter().any(|token| line.contains(token)) {
                    violations.push(format!("{file}:{}:{}", idx + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "no-bypass static scan failed for business layers:\n{}",
        violations.join("\n")
    );
}

#[test]
fn direct_backend_access_only_in_boundary_modules() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src");
    let forbidden = [
        // memory domain (state store)
        ".state_store.",
        ".state_store()",
        // provider domain
        ".providers.",
        ".providers()",
        // tool domain
        ".tools.",
        ".tools()",
        // mcp domain direct manager handles
        ".mcp_manager.",
        ".mcp_manager()",
        "services::mcp_manager",
        "::mcp_manager::",
        "McpManager",
    ];
    let allow_prefixes: BTreeSet<&str> = BTreeSet::from([
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
        "src/services/mod.rs",
        "src/services/mediator.rs",
        "src/services/mcp_manager.rs",
        "src/services/relation_facade.rs",
        "src/tools/mod.rs",
        "src/providers/mod.rs",
    ]);

    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);
    let mut violations = Vec::new();

    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/")))
            .unwrap_or(&file);
        let allowed = allow_prefixes.contains(rel);
        if allowed {
            continue;
        }

        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            if forbidden.iter().any(|token| line.contains(token)) {
                let domain = if line.contains(".providers.") || line.contains(".providers()") {
                    "provider"
                } else if line.contains(".tools.") || line.contains(".tools()") {
                    "tool"
                } else if line.contains("mcp_manager") || line.contains("McpManager") {
                    "mcp"
                } else {
                    "memory"
                };
                violations.push(format!("{rel}:{}:[{domain}] {}", idx + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "direct backend access leaked outside boundary modules (provider/tool/memory/mcp):\n{}",
        violations.join("\n")
    );
}

#[test]
fn no_bypass_ci_gate_provider_tool_memory_mcp() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);

    let allow_prefixes: BTreeSet<&str> = BTreeSet::from([
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
        "src/services/mod.rs",
        "src/services/mediator.rs",
        "src/services/relation_facade.rs",
        "src/services/mcp_manager.rs",
        "src/tools/mod.rs",
        "src/providers/mod.rs",
    ]);

    let provider_tokens = [".providers.", ".providers()"];
    let tool_tokens = [".tools.", ".tools()"];
    let memory_tokens = [".state_store.", ".state_store()"];
    let mcp_tokens = [
        ".mcp_manager.",
        ".mcp_manager()",
        "services::mcp_manager",
        "::mcp_manager::",
        "McpManager",
    ];

    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/")))
            .unwrap_or(&file);
        if allow_prefixes.contains(rel) {
            continue;
        }
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            let domain = if provider_tokens.iter().any(|t| line.contains(t)) {
                Some("provider")
            } else if tool_tokens.iter().any(|t| line.contains(t)) {
                Some("tool")
            } else if memory_tokens.iter().any(|t| line.contains(t)) {
                Some("memory")
            } else if mcp_tokens.iter().any(|t| line.contains(t)) {
                Some("mcp")
            } else {
                None
            };
            if let Some(domain) = domain {
                violations.push(format!("{rel}:{}:[{domain}] {}", idx + 1, line.trim()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "no-bypass CI gate failed (provider/tool/memory/mcp):\n{}",
        violations.join("\n")
    );
}

#[test]
fn autoloop_app_backend_fields_are_not_public() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib_rs = repo.join("src/lib.rs");
    let body = fs::read_to_string(lib_rs).expect("read src/lib.rs");
    let forbidden_public_fields = ["pub state_store:", "pub providers:", "pub tools:"];
    let leaked = forbidden_public_fields
        .iter()
        .filter(|token| body.contains(**token))
        .copied()
        .collect::<Vec<_>>();
    assert!(
        leaked.is_empty(),
        "AutoLoopApp leaked public backend fields: {:?}",
        leaked
    );
}

#[test]
fn mcp_manager_only_used_in_service_boundary() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src");
    let allow_prefixes: BTreeSet<&str> = BTreeSet::from([
        "src/lib.rs",
        "src/services/mod.rs",
        "src/services/mediator.rs",
        "src/services/mcp_manager.rs",
    ]);
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);
    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/")))
            .unwrap_or(&file);
        if allow_prefixes.contains(rel) {
            continue;
        }
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            if line.contains("McpManager")
                || line.contains("services::mcp_manager")
                || line.contains("::mcp_manager::")
            {
                violations.push(format!("{rel}:{}:{}", idx + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "mcp manager bypass leaked outside services boundary:\n{}",
        violations.join("\n")
    );
}

#[test]
fn agent_process_message_only_used_in_harness_boundaries() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src");
    let allow_prefixes: BTreeSet<&str> = BTreeSet::from([
        "src/lib.rs",
        "src/agent/mod.rs",
        "src/services/background_tasks.rs",
    ]);
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);
    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/")))
            .unwrap_or(&file);
        if allow_prefixes.contains(rel) {
            continue;
        }
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            if line.contains(".process_message(") {
                violations.push(format!("{rel}:{}:{}", idx + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "agent process_message bypass leaked outside harness boundaries:\n{}",
        violations.join("\n")
    );
}

#[test]
fn background_agent_task_start_only_allowed_in_app_boundary() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src");
    let allow_prefixes: BTreeSet<&str> = BTreeSet::from([
        "src/lib.rs",
        "src/services/background_tasks.rs",
    ]);
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);
    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/")))
            .unwrap_or(&file);
        if allow_prefixes.contains(rel) {
            continue;
        }
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            if line.contains(".start_agent_task(") {
                violations.push(format!("{rel}:{}:{}", idx + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "background agent start bypass leaked outside app boundary:\n{}",
        violations.join("\n")
    );
}

#[test]
fn session_id_filesystem_paths_must_be_sanitized() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);

    let suspicious_patterns = [
        r#"format!("{session_id}.json")"#,
        r#"format!("{session}.json")"#,
        r#"format!("{session_id}.log")"#,
        r#"format!("{session}.log")"#,
        r#".join(session_id)"#,
        r#".join(session)"#,
        r#"\\{session_id}.json"#,
        r#"\\{session}.json"#,
        r#"/{session_id}.json"#,
        r#"/{session}.json"#,
    ];
    let sanitizer_markers = [
        "sanitize_filesystem_component(",
        "sanitize_session_id(",
        "sanitize(session_id)",
        "sanitize(session)",
    ];

    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/")))
            .unwrap_or(&file);
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            let suspicious = suspicious_patterns.iter().any(|pattern| line.contains(pattern));
            if !suspicious {
                continue;
            }
            let has_sanitizer = sanitizer_markers
                .iter()
                .any(|marker| line.contains(marker));
            if has_sanitizer {
                continue;
            }
            violations.push(format!("{rel}:{}:{}", idx + 1, line.trim()));
        }
    }

    assert!(
        violations.is_empty(),
        "unsanitized session-based filesystem path detected:\n{}",
        violations.join("\n")
    );
}

#[test]
fn lease_write_path_is_only_via_branch_lease_manager() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src/org_kernel");
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);
    let allow_prefixes: BTreeSet<&str> = BTreeSet::from(["src/org_kernel/coordinator.rs"]);
    let mut violations = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(&format!("{}/", env!("CARGO_MANIFEST_DIR").replace('\\', "/")))
            .unwrap_or(&file);
        if allow_prefixes.contains(rel) {
            continue;
        }
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, line) in body.lines().enumerate() {
            if line.contains("fs::write(") || line.contains("std::fs::write(") {
                violations.push(format!("{rel}:{}:{}", idx + 1, line.trim()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "org_kernel lease write no-bypass violated (raw fs writes outside coordinator):\n{}",
        violations.join("\n")
    );
}




