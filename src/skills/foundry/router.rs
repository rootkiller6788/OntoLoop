use crate::contracts::skill_foundry::{
    ExtractionSpec, PromotionHint, RouteDecision, SkillFoundryLayer,
};

pub fn route_layer(spec: &ExtractionSpec, now_ms: u64) -> RouteDecision {
    let mut reasons = Vec::new();
    let has_call_api = contains_action(&spec.actions, "call_api");
    let has_script = contains_action(&spec.actions, "run_script");
    let action_count = spec.actions.len();
    let has_external_risk = spec
        .nondeterministic_risks
        .iter()
        .any(|item| contains_any(item, &["external", "remote", "network", "webhook"]));
    let has_script_hint = parse_bool_constraint(spec, "has_script_hint");
    let has_api_hint = parse_bool_constraint(spec, "has_api_hint");

    let selected = if has_call_api || has_api_hint || has_external_risk {
        reasons.push("external capability surface detected".to_string());
        SkillFoundryLayer::S3PromptMcp
    } else if has_script || has_script_hint || action_count >= 2 {
        reasons.push("repeatable scriptable workflow detected".to_string());
        SkillFoundryLayer::S2PromptScripts
    } else {
        reasons.push("method-oriented workflow without external capability".to_string());
        SkillFoundryLayer::S1PromptOnly
    };

    let risk_level = compute_risk_level(spec, has_call_api, has_script);
    reasons.push(format!("risk_level={risk_level}"));

    let confidence = compute_confidence(selected.clone(), spec, has_external_risk);
    let rejected_layers = match selected {
        SkillFoundryLayer::S1PromptOnly => {
            vec![SkillFoundryLayer::S2PromptScripts, SkillFoundryLayer::S3PromptMcp]
        }
        SkillFoundryLayer::S2PromptScripts => {
            vec![SkillFoundryLayer::S1PromptOnly, SkillFoundryLayer::S3PromptMcp]
        }
        SkillFoundryLayer::S3PromptMcp => {
            vec![SkillFoundryLayer::S1PromptOnly, SkillFoundryLayer::S2PromptScripts]
        }
    };

    let policy_notes = vec![
        "promotion_policy=upward_only".to_string(),
        "auto_mutation=disabled".to_string(),
    ];

    RouteDecision {
        decision_id: format!("route:{}:{}", spec.extraction_id, now_ms),
        selected_layer: selected,
        risk_level,
        confidence,
        reasons,
        rejected_layers,
        policy_notes,
        created_at_ms: now_ms,
    }
}

pub fn promotion_suggestion(
    spec: &ExtractionSpec,
    decision: &RouteDecision,
    now_ms: u64,
) -> Option<PromotionHint> {
    let execution_failures = parse_u32_constraint(spec, "execution_failure_count");
    let boundary_failures = parse_u32_constraint(spec, "capability_boundary_failure_count");
    let negative_failures = spec
        .nondeterministic_risks
        .iter()
        .filter(|item| item.to_ascii_lowercase().contains("negative_example:"))
        .count() as u32;

    match decision.selected_layer {
        SkillFoundryLayer::S1PromptOnly => {
            let observed = execution_failures.max(negative_failures);
            if observed >= 2 {
                return Some(PromotionHint {
                    hint_id: format!("promotion:{}:{}", decision.decision_id, now_ms),
                    from_layer: SkillFoundryLayer::S1PromptOnly,
                    to_layer: SkillFoundryLayer::S2PromptScripts,
                    trigger: "execution_failure_threshold_reached".to_string(),
                    observed_failures: observed,
                    evidence_refs: vec![spec.extraction_id.clone()],
                    recommended: true,
                    created_at_ms: now_ms,
                });
            }
        }
        SkillFoundryLayer::S2PromptScripts => {
            let observed = boundary_failures.max(negative_failures);
            if observed >= 2 {
                return Some(PromotionHint {
                    hint_id: format!("promotion:{}:{}", decision.decision_id, now_ms),
                    from_layer: SkillFoundryLayer::S2PromptScripts,
                    to_layer: SkillFoundryLayer::S3PromptMcp,
                    trigger: "capability_boundary_threshold_reached".to_string(),
                    observed_failures: observed,
                    evidence_refs: vec![spec.extraction_id.clone()],
                    recommended: true,
                    created_at_ms: now_ms,
                });
            }
        }
        SkillFoundryLayer::S3PromptMcp => {}
    }
    None
}

fn contains_action(actions: &[String], target: &str) -> bool {
    actions
        .iter()
        .any(|action| action.eq_ignore_ascii_case(target))
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    let lowered = value.to_ascii_lowercase();
    needles.iter().any(|needle| lowered.contains(needle))
}

fn parse_bool_constraint(spec: &ExtractionSpec, key: &str) -> bool {
    spec.constraints
        .get(key)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn parse_u32_constraint(spec: &ExtractionSpec, key: &str) -> u32 {
    spec.constraints
        .get(key)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0)
}

fn compute_risk_level(spec: &ExtractionSpec, has_call_api: bool, has_script: bool) -> String {
    let state_high_risk = spec
        .manipulated_state
        .iter()
        .any(|state| contains_any(state, &["policy", "database", "tenant"]));
    let state_medium_risk = spec
        .manipulated_state
        .iter()
        .any(|state| contains_any(state, &["repository", "artifact", "index"]));

    if has_call_api || state_high_risk {
        "high".to_string()
    } else if has_script || state_medium_risk {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

fn compute_confidence(
    selected: SkillFoundryLayer,
    spec: &ExtractionSpec,
    has_external_risk: bool,
) -> f32 {
    let mut confidence = 0.62_f32;
    if !spec.actions.is_empty() {
        confidence += 0.1;
    }
    if !spec.agent_readable_outputs.is_empty() {
        confidence += 0.08;
    }
    if !spec.deterministic_surfaces.is_empty() {
        confidence += 0.08;
    }
    if has_external_risk && selected == SkillFoundryLayer::S3PromptMcp {
        confidence += 0.07;
    }
    if selected == SkillFoundryLayer::S1PromptOnly && spec.actions.len() <= 1 {
        confidence += 0.05;
    }
    confidence.clamp(0.0, 0.99)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::skill_foundry::ExtractionSpec;
    use std::collections::BTreeMap;

    fn base_spec() -> ExtractionSpec {
        ExtractionSpec {
            extraction_id: "extract:test".to_string(),
            real_capability: "test".to_string(),
            manipulated_state: vec![],
            actions: vec![],
            agent_readable_outputs: vec!["json".to_string()],
            deterministic_surfaces: vec!["contract".to_string()],
            nondeterministic_risks: vec![],
            constraints: BTreeMap::new(),
        }
    }

    #[test]
    fn routes_to_s3_when_external_capability_is_required() {
        let mut spec = base_spec();
        spec.actions = vec!["call_api".to_string()];
        spec.constraints
            .insert("has_api_hint".to_string(), "true".to_string());

        let decision = route_layer(&spec, 1);
        assert_eq!(decision.selected_layer, SkillFoundryLayer::S3PromptMcp);
        assert_eq!(decision.risk_level, "high");
        assert!(decision.confidence > 0.6);
    }

    #[test]
    fn routes_to_s2_for_repeatable_script_workflow() {
        let mut spec = base_spec();
        spec.actions = vec!["run_script".to_string(), "validate".to_string()];
        spec.constraints
            .insert("has_script_hint".to_string(), "true".to_string());

        let decision = route_layer(&spec, 2);
        assert_eq!(decision.selected_layer, SkillFoundryLayer::S2PromptScripts);
        assert_eq!(decision.risk_level, "medium");
    }

    #[test]
    fn emits_upward_only_promotion_suggestions() {
        let mut spec = base_spec();
        spec.constraints
            .insert("execution_failure_count".to_string(), "3".to_string());

        let decision = route_layer(&spec, 3);
        assert_eq!(decision.selected_layer, SkillFoundryLayer::S1PromptOnly);
        let hint = promotion_suggestion(&spec, &decision, 4).expect("hint");
        assert_eq!(hint.from_layer, SkillFoundryLayer::S1PromptOnly);
        assert_eq!(hint.to_layer, SkillFoundryLayer::S2PromptScripts);
        assert!(hint.recommended);
    }
}
