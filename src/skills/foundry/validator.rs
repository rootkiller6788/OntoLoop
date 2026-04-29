use std::fs;
use std::path::Path;

use crate::contracts::skill_foundry::{PackageMeta, SkillFoundryLayer, ValidationCheck, ValidationReport};

pub fn validate_skill_contract(meta: &PackageMeta, now_ms: u64) -> ValidationReport {
    let root = Path::new(&meta.artifact_path);
    let mut checks = Vec::new();

    checks.push(check_frontmatter(meta, root));
    checks.push(check_trigger_wording(meta, root));
    checks.push(check_cli_json_contract(meta, root));

    match meta.layer {
        SkillFoundryLayer::S1PromptOnly => {}
        SkillFoundryLayer::S2PromptScripts => {
            checks.push(check_s2_script_smoke(meta, root));
        }
        SkillFoundryLayer::S3PromptMcp => {
            checks.push(check_s2_script_smoke(meta, root));
            checks.push(check_s3_mcp_schema(meta, root));
            checks.push(check_s3_mcp_connectivity_mock(meta, root));
        }
    }

    let warning_count = checks
        .iter()
        .filter(|check| check.severity.eq_ignore_ascii_case("warning") && !check.passed)
        .count() as u32;
    let error_count = checks
        .iter()
        .filter(|check| check.severity.eq_ignore_ascii_case("error") && !check.passed)
        .count() as u32;

    ValidationReport {
        validation_id: format!("validate:{}:{}", meta.package_id, now_ms),
        skill_name: meta.skill_name.clone(),
        layer: meta.layer.clone(),
        passed: error_count == 0,
        warning_count,
        error_count,
        checks,
        generated_at_ms: now_ms,
    }
}

fn check_frontmatter(meta: &PackageMeta, root: &Path) -> ValidationCheck {
    let id = format!("{}:frontmatter", meta.package_id);
    let skill_md = root.join("SKILL.md");
    let content = fs::read_to_string(&skill_md).unwrap_or_default();
    let starts_with_h1 = content
        .lines()
        .next()
        .map(|line| line.trim_start().starts_with("# "))
        .unwrap_or(false);
    let has_trigger_section = content.contains("## Trigger");
    let passed = starts_with_h1 && has_trigger_section;
    ValidationCheck {
        check_id: id,
        name: "frontmatter_lint".to_string(),
        passed,
        severity: "error".to_string(),
        detail: if passed {
            "SKILL.md has minimal frontmatter contract (# title + ## Trigger).".to_string()
        } else {
            "SKILL.md missing title or trigger section required by template.".to_string()
        },
    }
}

fn check_trigger_wording(meta: &PackageMeta, root: &Path) -> ValidationCheck {
    let id = format!("{}:trigger", meta.package_id);
    let skill_md = root.join("SKILL.md");
    let content = fs::read_to_string(&skill_md).unwrap_or_default().to_ascii_lowercase();
    let has_words = content.contains("trigger") && (content.contains("usage") || content.contains("example"));
    ValidationCheck {
        check_id: id,
        name: "trigger_wording_check".to_string(),
        passed: has_words,
        severity: "warning".to_string(),
        detail: if has_words {
            "Trigger wording and usage hints present.".to_string()
        } else {
            "Trigger wording too thin; add explicit trigger phrases + usage examples.".to_string()
        },
    }
}

fn check_cli_json_contract(meta: &PackageMeta, root: &Path) -> ValidationCheck {
    let id = format!("{}:cli_json_contract", meta.package_id);
    let smoke = root.join("tests").join("smoke.json");
    let parsed = fs::read_to_string(&smoke)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    let passed = parsed
        .as_ref()
        .map(|value| value.is_object() && value.get("name").is_some())
        .unwrap_or(false);
    ValidationCheck {
        check_id: id,
        name: "cli_json_contract_check".to_string(),
        passed,
        severity: "error".to_string(),
        detail: if passed {
            "tests/smoke.json parsed and contains contract name.".to_string()
        } else {
            "tests/smoke.json missing or invalid JSON contract.".to_string()
        },
    }
}

fn check_s2_script_smoke(meta: &PackageMeta, root: &Path) -> ValidationCheck {
    let id = format!("{}:s2_script_smoke", meta.package_id);
    let script = root.join("scripts").join("run.sh");
    let content = fs::read_to_string(&script).unwrap_or_default();
    let passed = script.exists() && content.contains("echo") && content.contains("{\"ok\":true}");
    ValidationCheck {
        check_id: id,
        name: "s2_script_smoke".to_string(),
        passed,
        severity: "error".to_string(),
        detail: if passed {
            "scripts/run.sh exists with deterministic smoke output.".to_string()
        } else {
            "scripts/run.sh missing or smoke output contract not found.".to_string()
        },
    }
}

fn check_s3_mcp_schema(meta: &PackageMeta, root: &Path) -> ValidationCheck {
    let id = format!("{}:s3_mcp_schema", meta.package_id);
    let schema_file = root.join("mcp").join("schema.json");
    let parsed = fs::read_to_string(&schema_file)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    let passed = parsed
        .as_ref()
        .map(|value| {
            value.is_object()
                && value
                    .get("$schema")
                    .and_then(|v| v.as_str())
                    .map(|v| v.contains("json-schema.org"))
                    .unwrap_or(false)
                && value
                    .get("type")
                    .and_then(|v| v.as_str())
                    .map(|v| v == "object")
                    .unwrap_or(false)
        })
        .unwrap_or(false);

    ValidationCheck {
        check_id: id,
        name: "s3_mcp_schema_check".to_string(),
        passed,
        severity: "error".to_string(),
        detail: if passed {
            "mcp/schema.json is valid placeholder schema.".to_string()
        } else {
            "mcp/schema.json missing or invalid schema contract.".to_string()
        },
    }
}

fn check_s3_mcp_connectivity_mock(meta: &PackageMeta, root: &Path) -> ValidationCheck {
    let id = format!("{}:s3_mcp_connectivity_mock", meta.package_id);
    let has_schema = root.join("mcp").join("schema.json").exists();
    ValidationCheck {
        check_id: id,
        name: "s3_mcp_connectivity_mock".to_string(),
        passed: has_schema,
        severity: "warning".to_string(),
        detail: if has_schema {
            "Mock connectivity passed (schema presence as preflight gate).".to_string()
        } else {
            "Mock connectivity failed because schema preflight is absent.".to_string()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::skill_foundry::SkillFoundryLayer;
    use std::collections::BTreeMap;

    #[test]
    fn validator_passes_s3_template_contract() {
        let temp_root = std::env::temp_dir().join(format!("foundry_validator_{}", 1775807000000u64));
        let _ = fs::remove_dir_all(&temp_root);
        fs::create_dir_all(temp_root.join("tests")).expect("tests dir");
        fs::create_dir_all(temp_root.join("scripts")).expect("scripts dir");
        fs::create_dir_all(temp_root.join("mcp")).expect("mcp dir");
        fs::write(
            temp_root.join("SKILL.md"),
            "# demo\n\n## Trigger\nusage example\n",
        )
        .expect("skill");
        fs::write(
            temp_root.join("tests").join("smoke.json"),
            "{ \"name\": \"smoke\" }",
        )
        .expect("smoke");
        fs::write(
            temp_root.join("scripts").join("run.sh"),
            "#!/usr/bin/env bash\necho '{\"ok\":true}'\n",
        )
        .expect("run.sh");
        fs::write(
            temp_root.join("mcp").join("schema.json"),
            "{ \"$schema\": \"https://json-schema.org/draft/2020-12/schema\", \"type\": \"object\" }",
        )
        .expect("schema");

        let meta = PackageMeta {
            package_id: "pkg:test".to_string(),
            skill_name: "demo".to_string(),
            version: "v1".to_string(),
            layer: SkillFoundryLayer::S3PromptMcp,
            artifact_path: temp_root.display().to_string(),
            install_scope: "local".to_string(),
            digest: None,
            enabled: true,
            metadata: BTreeMap::new(),
            created_at_ms: 1775807000000,
        };

        let report = validate_skill_contract(&meta, 1775807000001);
        assert!(report.passed);
        assert_eq!(report.error_count, 0);
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.name == "s3_mcp_schema_check" && check.passed)
        );

        let _ = fs::remove_dir_all(&temp_root);
    }
}
