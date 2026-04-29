use std::fs;
use std::path::Path;

use autoloop::{
    contracts::skill_foundry::{RouteDecision, SkillFoundryLayer},
    skills::foundry::build_skill_skeleton,
};

fn route(layer: SkillFoundryLayer) -> RouteDecision {
    RouteDecision {
        decision_id: format!("route:{:?}", layer),
        selected_layer: layer,
        risk_level: "low".to_string(),
        confidence: 0.9,
        reasons: vec![],
        rejected_layers: vec![],
        policy_notes: vec![],
        created_at_ms: 1,
    }
}

#[test]
fn foundry_builder_template_matches_layer_contracts() {
    let root = std::env::temp_dir().join(format!("pq11_foundry_builder_{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp root");

    let s1 = build_skill_skeleton(
        "pq11-s1",
        &route(SkillFoundryLayer::S1PromptOnly),
        root.to_str().expect("root str"),
        11,
    )
    .expect("build s1");
    let s1_root = Path::new(&s1.artifact_path);
    assert!(s1_root.join("SKILL.md").exists());
    assert!(!s1_root.join("scripts").exists());

    let s2 = build_skill_skeleton(
        "pq11-s2",
        &route(SkillFoundryLayer::S2PromptScripts),
        root.to_str().expect("root str"),
        12,
    )
    .expect("build s2");
    let s2_root = Path::new(&s2.artifact_path);
    assert!(s2_root.join("scripts").join("run.sh").exists());
    assert!(!s2_root.join("mcp").exists());

    let s3 = build_skill_skeleton(
        "pq11-s3",
        &route(SkillFoundryLayer::S3PromptMcp),
        root.to_str().expect("root str"),
        13,
    )
    .expect("build s3");
    let s3_root = Path::new(&s3.artifact_path);
    assert!(s3_root.join("scripts").join("run.sh").exists());
    assert!(s3_root.join("mcp").join("schema.json").exists());

    let _ = fs::remove_dir_all(&root);
}



