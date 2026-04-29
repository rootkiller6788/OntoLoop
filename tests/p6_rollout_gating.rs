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

fn envelope(session: &str) -> TaskEnvelope {
    TaskEnvelope {
        session_id: SessionId::from(session),
        trace_id: TraceId::from(format!("{session}:trace")),
        task_id: TaskId::from("task-1"),
        capability_id: CapabilityId::from("read_file"),
        identity: identity_for(session),
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
            requires_human_approval: true,
        },
        trust_plan: None,
    }
}

async fn db() -> StateStore {
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    });
    seed_identity(&db, "shadow-session").await;
    seed_identity(&db, "full-session").await;
    seed_identity(&db, "canary-zero").await;
    seed_identity(&db, "canary-one").await;
    db
}

#[tokio::test]
async fn shadow_mode_records_but_does_not_block() {
    let mut config = AppConfig::default();
    config.runtime.gate_mode = RuntimeGateMode::Shadow;
    let runtime = RuntimeKernel::from_config(&config.runtime);
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    let db = db().await;
    let result = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "shadow-session",
            &envelope("shadow-session"),
            None,
            None,
        )
        .await
        .expect("shadow execution");
    assert_eq!(
        result.guard_report.decision,
        autoloop::runtime::GuardDecision::Allow
    );
    assert!(result.guard_report.reason.contains("shadow-observe-only"));
}

#[tokio::test]
async fn full_mode_enforces_guard() {
    let mut config = AppConfig::default();
    config.runtime.gate_mode = RuntimeGateMode::Full;
    let runtime = RuntimeKernel::from_config(&config.runtime);
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    let db = db().await;
    let result = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "full-session",
            &envelope("full-session"),
            None,
            None,
        )
        .await
        .expect("full execution");
    assert_ne!(
        result.guard_report.decision,
        autoloop::runtime::GuardDecision::Allow
    );
}

#[tokio::test]
async fn canary_ratio_controls_enforcement() {
    let mut config = AppConfig::default();
    config.runtime.gate_mode = RuntimeGateMode::Canary;
    config.runtime.gate_enforce_ratio = 0.0;
    let runtime_shadow_like = RuntimeKernel::from_config(&config.runtime);
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    let db = db().await;
    let loose = runtime_shadow_like
        .execute(
            &db,
            &tools,
            &providers,
            "canary-zero",
            &envelope("canary-zero"),
            None,
            None,
        )
        .await
        .expect("canary 0");
    assert_eq!(
        loose.guard_report.decision,
        autoloop::runtime::GuardDecision::Allow
    );

    config.runtime.gate_enforce_ratio = 1.0;
    let runtime_full_like = RuntimeKernel::from_config(&config.runtime);
    let strict = runtime_full_like
        .execute(
            &db,
            &tools,
            &providers,
            "canary-one",
            &envelope("canary-one"),
            None,
            None,
        )
        .await
        .expect("canary 1");
    assert_ne!(
        strict.guard_report.decision,
        autoloop::runtime::GuardDecision::Allow
    );
}

fn identity_for(session_id: &str) -> ExecutionIdentity {
    ExecutionIdentity {
        tenant_id: "tenant:test".into(),
        principal_id: format!("principal:{session_id}"),
        policy_id: "policy:test".into(),
        lease_token: format!("lease:{session_id}"),
    }
}

async fn seed_identity(db: &StateStore, session_id: &str) {
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
        principal_id: format!("principal:{session_id}"),
        tenant_id: "tenant:test".into(),
        principal_type: "user".into(),
        status: "active".into(),
        created_at_ms: now,
    })
    .await
    .expect("principal");
    db.upsert_role_binding(RoleBinding {
        tenant_id: "tenant:test".into(),
        principal_id: format!("principal:{session_id}"),
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
        lease_token: format!("lease:{session_id}"),
        session_id: session_id.into(),
        tenant_id: "tenant:test".into(),
        principal_id: format!("principal:{session_id}"),
        policy_id: "policy:test".into(),
        expires_at_ms: now.saturating_add(60_000),
        issued_at_ms: now,
    })
    .await
    .expect("lease");
}




