use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::contracts::skill_foundry::{PackageMeta, RouteDecision, SkillFoundryLayer};

pub fn build_skill_skeleton(
    skill_name: &str,
    route: &RouteDecision,
    workspace_path: &str,
    now_ms: u64,
) -> Result<PackageMeta> {
    let skill_slug = sanitize_skill_name(skill_name);
    let workspace = Path::new(workspace_path);
    fs::create_dir_all(workspace)?;

    let skill_root = workspace.join(&skill_slug);
    let references_dir = skill_root.join("references");
    let scripts_dir = skill_root.join("scripts");
    let tests_dir = skill_root.join("tests");
    let assets_dir = skill_root.join("assets");
    let mcp_dir = skill_root.join("mcp");

    fs::create_dir_all(&references_dir)?;
    fs::create_dir_all(&tests_dir)?;
    fs::create_dir_all(&assets_dir)?;

    match route.selected_layer {
        SkillFoundryLayer::S1PromptOnly => {}
        SkillFoundryLayer::S2PromptScripts => {
            fs::create_dir_all(&scripts_dir)?;
        }
        SkillFoundryLayer::S3PromptMcp => {
            fs::create_dir_all(&scripts_dir)?;
            fs::create_dir_all(&mcp_dir)?;
        }
    }

    fs::write(
        skill_root.join("SKILL.md"),
        render_skill_markdown(skill_name, &route.selected_layer),
    )?;
    fs::write(
        skill_root.join("manifest.json"),
        render_manifest(skill_name, &route.selected_layer, now_ms),
    )?;
    fs::write(
        references_dir.join("README.md"),
        "# References\n\nPut examples, docs, and links here.\n",
    )?;
    fs::write(
        tests_dir.join("smoke.json"),
        "{\n  \"name\": \"foundry-smoke\",\n  \"expected\": \"json\"\n}\n",
    )?;
    fs::write(assets_dir.join(".gitkeep"), "")?;

    if route.selected_layer == SkillFoundryLayer::S2PromptScripts
        || route.selected_layer == SkillFoundryLayer::S3PromptMcp
    {
        fs::write(
            scripts_dir.join("run.sh"),
            "#!/usr/bin/env bash\nset -euo pipefail\necho '{\"ok\":true}'\n",
        )?;
    }
    if route.selected_layer == SkillFoundryLayer::S3PromptMcp {
        fs::write(
            mcp_dir.join("schema.json"),
            "{\n  \"$schema\": \"https://json-schema.org/draft/2020-12/schema\",\n  \"title\": \"mcp_placeholder\",\n  \"type\": \"object\",\n  \"properties\": {}\n}\n",
        )?;
    }

    let mut metadata = BTreeMap::new();
    metadata.insert("skill_root".to_string(), skill_root.display().to_string());
    metadata.insert(
        "scripts_required".to_string(),
        if route.selected_layer == SkillFoundryLayer::S2PromptScripts
            || route.selected_layer == SkillFoundryLayer::S3PromptMcp
        {
            "true".to_string()
        } else {
            "false".to_string()
        },
    );
    metadata.insert(
        "mcp_required".to_string(),
        if route.selected_layer == SkillFoundryLayer::S3PromptMcp {
            "true".to_string()
        } else {
            "false".to_string()
        },
    );

    Ok(PackageMeta {
        package_id: format!("pkg:{}:{}", skill_name, now_ms),
        skill_name: skill_name.to_string(),
        version: "v1".to_string(),
        layer: route.selected_layer.clone(),
        artifact_path: skill_root.display().to_string(),
        install_scope: "local".to_string(),
        digest: None,
        enabled: true,
        metadata,
        created_at_ms: now_ms,
    })
}

fn sanitize_skill_name(skill_name: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in skill_name.chars() {
        let normalized = c.to_ascii_lowercase();
        if normalized.is_ascii_alphanumeric() {
            slug.push(normalized);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "foundry-skill".to_string()
    } else {
        trimmed.to_string()
    }
}

fn render_skill_markdown(skill_name: &str, layer: &SkillFoundryLayer) -> String {
    let layer_hint = match layer {
        SkillFoundryLayer::S1PromptOnly => "S1 prompt-only",
        SkillFoundryLayer::S2PromptScripts => "S2 prompt+scripts",
        SkillFoundryLayer::S3PromptMcp => "S3 prompt+MCP",
    };
    format!(
        "# {skill_name}\n\n## Layer\n{layer_hint}\n\n## Trigger\nDescribe trigger phrases and usage examples.\n\n## Output Contract\nReturn agent-readable structured output.\n"
    )
}

fn render_manifest(skill_name: &str, layer: &SkillFoundryLayer, now_ms: u64) -> String {
    let layer_value = match layer {
        SkillFoundryLayer::S1PromptOnly => "s1_prompt_only",
        SkillFoundryLayer::S2PromptScripts => "s2_prompt_scripts",
        SkillFoundryLayer::S3PromptMcp => "s3_prompt_mcp",
    };
    format!(
        "{{\n  \"id\": \"{}\",\n  \"version\": \"v1\",\n  \"layer\": \"{}\",\n  \"created_at_ms\": {},\n  \"enabled\": true\n}}\n",
        sanitize_skill_name(skill_name),
        layer_value,
        now_ms
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn builder_enforces_s3_mcp_placeholder() {
        let temp_root = std::env::temp_dir().join(format!("foundry_builder_{}", 1775806000000u64));
        let _ = fs::remove_dir_all(&temp_root);
        fs::create_dir_all(&temp_root).expect("create temp root");

        let route = RouteDecision {
            decision_id: "route:test".to_string(),
            selected_layer: SkillFoundryLayer::S3PromptMcp,
            risk_level: "high".to_string(),
            confidence: 0.9,
            reasons: vec!["needs external capability".to_string()],
            rejected_layers: vec![
                SkillFoundryLayer::S1PromptOnly,
                SkillFoundryLayer::S2PromptScripts,
            ],
            policy_notes: vec![],
            created_at_ms: 1775806000000,
        };

        let built = build_skill_skeleton("demo", &route, &temp_root.display().to_string(), 1775806000000)
            .expect("build skeleton");
        let skill_root = PathBuf::from(built.artifact_path);

        assert!(skill_root.join("SKILL.md").exists());
        assert!(skill_root.join("manifest.json").exists());
        assert!(skill_root.join("scripts").join("run.sh").exists());
        assert!(skill_root.join("mcp").join("schema.json").exists());

        let _ = fs::remove_dir_all(&temp_root);
    }
}
