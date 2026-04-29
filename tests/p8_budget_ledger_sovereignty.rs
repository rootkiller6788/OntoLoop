use autoloop::{
    config::{AppConfig, RuntimeGateMode},
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    providers::{ChatMessage, ProviderRegistry},
    runtime::RuntimeKernel,
    state_store_adapter::{
        BudgetAccount, PermissionAction, PolicyBinding, Principal, RoleBinding, SessionLease,
        StateStoreBackend, StateStore, StateStoreConfig, Tenant,
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
        pool_size: 8,
    })
}

fn runtime_tools_providers(
    default_budget_micros: u64,
    quota_window_budget_micros: u64,
) -> (RuntimeKernel, ToolRegistry, ProviderRegistry) {
    let mut config = AppConfig::default();
    config.runtime.gate_mode = RuntimeGateMode::Full;
    config.runtime.budget_enforced = true;
    config.runtime.default_budget_micros = default_budget_micros;
    config.runtime.quota_window_budget_micros = quota_window_budget_micros;
    let isolate = now_ms();
    config.runtime.trust_evidence_ledger_path =
        format!("target/test-ledger/p8-evidence-{isolate}.log");
    config.runtime.trust_budget_ledger_path = format!("target/test-ledger/p8-budget-{isolate}.log");
    (
        RuntimeKernel::from_config(&config.runtime),
        ToolRegistry::from_config(&config.tools),
        ProviderRegistry::from_config(&config.providers),
    )
}

async fn seed_identity(
    db: &StateStore,
    session_id: &str,
    tenant_id: &str,
    principal_id: &str,
    policy_id: &str,
    capability_prefixes: Vec<String>,
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
        role: "operator".into(),
        updated_at_ms: now,
    })
    .await
    .expect("role");
    db.upsert_policy_binding(PolicyBinding {
        policy_id: policy_id.into(),
        tenant_id: tenant_id.into(),
        role: "operator".into(),
        allowed_actions: vec![
            PermissionAction::Read,
            PermissionAction::Write,
            PermissionAction::Dispatch,
        ],
        capability_prefixes,
        max_memory_mb: 2048,
        max_tokens: 64000,
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
        expires_at_ms: now.saturating_add(120_000),
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

fn provider_envelope(
    session_id: &str,
    task_id: &str,
    identity: ExecutionIdentity,
    text: &str,
) -> TaskEnvelope {
    let messages = vec![ChatMessage {
        role: "user".into(),
        content: text.into(),
    }];
    TaskEnvelope {
        session_id: SessionId::from(session_id),
        trace_id: TraceId::from(format!("{session_id}:{task_id}:trace")),
        task_id: TaskId::from(task_id),
        capability_id: CapabilityId::from("provider:default"),
        identity,
        payload: serde_json::to_value(messages).expect("messages payload"),
        constraints: ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: 512,
            timeout_ms: 30_000,
            max_retries: 1,
            max_tokens: 8_000,
            io_allow_paths: vec![".".into()],
            io_deny_paths: vec!["/etc".into()],
            sandbox_profile: "provider".into(),
            requires_human_approval: false,
        },
        trust_plan: None,
    }
}

fn tool_envelope(
    session_id: &str,
    task_id: &str,
    identity: ExecutionIdentity,
    capability: &str,
) -> TaskEnvelope {
    TaskEnvelope {
        session_id: SessionId::from(session_id),
        trace_id: TraceId::from(format!("{session_id}:{task_id}:trace")),
        task_id: TaskId::from(task_id),
        capability_id: CapabilityId::from(capability),
        identity,
        payload: serde_json::Value::String("{}".into()),
        constraints: ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: 512,
            timeout_ms: 30_000,
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[tokio::test]
async fn p8_over_budget_blocks_before_execution() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers(1_000, 1_000);
    let identity = seed_identity(
        &db,
        "p8-budget-block",
        "tenant-p8",
        "principal-p8",
        "policy-p8",
        vec!["provider:".into()],
    )
    .await;
    let envelope = provider_envelope(
        "p8-budget-block",
        "task-over",
        identity,
        &"x".repeat(12_000),
    );
    let err = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "p8-budget-block",
            &envelope,
            None,
            None,
        )
        .await
        .expect_err("must block when precharge exceeds budget");
    assert!(err.to_string().contains("budget precharge exceeded"));
}

#[tokio::test]
async fn p8_concurrent_charging_is_consistent() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers(5_000_000, 5_000_000);
    let identity = seed_identity(
        &db,
        "p8-concurrency",
        "tenant-p8",
        "principal-concurrency",
        "policy-p8",
        vec!["provider:".into()],
    )
    .await;
    let account_id = format!(
        "{}:{}:{}",
        identity.tenant_id, identity.principal_id, identity.policy_id
    );
    db.upsert_budget_account(BudgetAccount {
        account_id: account_id.clone(),
        tenant_id: identity.tenant_id.clone(),
        principal_id: identity.principal_id.clone(),
        policy_id: identity.policy_id.clone(),
        total_budget_micros: 5_000_000,
        reserved_micros: 0,
        spent_micros: 0,
        blocked_count: 0,
        updated_at_ms: now_ms(),
    })
    .await
    .expect("budget account");

    let mut handles = Vec::new();
    for i in 0..16u32 {
        let runtime = runtime.clone();
        let db = db.clone();
        let tools = tools.clone();
        let providers = providers.clone();
        let envelope = provider_envelope(
            "p8-concurrency",
            &format!("task-{i}"),
            identity.clone(),
            &format!("ping {i}"),
        );
        handles.push(tokio::spawn(async move {
            runtime
                .execute(
                    &db,
                    &tools,
                    &providers,
                    "p8-concurrency",
                    &envelope,
                    None,
                    None,
                )
                .await
        }));
    }
    for handle in handles {
        let result = handle.await.expect("join");
        assert!(
            result.is_ok(),
            "concurrent execution should settle successfully"
        );
    }

    let report = runtime
        .reconcile_budget_account(&db, "tenant-p8", &account_id)
        .await
        .expect("reconcile");
    assert!(report.consistent, "ledger and account should match");
    assert!(report.account_spent_micros > 0);
}

#[tokio::test]
async fn p8_rollback_compensation_restores_reserve() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers(1_000_000, 1_000_000);
    let identity = seed_identity(
        &db,
        "p8-rollback",
        "tenant-p8",
        "principal-rollback",
        "policy-p8",
        vec!["unknown".into()],
    )
    .await;
    let account_id = format!(
        "{}:{}:{}",
        identity.tenant_id, identity.principal_id, identity.policy_id
    );
    let envelope = tool_envelope("p8-rollback", "task-fail", identity.clone(), "unknown_tool");
    let execution = runtime
        .execute(
            &db,
            &tools,
            &providers,
            "p8-rollback",
            &envelope,
            None,
            None,
        )
        .await;
    match execution {
        Ok(result) => {
            assert!(
                result.content.contains("degraded-tool-fallback")
                    || result.content.contains("unknown tool"),
                "degraded fallback should still surface unknown tool reason"
            );
        }
        Err(error) => {
            assert!(error.to_string().contains("unknown tool"));
        }
    }

    let report = runtime
        .reconcile_budget_account(&db, "tenant-p8", &account_id)
        .await
        .expect("reconcile");
    assert_eq!(report.account_reserved_micros, 0);
    assert!(report.ledger_reserved_open_micros <= 0);
}

#[tokio::test]
async fn p8_task_cost_attribution_contains_token_tool_and_duration_breakdown() {
    let db = db();
    let (runtime, tools, providers) = runtime_tools_providers(2_000_000, 2_000_000);
    let identity = seed_identity(
        &db,
        "p8-attribution",
        "tenant-p8",
        "principal-attribution",
        "policy-p8",
        vec!["provider:".into()],
    )
    .await;
    let envelope = provider_envelope(
        "p8-attribution",
        "task-cost",
        identity.clone(),
        "hello budget",
    );
    runtime
        .execute(
            &db,
            &tools,
            &providers,
            "p8-attribution",
            &envelope,
            None,
            None,
        )
        .await
        .expect("execution");

    let attributions = db
        .list_cost_attribution_by_session(&identity.tenant_id, "p8-attribution")
        .await
        .expect("list attribution");
    let item = attributions
        .iter()
        .find(|item| item.task_id == "task-cost")
        .expect("task attribution");
    assert!(item.token_cost_micros > 0);
    assert!(item.duration_cost_micros > 0);
    assert_eq!(
        item.total_cost_micros,
        item.token_cost_micros
            .saturating_add(item.tool_cost_micros)
            .saturating_add(item.duration_cost_micros)
    );
}




