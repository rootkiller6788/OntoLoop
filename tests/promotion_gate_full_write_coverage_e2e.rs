use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::{
        services::{ServiceCall, ServiceDomain},
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

#[tokio::test]
async fn trusted_prior_and_next_gen_writes_include_production_gate_triad() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "promotion-gate-trusted-prior";

    let _ = app
        .process_direct(session_id, "trigger evolution shadow and trusted prior activation")
        .await;

    let trusted_prior = app
        .state_store()
        .get_knowledge(&format!("evolution:trusted-prior:{session_id}:latest"))
        .await
        .expect("db read trusted prior latest");
    let trusted_prior = if let Some(record) = trusted_prior {
        record
    } else {
        app.state_store()
            .list_knowledge_by_prefix(&format!("evolution:trusted-prior:block:{session_id}:"))
            .await
            .expect("db list trusted prior block")
            .into_iter()
            .max_by(|left, right| left.key.cmp(&right.key))
            .expect("trusted prior blocked record exists")
    };
    let trusted_prior_json: serde_json::Value =
        serde_json::from_str(&trusted_prior.value).expect("trusted prior json");
    for key in ["board_decision", "policy_allow", "evidence_ref", "deny_reason"] {
        assert!(
            trusted_prior_json.get(key).is_some(),
            "trusted prior write missing gate field {key}"
        );
    }
    assert!(
        trusted_prior_json
            .get("evidence_ref")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        "trusted prior write must carry non-empty evidence_ref"
    );

    let next_gen = app
        .state_store()
        .get_knowledge(&format!("evolution:next-gen:{session_id}:latest"))
        .await
        .expect("db read next gen")
        .expect("next gen snapshot exists");
    let next_gen_json: serde_json::Value =
        serde_json::from_str(&next_gen.value).expect("next gen json");
    for key in ["board_decision", "policy_allow", "evidence_ref", "deny_reason"] {
        assert!(
            next_gen_json.get(key).is_some(),
            "next-gen write missing gate field {key}"
        );
    }
}

#[tokio::test]
async fn foundry_promotion_write_persists_gate_triad() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "promotion-gate-skill";
    let trace_id = "trace:promotion-gate-skill";
    let builder = "promotion-gate-skill-builder";
    let hint = PromotionHint {
        hint_id: "hint:promotion-gate-skill".to_string(),
        from_layer: SkillFoundryLayer::S1PromptOnly,
        to_layer: SkillFoundryLayer::S2PromptScripts,
        trigger: "execution_failures".to_string(),
        observed_failures: 3,
        evidence_refs: vec!["evidence:foundry:seed".to_string()],
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
        .expect("seed pending promotion hint");

    let response = app
        .services
        .mediate_call(&foundry_call(
            session_id,
            trace_id,
            "approve_promotion",
            serde_json::json!({
                "builder": builder,
                "requested_by": "principal:test",
                "hint_id": hint.hint_id,
            }),
        ))
        .await
        .expect("approve promotion");
    assert!(response.success);
    assert_ne!(
        response
            .output
            .get("status")
            .and_then(serde_json::Value::as_str),
        Some("blocked"),
        "approve_promotion unexpectedly blocked by write gate"
    );

    let layer_state = app
        .state_store()
        .get_knowledge(&format!("foundry:skill-layer:{builder}:latest"))
        .await
        .expect("read layer state")
        .expect("layer state persisted");
    let layer_state_json: serde_json::Value =
        serde_json::from_str(&layer_state.value).expect("layer state json");
    for key in ["board_decision", "policy_allow", "evidence_ref", "deny_reason"] {
        assert!(
            layer_state_json.get(key).is_some(),
            "skill promotion write missing gate field {key}"
        );
    }
    assert!(
        layer_state_json
            .get("evidence_ref")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        "skill promotion write must carry non-empty evidence_ref"
    );
}
