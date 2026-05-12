use std::{collections::BTreeSet, fs, path::Path};

use autoloop::{
    config::PolicyMode as RuntimePolicyMode,
    contracts::{
        capability::CapabilityIntent,
        services::{SERVICE_GATE_TOKEN_FIELD, ServiceCall, ServiceDomain},
        types::ExecutionIdentity,
    },
    providers::ProviderRegistry,
    security::capability_admission::CapabilityAdmissionEngine,
    state_store_adapter::{
        PermissionAction, PolicyBinding, Principal, RoleBinding, SessionLease, StateStore,
        StateStoreBackend, StateStoreConfig, Tenant,
    },
    tools::{ForgedMcpToolManifest, ToolRegistry},
    AutoLoopApp,
};
use autoloop::runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage};

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

fn in_memory_db() -> StateStore {
    StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    })
}

fn current_time_ms() -> u64 {
    autoloop::orchestration::current_time_ms()
}

fn high_risk_manifest() -> ForgedMcpToolManifest {
    serde_json::from_value(serde_json::json!({
        "capability_id": "cap-high-risk:v1",
        "registered_tool_name": "mcp::high-risk::invoke",
        "delegate_tool_name": "mcp::local-mcp::invoke",
        "server": "local-mcp",
        "capability_name": "high-risk",
        "purpose": "d10-d11 test",
        "executable": "echo",
        "command_template": "echo {{payload}}",
        "payload_template": {"payload": "{{payload}}"},
        "output_mode": "json",
        "help_text": "help",
        "skill_markdown": "skill",
        "examples": ["ex"],
        "version": 1,
        "lineage_key": "cap-high-risk",
        "status": "active",
        "approval_status": "verified",
        "health_score": 0.95,
        "scope": "session",
        "tags": ["policy", "test"],
        "risk": "high",
        "requested_by": "tester",
        "created_at_ms": 1,
        "updated_at_ms": 1,
        "approved_at_ms": 1,
        "rollback_to_version": null,
        "trust_status": "trusted",
        "trust_findings": [],
        "artifact": {"artifact_id":"a","digest_sha256":"d","source_uri":"u","build_epoch":1},
        "signature": {"signer":"autoloop","algorithm":"deterministic_v1","signed_payload_hash":"x","signature":"y","signed_at_ms":1},
        "provenance": {"source_repo":"r","source_ref":"ref","builder":"b","generated_by":"g"},
        "sbom": {"components": []},
        "trust_policy": {"required_signers":["autoloop"],"blocked_dependencies":[],"min_provenance_ref_len":1}
    }))
    .expect("manifest json")
}

async fn seed_identity(db: &StateStore, session_id: &str, identity: &ExecutionIdentity) {
    let now = current_time_ms();
    db.upsert_tenant(Tenant {
        tenant_id: identity.tenant_id.clone(),
        name: identity.tenant_id.clone(),
        status: "active".into(),
        created_at_ms: now,
    })
    .await
    .expect("tenant");
    db.upsert_principal(Principal {
        principal_id: identity.principal_id.clone(),
        tenant_id: identity.tenant_id.clone(),
        principal_type: "user".into(),
        status: "active".into(),
        created_at_ms: now,
    })
    .await
    .expect("principal");
    db.upsert_role_binding(RoleBinding {
        tenant_id: identity.tenant_id.clone(),
        principal_id: identity.principal_id.clone(),
        role: "operator".into(),
        updated_at_ms: now,
    })
    .await
    .expect("role");
    db.upsert_policy_binding(PolicyBinding {
        policy_id: identity.policy_id.clone(),
        tenant_id: identity.tenant_id.clone(),
        role: "operator".into(),
        allowed_actions: vec![
            PermissionAction::Read,
            PermissionAction::Write,
            PermissionAction::Dispatch,
        ],
        capability_prefixes: vec!["mcp::".into(), "provider:".into(), "cli::".into()],
        max_memory_mb: 2048,
        max_tokens: 32000,
        updated_at_ms: now,
    })
    .await
    .expect("policy");
    db.upsert_session_lease(SessionLease {
        lease_token: identity.lease_token.clone(),
        session_id: session_id.to_string(),
        tenant_id: identity.tenant_id.clone(),
        principal_id: identity.principal_id.clone(),
        policy_id: identity.policy_id.clone(),
        expires_at_ms: now + 120_000,
        issued_at_ms: now,
    })
    .await
    .expect("lease");
}

#[test]
fn no_bypass_static_scan_fails_on_direct_store_call() {
    let line = "let _ = self.state_store().get_knowledge(key).await?;";
    let forbidden = [".state_store.", ".state_store()", ".providers.", ".providers()", ".tools.", ".tools()"];
    assert!(forbidden.iter().any(|item| line.contains(item)));

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_root = repo.join("src");
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
        "src/services/mediator.rs",
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
        if allow_prefixes.contains(rel) {
            continue;
        }
        let Ok(body) = fs::read_to_string(&file) else {
            continue;
        };
        for (idx, text) in body.lines().enumerate() {
            if forbidden.iter().any(|token| text.contains(token)) {
                violations.push(format!("{rel}:{}:{}", idx + 1, text.trim()));
            }
        }
    }
    assert!(violations.is_empty(), "found no-bypass violations:\n{}", violations.join("\n"));
}

#[test]
fn no_bypass_compile_gate_blocks_direct_provider_access() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib_rs = repo.join("src/lib.rs");
    let body = fs::read_to_string(lib_rs).expect("read src/lib.rs");
    assert!(!body.contains("pub providers:"), "providers must not be public");
    assert!(!body.contains("pub tools:"), "tools must not be public");
    assert!(!body.contains("pub state_store:"), "state_store must not be public");
}

#[tokio::test]
async fn no_bypass_runtime_gate_rejects_missing_token() {
    let app = AutoLoopApp::new(autoloop::config::AppConfig::default());
    let session_id = "d10-no-bypass-runtime";
    app.ensure_session_identity(session_id, "tenant:d10", "principal:d10", "policy:d10", 60_000)
        .await
        .expect("identity");

    let call = ServiceCall {
        session_id: session_id.into(),
        trace_id: "trace:d10:no-token".into(),
        service_domain: ServiceDomain::Tool,
        service_name: "read_file".into(),
        operation: "execute".into(),
        input: serde_json::json!({"name":"read_file","arguments":"{\"path\":\"README.md\"}"}),
        budget_scope: "default".into(),
        requested_at_ms: 0,
    };
    let result = app.services.mediate_call(&call).await;
    assert!(result.is_err(), "missing gate token must be denied");
    let reason = result.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(reason.contains("no_bypass_gate_missing_token"));
    assert!(!reason.contains(SERVICE_GATE_TOKEN_FIELD) || reason.contains("gate token"));
}

#[tokio::test]
async fn evidence_worm_detects_tamper_and_breaks_chain() {
    let db = in_memory_db();
    let session_id = "d11-worm-session";
    let trace_id = "trace-d11-worm";
    let first_key = EvidenceLedgerWriter::append_stage(
        &db,
        session_id,
        trace_id,
        EvidenceStage::Admission,
        serde_json::json!({"status":"admitted"}),
        None,
    )
    .await
    .expect("append first");
    let tamper = db
        .upsert_json_knowledge(first_key, &serde_json::json!({"tampered":true}), "tamper")
        .await;
    assert!(tamper.is_err(), "worm append-only must block tamper");
}

#[tokio::test]
async fn high_risk_enforced_denied_with_deny_reason() {
    let db = in_memory_db();
    let config = autoloop::config::AppConfig::default();
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    tools.attach_state_store(db.clone());
    tools.hydrate_manifest(high_risk_manifest());

    let session_id = "d11-enforced";
    let task_id = "task-enforced";
    let identity = ExecutionIdentity {
        tenant_id: "tenant:d11".into(),
        principal_id: "principal:d11".into(),
        policy_id: "policy:d11".into(),
        lease_token: "lease:d11".into(),
    };
    seed_identity(&db, session_id, &identity).await;
    db.upsert_json_knowledge(
        format!("approval:capability:{session_id}:{task_id}:mcp::high-risk::invoke"),
        &serde_json::json!({"approved": true}),
        "approval",
    )
    .await
    .expect("approval");
    db.upsert_json_knowledge(
        format!("policy-pdp:strategy-deny:{session_id}:{task_id}:mcp::high-risk::invoke"),
        &serde_json::json!({"allow": true}),
        "policy-pdp",
    )
    .await
    .expect("deny-seed");

    let engine = CapabilityAdmissionEngine::with_policy_mode(RuntimePolicyMode::Enforced);
    let decision = engine
        .admit_selected(
            &db,
            &tools,
            &providers.factory_artifacts(),
            session_id,
            task_id,
            &identity,
            &CapabilityIntent {
                session_id: session_id.into(),
                objective: "test high-risk".into(),
                required_tags: vec!["Execution".into()],
                preferred_servers: vec!["local-mcp".into()],
            },
            "mcp::high-risk::invoke",
            Some("local-mcp"),
        )
        .await
        .expect("enforced decision");
    assert!(!decision.allowed);
    assert!(!decision.reason.trim().is_empty(), "deny_reason must be present");
}

#[tokio::test]
async fn high_risk_shadow_records_decision_diff() {
    let db = in_memory_db();
    let config = autoloop::config::AppConfig::default();
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    tools.attach_state_store(db.clone());
    tools.hydrate_manifest(high_risk_manifest());

    let session_id = "d11-shadow";
    let task_id = "task-shadow";
    let identity = ExecutionIdentity {
        tenant_id: "tenant:d11-shadow".into(),
        principal_id: "principal:d11-shadow".into(),
        policy_id: "policy:d11-shadow".into(),
        lease_token: "lease:d11-shadow".into(),
    };
    seed_identity(&db, session_id, &identity).await;
    db.upsert_json_knowledge(
        format!("approval:capability:{session_id}:{task_id}:mcp::high-risk::invoke"),
        &serde_json::json!({"approved": true}),
        "approval",
    )
    .await
    .expect("approval");
    db.upsert_json_knowledge(
        format!("policy-pdp:strategy-deny:{session_id}:{task_id}:mcp::high-risk::invoke"),
        &serde_json::json!({"allow": true}),
        "policy-pdp",
    )
    .await
    .expect("deny-seed");

    let engine = CapabilityAdmissionEngine::with_policy_mode(RuntimePolicyMode::Shadow);
    let decision = engine
        .admit_selected(
            &db,
            &tools,
            &providers.factory_artifacts(),
            session_id,
            task_id,
            &identity,
            &CapabilityIntent {
                session_id: session_id.into(),
                objective: "test high-risk shadow".into(),
                required_tags: vec!["Execution".into()],
                preferred_servers: vec!["local-mcp".into()],
            },
            "mcp::high-risk::invoke",
            Some("local-mcp"),
        )
        .await
        .expect("shadow decision");
    assert!(decision.allowed);

    let shadow_diffs = db
        .list_knowledge_by_prefix(&format!("policy-pdp:shadow-diff:{session_id}:{task_id}:"))
        .await
        .expect("shadow diff list");
    assert!(!shadow_diffs.is_empty(), "shadow diff evidence must be persisted");
}
