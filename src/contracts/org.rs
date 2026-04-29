use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct QuotaSnapshot {
    pub account_id: Option<String>,
    pub total_budget_micros: u64,
    pub reserved_micros: u64,
    pub spent_micros: u64,
    pub blocked_count: u64,
    pub max_tokens: u32,
    pub max_memory_mb: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct OrganizationContext {
    pub session_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub role: String,
    pub approval_policy: String,
    pub kb_refs: Vec<String>,
    pub plaza_refs: Vec<String>,
    pub quotas: QuotaSnapshot,
    pub metadata: BTreeMap<String, String>,
}
