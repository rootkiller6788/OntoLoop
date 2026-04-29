use std::collections::BTreeMap;

use crate::contracts::skill_foundry::{ExtractionSpec, FoundryIntake};

pub fn extract_first_principles(intake: &FoundryIntake) -> ExtractionSpec {
    let mut all_inputs = Vec::new();
    all_inputs.extend(intake.concrete_examples.clone());
    all_inputs.extend(intake.negative_examples.clone());
    all_inputs.extend(intake.existing_scripts.clone());
    all_inputs.extend(intake.existing_apis.clone());
    all_inputs.extend(intake.existing_software.clone());
    all_inputs.push(intake.expected_output.clone());

    let real_capability = derive_real_capability(intake);
    let actions = derive_actions(&all_inputs);
    let manipulated_state = derive_state_surface(&all_inputs);
    let (deterministic_surfaces, nondeterministic_risks) =
        derive_determinism_profile(&all_inputs, &intake.negative_examples);
    let agent_readable_outputs = derive_output_contract(intake);
    let constraints = derive_constraints(intake, &actions);

    ExtractionSpec {
        extraction_id: format!("extract:{}:{}", intake.intake_id, intake.created_at_ms),
        real_capability,
        manipulated_state,
        actions,
        agent_readable_outputs,
        deterministic_surfaces,
        nondeterministic_risks,
        constraints,
    }
}

fn derive_real_capability(intake: &FoundryIntake) -> String {
    if !intake.task_name.trim().is_empty() {
        return intake.task_name.trim().to_string();
    }
    let fallback = intake
        .expected_output
        .split('\n')
        .next()
        .unwrap_or("general capability")
        .trim();
    if fallback.is_empty() {
        "general capability".to_string()
    } else {
        fallback.to_string()
    }
}

fn derive_actions(inputs: &[String]) -> Vec<String> {
    let action_keywords = [
        "create", "update", "delete", "list", "search", "build", "validate", "extract", "route",
        "compile", "package", "install", "enable", "disable", "sync", "export", "import",
        "retry", "resume", "rollback", "approve", "reject",
    ];
    let mut actions = std::collections::BTreeSet::new();
    for raw in inputs {
        let lowered = raw.to_ascii_lowercase();
        for keyword in action_keywords {
            if lowered.contains(keyword) {
                actions.insert(keyword.to_string());
            }
        }
        if lowered.contains("curl ")
            || lowered.contains("http://")
            || lowered.contains("https://")
            || lowered.contains(" api ")
            || lowered.ends_with(" api")
        {
            actions.insert("call_api".to_string());
        }
        if lowered.contains("cargo ")
            || lowered.contains("python ")
            || lowered.contains("npm ")
            || lowered.contains("bash ")
            || lowered.contains("powershell ")
            || lowered.contains("script")
        {
            actions.insert("run_script".to_string());
        }
    }
    actions.into_iter().collect()
}

fn derive_state_surface(inputs: &[String]) -> Vec<String> {
    let mappings = [
        ("file", "filesystem"),
        ("repo", "repository"),
        ("branch", "repository_branch"),
        ("database", "database_records"),
        ("db", "database_records"),
        ("memory", "memory_records"),
        ("index", "retrieval_index"),
        ("cache", "cache_state"),
        ("session", "session_state"),
        ("policy", "policy_config"),
        ("plugin", "plugin_registry"),
        ("skill", "skill_registry"),
        ("task", "task_queue"),
        ("queue", "task_queue"),
        ("artifact", "artifact_store"),
    ];
    let mut states = std::collections::BTreeSet::new();
    for raw in inputs {
        let lowered = raw.to_ascii_lowercase();
        for (needle, state) in mappings {
            if lowered.contains(needle) {
                states.insert(state.to_string());
            }
        }
    }
    states.into_iter().collect()
}

fn derive_determinism_profile(
    inputs: &[String],
    negative_examples: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut deterministic = std::collections::BTreeSet::new();
    let mut nondeterministic = std::collections::BTreeSet::new();

    for raw in inputs {
        let lowered = raw.to_ascii_lowercase();
        if lowered.contains("json")
            || lowered.contains("schema")
            || lowered.contains("id")
            || lowered.contains("version")
            || lowered.contains("contract")
        {
            deterministic.insert("structured_contract_output".to_string());
        }
        if lowered.contains("script")
            || lowered.contains("cli")
            || lowered.contains("command")
            || lowered.contains("deterministic")
        {
            deterministic.insert("replayable_command_surface".to_string());
        }

        if lowered.contains("network")
            || lowered.contains("remote")
            || lowered.contains("webhook")
            || lowered.contains("external")
            || lowered.contains("timeout")
            || lowered.contains("latency")
            || lowered.contains("manual")
            || lowered.contains("human")
            || lowered.contains("random")
        {
            nondeterministic.insert("external_or_runtime_variance".to_string());
        }
    }

    for raw in negative_examples {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            nondeterministic.insert(format!("negative_example: {}", trimmed));
        }
    }

    (
        deterministic.into_iter().collect(),
        nondeterministic.into_iter().collect(),
    )
}

fn derive_output_contract(intake: &FoundryIntake) -> Vec<String> {
    let mut outputs = Vec::new();
    let expected = intake.expected_output.trim();
    if !expected.is_empty() {
        outputs.push(expected.to_string());
    }

    let lowered = expected.to_ascii_lowercase();
    if lowered.contains("json") {
        outputs.push("machine_readable:json".to_string());
    }
    if lowered.contains("markdown") || lowered.contains("md") {
        outputs.push("human_readable:markdown".to_string());
    }
    if lowered.contains("table") {
        outputs.push("renderable:table".to_string());
    }
    if outputs.is_empty() {
        outputs.push("machine_readable:plain_text".to_string());
    }
    outputs
}

fn derive_constraints(intake: &FoundryIntake, actions: &[String]) -> BTreeMap<String, String> {
    let mut constraints = BTreeMap::new();
    constraints.insert("requires_expected_output".to_string(), "true".to_string());
    constraints.insert(
        "source_examples_count".to_string(),
        intake.concrete_examples.len().to_string(),
    );
    constraints.insert(
        "negative_examples_count".to_string(),
        intake.negative_examples.len().to_string(),
    );
    constraints.insert(
        "has_script_hint".to_string(),
        (!intake.existing_scripts.is_empty()).to_string(),
    );
    constraints.insert(
        "has_api_hint".to_string(),
        (!intake.existing_apis.is_empty()).to_string(),
    );
    constraints.insert("action_count".to_string(), actions.len().to_string());
    constraints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extractor_derives_five_principles_from_rich_inputs() {
        let intake = FoundryIntake {
            intake_id: "intake:test".to_string(),
            task_name: "sync plugin release".to_string(),
            concrete_examples: vec![
                "create release notes and update index".to_string(),
                "export json contract and validate schema".to_string(),
            ],
            negative_examples: vec!["manual copy-paste caused mismatch".to_string()],
            expected_output: "JSON summary with deterministic fields".to_string(),
            existing_software: vec!["git repo + release pipeline".to_string()],
            existing_apis: vec!["https://api.example.com/releases".to_string()],
            existing_scripts: vec!["powershell script build-release.ps1".to_string()],
            requested_by: "principal:test".to_string(),
            session_id: "session:test".to_string(),
            created_at_ms: 1,
        };

        let spec = extract_first_principles(&intake);
        assert_eq!(spec.real_capability, "sync plugin release");
        assert!(!spec.actions.is_empty());
        assert!(!spec.manipulated_state.is_empty());
        assert!(
            spec.agent_readable_outputs
                .iter()
                .any(|item| item.contains("JSON"))
        );
        assert!(!spec.deterministic_surfaces.is_empty());
        assert!(!spec.nondeterministic_risks.is_empty());
        assert_eq!(
            spec.constraints.get("requires_expected_output"),
            Some(&"true".to_string())
        );
    }
}
