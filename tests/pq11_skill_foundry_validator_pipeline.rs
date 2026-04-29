use std::fs;

use autoloop::{
    contracts::skill_foundry::{RouteDecision, SkillFoundryLayer},
    skills::foundry::{build_skill_skeleton, validate_skill_contract},
};

fn s3_route() -> RouteDecision {
    RouteDecision {
        decision_id: "route:pq11:validator".to_string(),
        selected_layer: SkillFoundryLayer::S3PromptMcp,
        risk_level: "high".to_string(),
        confidence: 0.95,
        reasons: vec![],
        rejected_layers: vec![
            SkillFoundryLayer::S1PromptOnly,
            SkillFoundryLayer::S2PromptScripts,
        ],
        policy_notes: vec![],
        created_at_ms: 1,
    }
}

#[test]
fn foundry_validator_pipeline_covers_pass_and_fail_paths() {
    let root = std::env::temp_dir().join(format!("pq11_foundry_validator_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp root");

    let built = build_skill_skeleton(
        "pq11-validator",
        &s3_route(),
        root.to_str().expect("root str"),
        21,
    )
    .expect("build skeleton");

    let pass_report = validate_skill_contract(&built, 22);
    assert!(pass_report.passed);
    assert_eq!(pass_report.error_count, 0);
    assert!(pass_report
        .checks
        .iter()
        .any(|item| item.name == "s3_mcp_schema_check" && item.passed));

    fs::remove_file(format!("{}\\tests\\smoke.json", built.artifact_path)).expect("remove smoke");
    let fail_report = validate_skill_contract(&built, 23);
    assert!(!fail_report.passed);
    assert!(fail_report.error_count >= 1);
    assert!(fail_report
        .checks
        .iter()
        .any(|item| item.name == "cli_json_contract_check" && !item.passed));

    let _ = fs::remove_dir_all(&root);
}



