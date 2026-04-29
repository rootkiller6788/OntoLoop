use std::{path::PathBuf, sync::Arc};

use autoloop::{
    config::PolicyMode as RuntimePolicyMode,
    contracts::{
        capability::CapabilityIntent,
        policy_pdp::PolicyVersion,
        types::ExecutionIdentity,
    },
    providers::ProviderRegistry,
    runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
    security::{
        capability_admission::CapabilityAdmissionEngine,
        policy_host::{
            BundleActivationManager, DefaultPolicyBundleVerifier, DiscoveryPollOutcome,
            LoadedPolicyBundle, PassThroughCanary, PolicyBundleManifest,
            PolicyBundleVerifier, PolicyBundleVerifyRequirements, PolicyDiscoveryConfig,
            PolicyDiscoveryService,
        },
    },
    state_store_adapter::{
        PermissionAction, PolicyBinding, Principal, RoleBinding, SessionLease, StateStoreBackend,
        StateStore, StateStoreConfig, Tenant,
    },
    tools::{ForgedMcpToolManifest, ToolRegistry},
};
use autoloop::security::policy_host::verify::enforce_verified;

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

fn temp_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ontoloop-policy-acceptance-{}-{}-{}",
        name,
        std::process::id(),
        current_time_ms()
    ))
}

fn high_risk_manifest() -> ForgedMcpToolManifest {
    serde_json::from_value(serde_json::json!({
        "capability_id": "cap-high-risk:v1",
        "registered_tool_name": "mcp::high-risk::invoke",
        "delegate_tool_name": "mcp::local-mcp::invoke",
        "server": "local-mcp",
        "capability_name": "high-risk",
        "purpose": "policy acceptance test",
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

#[tokio::test]
async fn bundle_signature_mismatch_rejected() {
    let root = temp_dir("bundle-signature");
    std::fs::create_dir_all(&root).expect("temp root");
    let archive_path = root.join("bundle.tar.gz");
    std::fs::write(&archive_path, b"fake-bundle").expect("archive");

    let bundle = LoadedPolicyBundle {
        source_archive: archive_path,
        extracted_dir: root.clone(),
        manifest_path: root.join("manifest.json"),
        wasm_path: root.join("policy.wasm"),
        data_path: root.join("data.json"),
        manifest: PolicyBundleManifest {
            policy_id: "policy-a".into(),
            policy_version: PolicyVersion {
                id: "v1".into(),
                revision: 1,
            },
            wasm_entrypoint: "eval".into(),
            wasm_file: "policy.wasm".into(),
            data_file: "data.json".into(),
            metadata: serde_json::json!({
                "capabilities_version": "caps-v1",
                "bundle_hash": "wrong",
                "signature_digest": "wrong"
            }),
        },
        data: serde_json::json!({"ok": true}),
    };

    let verify = DefaultPolicyBundleVerifier
        .verify(
            &bundle,
            &PolicyBundleVerifyRequirements {
                policy_version: PolicyVersion {
                    id: "v1".into(),
                    revision: 1,
                },
                capabilities_version: "caps-v1".into(),
            },
        )
        .expect("verify");
    assert!(!verify.verified);
    assert!(enforce_verified(&verify).is_err());
}

#[tokio::test]
async fn discovery_fetch_failure_auto_rollback_keeps_stable_current() {
    let root = temp_dir("discovery-fetch");
    let current_dir = root.join("current");
    std::fs::create_dir_all(&current_dir).expect("current dir");
    let marker = current_dir.join("marker.txt");
    std::fs::write(&marker, "stable-current").expect("marker write");

    let activation = BundleActivationManager::new(&root);
    let discovery = PolicyDiscoveryService::new(
        PolicyDiscoveryConfig {
            discovery_url: "http://127.0.0.1:1/unreachable-bundle.tar.gz".into(),
            poll_interval_ms: 100,
            max_backoff_ms: 1_000,
            request_timeout_ms: 1_000,
            canary_enabled: true,
        },
        activation,
        DefaultPolicyBundleVerifier,
        PolicyBundleVerifyRequirements {
            policy_version: PolicyVersion {
                id: "v1".into(),
                revision: 1,
            },
            capabilities_version: "caps-v1".into(),
        },
        Arc::new(PassThroughCanary),
    )
    .expect("discovery service");

    let outcome = discovery.poll_once().await.expect("poll once");
    match outcome {
        DiscoveryPollOutcome::FetchError { .. } => {}
        other => panic!("expected fetch error outcome, got {:?}", other),
    }

    let marker_after = std::fs::read_to_string(&marker).expect("marker still exists");
    assert_eq!(marker_after, "stable-current");
}

#[tokio::test]
async fn enforced_mode_high_risk_deny_effective() {
    let db = in_memory_db();
    let config = autoloop::config::AppConfig::default();
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    tools.attach_state_store(db.clone());
    tools.hydrate_manifest(high_risk_manifest());

    let identity = ExecutionIdentity {
        tenant_id: "tenant-enforced".into(),
        principal_id: "principal-enforced".into(),
        policy_id: "policy-enforced".into(),
        lease_token: "lease-enforced".into(),
    };
    seed_identity(&db, "policy-enforced-session", &identity).await;

    db.upsert_json_knowledge(
        "approval:capability:policy-enforced-session:task-enforced:mcp::high-risk::invoke",
        &serde_json::json!({"approved": true}),
        "approval",
    )
    .await
    .expect("approval seed");
    db.upsert_json_knowledge(
        "policy-pdp:strategy-deny:policy-enforced-session:task-enforced:mcp::high-risk::invoke",
        &serde_json::json!({"allow": true}),
        "policy-pdp",
    )
    .await
    .expect("strategy deny seed");

    let engine = CapabilityAdmissionEngine::with_policy_mode(RuntimePolicyMode::Enforced);
    let decision = engine
        .admit_selected(
            &db,
            &tools,
            &providers.factory_artifacts(),
            "policy-enforced-session",
            "task-enforced",
            &identity,
            &CapabilityIntent {
                session_id: "policy-enforced-session".into(),
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
    assert!(decision.reason.contains("pdp enforced deny"));
}

#[tokio::test]
async fn shadow_mode_diff_traceable() {
    let db = in_memory_db();
    let config = autoloop::config::AppConfig::default();
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    tools.attach_state_store(db.clone());
    tools.hydrate_manifest(high_risk_manifest());

    let identity = ExecutionIdentity {
        tenant_id: "tenant-shadow".into(),
        principal_id: "principal-shadow".into(),
        policy_id: "policy-shadow".into(),
        lease_token: "lease-shadow".into(),
    };
    seed_identity(&db, "policy-shadow-session", &identity).await;

    db.upsert_json_knowledge(
        "approval:capability:policy-shadow-session:task-shadow:mcp::high-risk::invoke",
        &serde_json::json!({"approved": true}),
        "approval",
    )
    .await
    .expect("approval seed");
    db.upsert_json_knowledge(
        "policy-pdp:strategy-deny:policy-shadow-session:task-shadow:mcp::high-risk::invoke",
        &serde_json::json!({"allow": true}),
        "policy-pdp",
    )
    .await
    .expect("strategy deny seed");

    let engine = CapabilityAdmissionEngine::with_policy_mode(RuntimePolicyMode::Shadow);
    let decision = engine
        .admit_selected(
            &db,
            &tools,
            &providers.factory_artifacts(),
            "policy-shadow-session",
            "task-shadow",
            &identity,
            &CapabilityIntent {
                session_id: "policy-shadow-session".into(),
                objective: "execute high risk task".into(),
                required_tags: vec!["Execution".into()],
                preferred_servers: vec!["local-mcp".into()],
            },
            "mcp::high-risk::invoke",
            Some("local-mcp"),
        )
        .await
        .expect("decision");
    assert!(decision.allowed);

    let shadow_diffs = db
        .list_knowledge_by_prefix("policy-pdp:shadow-diff:policy-shadow-session:task-shadow:")
        .await
        .expect("shadow diffs");
    assert!(!shadow_diffs.is_empty());
    let latest = shadow_diffs
        .iter()
        .max_by_key(|record| record.key.as_str())
        .expect("latest shadow diff");
    let latest_json: serde_json::Value =
        serde_json::from_str(&latest.value).expect("shadow diff json");
    let evidence_ref = latest_json
        .get("evidence_ref")
        .and_then(serde_json::Value::as_str)
        .expect("shadow diff evidence_ref");
    assert!(
        evidence_ref.starts_with("evidence:stage:"),
        "shadow diff should carry evidence reference"
    );
    let evidence = db
        .get_knowledge(evidence_ref)
        .await
        .expect("load shadow diff evidence");
    assert!(evidence.is_some(), "shadow diff evidence should exist");
}

#[tokio::test]
async fn high_risk_task_enforced_reject_and_shadow_diff_are_traceable() {
    let db = in_memory_db();
    let config = autoloop::config::AppConfig::default();
    let tools = ToolRegistry::from_config(&config.tools);
    let providers = ProviderRegistry::from_config(&config.providers);
    tools.attach_state_store(db.clone());
    tools.hydrate_manifest(high_risk_manifest());

    let identity = ExecutionIdentity {
        tenant_id: "tenant-d11".into(),
        principal_id: "principal-d11".into(),
        policy_id: "policy-d11".into(),
        lease_token: "lease-d11".into(),
    };
    let session_id = "policy-d11-session";
    let task_id = "task-high-risk";
    seed_identity(&db, session_id, &identity).await;

    db.upsert_json_knowledge(
        format!("approval:capability:{session_id}:{task_id}:mcp::high-risk::invoke"),
        &serde_json::json!({"approved": true}),
        "approval",
    )
    .await
    .expect("approval seed");
    db.upsert_json_knowledge(
        format!("policy-pdp:strategy-deny:{session_id}:{task_id}:mcp::high-risk::invoke"),
        &serde_json::json!({"allow": true}),
        "policy-pdp",
    )
    .await
    .expect("strategy deny seed");

    let intent = CapabilityIntent {
        session_id: session_id.into(),
        objective: "execute high risk task".into(),
        required_tags: vec!["Execution".into()],
        preferred_servers: vec!["local-mcp".into()],
    };

    let enforced_engine = CapabilityAdmissionEngine::with_policy_mode(RuntimePolicyMode::Enforced);
    let enforced = enforced_engine
        .admit_selected(
            &db,
            &tools,
            &providers.factory_artifacts(),
            session_id,
            task_id,
            &identity,
            &intent,
            "mcp::high-risk::invoke",
            Some("local-mcp"),
        )
        .await
        .expect("enforced decision");
    assert!(!enforced.allowed, "enforced mode must deny high-risk task");
    assert!(enforced.reason.contains("pdp enforced deny"));
    assert!(
        enforced
            .evidence_ref
            .as_deref()
            .is_some_and(|value| value.starts_with("evidence:stage:")),
        "enforced denial should be traceable by evidence_ref"
    );

    let shadow_engine = CapabilityAdmissionEngine::with_policy_mode(RuntimePolicyMode::Shadow);
    let shadow = shadow_engine
        .admit_selected(
            &db,
            &tools,
            &providers.factory_artifacts(),
            session_id,
            task_id,
            &identity,
            &intent,
            "mcp::high-risk::invoke",
            Some("local-mcp"),
        )
        .await
        .expect("shadow decision");
    assert!(shadow.allowed, "shadow mode should not block execution");

    let shadow_diffs = db
        .list_knowledge_by_prefix(&format!("policy-pdp:shadow-diff:{session_id}:{task_id}:"))
        .await
        .expect("shadow diffs");
    assert!(
        !shadow_diffs.is_empty(),
        "shadow mode should persist policy diff records"
    );
    let latest = shadow_diffs
        .iter()
        .max_by_key(|record| record.key.as_str())
        .expect("latest shadow diff");
    let latest_json: serde_json::Value =
        serde_json::from_str(&latest.value).expect("shadow diff json");
    let old_allowed = latest_json
        .get("old_decision")
        .and_then(|value| value.get("allowed"))
        .and_then(serde_json::Value::as_bool)
        .expect("old allowed");
    let new_allowed = latest_json
        .get("new_decision")
        .and_then(|value| value.get("allowed"))
        .and_then(serde_json::Value::as_bool)
        .expect("new allowed");
    assert!(old_allowed, "shadow baseline decision should allow");
    assert!(!new_allowed, "pdp decision should deny to create diff");

    let diff_evidence_ref = latest_json
        .get("evidence_ref")
        .and_then(serde_json::Value::as_str)
        .expect("shadow diff evidence_ref");
    let diff_evidence = db
        .get_knowledge(diff_evidence_ref)
        .await
        .expect("load diff evidence");
    assert!(diff_evidence.is_some(), "shadow diff evidence should exist");
}

#[tokio::test]
async fn mask_drop_logs_do_not_leak_sensitive_fields() {
    let db = in_memory_db();
    let stage_key = EvidenceLedgerWriter::append_stage(
        &db,
        "policy-mask-session",
        "trace-mask",
        EvidenceStage::Admission,
        serde_json::json!({
            "secret": "super-secret-token",
            "drop_me": "do-not-store",
            "decision_log_policy": {
                "policy_version": {"id":"policy-mask-v1","revision":1},
                "mask_rules": [{"id":"mask-secret","selector":"secret","strategy":"last4"}],
                "drop_rules": [{"id":"drop-sensitive","selector":"drop_me"}]
            }
        }),
        None,
    )
    .await
    .expect("append stage");

    let stage = db
        .get_knowledge(&stage_key)
        .await
        .expect("get stage")
        .expect("stage exists");
    assert!(!stage.value.contains("super-secret-token"));
    assert!(!stage.value.contains("\"drop_me\":\"do-not-store\""));
    assert!(stage.value.contains("decision_log_artifact"));
}




