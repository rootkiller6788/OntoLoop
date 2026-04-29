use autoloop::{
    config::{AppConfig, RuntimeGateMode},
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    orchestration::{ExecutionReport, SwarmTask},
    providers::ProviderRegistry,
    runtime::RuntimeKernel,
    state_store_adapter::{
        PolicyBinding, Principal, RoleBinding, SessionLease, StateStoreBackend, StateStore,
        StateStoreConfig, Tenant,
    },
    tools::ToolRegistry,
};

#[tokio::test]
async fn baseline_concurrent_execute_is_stable() {
    let mut app_config = AppConfig::default();
    app_config.runtime.gate_mode = RuntimeGateMode::Full;
    let runtime = RuntimeKernel::from_config(&app_config.runtime);
    let providers = ProviderRegistry::from_config(&app_config.providers);
    let tools = ToolRegistry::from_config(&app_config.tools);
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 8,
    });
    seed_identity(&db, "perf-session").await;

    let mut handles = Vec::new();
    for i in 0..24u32 {
        let runtime = runtime.clone();
        let providers = providers.clone();
        let tools = tools.clone();
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            let messages = vec![autoloop::providers::ChatMessage {
                role: "user".into(),
                content: format!("concurrency case {i}"),
            }];
            let envelope = TaskEnvelope {
                session_id: SessionId::from("perf-session"),
                trace_id: TraceId::from(format!("perf-trace-{i}")),
                task_id: TaskId::from(format!("perf-task-{i}")),
                capability_id: CapabilityId::from("provider:default"),
                identity: identity_for("perf-session"),
                payload: serde_json::to_value(&messages).expect("payload"),
                constraints: ConstraintSet {
                    max_cpu_percent: 80,
                    max_memory_mb: 512,
                    timeout_ms: 60_000,
                    max_retries: 1,
                    max_tokens: 8_000,
                    io_allow_paths: vec![".".into()],
                    io_deny_paths: vec!["/etc".into()],
                    sandbox_profile: "provider".into(),
                    requires_human_approval: false,
                },
                trust_plan: None,
            };
            runtime
                .execute(
                    &db,
                    &tools,
                    &providers,
                    "perf-session",
                    &envelope,
                    None,
                    None,
                )
                .await
        }));
    }

    for handle in handles {
        let result = handle.await.expect("join");
        assert!(result.is_ok(), "concurrent execute should remain stable");
    }
}

#[tokio::test]
async fn baseline_circuit_recovery_state_is_observable() {
    let mut app_config = AppConfig::default();
    app_config.runtime.tool_breaker_failure_threshold = 1;
    app_config.runtime.tool_breaker_cooldown_ms = 1;
    let runtime = RuntimeKernel::from_config(&app_config.runtime);
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    });

    let report = ExecutionReport {
        task: SwarmTask {
            task_id: "perf-circuit-task".into(),
            agent_name: "execution-agent".into(),
            role: "Execution".into(),
            objective: "trigger failure".into(),
            depends_on: Vec::new(),
        },
        output: "failed with an error".into(),
        tool_used: Some("mcp::local-mcp::invoke".into()),
        mcp_server: Some("local-mcp".into()),
        invocation_payload: Some("{}".into()),
        outcome_score: -5,
        route_variant: "control".into(),
        control_score: -5,
        treatment_score: -5,
        guard_decision: "Allow".into(),
    };
    runtime
        .record_execution_outcome(&db, &report)
        .await
        .expect("record outcome");
    let snapshot = runtime.circuit_snapshot(&db).await.expect("snapshot");
    assert!(
        snapshot
            .keys()
            .any(|key| key.contains("metrics:circuit:tool:mcp::local-mcp::invoke")),
        "circuit snapshot should contain tool breaker state"
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
        capability_prefixes: vec!["provider:".into(), "mcp::".into(), "read_".into()],
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




