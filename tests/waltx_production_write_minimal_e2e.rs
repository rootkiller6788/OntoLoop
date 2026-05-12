use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::{
        services::{
            ServiceCall, ServiceDomain, attach_service_gate_token, build_service_gate_token,
        },
        skill_foundry::{PromotionHint, SkillFoundryLayer},
    },
};

fn foundry_call(
    session_id: &str,
    trace_id: &str,
    operation: &str,
    input: serde_json::Value,
) -> ServiceCall {
    ServiceCall {
        session_id: session_id.into(),
        trace_id: trace_id.into(),
        service_domain: ServiceDomain::SkillFoundry,
        service_name: "skill_foundry".to_string(),
        operation: operation.to_string(),
        input,
        budget_scope: "default".into(),
        requested_at_ms: 0,
    }
}

fn relation_call(
    session_id: &str,
    trace_id: &str,
    operation: &str,
    mut input: serde_json::Value,
) -> ServiceCall {
    let token = build_service_gate_token(
        &autoloop::contracts::ids::SessionId::from(session_id),
        &ServiceDomain::Relation,
        autoloop::orchestration::current_time_ms(),
    );
    attach_service_gate_token(&mut input, token);
    ServiceCall {
        session_id: session_id.into(),
        trace_id: trace_id.into(),
        service_domain: ServiceDomain::Relation,
        service_name: "relation_facade".to_string(),
        operation: operation.to_string(),
        input,
        budget_scope: "relation".into(),
        requested_at_ms: 0,
    }
}

async fn assert_waltx_write_proof_complete(
    app: &AutoLoopApp,
    session_id: &str,
    op: &str,
) -> anyhow::Result<()> {
    let records = app
        .state_store()
        .list_knowledge_by_prefix(&format!("relation:write_proof:{session_id}:"))
        .await?;
    let proof = records
        .iter()
        .filter_map(|record| {
            serde_json::from_str::<serde_json::Value>(&record.value)
                .ok()
                .map(|value| (record.key.clone(), value))
        })
        .find(|(_, value)| value.get("op").and_then(serde_json::Value::as_str) == Some(op));

    let (key, value) = proof.unwrap_or_else(|| {
        let keys = records
            .iter()
            .map(|record| record.key.clone())
            .collect::<Vec<_>>();
        panic!("expected WalTx write_proof record for op={op}; keys={keys:?}");
    });
    assert!(
        value
            .get("state_hash")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| !v.trim().is_empty()),
        "write_proof missing state_hash for op={op}, key={key}"
    );
    assert!(
        value
            .get("evidence_ref")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| !v.trim().is_empty()),
        "write_proof missing evidence_ref for op={op}, key={key}"
    );
    assert!(
        value
            .get("replay_fp")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| !v.trim().is_empty()),
        "write_proof missing replay_fp for op={op}, key={key}"
    );
    assert!(
        value
            .get("wal_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|v| !v.trim().is_empty()),
        "write_proof missing wal_id for op={op}, key={key}"
    );
    Ok(())
}

#[tokio::test]
async fn minimal_waltx_evidence_complete_for_three_production_write_entries() {
    let mut config = AppConfig::default();
    config.storage.backend = autoloop::config::StorageBackend::Postgres;
    config.storage.postgres.enabled = true;
    config.storage.postgres.uri = std::env::var("ONTOLOOP_TEST_POSTGRES_URI")
        .unwrap_or_else(|_| "postgres://postgres:123456@localhost:5432/postgres".to_string());
    config.storage.shadow_read_preference = "postgres".to_string();
    let app = AutoLoopApp::new(config);
    let session_id = "waltx-minimal-e2e";
    let trace_id = "trace:waltx-minimal-e2e";

    let relation_one = app
        .services
        .mediate_call(&relation_call(
            session_id,
            trace_id,
            "commit_waltx",
            serde_json::json!({
                "operation": "trusted_prior_activate_snapshot",
                "state_key": format!("evolution:trusted-prior:{session_id}:latest"),
                "state_payload": {
                    "trusted_prior": {"prior_id": "prior:test"},
                    "board_decision": "PromoteRuntimeUpdate",
                    "policy_allow": true,
                    "evidence_ref": "evidence:seed:trusted-prior",
                    "deny_reason": "allowed"
                },
                "event_type": "evolution.trusted_prior.activated",
                "event_payload": {
                    "kind": "snapshot",
                    "review_transition": {
                        "from": "accepted",
                        "to": "committed"
                    },
                    "evidence_ref": "evidence:seed:trusted-prior",
                    "write_proof": {
                        "sha256": "seeded-trusted-prior-proof"
                    }
                }
            }),
        ))
        .await
        .expect("commit trusted_prior WalTx");
    assert!(
        relation_one.success,
        "commit_waltx trusted_prior failed: {:?}",
        relation_one.error
    );
    assert_waltx_write_proof_complete(&app, session_id, "trusted_prior_activate_snapshot")
        .await
        .expect("trusted_prior write proof");

    let relation_two = app
        .services
        .mediate_call(&relation_call(
            session_id,
            trace_id,
            "commit_waltx",
            serde_json::json!({
                "operation": "governance_version_write",
                "state_key": format!("policy:evolution:governance:{session_id}:version:test"),
                "state_payload": {
                    "governance_version": "v-test",
                    "board_decision": "PromoteGovernanceContract",
                    "policy_allow": true,
                    "evidence_ref": "evidence:seed:governance",
                    "deny_reason": "allowed"
                },
                "event_type": "governance.config.version_written",
                "event_payload": {
                    "kind": "version",
                    "review_transition": {
                        "from": "accepted",
                        "to": "committed"
                    },
                    "evidence_ref": "evidence:seed:governance",
                    "write_proof": {
                        "sha256": "seeded-governance-proof"
                    }
                }
            }),
        ))
        .await
        .expect("commit governance WalTx");
    assert!(
        relation_two.success,
        "commit_waltx governance failed: {:?}",
        relation_two.error
    );
    assert_waltx_write_proof_complete(&app, session_id, "governance_version_write")
        .await
        .expect("governance write proof");

    let builder = "waltx-skill-builder";
    let hint = PromotionHint {
        hint_id: "hint:waltx-skill".to_string(),
        from_layer: SkillFoundryLayer::S1PromptOnly,
        to_layer: SkillFoundryLayer::S2PromptScripts,
        trigger: "execution_failures".to_string(),
        observed_failures: 3,
        evidence_refs: vec!["evidence:seed:skill".to_string()],
        recommended: true,
        created_at_ms: 1,
    };
    app.state_store()
        .upsert_json_knowledge(
            format!("foundry:promotion:pending:{session_id}:{builder}:latest"),
            &hint,
            "test-seed",
        )
        .await
        .expect("seed promotion hint");
    let resp = app
        .services
        .mediate_call(&foundry_call(
            session_id,
            "trace:waltx-skill",
            "approve_promotion",
            serde_json::json!({
                "builder": builder,
                "requested_by": "principal:test",
                "hint_id": hint.hint_id,
            }),
        ))
        .await
        .expect("approve promotion");
    assert!(
        resp.success,
        "approve_promotion should succeed: {:?}",
        resp.error
    );
    assert_waltx_write_proof_complete(&app, session_id, "skill_promotion_approval")
        .await
        .expect("skill promotion write proof");
}
