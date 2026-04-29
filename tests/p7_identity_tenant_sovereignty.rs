use autoloop::{
    config::{AppConfig, RuntimeGateMode},
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    providers::ProviderRegistry,
    runtime::RuntimeKernel,
    state_store_adapter::{
        PolicyBinding, Principal, RoleBinding, SessionLease, StateStoreBackend, StateStore,
        StateStoreConfig, Tenant,
    },
    tools::ToolRegistry,
};

fn db() -> StateStore {
    StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    })
}

fn runtime_tools_providers() -> (RuntimeKernel, ToolRegistry, ProviderRegistry) {
    let mut config = AppConfig::default();
    config.runtime.gate_mode = RuntimeGateMode::Full;
    (
        RuntimeKernel::from_config(&config.runtime),
        ToolRegistry::from_config(&config.tools),
        ProviderRegistry::from_config(&config.providers),
    )
}

fn base_envelope(session_id: &str, capability: &str, identity: ExecutionIdentity) -> TaskEnvelope {
    TaskEnvelope {
        session_id: SessionId::from(session_id),
        trace_id: TraceId::from(format!("{session_id}:trace")),
        task_id: TaskId::from("task-1"),
        capability_id: CapabilityId::from(capability),
        identity,
        payload: serde_json::Value::String("Cargo.toml".into()),
        constraints: ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: 512,
            timeout_ms: 60_000,
            max_retries: 1,
            max_tokens: 8_000,
            io_allow_paths: vec![".".into()],
            io_deny_paths: vec!["/etc".into()],
            sandbox_profile: "standard".into(),
            requires_human_approval: false,
        },
        trust_plan: None,
    }
}

async fn seed_identity(
    db: &StateStore,
    session_id: &str,
    tenant_id: &str,
    principal_id: &str,
    policy_id: &str,
    role_binding: &str,
    role_policy: &str,
    prefixes: Vec<String>,
    expires_at_ms: u64,
) -> ExecutionIdentity {
    let now = now_ms();
    db.upsert_tenant(Tenant {
        tenant_id: tenant_id.into(),
        name: tenant_id.into(),
        status: "active".into(),
        created_at_ms: now,
    })
    .await
    .expect("tenant");
    db.upsert_principal(Principal {
        principal_id: principal_id.into(),
        tenant_id: tenant_id.into(),
        principal_type: "user".into(),
        status: "active".into(),
        created_at_ms: now,
    })
    .await
    .expect("principal");
    db.upsert_role_binding(RoleBinding {
        tenant_id: tenant_id.into(),
        principal_id: principal_id.into(),
        role: role_binding.into(),
        updated_at_ms: now,
    })
    .await
    .expect("role");
    db.upsert_policy_binding(PolicyBinding {
        policy_id: policy_id.into(),
        tenant_id: tenant_id.into(),
        role: role_policy.into(),
        allowed_actions: vec![],
        capability_prefixes: prefixes,
        max_memory_mb: 2048,
        max_tokens: 32000,
        updated_at_ms: now,
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
        expires_at_ms,
        issued_at_ms: now,
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[tokio::test]
async fn p7_cross_tenant_access_is_denied() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers();
    let _identity_a = seed_identity(
        &db,
        "session-a",
        "tenant-a",
        "principal-a",
        "policy-a",
        "operator",
        "operator",
        vec!["read_".into()],
        now_ms().saturating_add(60_000),
    )
    .await;
    let wrong_identity = ExecutionIdentity {
        tenant_id: "tenant-b".into(),
        principal_id: "principal-a".into(),
        policy_id: "policy-a".into(),
        lease_token: "lease:session-a".into(),
    };
    let envelope = base_envelope("session-a", "read_file", wrong_identity);

    let err = runtime
        .execute(&db, &tools, &providers, "session-a", &envelope, None, None)
        .await
        .expect_err("cross tenant must fail");
    assert!(err.to_string().contains("tenant not found"));
}

#[tokio::test]
async fn p7_expired_token_is_denied() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers();
    let identity = seed_identity(
        &db,
        "session-expired",
        "tenant-expired",
        "principal-expired",
        "policy-expired",
        "operator",
        "operator",
        vec!["read_".into()],
        now_ms().saturating_sub(1_000),
    )
    .await;
    let envelope = base_envelope("session-expired", "read_file", identity);

    let err = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "session-expired",
            &envelope,
            None,
            None,
        )
        .await
        .expect_err("expired lease must fail");
    assert!(err.to_string().contains("session lease expired"));
}

#[tokio::test]
async fn p7_role_downgrade_is_denied() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers();
    let identity = seed_identity(
        &db,
        "session-downgrade",
        "tenant-downgrade",
        "principal-downgrade",
        "policy-downgrade",
        "viewer",
        "operator",
        vec!["read_".into()],
        now_ms().saturating_add(60_000),
    )
    .await;
    let envelope = base_envelope("session-downgrade", "read_file", identity);

    let err = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "session-downgrade",
            &envelope,
            None,
            None,
        )
        .await
        .expect_err("role mismatch must fail");
    assert!(err.to_string().contains("role downgraded or mismatched"));
}

#[tokio::test]
async fn p7_least_privilege_allows_read_and_denies_write() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers();
    let identity = seed_identity(
        &db,
        "session-least",
        "tenant-least",
        "principal-least",
        "policy-least",
        "operator",
        "operator",
        vec!["read_".into()],
        now_ms().saturating_add(60_000),
    )
    .await;
    let read_envelope = base_envelope("session-least", "read_file", identity.clone());
    let write_envelope = base_envelope("session-least", "write_file", identity);

    let read_result = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "session-least",
            &read_envelope,
            None,
            None,
        )
        .await;
    assert!(
        read_result.is_ok(),
        "least privilege should allow read capability"
    );

    let write_error = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "session-least",
            &write_envelope,
            None,
            None,
        )
        .await
        .expect_err("write capability should be blocked by policy prefixes");
    assert!(
        write_error
            .to_string()
            .contains("capability not allowed by policy")
    );
}




