use std::collections::BTreeMap;

use super::org::QuotaSnapshot;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextItemKind {
    Knowledge,
    Supermemory,
    Session,
    ToolState,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ContextItem {
    pub session_id: String,
    pub item_id: String,
    pub kind: String,
    pub source_ref: String,
    pub permission_scope: String,
    pub priority: f32,
    pub budget_micros: u64,
    pub metadata: BTreeMap<String, String>,
}

impl ContextItem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: impl Into<String>,
        item_id: impl Into<String>,
        kind: ContextItemKind,
        source_ref: impl Into<String>,
        permission_scope: impl Into<String>,
        priority: f32,
        budget_micros: u64,
        metadata: BTreeMap<String, String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            item_id: item_id.into(),
            kind: match kind {
                ContextItemKind::Knowledge => "knowledge",
                ContextItemKind::Supermemory => "supermemory",
                ContextItemKind::Session => "session",
                ContextItemKind::ToolState => "tool_state",
            }
            .to_string(),
            source_ref: source_ref.into(),
            permission_scope: permission_scope.into(),
            priority,
            budget_micros,
            metadata,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct GovernanceContext {
    pub session_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub role: String,
    pub approval_policy: String,
    pub risk_tier: String,
    pub route_policy: Vec<String>,
    pub quotas: QuotaSnapshot,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProjectMemoryPolicy {
    pub retrieval_criteria: Vec<String>,
    pub multilingual: bool,
    pub enable_graph: bool,
}

impl Default for ProjectMemoryPolicy {
    fn default() -> Self {
        Self {
            retrieval_criteria: vec![
                "relevance".to_string(),
                "recency".to_string(),
                "evidence".to_string(),
            ],
            multilingual: true,
            enable_graph: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct KnowledgeContext {
    pub session_id: String,
    pub kb_refs: Vec<String>,
    pub plaza_refs: Vec<String>,
    pub replay_scope_refs: Vec<String>,
    pub playbook_refs: Vec<String>,
    pub org_memory_slice_refs: Vec<String>,
    pub private_memory_refs: Vec<String>,
    pub source_evidence_refs: Vec<String>,
    pub context_bundle_refs: Vec<String>,
    #[serde(default)]
    pub context_items: Vec<ContextItem>,
    pub project_policy: ProjectMemoryPolicy,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct MemoryScopeSpec {
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
    pub actor_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct MemoryScopeContract {
    pub scope: MemoryScopeSpec,
    pub metadata_template: BTreeMap<String, String>,
    pub query_filters: BTreeMap<String, String>,
    pub metadata_filter_dsl: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct UnifiedQueryView {
    pub session_id: String,
    pub trace_id: Option<String>,
    pub metrics: serde_json::Value,
    pub traces: serde_json::Value,
    pub logs: serde_json::Value,
    pub events: serde_json::Value,
    pub ledger: serde_json::Value,
    pub graph: serde_json::Value,
    pub replay: serde_json::Value,
    pub generated_at_ms: u64,
}
