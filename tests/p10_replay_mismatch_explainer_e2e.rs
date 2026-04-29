use autoloop::{
    config::AppConfig,
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    observability::event_stream::{DeterminismBoundary, ReplaySnapshot, persist_replay_snapshot},
    providers::{ChatMessage, ProviderRegistry},
    runtime::{ReplayRunRequest, RuntimeKernel},
    state_store_adapter::{
        PermissionAction, PolicyBinding, Principal, RoleBinding, SessionLease, StateStoreBackend,
        StateStore, StateStoreConfig, Tenant,
    },
    tools::ToolRegistry,
};

async fn seed_identity(db: &StateStore, session_id: &str, identity: &ExecutionIdentity) {
    let now = autoloop::orchestration::current_time_ms();
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
        capability_prefixes: vec!["provider:".into(), "mcp::".into(), "read_".into()],
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
        expires_at_ms: now + 60_000,
        issued_at_ms: now,
    })
    .await
    .expect("lease");
}

#[tokio::test]
async fn replay_mismatch_produces_explainer() {
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    });
    let config = AppConfig::default();
    let runtime = RuntimeKernel::from_config(&config.runtime);
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);

    let session_id = "replay-session";
    let identity = ExecutionIdentity {
        tenant_id: "tenant-r".into(),
        principal_id: "principal-r".into(),
        policy_id: "policy-r".into(),
        lease_token: "lease-r".into(),
    };
    seed_identity(&db, session_id, &identity).await;

    let envelope = TaskEnvelope {
        session_id: SessionId::from(session_id),
        trace_id: TraceId::from("trace-r"),
        task_id: TaskId::from("task-r"),
        capability_id: CapabilityId::from("provider:default"),
        identity: identity.clone(),
        payload: serde_json::to_value(vec![ChatMessage {
            role: "user".into(),
            content: "replay mismatch test".into(),
        }])
        .expect("payload"),
        constraints: ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: 512,
            timeout_ms: 60_000,
            max_retries: 1,
            max_tokens: 4096,
            io_allow_paths: vec![".".into()],
            io_deny_paths: vec!["/etc".into()],
            sandbox_profile: "provider".into(),
            requires_human_approval: false,
        },
        trust_plan: None,
    };

    let snapshot = ReplaySnapshot {
        snapshot_id: String::new(),
        session_id: session_id.into(),
        trace_id: "trace-r".into(),
        task_id: "task-r".into(),
        capability_id: "provider:default".into(),
        actor_id: "tester".into(),
        preferred_model: None,
        route_model: None,
        input_digest: "input-digest".into(),
        parameters_digest: "params-digest".into(),
        output_digest: "definitely-not-matching".into(),
        artifacts: vec![],
        boundary: DeterminismBoundary {
            mode: "strict".into(),
            locked_fields: vec!["payload".into()],
            non_deterministic_steps: vec!["provider_response_generation".into()],
            external_dependencies: vec!["provider".into()],
        },
        seed: None,
        replay_input: serde_json::json!({ "actor_id": "tester", "preferred_model": null, "execution_surface": "provider", "envelope": envelope }),
        created_at_ms: 0,
    };

    let persisted = persist_replay_snapshot(&db, snapshot)
        .await
        .expect("persist replay snapshot");
    let report = runtime
        .replay_from_snapshot(
            &db,
            &tools,
            &providers,
            &ReplayRunRequest {
                snapshot_id: persisted.snapshot_id.clone(),
            },
        )
        .await
        .expect("replay report");

    assert!(!report.matched);
    assert!(
        report
            .deviations
            .iter()
            .any(|item| item.field == "output_digest")
    );
    assert!(
        report
            .deviations
            .iter()
            .any(|item| item.explanation.contains("replay mismatch under boundary"))
    );
}




