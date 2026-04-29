use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use autoloop::config::{AppConfig, PolicyMode};
use autoloop::contracts::ids::{CapabilityId, SessionId, TaskId, TraceId};
use autoloop::contracts::types::{
    ConstraintSet, ExecutionIdentity, TaskEnvelope, TrustExecutionPlan,
};
use autoloop::runtime::{GuardDecision, RuntimeKernel};
use autoloop::tools::{
    ApprovalStatus, CapabilityRisk, CapabilityScope, CapabilityStatus, CliOutputMode,
    ForgedMcpToolManifest, TrustStatus,
};
use autoloop::{providers::ProviderRegistry, tools::ToolRegistry};
use autoloop_state_adapter::{
    PolicyBinding, Principal, RoleBinding, SessionLease, StateStoreBackend, StateStore,
    StateStoreConfig, Tenant,
};
use serde_json::json;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn test_db() -> StateStore {
    StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    })
}

fn test_manifest(risk: CapabilityRisk) -> ForgedMcpToolManifest {
    ForgedMcpToolManifest {
        capability_id: "capability:test".into(),
        registered_tool_name: "mcp::local-mcp::test".into(),
        delegate_tool_name: "mcp::local-mcp::invoke".into(),
        server: "local-mcp".into(),
        capability_name: "test".into(),
        purpose: "test purpose".into(),
        executable: "test-cli".into(),
        command_template: "test-cli run".into(),
        payload_template: json!({}),
        output_mode: CliOutputMode::Json,
        working_directory: Some(".".into()),
        success_signal: Some("completed".into()),
        help_text: "help".into(),
        skill_markdown: "# skill".into(),
        examples: vec![],
        version: 1,
        lineage_key: "capability:test".into(),
        status: CapabilityStatus::Active,
        approval_status: ApprovalStatus::Verified,
        health_score: 0.8,
        scope: CapabilityScope::TaskFamily,
        tags: vec![],
        risk,
        requested_by: "runtime-test".into(),
        created_at_ms: 0,
        updated_at_ms: 0,
        approved_at_ms: None,
        rollback_to_version: None,
        trust_status: TrustStatus::Trusted,
        trust_findings: vec![],
        ..ForgedMcpToolManifest::default()
    }
}

async fn provision_identity(
    db: &StateStore,
    session_id: &str,
    tenant_id: &str,
    principal_id: &str,
    policy_id: &str,
    capability_prefixes: Vec<String>,
) -> ExecutionIdentity {
    let ts = now_ms();
    db.upsert_tenant(Tenant {
        tenant_id: tenant_id.into(),
        name: tenant_id.into(),
        status: "active".into(),
        created_at_ms: ts,
    })
    .await
    .expect("tenant");
    db.upsert_principal(Principal {
        principal_id: principal_id.into(),
        tenant_id: tenant_id.into(),
        principal_type: "user".into(),
        status: "active".into(),
        created_at_ms: ts,
    })
    .await
    .expect("principal");
    db.upsert_role_binding(RoleBinding {
        tenant_id: tenant_id.into(),
        principal_id: principal_id.into(),
        role: "operator".into(),
        updated_at_ms: ts,
    })
    .await
    .expect("role");
    db.upsert_policy_binding(PolicyBinding {
        policy_id: policy_id.into(),
        tenant_id: tenant_id.into(),
        role: "operator".into(),
        allowed_actions: vec![],
        capability_prefixes,
        max_memory_mb: 2048,
        max_tokens: 8192,
        updated_at_ms: ts,
    })
    .await
    .expect("policy");

    let lease_token = format!("lease:{session_id}");
    db.upsert_session_lease(SessionLease {
        lease_token: lease_token.clone(),
        session_id: session_id.into(),
        tenant_id: tenant_id.into(),
        principal_id: principal_id.into(),
        policy_id: policy_id.into(),
        expires_at_ms: ts.saturating_add(60_000),
        issued_at_ms: ts,
    })
    .await
    .expect("lease");

    ExecutionIdentity {
        tenant_id: tenant_id.into(),
        principal_id: principal_id.into(),
        policy_id: policy_id.into(),
        lease_token,
    }
}

fn wasm_fixture_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ontoloop-d9-{tag}-{}.wasm", now_ms()))
}

fn write_success_wasm(path: &PathBuf) {
    let wat = r#"
        (module
          (import "autoloop" "log_utf8" (func $log (param i32 i32) (result i32)))
          (import "autoloop" "emit_event_json" (func $emit (param i32 i32) (result i32)))
          (memory (export "memory") 1)
          (global $heap (mut i32) (i32.const 256))
          (func (export "autoloop_alloc") (param $n i32) (result i32)
            global.get $heap
            global.get $heap
            local.get $n
            i32.add
            global.set $heap)
          (data (i32.const 0) "sandbox-log")
          (data (i32.const 16) "{\"kind\":\"sandbox\"}")
          (data (i32.const 64) "{\"ok\":true,\"engine\":\"wasmtime\"}\00")
          (func (export "autoloop_run") (param i32 i32) (result i32)
            i32.const 0
            i32.const 11
            call $log
            drop
            i32.const 16
            i32.const 18
            call $emit
            drop
            i32.const 64)
        )
    "#;
    let wasm = wat::parse_str(wat).expect("valid wat");
    fs::write(path, wasm).expect("write wasm fixture");
}

fn base_constraints() -> ConstraintSet {
    ConstraintSet {
        max_cpu_percent: 80,
        max_memory_mb: 512,
        timeout_ms: 60_000,
        max_retries: 0,
        max_tokens: 2048,
        io_allow_paths: vec![".".into()],
        io_deny_paths: vec![],
        sandbox_profile: "default".into(),
        requires_human_approval: false,
    }
}

#[tokio::test]
async fn d9_toolsandboxed_executes_wasm_and_reports_host_logs_events() {
    let mut config = AppConfig::default();
    config.tools.allow_shell = true;

    let runtime = RuntimeKernel::from_config(&config.runtime);
    let db = test_db();
    let providers = ProviderRegistry::from_config(&config.providers);
    let tools = ToolRegistry::from_config(&config.tools);
    let manifest = test_manifest(CapabilityRisk::Low);

    let session_id = "session-d9-sandbox";
    let identity = provision_identity(
        &db,
        session_id,
        "tenant:d9",
        "principal:d9",
        "policy:d9",
        vec!["mcp::".into()],
    )
    .await;

    let wasm_path = wasm_fixture_path("sandboxed");
    write_success_wasm(&wasm_path);

    let envelope = TaskEnvelope {
        session_id: SessionId::from(session_id),
        trace_id: TraceId::from("trace-d9-sandbox"),
        task_id: TaskId::from("task-d9-sandbox"),
        capability_id: CapabilityId::from("mcp::local-mcp::test"),
        identity,
        payload: json!({
            "arguments": {
                "__wasm_sandbox": {
                    "module": wasm_path.to_string_lossy(),
                    "entrypoint": "autoloop_run",
                    "payload": { "q": "ping" }
                }
            }
        }),
        constraints: base_constraints(),
        trust_plan: None,
    };

    let result = runtime
        .execute(
            &db,
            &tools,
            &providers,
            session_id,
            &envelope,
            Some(&manifest),
            None,
        )
        .await
        .expect("runtime execute");

    assert_eq!(result.guard_report.decision, GuardDecision::Allow);
    let parsed = serde_json::from_str::<serde_json::Value>(&result.content).expect("json output");
    assert_eq!(parsed["entrypoint"], "autoloop_run");
    assert_eq!(parsed["output"]["ok"], true);
    assert_eq!(parsed["host_logs"][0], "sandbox-log");
    assert_eq!(parsed["host_events"][0]["kind"], "sandbox");
}

#[tokio::test]
async fn d9_trusted_high_risk_preflight_failure_blocks_execution() {
    let mut config = AppConfig::default();
    config.tools.allow_shell = true;
    config.runtime.policy_mode = PolicyMode::Enforced;

    let runtime = RuntimeKernel::from_config(&config.runtime);
    let db = test_db();
    let providers = ProviderRegistry::from_config(&config.providers);
    let tools = ToolRegistry::from_config(&config.tools);
    let manifest = test_manifest(CapabilityRisk::Low);

    let session_id = "session-d9-highrisk";
    let identity = provision_identity(
        &db,
        session_id,
        "tenant:d9h",
        "principal:d9h",
        "policy:d9h",
        vec!["mcp::".into()],
    )
    .await;

    let wasm_path = wasm_fixture_path("trusted");
    write_success_wasm(&wasm_path);

    let envelope = TaskEnvelope {
        session_id: SessionId::from(session_id),
        trace_id: TraceId::from("trace-d9-highrisk"),
        task_id: TaskId::from("task-d9-highrisk"),
        capability_id: CapabilityId::from("mcp::local-mcp::test"),
        identity,
        payload: json!({
            "arguments": {
                "__wasm_sandbox": {
                    "module": wasm_path.to_string_lossy(),
                    "entrypoint": "autoloop_run",
                    "payload": { "q": "ping" }
                }
            }
        }),
        constraints: base_constraints(),
        trust_plan: Some(TrustExecutionPlan {
            trust_level: "strict".into(),
            verify_identity: true,
            verify_environment: true,
            rollout_gate: "canary".into(),
            attestation_backend: "env".into(),
            attestation_required: true,
            attestation_policy_version: Some("attestation.v1".into()),
            policy_refs: vec!["policy:d9h".into()],
            budget_scope: "tenant".into(),
        }),
    };

    let result = runtime
        .execute(
            &db,
            &tools,
            &providers,
            session_id,
            &envelope,
            Some(&manifest),
            None,
        )
        .await
        .expect("runtime execute returns guarded content");

    assert_eq!(result.guard_report.decision, GuardDecision::Blocked);
    assert!(
        result
            .content
            .contains("trusted high-risk preflight rejected execution"),
        "expected preflight block reason in content, got: {}",
        result.content
    );
}




