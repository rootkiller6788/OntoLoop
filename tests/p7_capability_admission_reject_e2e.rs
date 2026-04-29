use autoloop::{
    contracts::capability::CapabilityIntent,
    contracts::types::ExecutionIdentity,
    providers::ProviderRegistry,
    security::capability_admission::CapabilityAdmissionEngine,
    state_store_adapter::{
        BudgetAccount, PermissionAction, PolicyBinding, Principal, RoleBinding, SessionLease,
        StateStoreBackend, StateStore, StateStoreConfig, Tenant,
    },
    tools::{ForgedMcpToolManifest, ToolRegistry},
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
        expires_at_ms: now + 60_000,
        issued_at_ms: now,
    })
    .await
    .expect("lease");
}

#[tokio::test]
async fn capability_admission_reject_blocks_and_audits() {
    let db = StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    });
    let config = autoloop::config::AppConfig::default();
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    tools.attach_state_store(db.clone());

    let manifest: ForgedMcpToolManifest = serde_json::from_value(serde_json::json!({
        "capability_id": "cap-high-risk:v1", "registered_tool_name": "mcp::high-risk::invoke", "delegate_tool_name": "mcp::local-mcp::invoke", "server": "local-mcp", "capability_name": "high-risk", "purpose": "test high risk", "executable": "echo", "command_template": "echo {{payload}}", "payload_template": {"payload": "{{payload}}"}, "output_mode": "json", "help_text": "help", "skill_markdown": "skill", "examples": ["ex"], "version": 1, "lineage_key": "cap-high-risk", "status": "active", "approval_status": "verified", "health_score": 0.9, "scope": "session", "tags": ["test"], "risk": "high", "requested_by": "tester", "created_at_ms": 1, "updated_at_ms": 1, "approved_at_ms": 1, "rollback_to_version": null, "trust_status": "trusted", "trust_findings": [], "artifact": {"artifact_id":"a","digest_sha256":"d","source_uri":"u","build_epoch":1}, "signature": {"signer":"autoloop","algorithm":"deterministic_v1","signed_payload_hash":"x","signature":"y","signed_at_ms":1}, "provenance": {"source_repo":"r","source_ref":"ref","builder":"b","generated_by":"g"}, "sbom": {"components": []}, "trust_policy": {"required_signers":["autoloop"],"blocked_dependencies":[],"min_provenance_ref_len":1}
    })).expect("manifest");
    tools.hydrate_manifest(manifest);

    let identity = ExecutionIdentity {
        tenant_id: "tenant-a".into(),
        principal_id: "principal-a".into(),
        policy_id: "policy-a".into(),
        lease_token: "lease-a".into(),
    };
    seed_identity(&db, "session-a", &identity).await;
    db.upsert_budget_account(BudgetAccount {
        account_id: "principal-a".into(),
        tenant_id: identity.tenant_id.clone(),
        principal_id: identity.principal_id.clone(),
        policy_id: identity.policy_id.clone(),
        total_budget_micros: 100,
        reserved_micros: 0,
        spent_micros: 0,
        blocked_count: 0,
        updated_at_ms: autoloop::orchestration::current_time_ms(),
    })
    .await
    .expect("budget");

    let engine = CapabilityAdmissionEngine::new();
    let decision = engine
        .admit_selected(
            &db,
            &tools,
            &providers.factory_artifacts(),
            "session-a",
            "task-a",
            &identity,
            &CapabilityIntent {
                session_id: "session-a".into(),
                objective: "execute high risk task".into(),
                required_tags: vec!["Execution".into()],
                preferred_servers: vec!["local-mcp".into()],
            },
            "mcp::high-risk::invoke",
            Some("local-mcp"),
        )
        .await
        .expect("decision");

    assert!(!decision.allowed);
    assert!(decision.reason.contains("approval required"));
    let evidence_key = decision.evidence_ref.clone().expect("evidence ref");
    let evidence_record = db
        .get_knowledge(&evidence_key)
        .await
        .expect("evidence get")
        .expect("evidence exists");
    assert!(
        evidence_record
            .value
            .contains("factory:llm:mcp:local-mcp:v1")
    );
    let rejects = db
        .list_knowledge_by_prefix("policy-reject:session-a:task-a:")
        .await
        .expect("policy rejects");
    assert!(!rejects.is_empty());
}




