use std::collections::BTreeMap;

use autoloop::{
    contracts::skill_foundry::{ExtractionSpec, SkillFoundryLayer},
    skills::foundry::{promotion_suggestion, route_layer},
};

fn base_spec() -> ExtractionSpec {
    ExtractionSpec {
        extraction_id: "extract:pq11".to_string(),
        real_capability: "pq11".to_string(),
        manipulated_state: vec![],
        actions: vec![],
        agent_readable_outputs: vec!["json".to_string()],
        deterministic_surfaces: vec!["contract".to_string()],
        nondeterministic_risks: vec![],
        constraints: BTreeMap::new(),
    }
}

#[test]
fn foundry_router_matrix_routes_s1_s2_s3() {
    let now_ms = 1_775_900_000_001_u64;

    let s1 = base_spec();
    let s1_decision = route_layer(&s1, now_ms);
    assert_eq!(s1_decision.selected_layer, SkillFoundryLayer::S1PromptOnly);

    let mut s2 = base_spec();
    s2.actions = vec!["run_script".to_string(), "validate".to_string()];
    s2.constraints
        .insert("has_script_hint".to_string(), "true".to_string());
    let s2_decision = route_layer(&s2, now_ms + 1);
    assert_eq!(s2_decision.selected_layer, SkillFoundryLayer::S2PromptScripts);

    let mut s3 = base_spec();
    s3.actions = vec!["call_api".to_string()];
    s3.constraints
        .insert("has_api_hint".to_string(), "true".to_string());
    let s3_decision = route_layer(&s3, now_ms + 2);
    assert_eq!(s3_decision.selected_layer, SkillFoundryLayer::S3PromptMcp);
}

#[test]
fn foundry_router_matrix_emits_upward_only_promotion_hints() {
    let now_ms = 1_775_900_000_101_u64;

    let mut s1 = base_spec();
    s1.constraints
        .insert("execution_failure_count".to_string(), "3".to_string());
    let s1_decision = route_layer(&s1, now_ms);
    let s1_hint = promotion_suggestion(&s1, &s1_decision, now_ms + 1).expect("s1 hint");
    assert_eq!(s1_hint.from_layer, SkillFoundryLayer::S1PromptOnly);
    assert_eq!(s1_hint.to_layer, SkillFoundryLayer::S2PromptScripts);
    assert!(s1_hint.recommended);

    let mut s2 = base_spec();
    s2.actions = vec!["run_script".to_string(), "validate".to_string()];
    s2.constraints
        .insert("has_script_hint".to_string(), "true".to_string());
    s2.constraints.insert(
        "capability_boundary_failure_count".to_string(),
        "2".to_string(),
    );
    let s2_decision = route_layer(&s2, now_ms + 2);
    let s2_hint = promotion_suggestion(&s2, &s2_decision, now_ms + 3).expect("s2 hint");
    assert_eq!(s2_hint.from_layer, SkillFoundryLayer::S2PromptScripts);
    assert_eq!(s2_hint.to_layer, SkillFoundryLayer::S3PromptMcp);
    assert!(s2_hint.recommended);
}



