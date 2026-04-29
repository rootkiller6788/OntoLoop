use std::collections::BTreeMap;

use anyhow::Result;

use crate::contracts::skill_foundry::{PackageMeta, RouteDecision};
use crate::skills::{SkillManifest, SkillRegistry};

pub fn build_package_meta(
    skill_name: &str,
    version: &str,
    route: &RouteDecision,
    artifact_path: &str,
    now_ms: u64,
) -> PackageMeta {
    PackageMeta {
        package_id: format!("package:{}:{}", skill_name, now_ms),
        skill_name: skill_name.to_string(),
        version: version.to_string(),
        layer: route.selected_layer.clone(),
        artifact_path: artifact_path.to_string(),
        install_scope: "project".to_string(),
        digest: None,
        enabled: false,
        metadata: BTreeMap::new(),
        created_at_ms: now_ms,
    }
}

pub fn package_skill(
    skill_name: &str,
    version: &str,
    route: &RouteDecision,
    artifact_path: &str,
    now_ms: u64,
) -> PackageMeta {
    let mut meta = build_package_meta(skill_name, version, route, artifact_path, now_ms);
    meta.metadata
        .insert("packaged_by".to_string(), "skill_foundry".to_string());
    meta.metadata.insert(
        "reversible_install".to_string(),
        "true:disable_only_no_delete".to_string(),
    );
    meta
}

pub async fn install_skill(
    registry: &SkillRegistry,
    package: &PackageMeta,
    source: &str,
    markdown: &str,
) -> Result<SkillManifest> {
    let now_ms = crate::orchestration::current_time_ms();
    let signal = crate::memory::LearningSignal {
        signal_id: format!("foundry-packager:{}:{now_ms}", package.skill_name),
        session_id: "global-skill-registry".to_string(),
        trace_id: format!("trace:foundry:install:{}:{now_ms}", package.skill_name),
        source: "skills.foundry.packager.install".to_string(),
        evidence_ref: format!("evidence:skill-registry:foundry-install:{}:{now_ms}", package.skill_name),
        metadata: BTreeMap::new(),
    };
    registry
        .install_from_package(package, source, markdown, &signal)
        .await
}

pub async fn enable_skill(
    registry: &SkillRegistry,
    skill_id: &str,
    reason: &str,
) -> Result<SkillManifest> {
    let now_ms = crate::orchestration::current_time_ms();
    let signal = crate::memory::LearningSignal {
        signal_id: format!("foundry-packager:{skill_id}:{now_ms}"),
        session_id: "global-skill-registry".to_string(),
        trace_id: format!("trace:foundry:enable:{skill_id}:{now_ms}"),
        source: "skills.foundry.packager.enable".to_string(),
        evidence_ref: format!("evidence:skill-registry:foundry-enable:{skill_id}:{now_ms}"),
        metadata: BTreeMap::new(),
    };
    registry.set_enabled(skill_id, true, reason, &signal).await
}

pub async fn disable_skill(
    registry: &SkillRegistry,
    skill_id: &str,
    reason: &str,
) -> Result<SkillManifest> {
    let now_ms = crate::orchestration::current_time_ms();
    let signal = crate::memory::LearningSignal {
        signal_id: format!("foundry-packager:{skill_id}:{now_ms}"),
        session_id: "global-skill-registry".to_string(),
        trace_id: format!("trace:foundry:disable:{skill_id}:{now_ms}"),
        source: "skills.foundry.packager.disable".to_string(),
        evidence_ref: format!("evidence:skill-registry:foundry-disable:{skill_id}:{now_ms}"),
        metadata: BTreeMap::new(),
    };
    registry.set_enabled(skill_id, false, reason, &signal).await
}
