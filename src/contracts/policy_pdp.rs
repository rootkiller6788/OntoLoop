use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    Off,
    Shadow,
    Enforced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyVersion {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyInput {
    pub tenant_id: String,
    pub subject: String,
    pub action: String,
    #[serde(default)]
    pub resource: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub context_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionReason {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub rule_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MaskRule {
    pub id: String,
    pub selector: String,
    pub strategy: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropRule {
    pub id: String,
    pub selector: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyDecision {
    pub allowed: bool,
    pub mode: PolicyMode,
    pub version: PolicyVersion,
    #[serde(default)]
    pub reasons: Vec<DecisionReason>,
    #[serde(default)]
    pub mask_rules: Vec<MaskRule>,
    #[serde(default)]
    pub drop_rules: Vec<DropRule>,
}
