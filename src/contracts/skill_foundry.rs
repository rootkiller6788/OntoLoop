use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillFoundryLayer {
    S1PromptOnly,
    S2PromptScripts,
    S3PromptMcp,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FoundryIntake {
    pub intake_id: String,
    pub task_name: String,
    pub concrete_examples: Vec<String>,
    pub negative_examples: Vec<String>,
    pub expected_output: String,
    pub existing_software: Vec<String>,
    pub existing_apis: Vec<String>,
    pub existing_scripts: Vec<String>,
    pub requested_by: String,
    pub session_id: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtractionSpec {
    pub extraction_id: String,
    pub real_capability: String,
    pub manipulated_state: Vec<String>,
    pub actions: Vec<String>,
    pub agent_readable_outputs: Vec<String>,
    pub deterministic_surfaces: Vec<String>,
    pub nondeterministic_risks: Vec<String>,
    pub constraints: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RouteDecision {
    pub decision_id: String,
    pub selected_layer: SkillFoundryLayer,
    pub risk_level: String,
    pub confidence: f32,
    pub reasons: Vec<String>,
    pub rejected_layers: Vec<SkillFoundryLayer>,
    pub policy_notes: Vec<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PromotionHint {
    pub hint_id: String,
    pub from_layer: SkillFoundryLayer,
    pub to_layer: SkillFoundryLayer,
    pub trigger: String,
    pub observed_failures: u32,
    pub evidence_refs: Vec<String>,
    pub recommended: bool,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationCheck {
    pub check_id: String,
    pub name: String,
    pub passed: bool,
    pub severity: String,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationReport {
    pub validation_id: String,
    pub skill_name: String,
    pub layer: SkillFoundryLayer,
    pub passed: bool,
    pub checks: Vec<ValidationCheck>,
    pub warning_count: u32,
    pub error_count: u32,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PackageMeta {
    pub package_id: String,
    pub skill_name: String,
    pub version: String,
    pub layer: SkillFoundryLayer,
    pub artifact_path: String,
    pub install_scope: String,
    pub digest: Option<String>,
    pub enabled: bool,
    pub metadata: BTreeMap<String, String>,
    pub created_at_ms: u64,
}


#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FoundryPromotionPolicy {
    pub policy_id: String,
    pub s1_execution_failure_threshold: u32,
    pub s2_boundary_failure_threshold: u32,
    pub max_counted_failures: u32,
    pub manual_approval_required: bool,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FoundrySkillLayerState {
    pub skill_name: String,
    pub current_layer: SkillFoundryLayer,
    pub approved_hint_id: Option<String>,
    pub reason: String,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub board_decision: Option<String>,
    #[serde(default)]
    pub policy_allow: Option<bool>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub deny_reason: Option<String>,
}
