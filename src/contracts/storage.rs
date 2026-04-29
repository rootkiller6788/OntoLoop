use std::collections::BTreeMap;

use async_trait::async_trait;

use super::errors::ContractError;

pub const STORAGE_CONTRACT_VERSION: &str = "storage/v3";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageEventKind {
    Knowledge,
    Replay,
    Evidence,
    Policy,
    QueryPlane,
    Custom,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KvEventRecord {
    pub key: String,
    pub value: serde_json::Value,
    pub source: String,
    pub kind: StorageEventKind,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TenantRecord {
    pub tenant_id: String,
    pub name: String,
    pub status: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PrincipalRecord {
    pub principal_id: String,
    pub tenant_id: String,
    pub principal_type: String,
    pub status: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoleBindingRecord {
    pub tenant_id: String,
    pub principal_id: String,
    pub role: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PolicyBindingRecord {
    pub policy_id: String,
    pub tenant_id: String,
    pub role: String,
    pub allowed_actions: Vec<String>,
    pub capability_prefixes: Vec<String>,
    pub max_memory_mb: u32,
    pub max_tokens: u32,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionLeaseRecord {
    pub lease_token: String,
    pub session_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub expires_at_ms: u64,
    pub issued_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BudgetAccountRecord {
    pub account_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub total_budget_micros: u64,
    pub reserved_micros: u64,
    pub spent_micros: u64,
    pub blocked_count: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpendLedgerKind {
    Reserve,
    Settle,
    Refund,
    Blocked,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpendLedgerRecord {
    pub ledger_id: String,
    pub tenant_id: String,
    pub account_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub kind: SpendLedgerKind,
    pub amount_micros: i64,
    pub token_cost_micros: u64,
    pub tool_cost_micros: u64,
    pub duration_cost_micros: u64,
    pub reason: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuotaWindowRecord {
    pub window_id: String,
    pub tenant_id: String,
    pub account_id: String,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
    pub window_budget_micros: u64,
    pub consumed_micros: u64,
    pub blocked_count: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CostAttributionRecord {
    pub attribution_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub provider_tokens: u32,
    pub tool_invocations: u32,
    pub duration_ms: u64,
    pub token_cost_micros: u64,
    pub tool_cost_micros: u64,
    pub duration_cost_micros: u64,
    pub total_cost_micros: u64,
    pub settled_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScheduleEventRecord {
    pub id: u64,
    pub session_id: String,
    pub topic: String,
    pub tool_name: String,
    pub payload: serde_json::Value,
    pub actor_id: String,
    pub status: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentStateRecord {
    pub session_id: String,
    pub last_user_message: String,
    pub last_assistant_message: Option<String>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WalOutcome {
    Committed,
    RolledBack,
    Rejected,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WalRecord {
    pub wal_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub stage: String,
    pub outcome: WalOutcome,
    pub payload: serde_json::Value,
    pub replay_fingerprint: String,
    pub created_at_ms: u64,
}

#[async_trait]
pub trait KvEventStore: Send + Sync {
    async fn upsert_event(&self, record: KvEventRecord) -> Result<KvEventRecord, ContractError>;
    async fn get_event(&self, key: &str) -> Result<Option<KvEventRecord>, ContractError>;
    async fn list_events_by_prefix(&self, prefix: &str) -> Result<Vec<KvEventRecord>, ContractError>;
}

#[async_trait]
pub trait IdentityStore: Send + Sync {
    async fn upsert_tenant(&self, tenant: TenantRecord) -> Result<TenantRecord, ContractError>;
    async fn get_tenant(&self, tenant_id: &str) -> Result<Option<TenantRecord>, ContractError>;
    async fn upsert_principal(
        &self,
        principal: PrincipalRecord,
    ) -> Result<PrincipalRecord, ContractError>;
    async fn get_principal(
        &self,
        tenant_id: &str,
        principal_id: &str,
    ) -> Result<Option<PrincipalRecord>, ContractError>;
    async fn upsert_role_binding(
        &self,
        binding: RoleBindingRecord,
    ) -> Result<RoleBindingRecord, ContractError>;
    async fn get_role_binding(
        &self,
        tenant_id: &str,
        principal_id: &str,
    ) -> Result<Option<RoleBindingRecord>, ContractError>;
    async fn upsert_policy_binding(
        &self,
        binding: PolicyBindingRecord,
    ) -> Result<PolicyBindingRecord, ContractError>;
    async fn get_policy_binding(
        &self,
        tenant_id: &str,
        policy_id: &str,
    ) -> Result<Option<PolicyBindingRecord>, ContractError>;
    async fn upsert_session_lease(
        &self,
        lease: SessionLeaseRecord,
    ) -> Result<SessionLeaseRecord, ContractError>;
    async fn get_session_lease(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionLeaseRecord>, ContractError>;
}

#[async_trait]
pub trait BillingStore: Send + Sync {
    async fn upsert_budget_account(
        &self,
        account: BudgetAccountRecord,
    ) -> Result<BudgetAccountRecord, ContractError>;
    async fn get_budget_account(
        &self,
        tenant_id: &str,
        account_id: &str,
    ) -> Result<Option<BudgetAccountRecord>, ContractError>;
    async fn append_spend_ledger(
        &self,
        record: SpendLedgerRecord,
    ) -> Result<SpendLedgerRecord, ContractError>;
    async fn list_spend_ledger(
        &self,
        tenant_id: &str,
        account_id: &str,
    ) -> Result<Vec<SpendLedgerRecord>, ContractError>;
    async fn upsert_quota_window(
        &self,
        window: QuotaWindowRecord,
    ) -> Result<QuotaWindowRecord, ContractError>;
    async fn get_quota_window(
        &self,
        tenant_id: &str,
        account_id: &str,
        window_id: &str,
    ) -> Result<Option<QuotaWindowRecord>, ContractError>;
    async fn upsert_cost_attribution(
        &self,
        attribution: CostAttributionRecord,
    ) -> Result<CostAttributionRecord, ContractError>;
    async fn list_cost_attribution_by_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<Vec<CostAttributionRecord>, ContractError>;
}

#[async_trait]
pub trait SchedulerStore: Send + Sync {
    async fn create_schedule_event(
        &self,
        event: ScheduleEventRecord,
    ) -> Result<ScheduleEventRecord, ContractError>;
    async fn update_schedule_status(
        &self,
        event_id: u64,
        status: &str,
    ) -> Result<(), ContractError>;
    async fn list_schedule_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<ScheduleEventRecord>, ContractError>;
    async fn upsert_agent_state(
        &self,
        state: AgentStateRecord,
    ) -> Result<AgentStateRecord, ContractError>;
    async fn get_agent_state(
        &self,
        session_id: &str,
    ) -> Result<Option<AgentStateRecord>, ContractError>;
}

#[async_trait]
pub trait WalStore: Send + Sync {
    async fn append_wal(&self, record: WalRecord) -> Result<WalRecord, ContractError>;
    async fn list_wal_by_trace(
        &self,
        session_id: &str,
        trace_id: &str,
    ) -> Result<Vec<WalRecord>, ContractError>;
    async fn latest_wal_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<WalRecord>, ContractError>;
}

pub trait StorageContractV3:
    KvEventStore + IdentityStore + BillingStore + SchedulerStore + WalStore
{
}

impl<T> StorageContractV3 for T where
    T: KvEventStore + IdentityStore + BillingStore + SchedulerStore + WalStore
{
}
