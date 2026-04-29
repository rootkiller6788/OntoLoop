use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_postgres::NoTls;

static EVIDENCE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresDbConfig {
    pub enabled: bool,
    pub uri: String,
    pub schema: String,
    pub pool_size: usize,
    #[serde(default = "default_auto_migrate")]
    pub auto_migrate: bool,
}

fn default_auto_migrate() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    #[serde(alias = "Read")]
    Read,
    #[serde(alias = "Write")]
    Write,
    #[serde(alias = "Dispatch")]
    Dispatch,
    #[serde(alias = "Admin")]
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningEventKind {
    Failure,
    Success,
    ToolCall,
    RouteDecision,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleEvent {
    pub id: u64,
    pub session_id: String,
    pub topic: String,
    pub tool_name: String,
    pub payload: String,
    pub actor_id: String,
    pub status: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: u64,
    pub session_id: String,
    pub last_user_message: String,
    pub last_assistant_message: Option<String>,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeRecord {
    pub id: u64,
    pub key: String,
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionGrant {
    pub actor_id: String,
    pub permissions: Vec<PermissionAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub tenant_id: String,
    pub name: String,
    pub status: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub principal_id: String,
    pub tenant_id: String,
    pub principal_type: String,
    pub status: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBinding {
    pub tenant_id: String,
    pub principal_id: String,
    pub role: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBinding {
    pub policy_id: String,
    pub tenant_id: String,
    pub role: String,
    pub allowed_actions: Vec<PermissionAction>,
    pub capability_prefixes: Vec<String>,
    pub max_memory_mb: u32,
    pub max_tokens: u32,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLease {
    pub lease_token: String,
    pub session_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub expires_at_ms: u64,
    pub issued_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAccount {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpendLedgerKind {
    Reserve,
    Settle,
    Refund,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendLedger {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAttribution {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflexionEpisodeRecord {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub hypothesis: String,
    pub outcome: String,
    pub lesson: String,
    pub status: String,
    pub score: f32,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillLibraryRecord {
    pub id: String,
    pub session_id: String,
    pub name: String,
    pub trigger: String,
    pub procedure: String,
    pub confidence: f32,
    pub success_rate: f32,
    pub evidence_count: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalEdgeRecord {
    pub id: String,
    pub session_id: String,
    pub cause: String,
    pub effect: String,
    pub evidence: String,
    pub strength: f32,
    pub confidence: f32,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSessionRecord {
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub status: String,
    pub priority: f32,
    pub summary: String,
    pub started_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WitnessLogRecord {
    pub id: String,
    pub session_id: String,
    pub event_type: LearningEventKind,
    pub source: String,
    pub detail: String,
    pub score: f32,
    pub created_at_ms: u64,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicRelationWriteInput {
    pub session_id: String,
    pub trace_id: String,
    pub state_key: String,
    pub state_payload: serde_json::Value,
    pub relation_event_key: String,
    pub relation_event_payload: serde_json::Value,
    pub evidence_key: String,
    pub evidence_payload: serde_json::Value,
    pub write_proof_key: String,
    pub write_proof_payload: serde_json::Value,
    pub source: String,
    #[serde(default)]
    pub edge_current: Option<RelationEdgeCurrentWrite>,
    #[serde(default)]
    pub event_append: Option<RelationEventAppendWrite>,
    #[serde(default)]
    pub hot_index_entries: Vec<RelationHotIndexWrite>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationEdgeCurrentWrite {
    pub edge_id: String,
    pub from_node: String,
    pub to_node: String,
    pub edge_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationEventAppendWrite {
    pub event_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationHotIndexWrite {
    pub hot_key: String,
    pub relation_kind: String,
    pub relation_ref: String,
    pub score: f64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationEdgeCurrentRecord {
    pub session_id: String,
    pub trace_id: String,
    pub edge_id: String,
    pub from_node: String,
    pub to_node: String,
    pub edge_type: String,
    pub payload: serde_json::Value,
    pub evidence_ref: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationEventRecord {
    pub session_id: String,
    pub trace_id: String,
    pub event_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub evidence_ref: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationHotIndexRecord {
    pub session_id: String,
    pub trace_id: String,
    pub hot_key: String,
    pub relation_kind: String,
    pub relation_ref: String,
    pub score: f64,
    pub payload: serde_json::Value,
    pub evidence_ref: String,
    pub updated_at_ms: u64,
}

#[derive(Clone)]
pub struct PostgresDb {
    config: PostgresDbConfig,
}

impl PostgresDb {
    pub fn new(config: PostgresDbConfig) -> Self {
        Self { config }
    }

    pub async fn try_from_config(config: &PostgresDbConfig) -> Result<Self> {
        let db = Self::new(config.clone());
        db.ensure_ready().await?;
        Ok(db)
    }

    pub async fn from_config(config: &PostgresDbConfig) -> Self {
        Self::try_from_config(config)
            .await
            .expect("failed to initialize PostgresDb adapter")
    }

    pub fn validate(&self) -> Result<()> {
        if self.config.enabled && self.config.uri.trim().is_empty() {
            bail!("storage.postgres.uri must not be empty when enabled");
        }
        if self.config.enabled && self.config.pool_size == 0 {
            bail!("storage.postgres.pool_size must be greater than 0");
        }
        if !is_safe_identifier(&self.config.schema) {
            bail!("storage.postgres.schema contains invalid characters");
        }
        Ok(())
    }

    pub async fn ensure_ready(&self) -> Result<()> {
        self.validate()?;
        if self.config.auto_migrate {
            self.migrate().await?;
        } else {
            let _ = self.connect().await?;
        }
        Ok(())
    }

    async fn connect(&self) -> Result<tokio_postgres::Client> {
        let (client, connection) = tokio_postgres::connect(&self.config.uri, NoTls).await?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(client)
    }

    async fn migrate(&self) -> Result<()> {
        let schema = &self.config.schema;
        let client = self.connect().await?;
        client
            .batch_execute(&format!(
                r#"
CREATE SCHEMA IF NOT EXISTS {schema};

CREATE TABLE IF NOT EXISTS {schema}.kv_records (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    source TEXT NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_kv_records_key_prefix ON {schema}.kv_records (key text_pattern_ops);
CREATE INDEX IF NOT EXISTS idx_kv_records_source ON {schema}.kv_records (source);

CREATE TABLE IF NOT EXISTS {schema}.schedule_events (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL,
    topic TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    payload TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    status TEXT NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_schedule_events_session_id
    ON {schema}.schedule_events(session_id);
CREATE INDEX IF NOT EXISTS idx_schedule_events_status
    ON {schema}.schedule_events(status);

CREATE TABLE IF NOT EXISTS {schema}.agent_states (
    session_id TEXT PRIMARY KEY,
    last_user_message TEXT NOT NULL,
    last_assistant_message TEXT,
    evidence_ref TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS {schema}.permission_grants (
    actor_id TEXT PRIMARY KEY,
    permissions TEXT[] NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS {schema}.identity_tenants (
    tenant_id TEXT PRIMARY KEY,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS {schema}.identity_principals (
    tenant_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, principal_id)
);
CREATE TABLE IF NOT EXISTS {schema}.identity_role_bindings (
    tenant_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, principal_id)
);
CREATE TABLE IF NOT EXISTS {schema}.identity_policy_bindings (
    tenant_id TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, policy_id)
);
CREATE TABLE IF NOT EXISTS {schema}.identity_session_leases (
    session_id TEXT PRIMARY KEY,
    lease_token TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_identity_session_leases_tenant
    ON {schema}.identity_session_leases(tenant_id);

CREATE TABLE IF NOT EXISTS {schema}.billing_budget_accounts (
    tenant_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, account_id)
);
CREATE TABLE IF NOT EXISTS {schema}.billing_spend_ledger (
    tenant_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    ledger_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, account_id, ledger_id)
);
CREATE INDEX IF NOT EXISTS idx_billing_spend_ledger_session_task
    ON {schema}.billing_spend_ledger(session_id, task_id);
CREATE TABLE IF NOT EXISTS {schema}.billing_quota_windows (
    tenant_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    window_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, account_id, window_id)
);
CREATE TABLE IF NOT EXISTS {schema}.billing_cost_attribution (
    tenant_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    attribution_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, session_id, attribution_id)
);

CREATE TABLE IF NOT EXISTS {schema}.event_log (
    event_id BIGSERIAL PRIMARY KEY,
    stream_key TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_event_log_stream_key ON {schema}.event_log(stream_key);
CREATE INDEX IF NOT EXISTS idx_event_log_created_at ON {schema}.event_log(created_at DESC);

CREATE TABLE IF NOT EXISTS {schema}.relation_edges (
    session_id TEXT NOT NULL,
    edge_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    from_node TEXT NOT NULL,
    to_node TEXT NOT NULL,
    edge_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, edge_id)
);
CREATE INDEX IF NOT EXISTS idx_relation_edges_session_updated
    ON {schema}.relation_edges(session_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_relation_edges_trace_updated
    ON {schema}.relation_edges(trace_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_relation_edges_type
    ON {schema}.relation_edges(edge_type);
CREATE INDEX IF NOT EXISTS idx_relation_edges_nodes
    ON {schema}.relation_edges(from_node, to_node);

CREATE TABLE IF NOT EXISTS {schema}.relation_events (
    event_pk BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_relation_events_session_created
    ON {schema}.relation_events(session_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_relation_events_trace_created
    ON {schema}.relation_events(trace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_relation_events_id
    ON {schema}.relation_events(event_id);

CREATE TABLE IF NOT EXISTS {schema}.relation_hot_index (
    session_id TEXT NOT NULL,
    hot_key TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    relation_kind TEXT NOT NULL,
    relation_ref TEXT NOT NULL,
    score DOUBLE PRECISION NOT NULL DEFAULT 0,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (session_id, hot_key)
);
CREATE INDEX IF NOT EXISTS idx_relation_hot_index_session_score
    ON {schema}.relation_hot_index(session_id, score DESC, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_relation_hot_index_trace_updated
    ON {schema}.relation_hot_index(trace_id, updated_at DESC);

ALTER TABLE {schema}.schedule_events ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.agent_states ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.kv_records ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.permission_grants ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.permission_grants ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW();
ALTER TABLE {schema}.identity_tenants ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.identity_principals ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.identity_role_bindings ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.identity_policy_bindings ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.identity_session_leases ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.billing_budget_accounts ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.billing_spend_ledger ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.billing_quota_windows ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.billing_cost_attribution ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.event_log ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.relation_edges ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.relation_events ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE {schema}.relation_hot_index ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_event_log_evidence_ref ON {schema}.event_log(evidence_ref);
CREATE INDEX IF NOT EXISTS idx_relation_edges_evidence_ref ON {schema}.relation_edges(evidence_ref);
CREATE INDEX IF NOT EXISTS idx_relation_events_evidence_ref ON {schema}.relation_events(evidence_ref);
CREATE INDEX IF NOT EXISTS idx_relation_hot_index_evidence_ref ON {schema}.relation_hot_index(evidence_ref);
"#
            ))
            .await?;
        Ok(())
    }

    async fn append_event_log_tx(
        tx: &tokio_postgres::Transaction<'_>,
        schema: &str,
        stream_key: &str,
        event_type: &str,
        payload_json: &str,
        evidence_ref: &str,
        source: &str,
    ) -> Result<()> {
        if std::env::var("AUTOLOOP_PG_ATOMIC_FAILPOINT")
            .ok()
            .as_deref()
            == Some("before_event_log")
        {
            bail!("atomic failpoint before event_log insert");
        }
        tx.execute(
            &format!(
                "INSERT INTO {schema}.event_log(stream_key, event_type, payload, evidence_ref, source)
                 VALUES ($1, $2, ($3::text)::jsonb, $4, $5)"
            ),
            &[&stream_key, &event_type, &payload_json, &evidence_ref, &source],
        )
        .await?;
        let row = tx
            .query_one(
                &format!(
                    "SELECT COUNT(*) FROM {schema}.event_log
                     WHERE stream_key = $1 AND event_type = $2 AND evidence_ref = $3"
                ),
                &[&stream_key, &event_type, &evidence_ref],
            )
            .await?;
        let count: i64 = row.get(0);
        if count != 1 {
            bail!(
                "atomic audit assertion failed: stream_key={stream_key} event_type={event_type} evidence_ref={evidence_ref} count={count}"
            );
        }
        Ok(())
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn next_evidence_ref(scope: &str) -> String {
        let seq = EVIDENCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("evidence:{scope}:{}:{seq}", Self::now_ms())
    }

    async fn upsert_kv_tx(
        tx: &tokio_postgres::Transaction<'_>,
        schema: &str,
        key: &str,
        value: &str,
        source: &str,
        evidence_ref: &str,
    ) -> Result<()> {
        tx.execute(
            &format!(
                r#"
INSERT INTO {schema}.kv_records (key, value, source, evidence_ref)
VALUES ($1, $2, $3, $4)
ON CONFLICT (key) DO UPDATE
SET value = EXCLUDED.value,
    source = EXCLUDED.source,
    evidence_ref = EXCLUDED.evidence_ref,
    updated_at = NOW()
"#
            ),
            &[&key, &value, &source, &evidence_ref],
        )
        .await?;
        Ok(())
    }

    async fn upsert_relation_edge_tx(
        tx: &tokio_postgres::Transaction<'_>,
        schema: &str,
        session_id: &str,
        trace_id: &str,
        edge: &RelationEdgeCurrentWrite,
        evidence_ref: &str,
    ) -> Result<()> {
        tx.execute(
            &format!(
                r#"
INSERT INTO {schema}.relation_edges(
    session_id, edge_id, trace_id, from_node, to_node, edge_type, payload, evidence_ref
)
VALUES ($1, $2, $3, $4, $5, $6, ($7::text)::jsonb, $8)
ON CONFLICT (session_id, edge_id) DO UPDATE
SET trace_id = EXCLUDED.trace_id,
    from_node = EXCLUDED.from_node,
    to_node = EXCLUDED.to_node,
    edge_type = EXCLUDED.edge_type,
    payload = EXCLUDED.payload,
    evidence_ref = EXCLUDED.evidence_ref,
    updated_at = NOW()
"#
            ),
            &[
                &session_id,
                &edge.edge_id,
                &trace_id,
                &edge.from_node,
                &edge.to_node,
                &edge.edge_type,
                &edge.payload.to_string(),
                &evidence_ref,
            ],
        )
        .await?;
        Ok(())
    }

    async fn append_relation_event_tx(
        tx: &tokio_postgres::Transaction<'_>,
        schema: &str,
        session_id: &str,
        trace_id: &str,
        event: &RelationEventAppendWrite,
        evidence_ref: &str,
    ) -> Result<()> {
        tx.execute(
            &format!(
                r#"
INSERT INTO {schema}.relation_events(
    session_id, trace_id, event_id, event_type, payload, evidence_ref
)
VALUES ($1, $2, $3, $4, ($5::text)::jsonb, $6)
"#
            ),
            &[
                &session_id,
                &trace_id,
                &event.event_id,
                &event.event_type,
                &event.payload.to_string(),
                &evidence_ref,
            ],
        )
        .await?;
        Ok(())
    }

    async fn upsert_relation_hot_index_tx(
        tx: &tokio_postgres::Transaction<'_>,
        schema: &str,
        session_id: &str,
        trace_id: &str,
        item: &RelationHotIndexWrite,
        evidence_ref: &str,
    ) -> Result<()> {
        tx.execute(
            &format!(
                r#"
INSERT INTO {schema}.relation_hot_index(
    session_id, hot_key, trace_id, relation_kind, relation_ref, score, payload, evidence_ref
)
VALUES ($1, $2, $3, $4, $5, $6, ($7::text)::jsonb, $8)
ON CONFLICT (session_id, hot_key) DO UPDATE
SET trace_id = EXCLUDED.trace_id,
    relation_kind = EXCLUDED.relation_kind,
    relation_ref = EXCLUDED.relation_ref,
    score = EXCLUDED.score,
    payload = EXCLUDED.payload,
    evidence_ref = EXCLUDED.evidence_ref,
    updated_at = NOW()
"#
            ),
            &[
                &session_id,
                &item.hot_key,
                &trace_id,
                &item.relation_kind,
                &item.relation_ref,
                &item.score,
                &item.payload.to_string(),
                &evidence_ref,
            ],
        )
        .await?;
        Ok(())
    }

    pub async fn atomic_write_relation_bundle(
        &self,
        input: AtomicRelationWriteInput,
    ) -> Result<String> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let evidence_ref = Self::next_evidence_ref("relation.atomic_write");
        let tx = client.transaction().await?;

        Self::upsert_kv_tx(
            &tx,
            schema,
            &input.state_key,
            &input.state_payload.to_string(),
            &input.source,
            &evidence_ref,
        )
        .await?;
        Self::upsert_kv_tx(
            &tx,
            schema,
            &input.relation_event_key,
            &input.relation_event_payload.to_string(),
            &input.source,
            &evidence_ref,
        )
        .await?;
        Self::upsert_kv_tx(
            &tx,
            schema,
            &input.evidence_key,
            &input.evidence_payload.to_string(),
            &input.source,
            &evidence_ref,
        )
        .await?;
        Self::upsert_kv_tx(
            &tx,
            schema,
            &input.write_proof_key,
            &input.write_proof_payload.to_string(),
            &input.source,
            &evidence_ref,
        )
        .await?;

        if let Some(edge) = input.edge_current.as_ref() {
            Self::upsert_relation_edge_tx(
                &tx,
                schema,
                &input.session_id,
                &input.trace_id,
                edge,
                &evidence_ref,
            )
            .await?;
        }
        if let Some(event) = input.event_append.as_ref() {
            Self::append_relation_event_tx(
                &tx,
                schema,
                &input.session_id,
                &input.trace_id,
                event,
                &evidence_ref,
            )
            .await?;
        }
        for item in &input.hot_index_entries {
            Self::upsert_relation_hot_index_tx(
                &tx,
                schema,
                &input.session_id,
                &input.trace_id,
                item,
                &evidence_ref,
            )
            .await?;
        }

        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("relation:{}:{}", input.session_id, input.trace_id),
            "relation.atomic_bundle_written",
            &serde_json::json!({
                "session_id": input.session_id,
                "trace_id": input.trace_id,
                "state_key": input.state_key,
                "relation_event_key": input.relation_event_key,
                "evidence_key": input.evidence_key,
                "write_proof_key": input.write_proof_key,
            })
            .to_string(),
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;

        tx.commit().await?;
        Ok(evidence_ref)
    }

    pub async fn list_relation_edges(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<RelationEdgeCurrentRecord>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT session_id, trace_id, edge_id, from_node, to_node, edge_type, payload::text, evidence_ref, EXTRACT(EPOCH FROM updated_at)::BIGINT * 1000
                     FROM {schema}.relation_edges
                     WHERE session_id = $1
                     ORDER BY updated_at DESC
                     LIMIT $2"
                ),
                &[&session_id, &(limit.max(1) as i64)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| RelationEdgeCurrentRecord {
                session_id: row.get(0),
                trace_id: row.get(1),
                edge_id: row.get(2),
                from_node: row.get(3),
                to_node: row.get(4),
                edge_type: row.get(5),
                payload: serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(6))
                    .unwrap_or_else(|_| serde_json::json!({})),
                evidence_ref: row.get(7),
                updated_at_ms: row.get::<_, i64>(8).max(0) as u64,
            })
            .collect())
    }

    pub async fn list_relation_events(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<RelationEventRecord>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT session_id, trace_id, event_id, event_type, payload::text, evidence_ref, EXTRACT(EPOCH FROM created_at)::BIGINT * 1000
                     FROM {schema}.relation_events
                     WHERE session_id = $1
                     ORDER BY created_at DESC, event_pk DESC
                     LIMIT $2"
                ),
                &[&session_id, &(limit.max(1) as i64)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| RelationEventRecord {
                session_id: row.get(0),
                trace_id: row.get(1),
                event_id: row.get(2),
                event_type: row.get(3),
                payload: serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(4))
                    .unwrap_or_else(|_| serde_json::json!({})),
                evidence_ref: row.get(5),
                created_at_ms: row.get::<_, i64>(6).max(0) as u64,
            })
            .collect())
    }

    pub async fn list_relation_hot_index(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<RelationHotIndexRecord>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT session_id, trace_id, hot_key, relation_kind, relation_ref, score, payload::text, evidence_ref, EXTRACT(EPOCH FROM updated_at)::BIGINT * 1000
                     FROM {schema}.relation_hot_index
                     WHERE session_id = $1
                     ORDER BY score DESC, updated_at DESC
                     LIMIT $2"
                ),
                &[&session_id, &(limit.max(1) as i64)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| RelationHotIndexRecord {
                session_id: row.get(0),
                trace_id: row.get(1),
                hot_key: row.get(2),
                relation_kind: row.get(3),
                relation_ref: row.get(4),
                score: row.get(5),
                payload: serde_json::from_str::<serde_json::Value>(&row.get::<_, String>(6))
                    .unwrap_or_else(|_| serde_json::json!({})),
                evidence_ref: row.get(7),
                updated_at_ms: row.get::<_, i64>(8).max(0) as u64,
            })
            .collect())
    }

    pub async fn upsert_knowledge(
        &self,
        key: String,
        value: String,
        source: String,
    ) -> Result<KnowledgeRecord> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let evidence_ref = Self::next_evidence_ref("knowledge.upsert");
        let tx = client.transaction().await?;
        let row = tx
            .query_one(
                &format!(
                    r#"
INSERT INTO {schema}.kv_records (key, value, source, evidence_ref)
VALUES ($1, $2, $3, $4)
ON CONFLICT (key) DO UPDATE
SET value = EXCLUDED.value,
    source = EXCLUDED.source,
    evidence_ref = EXCLUDED.evidence_ref,
    updated_at = NOW()
RETURNING key, value, source, evidence_ref
"#
                ),
                &[&key, &value, &source, &evidence_ref],
            )
            .await?;
        let record = KnowledgeRecord {
            id: 0,
            key: row.get(0),
            value: row.get(1),
            source: row.get(2),
        };
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("kv:key:{}", record.key),
            "kv.upsert",
            &serde_json::json!({
                "key": record.key,
                "source": record.source,
            })
            .to_string(),
            &row.get::<_, String>(3),
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    pub async fn upsert_json_knowledge<T: Serialize>(
        &self,
        key: impl Into<String>,
        value: &T,
        source: impl Into<String>,
    ) -> Result<KnowledgeRecord> {
        self.upsert_knowledge(key.into(), serde_json::to_string(value)?, source.into())
            .await
    }

    pub async fn get_knowledge(&self, key: &str) -> Result<Option<KnowledgeRecord>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let row = client
            .query_opt(
                &format!("SELECT key, value, source FROM {schema}.kv_records WHERE key = $1"),
                &[&key],
            )
            .await?;
        Ok(row.map(|row| KnowledgeRecord {
            id: 0,
            key: row.get(0),
            value: row.get(1),
            source: row.get(2),
        }))
    }

    pub async fn list_knowledge_by_prefix(&self, prefix: &str) -> Result<Vec<KnowledgeRecord>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT key, value, source FROM {schema}.kv_records WHERE key LIKE ($1 || '%') ORDER BY key"
                ),
                &[&prefix],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| KnowledgeRecord {
                id: 0,
                key: row.get(0),
                value: row.get(1),
                source: row.get(2),
            })
            .collect())
    }

    pub async fn grant_permissions(
        &self,
        actor_id: impl Into<String>,
        permissions: Vec<PermissionAction>,
    ) -> Result<PermissionGrant> {
        let actor_id = actor_id.into();
        let encoded: Vec<String> = permissions.iter().map(|p| format!("{p:?}").to_lowercase()).collect();
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let evidence_ref = Self::next_evidence_ref("permissions.grant");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.permission_grants(actor_id, permissions, evidence_ref) VALUES ($1, $2, $3)
                     ON CONFLICT(actor_id) DO UPDATE
                     SET permissions = EXCLUDED.permissions,
                         evidence_ref = EXCLUDED.evidence_ref,
                         updated_at = NOW()"
                ),
                &[&actor_id, &encoded, &evidence_ref],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("permission:grant:{actor_id}"),
            "permission.grant_upserted",
            &serde_json::json!({
                "actor_id": actor_id,
                "permissions": encoded,
            })
            .to_string(),
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(PermissionGrant {
            actor_id,
            permissions,
        })
    }

    pub async fn has_permission(&self, actor_id: &str, action: PermissionAction) -> Result<bool> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let row = client
            .query_opt(
                &format!("SELECT permissions FROM {schema}.permission_grants WHERE actor_id = $1"),
                &[&actor_id],
            )
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let permissions: Vec<String> = row.get(0);
        Ok(permissions.contains(&format!("{action:?}").to_lowercase()))
    }

    pub async fn enforce_permission(&self, actor_id: &str, action: PermissionAction) -> Result<()> {
        if !self.has_permission(actor_id, action).await? {
            bail!("actor '{actor_id}' does not have permission '{action:?}'");
        }
        Ok(())
    }

    pub async fn create_schedule_event(
        &self,
        session_id: String,
        topic: String,
        tool_name: String,
        payload: String,
        actor_id: String,
    ) -> Result<ScheduleEvent> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let evidence_ref = Self::next_evidence_ref("schedule_event.create");
        let tx = client.transaction().await?;
        let row = tx
            .query_one(
                &format!(
                    "INSERT INTO {schema}.schedule_events(session_id, topic, tool_name, payload, actor_id, status, evidence_ref)
                     VALUES ($1, $2, $3, $4, $5, 'queued', $6)
                     RETURNING id, session_id, topic, tool_name, payload, actor_id, status, evidence_ref"
                ),
                &[&session_id, &topic, &tool_name, &payload, &actor_id, &evidence_ref],
            )
            .await?;
        let event = ScheduleEvent {
            id: row.get::<_, i64>(0) as u64,
            session_id: row.get(1),
            topic: row.get(2),
            tool_name: row.get(3),
            payload: row.get(4),
            actor_id: row.get(5),
            status: row.get(6),
            evidence_ref: row.get(7),
        };
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("schedule:event:{}:{}", event.session_id, event.id),
            "schedule_event.created",
            &serde_json::to_string(&event)?,
            &event.evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(event)
    }

    pub async fn update_schedule_status(&self, event_id: u64, status: impl Into<String>) -> Result<()> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let status = status.into();
        let evidence_ref = Self::next_evidence_ref("schedule_event.status");
        let tx = client.transaction().await?;
        let updated = tx
            .execute(
                &format!("UPDATE {schema}.schedule_events SET status = $1, evidence_ref = $2, updated_at = NOW() WHERE id = $3"),
                &[&status, &evidence_ref, &(event_id as i64)],
            )
            .await?;
        if updated != 1 {
            bail!("schedule event {event_id} not found");
        }
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("schedule:event:{event_id}"),
            "schedule_event.status_updated",
            &serde_json::json!({ "event_id": event_id, "status": status, "at_ms": Self::now_ms() }).to_string(),
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_schedule_events(&self, session_id: &str) -> Result<Vec<ScheduleEvent>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT id, session_id, topic, tool_name, payload, actor_id, status, evidence_ref
                     FROM {schema}.schedule_events WHERE session_id = $1 ORDER BY id"
                ),
                &[&session_id],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| ScheduleEvent {
                id: row.get::<_, i64>(0) as u64,
                session_id: row.get(1),
                topic: row.get(2),
                tool_name: row.get(3),
                payload: row.get(4),
                actor_id: row.get(5),
                status: row.get(6),
                evidence_ref: row.get(7),
            })
            .collect())
    }

    pub async fn upsert_agent_state(
        &self,
        session_id: String,
        last_user_message: String,
        last_assistant_message: Option<String>,
    ) -> Result<AgentState> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let evidence_ref = Self::next_evidence_ref("agent_state.upsert");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.agent_states(session_id, last_user_message, last_assistant_message, evidence_ref)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT(session_id) DO UPDATE
                     SET last_user_message = EXCLUDED.last_user_message,
                         last_assistant_message = EXCLUDED.last_assistant_message,
                         evidence_ref = EXCLUDED.evidence_ref"
                ),
                &[&session_id, &last_user_message, &last_assistant_message, &evidence_ref],
            )
            .await?;
        let state = AgentState {
            id: 0,
            session_id,
            last_user_message,
            last_assistant_message,
            evidence_ref,
        };
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("agent:state:{}", state.session_id),
            "agent_state.upserted",
            &serde_json::to_string(&state)?,
            &state.evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(state)
    }

    pub async fn get_agent_state(&self, session_id: &str) -> Result<Option<AgentState>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let row = client
            .query_opt(
                &format!(
                    "SELECT session_id, last_user_message, last_assistant_message, evidence_ref FROM {schema}.agent_states WHERE session_id = $1"
                ),
                &[&session_id],
            )
            .await?;
        Ok(row.map(|row| AgentState {
            id: 0,
            session_id: row.get(0),
            last_user_message: row.get(1),
            last_assistant_message: row.get(2),
            evidence_ref: row.get(3),
        }))
    }

    pub async fn upsert_tenant(&self, tenant: Tenant) -> Result<Tenant> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&tenant)?;
        let evidence_ref = Self::next_evidence_ref("identity.tenant");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.identity_tenants(tenant_id, payload, evidence_ref)
                     VALUES ($1, ($2::text)::jsonb, $3)
                     ON CONFLICT(tenant_id) DO UPDATE
                     SET payload = EXCLUDED.payload, evidence_ref = EXCLUDED.evidence_ref, updated_at = NOW()"
                ),
                &[&tenant.tenant_id, &payload, &evidence_ref],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("identity:tenant:{}", tenant.tenant_id),
            "identity.tenant_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(tenant)
    }

    pub async fn get_tenant(&self, tenant_id: &str) -> Result<Option<Tenant>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!("SELECT payload::text FROM {schema}.identity_tenants WHERE tenant_id = $1"),
                &[&tenant_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<Tenant>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn upsert_principal(&self, principal: Principal) -> Result<Principal> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&principal)?;
        let evidence_ref = Self::next_evidence_ref("identity.principal");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.identity_principals(tenant_id, principal_id, payload, evidence_ref)
                     VALUES ($1, $2, ($3::text)::jsonb, $4)
                     ON CONFLICT(tenant_id, principal_id) DO UPDATE
                     SET payload = EXCLUDED.payload, evidence_ref = EXCLUDED.evidence_ref, updated_at = NOW()"
                ),
                &[&principal.tenant_id, &principal.principal_id, &payload, &evidence_ref],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!(
                "identity:principal:{}:{}",
                principal.tenant_id, principal.principal_id
            ),
            "identity.principal_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(principal)
    }

    pub async fn get_principal(
        &self,
        tenant_id: &str,
        principal_id: &str,
    ) -> Result<Option<Principal>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!(
                    "SELECT payload::text FROM {schema}.identity_principals
                     WHERE tenant_id = $1 AND principal_id = $2"
                ),
                &[&tenant_id, &principal_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<Principal>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn upsert_role_binding(&self, binding: RoleBinding) -> Result<RoleBinding> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&binding)?;
        let evidence_ref = Self::next_evidence_ref("identity.role_binding");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.identity_role_bindings(tenant_id, principal_id, payload, evidence_ref)
                     VALUES ($1, $2, ($3::text)::jsonb, $4)
                     ON CONFLICT(tenant_id, principal_id) DO UPDATE
                     SET payload = EXCLUDED.payload, evidence_ref = EXCLUDED.evidence_ref, updated_at = NOW()"
                ),
                &[&binding.tenant_id, &binding.principal_id, &payload, &evidence_ref],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!(
                "identity:role-binding:{}:{}",
                binding.tenant_id, binding.principal_id
            ),
            "identity.role_binding_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(binding)
    }

    pub async fn get_role_binding(
        &self,
        tenant_id: &str,
        principal_id: &str,
    ) -> Result<Option<RoleBinding>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!(
                    "SELECT payload::text FROM {schema}.identity_role_bindings
                     WHERE tenant_id = $1 AND principal_id = $2"
                ),
                &[&tenant_id, &principal_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<RoleBinding>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn upsert_policy_binding(&self, binding: PolicyBinding) -> Result<PolicyBinding> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&binding)?;
        let evidence_ref = Self::next_evidence_ref("identity.policy_binding");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.identity_policy_bindings(tenant_id, policy_id, payload, evidence_ref)
                     VALUES ($1, $2, ($3::text)::jsonb, $4)
                     ON CONFLICT(tenant_id, policy_id) DO UPDATE
                     SET payload = EXCLUDED.payload, evidence_ref = EXCLUDED.evidence_ref, updated_at = NOW()"
                ),
                &[&binding.tenant_id, &binding.policy_id, &payload, &evidence_ref],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!(
                "identity:policy-binding:{}:{}",
                binding.tenant_id, binding.policy_id
            ),
            "identity.policy_binding_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(binding)
    }

    pub async fn get_policy_binding(
        &self,
        tenant_id: &str,
        policy_id: &str,
    ) -> Result<Option<PolicyBinding>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!(
                    "SELECT payload::text FROM {schema}.identity_policy_bindings
                     WHERE tenant_id = $1 AND policy_id = $2"
                ),
                &[&tenant_id, &policy_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<PolicyBinding>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn upsert_session_lease(&self, lease: SessionLease) -> Result<SessionLease> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&lease)?;
        let evidence_ref = Self::next_evidence_ref("identity.session_lease");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.identity_session_leases
                     (session_id, lease_token, tenant_id, principal_id, policy_id, payload, evidence_ref)
                     VALUES ($1, $2, $3, $4, $5, ($6::text)::jsonb, $7)
                     ON CONFLICT(session_id) DO UPDATE
                     SET lease_token = EXCLUDED.lease_token,
                         tenant_id = EXCLUDED.tenant_id,
                         principal_id = EXCLUDED.principal_id,
                         policy_id = EXCLUDED.policy_id,
                         payload = EXCLUDED.payload,
                         evidence_ref = EXCLUDED.evidence_ref,
                         updated_at = NOW()"
                ),
                &[
                    &lease.session_id,
                    &lease.lease_token,
                    &lease.tenant_id,
                    &lease.principal_id,
                    &lease.policy_id,
                    &payload,
                    &evidence_ref,
                ],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!("identity:session-lease:{}", lease.session_id),
            "identity.session_lease_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(lease)
    }

    pub async fn get_session_lease(&self, session_id: &str) -> Result<Option<SessionLease>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!(
                    "SELECT payload::text FROM {schema}.identity_session_leases WHERE session_id = $1"
                ),
                &[&session_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<SessionLease>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn upsert_budget_account(&self, account: BudgetAccount) -> Result<BudgetAccount> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&account)?;
        let evidence_ref = Self::next_evidence_ref("billing.budget_account");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.billing_budget_accounts(tenant_id, account_id, payload, evidence_ref)
                     VALUES ($1, $2, ($3::text)::jsonb, $4)
                     ON CONFLICT(tenant_id, account_id) DO UPDATE
                     SET payload = EXCLUDED.payload, evidence_ref = EXCLUDED.evidence_ref, updated_at = NOW()"
                ),
                &[&account.tenant_id, &account.account_id, &payload, &evidence_ref],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!(
                "billing:budget-account:{}:{}",
                account.tenant_id, account.account_id
            ),
            "billing.budget_account_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(account)
    }

    pub async fn get_budget_account(
        &self,
        tenant_id: &str,
        account_id: &str,
    ) -> Result<Option<BudgetAccount>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!(
                    "SELECT payload::text FROM {schema}.billing_budget_accounts
                     WHERE tenant_id = $1 AND account_id = $2"
                ),
                &[&tenant_id, &account_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<BudgetAccount>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn append_spend_ledger(&self, entry: SpendLedger) -> Result<SpendLedger> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&entry)?;
        let evidence_ref = Self::next_evidence_ref("billing.spend_ledger");
        let tx = client.transaction().await?;
        let inserted = tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.billing_spend_ledger
                     (tenant_id, account_id, ledger_id, session_id, task_id, payload, evidence_ref)
                     VALUES ($1, $2, $3, $4, $5, ($6::text)::jsonb, $7)
                     ON CONFLICT DO NOTHING"
                ),
                &[
                    &entry.tenant_id,
                    &entry.account_id,
                    &entry.ledger_id,
                    &entry.session_id,
                    &entry.task_id,
                    &payload,
                    &evidence_ref,
                ],
            )
            .await?;
        if inserted != 1 {
            bail!("spend ledger {} already exists", entry.ledger_id);
        }
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!(
                "billing:spend-ledger:{}:{}:{}",
                entry.tenant_id, entry.account_id, entry.ledger_id
            ),
            "billing.spend_ledger_appended",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(entry)
    }

    pub async fn list_spend_ledger(
        &self,
        tenant_id: &str,
        account_id: &str,
    ) -> Result<Vec<SpendLedger>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT payload::text
                     FROM {schema}.billing_spend_ledger
                     WHERE tenant_id = $1 AND account_id = $2
                     ORDER BY created_at ASC"
                ),
                &[&tenant_id, &account_id],
            )
            .await?;
        let mut entries = rows
            .into_iter()
            .filter_map(|row| {
                let payload: String = row.get(0);
                serde_json::from_str::<SpendLedger>(&payload).ok()
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.created_at_ms);
        Ok(entries)
    }

    pub async fn list_spend_ledger_by_task(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<Vec<SpendLedger>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT payload::text
                     FROM {schema}.billing_spend_ledger
                     WHERE session_id = $1 AND task_id = $2
                     ORDER BY created_at ASC"
                ),
                &[&session_id, &task_id],
            )
            .await?;
        let mut entries = rows
            .into_iter()
            .filter_map(|row| {
                let payload: String = row.get(0);
                serde_json::from_str::<SpendLedger>(&payload).ok()
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.created_at_ms);
        Ok(entries)
    }

    pub async fn upsert_quota_window(&self, window: QuotaWindow) -> Result<QuotaWindow> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&window)?;
        let evidence_ref = Self::next_evidence_ref("billing.quota_window");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.billing_quota_windows(tenant_id, account_id, window_id, payload, evidence_ref)
                     VALUES ($1, $2, $3, ($4::text)::jsonb, $5)
                     ON CONFLICT(tenant_id, account_id, window_id) DO UPDATE
                     SET payload = EXCLUDED.payload, evidence_ref = EXCLUDED.evidence_ref, updated_at = NOW()"
                ),
                &[&window.tenant_id, &window.account_id, &window.window_id, &payload, &evidence_ref],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!(
                "billing:quota-window:{}:{}:{}",
                window.tenant_id, window.account_id, window.window_id
            ),
            "billing.quota_window_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(window)
    }

    pub async fn get_quota_window(
        &self,
        tenant_id: &str,
        account_id: &str,
        window_id: &str,
    ) -> Result<Option<QuotaWindow>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!(
                    "SELECT payload::text FROM {schema}.billing_quota_windows
                     WHERE tenant_id = $1 AND account_id = $2 AND window_id = $3"
                ),
                &[&tenant_id, &account_id, &window_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<QuotaWindow>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn upsert_cost_attribution(
        &self,
        attribution: CostAttribution,
    ) -> Result<CostAttribution> {
        let mut client = self.connect().await?;
        let schema = &self.config.schema;
        let payload = serde_json::to_string(&attribution)?;
        let evidence_ref = Self::next_evidence_ref("billing.cost_attribution");
        let tx = client.transaction().await?;
        tx
            .execute(
                &format!(
                    "INSERT INTO {schema}.billing_cost_attribution
                     (tenant_id, session_id, attribution_id, payload, evidence_ref)
                     VALUES ($1, $2, $3, ($4::text)::jsonb, $5)
                     ON CONFLICT(tenant_id, session_id, attribution_id) DO UPDATE
                     SET payload = EXCLUDED.payload, evidence_ref = EXCLUDED.evidence_ref, updated_at = NOW()"
                ),
                &[
                    &attribution.tenant_id,
                    &attribution.session_id,
                    &attribution.attribution_id,
                    &payload,
                    &evidence_ref,
                ],
            )
            .await?;
        Self::append_event_log_tx(
            &tx,
            schema,
            &format!(
                "billing:cost-attribution:{}:{}:{}",
                attribution.tenant_id, attribution.session_id, attribution.attribution_id
            ),
            "billing.cost_attribution_upserted",
            &payload,
            &evidence_ref,
            "postgres-adapter",
        )
        .await?;
        tx.commit().await?;
        Ok(attribution)
    }

    pub async fn get_cost_attribution(
        &self,
        tenant_id: &str,
        session_id: &str,
        attribution_id: &str,
    ) -> Result<Option<CostAttribution>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        if let Some(row) = client
            .query_opt(
                &format!(
                    "SELECT payload::text FROM {schema}.billing_cost_attribution
                     WHERE tenant_id = $1 AND session_id = $2 AND attribution_id = $3"
                ),
                &[&tenant_id, &session_id, &attribution_id],
            )
            .await?
        {
            let payload: String = row.get(0);
            return Ok(serde_json::from_str::<CostAttribution>(&payload).ok());
        }
        Ok(None)
    }

    pub async fn list_cost_attribution_by_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<Vec<CostAttribution>> {
        let client = self.connect().await?;
        let schema = &self.config.schema;
        let rows = client
            .query(
                &format!(
                    "SELECT payload::text FROM {schema}.billing_cost_attribution
                     WHERE tenant_id = $1 AND session_id = $2
                     ORDER BY created_at ASC"
                ),
                &[&tenant_id, &session_id],
            )
            .await?;
        let mut records = rows
            .into_iter()
            .filter_map(|row| {
                let payload: String = row.get(0);
                serde_json::from_str::<CostAttribution>(&payload).ok()
            })
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.settled_at_ms);
        Ok(records)
    }

    pub async fn upsert_reflexion_episode(
        &self,
        record: ReflexionEpisodeRecord,
    ) -> Result<ReflexionEpisodeRecord> {
        self.upsert_json_knowledge(
            format!("learning:reflexion:{}:{}", record.session_id, record.id),
            &record,
            "learning",
        )
        .await?;
        Ok(record)
    }

    pub async fn list_reflexion_episodes(&self, session_id: &str) -> Result<Vec<ReflexionEpisodeRecord>> {
        let prefix = format!("learning:reflexion:{session_id}:");
        let mut rows = self
            .list_knowledge_by_prefix(&prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<ReflexionEpisodeRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        rows.sort_by_key(|entry| entry.created_at_ms);
        Ok(rows)
    }

    pub async fn upsert_skill_library_record(
        &self,
        record: SkillLibraryRecord,
    ) -> Result<SkillLibraryRecord> {
        self.upsert_json_knowledge(
            format!("learning:skill:{}:{}", record.session_id, record.id),
            &record,
            "learning",
        )
        .await?;
        Ok(record)
    }

    pub async fn list_skill_library_records(&self, session_id: &str) -> Result<Vec<SkillLibraryRecord>> {
        let prefix = format!("learning:skill:{session_id}:");
        let mut rows = self
            .list_knowledge_by_prefix(&prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<SkillLibraryRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        rows.sort_by_key(|entry| entry.updated_at_ms);
        Ok(rows)
    }

    pub async fn upsert_causal_edge_record(
        &self,
        record: CausalEdgeRecord,
    ) -> Result<CausalEdgeRecord> {
        self.upsert_json_knowledge(
            format!("learning:causal-edge:{}:{}", record.session_id, record.id),
            &record,
            "learning",
        )
        .await?;
        Ok(record)
    }

    pub async fn list_causal_edge_records(&self, session_id: &str) -> Result<Vec<CausalEdgeRecord>> {
        let prefix = format!("learning:causal-edge:{session_id}:");
        let mut rows = self
            .list_knowledge_by_prefix(&prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<CausalEdgeRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        rows.sort_by_key(|entry| entry.created_at_ms);
        Ok(rows)
    }

    pub async fn upsert_learning_session_record(
        &self,
        record: LearningSessionRecord,
    ) -> Result<LearningSessionRecord> {
        self.upsert_json_knowledge(
            format!("learning:session:{}:{}", record.session_id, record.id),
            &record,
            "learning",
        )
        .await?;
        Ok(record)
    }

    pub async fn list_learning_session_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<LearningSessionRecord>> {
        let prefix = format!("learning:session:{session_id}:");
        let mut rows = self
            .list_knowledge_by_prefix(&prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<LearningSessionRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        rows.sort_by_key(|entry| entry.started_at_ms);
        Ok(rows)
    }

    pub async fn append_witness_log_record(
        &self,
        record: WitnessLogRecord,
    ) -> Result<WitnessLogRecord> {
        let key = format!("learning:witness:{}:{}", record.session_id, record.id);
        if self.get_knowledge(&key).await?.is_some() {
            bail!("witness log {} already exists", record.id);
        }
        self.upsert_json_knowledge(key, &record, "learning").await?;
        Ok(record)
    }

    pub async fn list_witness_log_records(&self, session_id: &str) -> Result<Vec<WitnessLogRecord>> {
        let prefix = format!("learning:witness:{session_id}:");
        let mut rows = self
            .list_knowledge_by_prefix(&prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<WitnessLogRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        rows.sort_by_key(|entry| entry.created_at_ms);
        Ok(rows)
    }
}

fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tokio::runtime::Runtime;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static TEST_SCHEMA_SEQ: AtomicU64 = AtomicU64::new(1);

    fn test_db_config() -> PostgresDbConfig {
        let seq = TEST_SCHEMA_SEQ.fetch_add(1, Ordering::Relaxed);
        PostgresDbConfig {
            enabled: true,
            uri: std::env::var("AUTOLOOP_PG_TEST_URI")
                .unwrap_or_else(|_| "postgres://postgres:123456@localhost:5432/postgres".into()),
            schema: format!("autoloop_r2_{}_{}", PostgresDb::now_ms(), seq),
            pool_size: 4,
            auto_migrate: true,
        }
    }

    async fn event_log_count(db: &PostgresDb, evidence_ref: &str) -> Result<i64> {
        let client = db.connect().await?;
        let schema = &db.config.schema;
        let row = client
            .query_one(
                &format!("SELECT COUNT(*) FROM {schema}.event_log WHERE evidence_ref = $1"),
                &[&evidence_ref],
            )
            .await?;
        Ok(row.get(0))
    }

    async fn drop_test_schema(db: &PostgresDb) {
        if let Ok(client) = db.connect().await {
            let _ = client
                .batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", db.config.schema))
                .await;
        }
    }

    #[test]
    fn atomic_rollback_happens_when_event_log_fails() -> Result<()> {
        let rt = Runtime::new()?;
        rt.block_on(async {
        let _guard = TEST_LOCK.lock().expect("test lock poisoned");
        let db = PostgresDb::new(test_db_config());
        if let Err(error) = db.ensure_ready().await {
            eprintln!("skip atomic_rollback_happens_when_event_log_fails: {error}");
            return Ok(());
        }

        unsafe {
            std::env::set_var("AUTOLOOP_PG_ATOMIC_FAILPOINT", "before_event_log");
        }
        let result = db
            .create_schedule_event(
                "r2-session".into(),
                "wake.plan.execute".into(),
                "planner".into(),
                "{\"intent\":\"atomic\"}".into(),
                "agent-r2".into(),
            )
            .await;
        unsafe {
            std::env::remove_var("AUTOLOOP_PG_ATOMIC_FAILPOINT");
        }
        assert!(result.is_err(), "failpoint should force rollback");

        let listed = db.list_schedule_events("r2-session").await?;
        assert!(
            listed.is_empty(),
            "schedule state must rollback when event_log fails"
        );
        drop_test_schema(&db).await;
        Ok(())
        })
    }

    #[test]
    fn atomic_success_writes_state_and_single_event_log() -> Result<()> {
        let rt = Runtime::new()?;
        rt.block_on(async {
        let _guard = TEST_LOCK.lock().expect("test lock poisoned");
        let db = PostgresDb::new(test_db_config());
        if let Err(error) = db.ensure_ready().await {
            eprintln!("skip atomic_success_writes_state_and_single_event_log: {error}");
            return Ok(());
        }

        let account = db
            .upsert_budget_account(BudgetAccount {
                account_id: "acct-r2".into(),
                tenant_id: "tenant-r2".into(),
                principal_id: "principal-r2".into(),
                policy_id: "policy-r2".into(),
                total_budget_micros: 100_000,
                reserved_micros: 0,
                spent_micros: 0,
                blocked_count: 0,
                updated_at_ms: PostgresDb::now_ms(),
            })
            .await?;
        let got = db
            .get_budget_account(&account.tenant_id, &account.account_id)
            .await?;
        assert!(got.is_some(), "state write must persist");

        let client = db.connect().await?;
        let schema = &db.config.schema;
        let row = client
            .query_one(
                &format!(
                    "SELECT evidence_ref FROM {schema}.billing_budget_accounts
                     WHERE tenant_id = $1 AND account_id = $2"
                ),
                &[&account.tenant_id, &account.account_id],
            )
            .await?;
        let evidence_ref: String = row.get(0);
        assert!(!evidence_ref.is_empty(), "state must carry evidence_ref");
        assert_eq!(
            event_log_count(&db, &evidence_ref).await?,
            1,
            "atomic audit must write exactly one event_log row for evidence_ref"
        );

        drop_test_schema(&db).await;
        Ok(())
        })
    }

    #[test]
    fn atomic_update_rollback_keeps_previous_state() -> Result<()> {
        let rt = Runtime::new()?;
        rt.block_on(async {
        let _guard = TEST_LOCK.lock().expect("test lock poisoned");
        let db = PostgresDb::new(test_db_config());
        if let Err(error) = db.ensure_ready().await {
            eprintln!("skip atomic_update_rollback_keeps_previous_state: {error}");
            return Ok(());
        }

        let created = db
            .create_schedule_event(
                "r2-update-session".into(),
                "wake".into(),
                "planner".into(),
                "{}".into(),
                "agent-r2".into(),
            )
            .await?;
        assert_eq!(created.status, "queued");

        unsafe {
            std::env::set_var("AUTOLOOP_PG_ATOMIC_FAILPOINT", "before_event_log");
        }
        let result = db.update_schedule_status(created.id, "done").await;
        unsafe {
            std::env::remove_var("AUTOLOOP_PG_ATOMIC_FAILPOINT");
        }
        assert!(result.is_err(), "update must fail under failpoint");

        let events = db.list_schedule_events("r2-update-session").await?;
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].status, "queued",
            "update state must rollback when event_log fails"
        );

        drop_test_schema(&db).await;
        Ok(())
        })
    }

    #[test]
    fn atomic_relation_bundle_rolls_back_on_failpoint() -> Result<()> {
        let rt = Runtime::new()?;
        rt.block_on(async {
            let _guard = TEST_LOCK.lock().expect("test lock poisoned");
            let db = PostgresDb::new(test_db_config());
            if let Err(error) = db.ensure_ready().await {
                eprintln!("skip atomic_relation_bundle_rolls_back_on_failpoint: {error}");
                return Ok(());
            }

            let input = AtomicRelationWriteInput {
                session_id: "r5-session".into(),
                trace_id: "r5-trace".into(),
                state_key: "relation:state:r5-session:r5-trace:1".into(),
                state_payload: serde_json::json!({"state":"ok"}),
                relation_event_key: "relation:event:r5-session:r5-trace:1".into(),
                relation_event_payload: serde_json::json!({"event":"edge_upserted"}),
                evidence_key: "relation:evidence:r5-session:r5-trace:1".into(),
                evidence_payload: serde_json::json!({"event":"atomic"}),
                write_proof_key: "relation:write_proof:r5-session:r5-trace:1".into(),
                write_proof_payload: serde_json::json!({"hash":"abc"}),
                source: "test".into(),
                edge_current: Some(RelationEdgeCurrentWrite {
                    edge_id: "edge:r5".into(),
                    from_node: "a".into(),
                    to_node: "b".into(),
                    edge_type: "depends_on".into(),
                    payload: serde_json::json!({"w":1}),
                }),
                event_append: Some(RelationEventAppendWrite {
                    event_id: "evt:r5".into(),
                    event_type: "edge_upserted".into(),
                    payload: serde_json::json!({"edge_id":"edge:r5"}),
                }),
                hot_index_entries: vec![RelationHotIndexWrite {
                    hot_key: "hot:r5".into(),
                    relation_kind: "edge".into(),
                    relation_ref: "edge:r5".into(),
                    score: 1.0,
                    payload: serde_json::json!({"edge_id":"edge:r5"}),
                }],
            };

            unsafe {
                std::env::set_var("AUTOLOOP_PG_ATOMIC_FAILPOINT", "before_event_log");
            }
            let result = db.atomic_write_relation_bundle(input.clone()).await;
            unsafe {
                std::env::remove_var("AUTOLOOP_PG_ATOMIC_FAILPOINT");
            }
            assert!(result.is_err(), "failpoint should force rollback");

            assert!(db.get_knowledge(&input.state_key).await?.is_none());
            assert!(db.get_knowledge(&input.relation_event_key).await?.is_none());
            assert!(db.get_knowledge(&input.evidence_key).await?.is_none());
            assert!(db.get_knowledge(&input.write_proof_key).await?.is_none());

            drop_test_schema(&db).await;
            Ok(())
        })
    }

    #[test]
    fn relation_query_p95_under_150ms() -> Result<()> {
        let rt = Runtime::new()?;
        rt.block_on(async {
            let _guard = TEST_LOCK.lock().expect("test lock poisoned");
            let db = PostgresDb::new(test_db_config());
            if let Err(error) = db.ensure_ready().await {
                eprintln!("skip relation_query_p95_under_150ms: {error}");
                return Ok(());
            }

            let session_id = "r7-session";
            for idx in 0..120usize {
                let now = PostgresDb::now_ms();
                let _ = db
                    .atomic_write_relation_bundle(AtomicRelationWriteInput {
                        session_id: session_id.into(),
                        trace_id: format!("r7-trace-{idx}"),
                        state_key: format!("relation:state:{session_id}:{idx}"),
                        state_payload: serde_json::json!({"idx":idx}),
                        relation_event_key: format!("relation:event:{session_id}:{idx}"),
                        relation_event_payload: serde_json::json!({"idx":idx}),
                        evidence_key: format!("relation:evidence:{session_id}:{idx}"),
                        evidence_payload: serde_json::json!({"idx":idx}),
                        write_proof_key: format!("relation:write_proof:{session_id}:{idx}"),
                        write_proof_payload: serde_json::json!({"idx":idx}),
                        source: "perf-test".into(),
                        edge_current: Some(RelationEdgeCurrentWrite {
                            edge_id: format!("edge:{idx}"),
                            from_node: format!("n:{idx}"),
                            to_node: format!("n:{}", idx + 1),
                            edge_type: "depends_on".into(),
                            payload: serde_json::json!({"weight": 1}),
                        }),
                        event_append: Some(RelationEventAppendWrite {
                            event_id: format!("event:{idx}:{now}"),
                            event_type: "edge_upserted".into(),
                            payload: serde_json::json!({"edge_id": format!("edge:{idx}")}),
                        }),
                        hot_index_entries: vec![RelationHotIndexWrite {
                            hot_key: format!("edge-hot:{idx}"),
                            relation_kind: "edge".into(),
                            relation_ref: format!("edge:{idx}"),
                            score: (120 - idx) as f64,
                            payload: serde_json::json!({"rank": idx}),
                        }],
                    })
                    .await?;
            }

            let mut samples_ms = Vec::new();
            let client = db.connect().await?;
            let schema = db.config.schema.clone();
            for _ in 0..25 {
                let started = std::time::Instant::now();
                let _ = client
                    .query(
                        &format!(
                            "SELECT edge_id FROM {schema}.relation_edges
                             WHERE session_id = $1
                             ORDER BY updated_at DESC
                             LIMIT 100"
                        ),
                        &[&session_id],
                    )
                    .await?;
                let _ = client
                    .query(
                        &format!(
                            "SELECT event_id FROM {schema}.relation_events
                             WHERE session_id = $1
                             ORDER BY created_at DESC, event_pk DESC
                             LIMIT 100"
                        ),
                        &[&session_id],
                    )
                    .await?;
                let _ = client
                    .query(
                        &format!(
                            "SELECT hot_key FROM {schema}.relation_hot_index
                             WHERE session_id = $1
                             ORDER BY score DESC, updated_at DESC
                             LIMIT 100"
                        ),
                        &[&session_id],
                    )
                    .await?;
                samples_ms.push(started.elapsed().as_millis() as u64);
            }
            samples_ms.sort_unstable();
            let idx = ((samples_ms.len() as f64) * 0.95).ceil() as usize;
            let p95 = samples_ms[idx.saturating_sub(1).min(samples_ms.len().saturating_sub(1))];
            assert!(
                p95 < 150,
                "relation query p95 too high: {p95}ms (target < 150ms)"
            );

            drop_test_schema(&db).await;
            Ok(())
        })
    }
}
