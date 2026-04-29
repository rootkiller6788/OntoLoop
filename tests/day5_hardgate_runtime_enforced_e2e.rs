use autoloop::{
    config::AppConfig,
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    providers::ProviderRegistry,
    runtime::{GuardDecision, RuntimeKernel},
    state_store_adapter::{
        PolicyBinding, Principal, RoleBinding, SessionLease, StateStoreBackend, StateStore,
        StateStoreConfig, Tenant,
    },
    tools::ToolRegistry,
};

fn envelope(payload: serde_json::Value) -> TaskEnvelope {
    TaskEnvelope {
        session_id: SessionId::from("hardgate-session"),
        trace_id: TraceId::from("trace:hardgate"),
        task_id: TaskId::from("task-hardgate"),
        capability_id: CapabilityId::from("read_file"),
        identity: ExecutionIdentity {
            tenant_id: "tenant:test".into(),
            principal_id: "principal:hardgate-session".into(),
            policy_id: "policy:test".into(),
            lease_token: "lease:hardgate-session".into(),
        },
        payload,
        constraints: ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: 512,
            timeout_ms: 30_000,
            max_retries: 1,
            max_tokens: 8_000,
            io_allow_paths: vec![".".into()],
            io_deny_paths: vec![],
            sandbox_profile: "standard".into(),
            requires_human_approval: false,
        },
        trust_plan: None,
    }
}

async fn seeded_db() -> StateStore {
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    });

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);

    db.upsert_tenant(Tenant {
        tenant_id: "tenant:test".into(),
        name: "tenant:test".into(),
        status: "active".into(),
        created_at_ms: now,
    })
    .await
    .expect("tenant");
    db.upsert_principal(Principal {
        principal_id: "principal:hardgate-session".into(),
        tenant_id: "tenant:test".into(),
        principal_type: "user".into(),
        status: "active".into(),
        created_at_ms: now,
    })
    .await
    .expect("principal");
    db.upsert_role_binding(RoleBinding {
        tenant_id: "tenant:test".into(),
        principal_id: "principal:hardgate-session".into(),
        role: "operator".into(),
        updated_at_ms: now,
    })
    .await
    .expect("role");
    db.upsert_policy_binding(PolicyBinding {
        policy_id: "policy:test".into(),
        tenant_id: "tenant:test".into(),
        role: "operator".into(),
        allowed_actions: vec![],
        capability_prefixes: vec!["read_".into(), "provider:".into(), "mcp::".into()],
        max_memory_mb: 2048,
        max_tokens: 32000,
        updated_at_ms: now,
    })
    .await
    .expect("policy");
    db.upsert_session_lease(SessionLease {
        lease_token: "lease:hardgate-session".into(),
        session_id: "hardgate-session".into(),
        tenant_id: "tenant:test".into(),
        principal_id: "principal:hardgate-session".into(),
        policy_id: "policy:test".into(),
        expires_at_ms: now.saturating_add(120_000),
        issued_at_ms: now,
    })
    .await
    .expect("lease");

    db
}

#[tokio::test]
async fn runtime_blocks_when_hardgate_required_but_token_missing() {
    let config = AppConfig::default();
    let runtime = RuntimeKernel::from_config(&config.runtime);
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    let db = seeded_db().await;

    let result = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "hardgate-session",
            &envelope(serde_json::json!({
                "arguments": "Cargo.toml",
                "hardgate_required": true
            })),
            None,
            None,
        )
        .await
        .expect("execute should return guard result");

    assert_eq!(result.guard_report.decision, GuardDecision::Blocked);
    assert!(result.guard_report.reason.contains("hardgate_pass_token"));
}

#[tokio::test]
async fn runtime_allows_path_when_hardgate_token_present() {
    let config = AppConfig::default();
    let runtime = RuntimeKernel::from_config(&config.runtime);
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    let db = seeded_db().await;

    let result = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "hardgate-session",
            &envelope(serde_json::json!({
                "arguments": "Cargo.toml",
                "hardgate_required": true,
                "hardgate_pass_token": "hgt:day5-valid-token"
            })),
            None,
            None,
        )
        .await
        .expect("execute should proceed when token present");

    assert!(
        !result
            .guard_report
            .reason
            .contains("hardgate_pass_token missing")
    );
}




