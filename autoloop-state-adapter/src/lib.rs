use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        Mutex as StdMutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use autoloop_postgres_adapter::{AtomicRelationWriteInput as PostgresAtomicRelationWriteInput, PostgresDb};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::RwLock;

#[path = "../../src/module_bindings/generated/mod.rs"]
#[cfg(any())]
pub mod generated_bindings;

#[cfg(any())]
pub mod sdk {
    use anyhow::Result;

    use super::{
        AgentState,
        CausalEdgeRecord,
        KnowledgeRecord,
        LearningEventKind,
        LearningSessionRecord,
        PermissionAction,
        PermissionGrant,
        ReflexionEpisodeRecord,
        ScheduleEvent,
        SkillLibraryRecord,
        StateStoreConfig,
        WitnessLogRecord,
        generated_bindings::{
            self,
            AgentStateTableAccess,
            CausalEdgeRecordTableAccess,
            DbConnection,
            KnowledgeRecordTableAccess,
            LearningEventKind as GeneratedLearningEventKind,
            LearningSessionRecordTableAccess,
            PermissionAction as GeneratedPermissionAction,
            PermissionGrantTableAccess,
            ReflexionEpisodeTableAccess,
            ScheduleEventTableAccess,
            SkillLibraryRecordTableAccess,
            WitnessLogRecordTableAccess,
            append_witness_log_record as AppendWitnessLogRecordExt,
            create_schedule_event as CreateScheduleEventExt,
            grant_permissions as GrantPermissionsExt,
            upsert_causal_edge_record as UpsertCausalEdgeRecordExt,
            upsert_learning_session_record as UpsertLearningSessionRecordExt,
            update_schedule_status as UpdateScheduleStatusExt,
            upsert_agent_state as UpsertAgentStateExt,
            upsert_knowledge as UpsertKnowledgeExt,
            upsert_reflexion_episode as UpsertReflexionEpisodeExt,
            upsert_skill_library_record as UpsertSkillLibraryRecordExt,
        },
    };

    pub struct GeneratedModuleClient {
        connection: DbConnection,
    }

    impl GeneratedModuleClient {
        pub fn connect(config: &StateStoreConfig) -> Result<Self> {
            let connection = DbConnection::builder()
                .with_uri(config.uri.clone())
                .with_database_name(config.module_name.clone())
                .build()?;

            Ok(Self { connection })
        }

        pub fn connection(&self) -> &DbConnection {
            &self.connection
        }

        pub fn subscribe_all_tables(&self) {
            self.connection.subscription_builder().subscribe_to_all_tables();
        }

        pub fn frame_tick(&self) -> Result<()> {
            self.connection.frame_tick()?;
            Ok(())
        }

        pub fn grant_permissions(
            &self,
            actor_id: impl Into<String>,
            permissions: Vec<PermissionAction>,
        ) -> Result<()> {
            self.connection.reducers.grant_permissions(
                actor_id.into(),
                permissions.into_iter().map(into_generated_permission).collect(),
            )?;
            Ok(())
        }

        pub fn create_schedule_event(&self, event: ScheduleEvent) -> Result<()> {
            self.connection.reducers.create_schedule_event(
                event.id,
                event.session_id,
                event.topic,
                event.tool_name,
                event.payload,
                event.actor_id,
                event.status,
            )?;
            Ok(())
        }

        pub fn update_schedule_status(&self, event_id: u64, status: impl Into<String>) -> Result<()> {
            self.connection
                .reducers
                .update_schedule_status(event_id, status.into())?;
            Ok(())
        }

        pub fn upsert_agent_state(&self, state: AgentState) -> Result<()> {
            self.connection.reducers.upsert_agent_state(
                state.session_id,
                state.last_user_message,
                state.last_assistant_message,
            )?;
            Ok(())
        }

        pub fn upsert_knowledge(&self, record: KnowledgeRecord) -> Result<()> {
            self.connection
                .reducers
                .upsert_knowledge(record.key, record.value, record.source)?;
            Ok(())
        }

        pub fn list_schedule_events(&self) -> Vec<ScheduleEvent> {
            self.connection
                .db
                .schedule_event()
                .iter()
                .map(|row| ScheduleEvent {
                    id: row.id,
                    session_id: row.session_id,
                    topic: row.topic,
                    tool_name: row.tool_name,
                    payload: row.payload,
                    actor_id: row.actor_id,
                    status: row.status,
                })
                .collect()
        }

        pub fn get_agent_state(&self, session_id: &str) -> Option<AgentState> {
            self.connection.db.agent_state().session_id().find(&session_id.to_string()).map(
                |row| AgentState {
                    id: 0,
                    session_id: row.session_id,
                    last_user_message: row.last_user_message,
                    last_assistant_message: row.last_assistant_message,
                },
            )
        }

        pub fn get_knowledge(&self, key: &str) -> Option<KnowledgeRecord> {
            self.connection.db.knowledge_record().key().find(&key.to_string()).map(|row| {
                KnowledgeRecord {
                    id: 0,
                    key: row.key,
                    value: row.value,
                    source: row.source,
                }
            })
        }

        pub fn list_knowledge_by_prefix(&self, prefix: &str) -> Vec<KnowledgeRecord> {
            self.connection
                .db
                .knowledge_record()
                .iter()
                .filter(|row| row.key.starts_with(prefix))
                .map(|row| KnowledgeRecord {
                    id: 0,
                    key: row.key,
                    value: row.value,
                    source: row.source,
                })
                .collect()
        }

        pub fn get_permission_grant(&self, actor_id: &str) -> Option<PermissionGrant> {
            self.connection
                .db
                .permission_grant()
                .actor_id()
                .find(&actor_id.to_string())
                .map(|row| PermissionGrant {
                    actor_id: row.actor_id,
                    permissions: row
                        .permissions
                        .into_iter()
                        .map(from_generated_permission)
                        .collect(),
                })
        }

        pub fn upsert_reflexion_episode(&self, record: ReflexionEpisodeRecord) -> Result<()> {
            self.connection.reducers.upsert_reflexion_episode(
                record.id,
                record.session_id,
                record.objective,
                record.hypothesis,
                record.outcome,
                record.lesson,
                record.status,
                record.score,
                record.created_at_ms,
            )?;
            Ok(())
        }

        pub fn list_reflexion_episodes(&self) -> Vec<ReflexionEpisodeRecord> {
            self.connection
                .db
                .reflexion_episode()
                .iter()
                .map(|row| ReflexionEpisodeRecord {
                    id: row.id,
                    session_id: row.session_id,
                    objective: row.objective,
                    hypothesis: row.hypothesis,
                    outcome: row.outcome,
                    lesson: row.lesson,
                    status: row.status,
                    score: row.score,
                    created_at_ms: row.created_at_ms,
                })
                .collect()
        }

        pub fn upsert_skill_library_record(&self, record: SkillLibraryRecord) -> Result<()> {
            self.connection.reducers.upsert_skill_library_record(
                record.id,
                record.session_id,
                record.name,
                record.trigger,
                record.procedure,
                record.confidence,
                record.success_rate,
                record.evidence_count,
                record.created_at_ms,
                record.updated_at_ms,
            )?;
            Ok(())
        }

        pub fn list_skill_library_records(&self) -> Vec<SkillLibraryRecord> {
            self.connection
                .db
                .skill_library_record()
                .iter()
                .map(|row| SkillLibraryRecord {
                    id: row.id,
                    session_id: row.session_id,
                    name: row.name,
                    trigger: row.trigger,
                    procedure: row.procedure,
                    confidence: row.confidence,
                    success_rate: row.success_rate,
                    evidence_count: row.evidence_count,
                    created_at_ms: row.created_at_ms,
                    updated_at_ms: row.updated_at_ms,
                })
                .collect()
        }

        pub fn upsert_causal_edge_record(&self, record: CausalEdgeRecord) -> Result<()> {
            self.connection.reducers.upsert_causal_edge_record(
                record.id,
                record.session_id,
                record.cause,
                record.effect,
                record.evidence,
                record.strength,
                record.confidence,
                record.created_at_ms,
            )?;
            Ok(())
        }

        pub fn list_causal_edge_records(&self) -> Vec<CausalEdgeRecord> {
            self.connection
                .db
                .causal_edge_record()
                .iter()
                .map(|row| CausalEdgeRecord {
                    id: row.id,
                    session_id: row.session_id,
                    cause: row.cause,
                    effect: row.effect,
                    evidence: row.evidence,
                    strength: row.strength,
                    confidence: row.confidence,
                    created_at_ms: row.created_at_ms,
                })
                .collect()
        }

        pub fn upsert_learning_session_record(&self, record: LearningSessionRecord) -> Result<()> {
            self.connection.reducers.upsert_learning_session_record(
                record.id,
                record.session_id,
                record.objective,
                record.status,
                record.priority,
                record.summary,
                record.started_at_ms,
                record.completed_at_ms,
            )?;
            Ok(())
        }

        pub fn list_learning_session_records(&self) -> Vec<LearningSessionRecord> {
            self.connection
                .db
                .learning_session_record()
                .iter()
                .map(|row| LearningSessionRecord {
                    id: row.id,
                    session_id: row.session_id,
                    objective: row.objective,
                    status: row.status,
                    priority: row.priority,
                    summary: row.summary,
                    started_at_ms: row.started_at_ms,
                    completed_at_ms: row.completed_at_ms,
                })
                .collect()
        }

        pub fn append_witness_log_record(&self, record: WitnessLogRecord) -> Result<()> {
            self.connection.reducers.append_witness_log_record(
                record.id,
                record.session_id,
                into_generated_learning_event(record.event_type),
                record.source,
                record.detail,
                record.score,
                record.created_at_ms,
                record.metadata_json,
            )?;
            Ok(())
        }

        pub fn list_witness_log_records(&self) -> Vec<WitnessLogRecord> {
            self.connection
                .db
                .witness_log_record()
                .iter()
                .map(|row| WitnessLogRecord {
                    id: row.id,
                    session_id: row.session_id,
                    event_type: from_generated_learning_event(row.event_type),
                    source: row.source,
                    detail: row.detail,
                    score: row.score,
                    created_at_ms: row.created_at_ms,
                    metadata_json: row.metadata_json,
                })
                .collect()
        }
    }

    fn into_generated_permission(action: PermissionAction) -> GeneratedPermissionAction {
        match action {
            PermissionAction::Read => GeneratedPermissionAction::Read,
            PermissionAction::Write => GeneratedPermissionAction::Write,
            PermissionAction::Dispatch => GeneratedPermissionAction::Dispatch,
            PermissionAction::Admin => GeneratedPermissionAction::Admin,
        }
    }

    fn from_generated_permission(action: generated_bindings::PermissionAction) -> PermissionAction {
        match action {
            generated_bindings::PermissionAction::Read => PermissionAction::Read,
            generated_bindings::PermissionAction::Write => PermissionAction::Write,
            generated_bindings::PermissionAction::Dispatch => PermissionAction::Dispatch,
            generated_bindings::PermissionAction::Admin => PermissionAction::Admin,
        }
    }

    fn into_generated_learning_event(action: LearningEventKind) -> GeneratedLearningEventKind {
        match action {
            LearningEventKind::Failure => GeneratedLearningEventKind::Failure,
            LearningEventKind::Success => GeneratedLearningEventKind::Success,
            LearningEventKind::ToolCall => GeneratedLearningEventKind::ToolCall,
            LearningEventKind::RouteDecision => GeneratedLearningEventKind::RouteDecision,
            LearningEventKind::Audit => GeneratedLearningEventKind::Audit,
        }
    }

    fn from_generated_learning_event(action: GeneratedLearningEventKind) -> LearningEventKind {
        match action {
            GeneratedLearningEventKind::Failure => LearningEventKind::Failure,
            GeneratedLearningEventKind::Success => LearningEventKind::Success,
            GeneratedLearningEventKind::ToolCall => LearningEventKind::ToolCall,
            GeneratedLearningEventKind::RouteDecision => LearningEventKind::RouteDecision,
            GeneratedLearningEventKind::Audit => LearningEventKind::Audit,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateStoreBackend {
    InMemory,
}

fn default_backend() -> StateStoreBackend {
    StateStoreBackend::InMemory
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateStoreConfig {
    pub enabled: bool,
    #[serde(default = "default_backend")]
    pub backend: StateStoreBackend,
    pub uri: String,
    pub module_name: String,
    pub namespace: String,
    pub pool_size: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeMirrorMode {
    Shadow,
    Direct,
    Enforced,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeReadPreference {
    PrimaryStore,
    Postgres,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: u64,
    pub session_id: String,
    pub last_user_message: String,
    pub last_assistant_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeRecord {
    pub id: u64,
    pub key: String,
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowReadDiffReport {
    pub domain: String,
    pub probe: String,
    pub target: String,
    pub mismatch: bool,
    pub reason: Option<String>,
    pub diff_reason: String,
    pub diff_classes: Vec<String>,
    pub old: serde_json::Value,
    pub new: serde_json::Value,
    pub evidence_ref: String,
    pub primary_count: usize,
    pub mirror_count: Option<usize>,
    pub missing_in_mirror: Vec<String>,
    pub extra_in_mirror: Vec<String>,
    pub value_mismatches: Vec<String>,
    pub generated_at_ms: u64,
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
pub struct RelationHotIndexEntry {
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

#[async_trait::async_trait]
pub trait SpacetimeRepository: Send + Sync {
    async fn create_schedule_event(
        &self,
        session_id: String,
        topic: String,
        tool_name: String,
        payload: String,
        actor_id: String,
    ) -> Result<ScheduleEvent>;
    async fn update_schedule_status(&self, event_id: u64, status: String) -> Result<()>;
    async fn list_schedule_events(&self, session_id: &str) -> Result<Vec<ScheduleEvent>>;
    async fn upsert_agent_state(
        &self,
        session_id: String,
        last_user_message: String,
        last_assistant_message: Option<String>,
    ) -> Result<AgentState>;
    async fn get_agent_state(&self, session_id: &str) -> Result<Option<AgentState>>;
    async fn upsert_knowledge(
        &self,
        key: String,
        value: String,
        source: String,
    ) -> Result<KnowledgeRecord>;
    async fn get_knowledge(&self, key: &str) -> Result<Option<KnowledgeRecord>>;
    async fn list_knowledge_by_prefix(&self, prefix: &str) -> Result<Vec<KnowledgeRecord>>;
    async fn grant_permissions(
        &self,
        actor_id: String,
        permissions: Vec<PermissionAction>,
    ) -> Result<PermissionGrant>;
    async fn has_permission(&self, actor_id: &str, action: PermissionAction) -> Result<bool>;
    async fn upsert_reflexion_episode(
        &self,
        record: ReflexionEpisodeRecord,
    ) -> Result<ReflexionEpisodeRecord>;
    async fn list_reflexion_episodes(&self, session_id: &str) -> Result<Vec<ReflexionEpisodeRecord>>;
    async fn upsert_skill_library_record(
        &self,
        record: SkillLibraryRecord,
    ) -> Result<SkillLibraryRecord>;
    async fn list_skill_library_records(&self, session_id: &str) -> Result<Vec<SkillLibraryRecord>>;
    async fn upsert_causal_edge_record(
        &self,
        record: CausalEdgeRecord,
    ) -> Result<CausalEdgeRecord>;
    async fn list_causal_edge_records(&self, session_id: &str) -> Result<Vec<CausalEdgeRecord>>;
    async fn upsert_learning_session_record(
        &self,
        record: LearningSessionRecord,
    ) -> Result<LearningSessionRecord>;
    async fn list_learning_session_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<LearningSessionRecord>>;
    async fn append_witness_log_record(
        &self,
        record: WitnessLogRecord,
    ) -> Result<WitnessLogRecord>;
    async fn list_witness_log_records(&self, session_id: &str) -> Result<Vec<WitnessLogRecord>>;
}

#[derive(Default)]
struct InMemoryState {
    events: HashMap<u64, ScheduleEvent>,
    events_by_session: HashMap<String, Vec<u64>>,
    agent_state: HashMap<String, AgentState>,
    knowledge: HashMap<String, KnowledgeRecord>,
    permissions: HashMap<String, HashSet<PermissionAction>>,
    reflexion_episodes: HashMap<String, ReflexionEpisodeRecord>,
    skills: HashMap<String, SkillLibraryRecord>,
    causal_edges: HashMap<String, CausalEdgeRecord>,
    learning_sessions: HashMap<String, LearningSessionRecord>,
    witness_logs: HashMap<String, WitnessLogRecord>,
}

#[derive(Default)]
pub struct InMemorySpacetimeRepository {
    state: Arc<RwLock<InMemoryState>>,
    next_id: AtomicU64,
}

impl InMemorySpacetimeRepository {
    fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[cfg(any())]
pub struct SdkSpacetimeRepository {
    client: Arc<StdMutex<sdk::GeneratedModuleClient>>,
    next_event_id: AtomicU64,
}

#[cfg(any())]
impl SdkSpacetimeRepository {
    pub fn connect(config: &StateStoreConfig) -> Result<Self> {
        let client = sdk::GeneratedModuleClient::connect(config)?;
        client.subscribe_all_tables();
        let _ = client.frame_tick();

        Ok(Self {
            client: Arc::new(StdMutex::new(client)),
            next_event_id: AtomicU64::new(0),
        })
    }

    fn alloc_event_id(&self) -> u64 {
        self.next_event_id.fetch_add(1, Ordering::SeqCst) + 1
    }

    async fn with_client<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut sdk::GeneratedModuleClient) -> Result<T> + Send + 'static,
    {
        let client = Arc::clone(&self.client);
        task::spawn_blocking(move || {
            let mut guard = client.lock().expect("state_store sdk client mutex poisoned");
            f(&mut guard)
        })
        .await
        .map_err(|error| anyhow::anyhow!("state_store sdk task join error: {error}"))?
    }
}

#[async_trait::async_trait]
impl SpacetimeRepository for InMemorySpacetimeRepository {
    async fn create_schedule_event(
        &self,
        session_id: String,
        topic: String,
        tool_name: String,
        payload: String,
        actor_id: String,
    ) -> Result<ScheduleEvent> {
        let id = self.alloc_id();
        let event = ScheduleEvent {
            id,
            session_id: session_id.clone(),
            topic,
            tool_name,
            payload,
            actor_id,
            status: "queued".into(),
        };

        let mut state = self.state.write().await;
        state.events.insert(id, event.clone());
        state.events_by_session.entry(session_id).or_default().push(id);
        Ok(event)
    }

    async fn update_schedule_status(&self, event_id: u64, status: String) -> Result<()> {
        let mut state = self.state.write().await;
        let event = state
            .events
            .get_mut(&event_id)
            .ok_or_else(|| anyhow::anyhow!("event {event_id} not found"))?;
        event.status = status;
        Ok(())
    }

    async fn list_schedule_events(&self, session_id: &str) -> Result<Vec<ScheduleEvent>> {
        let state = self.state.read().await;
        let ids = state.events_by_session.get(session_id).cloned().unwrap_or_default();
        Ok(ids
            .into_iter()
            .filter_map(|id| state.events.get(&id).cloned())
            .collect())
    }

    async fn upsert_agent_state(
        &self,
        session_id: String,
        last_user_message: String,
        last_assistant_message: Option<String>,
    ) -> Result<AgentState> {
        let mut state = self.state.write().await;
        let id = state
            .agent_state
            .get(&session_id)
            .map(|current| current.id)
            .unwrap_or_else(|| self.alloc_id());
        let snapshot = AgentState {
            id,
            session_id: session_id.clone(),
            last_user_message,
            last_assistant_message,
        };
        state.agent_state.insert(session_id, snapshot.clone());
        Ok(snapshot)
    }

    async fn get_agent_state(&self, session_id: &str) -> Result<Option<AgentState>> {
        let state = self.state.read().await;
        Ok(state.agent_state.get(session_id).cloned())
    }

    async fn upsert_knowledge(
        &self,
        key: String,
        value: String,
        source: String,
    ) -> Result<KnowledgeRecord> {
        let mut state = self.state.write().await;
        let id = state
            .knowledge
            .get(&key)
            .map(|current| current.id)
            .unwrap_or_else(|| self.alloc_id());
        let record = KnowledgeRecord {
            id,
            key: key.clone(),
            value,
            source,
        };
        state.knowledge.insert(key, record.clone());
        Ok(record)
    }

    async fn get_knowledge(&self, key: &str) -> Result<Option<KnowledgeRecord>> {
        let state = self.state.read().await;
        Ok(state.knowledge.get(key).cloned())
    }

    async fn list_knowledge_by_prefix(&self, prefix: &str) -> Result<Vec<KnowledgeRecord>> {
        let state = self.state.read().await;
        let mut records = state
            .knowledge
            .values()
            .filter(|record| record.key.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(records)
    }

    async fn grant_permissions(
        &self,
        actor_id: String,
        permissions: Vec<PermissionAction>,
    ) -> Result<PermissionGrant> {
        let mut state = self.state.write().await;
        let entry = state.permissions.entry(actor_id.clone()).or_default();
        for permission in &permissions {
            entry.insert(*permission);
        }
        Ok(PermissionGrant {
            actor_id,
            permissions: entry.iter().copied().collect(),
        })
    }

    async fn has_permission(&self, actor_id: &str, action: PermissionAction) -> Result<bool> {
        let state = self.state.read().await;
        Ok(state.permissions.get(actor_id).is_some_and(|grants| {
            grants.contains(&action) || grants.contains(&PermissionAction::Admin)
        }))
    }

    async fn upsert_reflexion_episode(
        &self,
        record: ReflexionEpisodeRecord,
    ) -> Result<ReflexionEpisodeRecord> {
        let mut state = self.state.write().await;
        state
            .reflexion_episodes
            .insert(record.id.clone(), record.clone());
        Ok(record)
    }

    async fn list_reflexion_episodes(&self, session_id: &str) -> Result<Vec<ReflexionEpisodeRecord>> {
        let state = self.state.read().await;
        Ok(state
            .reflexion_episodes
            .values()
            .filter(|record| record.session_id == session_id)
            .cloned()
            .collect())
    }

    async fn upsert_skill_library_record(
        &self,
        record: SkillLibraryRecord,
    ) -> Result<SkillLibraryRecord> {
        let mut state = self.state.write().await;
        state.skills.insert(record.id.clone(), record.clone());
        Ok(record)
    }

    async fn list_skill_library_records(&self, session_id: &str) -> Result<Vec<SkillLibraryRecord>> {
        let state = self.state.read().await;
        Ok(state
            .skills
            .values()
            .filter(|record| record.session_id == session_id)
            .cloned()
            .collect())
    }

    async fn upsert_causal_edge_record(
        &self,
        record: CausalEdgeRecord,
    ) -> Result<CausalEdgeRecord> {
        let mut state = self.state.write().await;
        state
            .causal_edges
            .insert(record.id.clone(), record.clone());
        Ok(record)
    }

    async fn list_causal_edge_records(&self, session_id: &str) -> Result<Vec<CausalEdgeRecord>> {
        let state = self.state.read().await;
        Ok(state
            .causal_edges
            .values()
            .filter(|record| record.session_id == session_id)
            .cloned()
            .collect())
    }

    async fn upsert_learning_session_record(
        &self,
        record: LearningSessionRecord,
    ) -> Result<LearningSessionRecord> {
        let mut state = self.state.write().await;
        state
            .learning_sessions
            .insert(record.id.clone(), record.clone());
        Ok(record)
    }

    async fn list_learning_session_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<LearningSessionRecord>> {
        let state = self.state.read().await;
        Ok(state
            .learning_sessions
            .values()
            .filter(|record| record.session_id == session_id)
            .cloned()
            .collect())
    }

    async fn append_witness_log_record(
        &self,
        record: WitnessLogRecord,
    ) -> Result<WitnessLogRecord> {
        let mut state = self.state.write().await;
        state
            .witness_logs
            .insert(record.id.clone(), record.clone());
        Ok(record)
    }

    async fn list_witness_log_records(&self, session_id: &str) -> Result<Vec<WitnessLogRecord>> {
        let state = self.state.read().await;
        Ok(state
            .witness_logs
            .values()
            .filter(|record| record.session_id == session_id)
            .cloned()
            .collect())
    }
}

#[cfg(any())]
#[async_trait::async_trait]
impl SpacetimeRepository for SdkSpacetimeRepository {
    async fn create_schedule_event(
        &self,
        session_id: String,
        topic: String,
        tool_name: String,
        payload: String,
        actor_id: String,
    ) -> Result<ScheduleEvent> {
        let event = ScheduleEvent {
            id: self.alloc_event_id(),
            session_id,
            topic,
            tool_name,
            payload,
            actor_id,
            status: "queued".into(),
        };

        let event_for_write = event.clone();
        self.with_client(move |client| {
            client.create_schedule_event(event_for_write)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;

        Ok(event)
    }

    async fn update_schedule_status(&self, event_id: u64, status: String) -> Result<()> {
        self.with_client(move |client| {
            client.update_schedule_status(event_id, status)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await
    }

    async fn list_schedule_events(&self, session_id: &str) -> Result<Vec<ScheduleEvent>> {
        let session_id = session_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client
                .list_schedule_events()
                .into_iter()
                .filter(|event| event.session_id == session_id)
                .collect())
        })
        .await
    }

    async fn upsert_agent_state(
        &self,
        session_id: String,
        last_user_message: String,
        last_assistant_message: Option<String>,
    ) -> Result<AgentState> {
        let snapshot = AgentState {
            id: 0,
            session_id,
            last_user_message,
            last_assistant_message,
        };
        let snapshot_for_write = snapshot.clone();

        self.with_client(move |client| {
            client.upsert_agent_state(snapshot_for_write)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;

        Ok(snapshot)
    }

    async fn get_agent_state(&self, session_id: &str) -> Result<Option<AgentState>> {
        let session_id = session_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client.get_agent_state(&session_id))
        })
        .await
    }

    async fn upsert_knowledge(
        &self,
        key: String,
        value: String,
        source: String,
    ) -> Result<KnowledgeRecord> {
        let record = KnowledgeRecord {
            id: 0,
            key,
            value,
            source,
        };
        let record_for_write = record.clone();

        self.with_client(move |client| {
            client.upsert_knowledge(record_for_write)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;

        Ok(record)
    }

    async fn get_knowledge(&self, key: &str) -> Result<Option<KnowledgeRecord>> {
        let key = key.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client.get_knowledge(&key))
        })
        .await
    }

    async fn list_knowledge_by_prefix(&self, prefix: &str) -> Result<Vec<KnowledgeRecord>> {
        let prefix = prefix.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client.list_knowledge_by_prefix(&prefix))
        })
        .await
    }

    async fn grant_permissions(
        &self,
        actor_id: String,
        permissions: Vec<PermissionAction>,
    ) -> Result<PermissionGrant> {
        let permissions_for_result = permissions.clone();
        self.with_client(move |client| {
            client.grant_permissions(actor_id.clone(), permissions)?;
            let _ = client.frame_tick();
            Ok(PermissionGrant {
                actor_id,
                permissions: permissions_for_result,
            })
        })
        .await
    }

    async fn has_permission(&self, actor_id: &str, action: PermissionAction) -> Result<bool> {
        let actor_id = actor_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client.get_permission_grant(&actor_id).is_some_and(|grant| {
                grant.permissions.contains(&action) || grant.permissions.contains(&PermissionAction::Admin)
            }))
        })
        .await
    }

    async fn upsert_reflexion_episode(
        &self,
        record: ReflexionEpisodeRecord,
    ) -> Result<ReflexionEpisodeRecord> {
        let value = record.clone();
        self.with_client(move |client| {
            client.upsert_reflexion_episode(value)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;
        Ok(record)
    }

    async fn list_reflexion_episodes(&self, session_id: &str) -> Result<Vec<ReflexionEpisodeRecord>> {
        let session_id = session_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client
                .list_reflexion_episodes()
                .into_iter()
                .filter(|record| record.session_id == session_id)
                .collect())
        })
        .await
    }

    async fn upsert_skill_library_record(
        &self,
        record: SkillLibraryRecord,
    ) -> Result<SkillLibraryRecord> {
        let value = record.clone();
        self.with_client(move |client| {
            client.upsert_skill_library_record(value)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;
        Ok(record)
    }

    async fn list_skill_library_records(&self, session_id: &str) -> Result<Vec<SkillLibraryRecord>> {
        let session_id = session_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client
                .list_skill_library_records()
                .into_iter()
                .filter(|record| record.session_id == session_id)
                .collect())
        })
        .await
    }

    async fn upsert_causal_edge_record(
        &self,
        record: CausalEdgeRecord,
    ) -> Result<CausalEdgeRecord> {
        let value = record.clone();
        self.with_client(move |client| {
            client.upsert_causal_edge_record(value)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;
        Ok(record)
    }

    async fn list_causal_edge_records(&self, session_id: &str) -> Result<Vec<CausalEdgeRecord>> {
        let session_id = session_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client
                .list_causal_edge_records()
                .into_iter()
                .filter(|record| record.session_id == session_id)
                .collect())
        })
        .await
    }

    async fn upsert_learning_session_record(
        &self,
        record: LearningSessionRecord,
    ) -> Result<LearningSessionRecord> {
        let value = record.clone();
        self.with_client(move |client| {
            client.upsert_learning_session_record(value)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;
        Ok(record)
    }

    async fn list_learning_session_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<LearningSessionRecord>> {
        let session_id = session_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client
                .list_learning_session_records()
                .into_iter()
                .filter(|record| record.session_id == session_id)
                .collect())
        })
        .await
    }

    async fn append_witness_log_record(
        &self,
        record: WitnessLogRecord,
    ) -> Result<WitnessLogRecord> {
        let value = record.clone();
        self.with_client(move |client| {
            client.append_witness_log_record(value)?;
            let _ = client.frame_tick();
            Ok(())
        })
        .await?;
        Ok(record)
    }

    async fn list_witness_log_records(&self, session_id: &str) -> Result<Vec<WitnessLogRecord>> {
        let session_id = session_id.to_string();
        self.with_client(move |client| {
            let _ = client.frame_tick();
            Ok(client
                .list_witness_log_records()
                .into_iter()
                .filter(|record| record.session_id == session_id)
                .collect())
        })
        .await
    }
}

#[derive(Clone)]
pub struct StateStore {
    config: StateStoreConfig,
    repo: Arc<dyn SpacetimeRepository>,
    knowledge_mirror: Arc<StdMutex<Option<KnowledgeMirrorConfig>>>,
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

#[derive(Clone)]
struct KnowledgeMirrorConfig {
    postgres: PostgresDb,
    mode: KnowledgeMirrorMode,
    read_preference: KnowledgeReadPreference,
    shadow_read_rollout_percent: u8,
}

#[derive(Debug, Clone)]
struct ControlledRollbackWindow {
    until_ms: u64,
    ticket: String,
    reason: String,
}

impl StateStore {
    pub fn try_from_config(config: &StateStoreConfig) -> Result<Self> {
        let repo: Arc<dyn SpacetimeRepository> = match config.backend {
            StateStoreBackend::InMemory => Arc::new(InMemorySpacetimeRepository::default()),
        };

        Ok(Self {
            config: config.clone(),
            repo,
            knowledge_mirror: Arc::new(StdMutex::new(None)),
        })
    }

    pub fn from_config(config: &StateStoreConfig) -> Self {
        Self::try_from_config(config).expect("failed to initialize StateStore backend")
    }

    pub fn validate(&self) -> Result<()> {
        if self.config.enabled && self.config.uri.trim().is_empty() {
            bail!("state_store.uri must not be empty when enabled");
        }
        if self.config.enabled && self.config.pool_size == 0 {
            bail!("state_store.pool_size must be greater than 0");
        }
        Ok(())
    }

    pub fn configure_knowledge_mirror(
        &self,
        postgres: PostgresDb,
        mode: KnowledgeMirrorMode,
        read_preference: KnowledgeReadPreference,
        shadow_read_rollout_percent: u8,
    ) -> Result<()> {
        let mut guard = self
            .knowledge_mirror
            .lock()
            .expect("knowledge mirror mutex poisoned");
        *guard = Some(KnowledgeMirrorConfig {
            postgres,
            mode,
            read_preference,
            shadow_read_rollout_percent: shadow_read_rollout_percent.min(100),
        });
        Ok(())
    }

    pub fn clear_knowledge_mirror(&self) {
        let mut guard = self
            .knowledge_mirror
            .lock()
            .expect("knowledge mirror mutex poisoned");
        *guard = None;
    }

    fn knowledge_mirror_snapshot(&self) -> Option<KnowledgeMirrorConfig> {
        self.knowledge_mirror
            .lock()
            .expect("knowledge mirror mutex poisoned")
            .clone()
    }

    fn fallback_allowed_on_error(mode: KnowledgeMirrorMode) -> bool {
        matches!(mode, KnowledgeMirrorMode::Shadow)
    }

    fn parse_controlled_rollback_window() -> Option<ControlledRollbackWindow> {
        let until_ms = std::env::var("AUTOLOOP_STORAGE_ROLLBACK_UNTIL_MS")
            .ok()?
            .parse::<u64>()
            .ok()?;
        let now_ms = shadow_now_ms();
        if now_ms >= until_ms {
            return None;
        }
        let ticket = std::env::var("AUTOLOOP_STORAGE_ROLLBACK_TICKET")
            .unwrap_or_else(|_| "manual-rollback-ticket-missing".into());
        let reason = std::env::var("AUTOLOOP_STORAGE_ROLLBACK_REASON")
            .unwrap_or_else(|_| "temporary controlled rollback".into());
        Some(ControlledRollbackWindow {
            until_ms,
            ticket,
            reason,
        })
    }

    async fn record_controlled_rollback_event(
        &self,
        domain: &str,
        operation: &str,
        target: &str,
        error: &anyhow::Error,
    ) -> Result<()> {
        let Some(window) = Self::parse_controlled_rollback_window() else {
            return Ok(());
        };
        let at_ms = shadow_now_ms();
        let key = format!(
            "storage:rollback:event:{domain}:{operation}:{target}:{at_ms}"
        );
        let payload = serde_json::json!({
            "domain": domain,
            "operation": operation,
            "target": target,
            "error": error.to_string(),
            "ticket": window.ticket,
            "reason": window.reason,
            "until_ms": window.until_ms,
            "at_ms": at_ms,
        });
        let _ = self
            .repo
            .upsert_knowledge(key, payload.to_string(), "storage-rollback".into())
            .await;
        Ok(())
    }

    pub async fn enforce_permission(&self, actor_id: &str, action: PermissionAction) -> Result<()> {
        if !self.has_permission(actor_id, action).await? {
            bail!("actor '{actor_id}' does not have permission '{action:?}'");
        }
        Ok(())
    }

    pub async fn has_permission(&self, actor_id: &str, action: PermissionAction) -> Result<bool> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .has_permission(actor_id, permission_action_to_postgres(action)?)
                .await
            {
                Ok(allowed) => return Ok(allowed),
                Err(error) => {
                    if !Self::fallback_allowed_on_error(mirror.mode) {
                        return Err(error);
                    }
                }
            }
        }
        self.repo.has_permission(actor_id, action).await
    }

    pub async fn create_schedule_event(
        &self,
        session_id: String,
        topic: String,
        tool_name: String,
        payload: String,
        actor_id: String,
    ) -> Result<ScheduleEvent> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .create_schedule_event(
                    session_id.clone(),
                    topic.clone(),
                    tool_name.clone(),
                    payload.clone(),
                    actor_id.clone(),
                )
                .await
            {
                Ok(event) => return Ok(schedule_event_from_postgres(event)),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "scheduler",
                        "create_schedule_event",
                        "session",
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.repo
            .create_schedule_event(session_id, topic, tool_name, payload, actor_id)
            .await
    }

    pub async fn update_schedule_status(&self, event_id: u64, status: impl Into<String>) -> Result<()> {
        let status = status.into();
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror.postgres.update_schedule_status(event_id, status.clone()).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "scheduler",
                        "update_schedule_status",
                        &event_id.to_string(),
                        &error,
                    )
                    .await?;
                }
            }
        }
        self.repo.update_schedule_status(event_id, status).await
    }

    pub async fn list_schedule_events(&self, session_id: &str) -> Result<Vec<ScheduleEvent>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.repo.list_schedule_events(session_id).await;
        };
        match mirror.postgres.list_schedule_events(session_id).await {
            Ok(events) => Ok(events.into_iter().map(schedule_event_from_postgres).collect()),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.repo.list_schedule_events(session_id).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_agent_state(
        &self,
        session_id: String,
        last_user_message: String,
        last_assistant_message: Option<String>,
    ) -> Result<AgentState> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_agent_state(
                    session_id.clone(),
                    last_user_message.clone(),
                    last_assistant_message.clone(),
                )
                .await
            {
                Ok(state) => return Ok(agent_state_from_postgres(state)),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "scheduler",
                        "upsert_agent_state",
                        "session",
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.repo
            .upsert_agent_state(session_id, last_user_message, last_assistant_message)
            .await
    }

    pub async fn get_agent_state(&self, session_id: &str) -> Result<Option<AgentState>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.repo.get_agent_state(session_id).await;
        };
        match mirror.postgres.get_agent_state(session_id).await {
            Ok(Some(state)) => Ok(Some(agent_state_from_postgres(state))),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.repo.get_agent_state(session_id).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_knowledge(
        &self,
        key: String,
        value: String,
        source: String,
    ) -> Result<KnowledgeRecord> {
        self.enforce_evidence_worm_write(&key, &value).await?;
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_knowledge(key.clone(), value.clone(), source.clone())
                .await
            {
                Ok(written) => {
                    return Ok(KnowledgeRecord {
                        id: written.id,
                        key: written.key,
                        value: written.value,
                        source: written.source,
                    })
                }
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "knowledge",
                        "upsert_knowledge",
                        &key,
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.repo.upsert_knowledge(key, value, source).await
    }

    async fn enforce_evidence_worm_write(&self, key: &str, value: &str) -> Result<()> {
        if !key.starts_with("evidence:") {
            return Ok(());
        }

        if let Some(existing) = self.repo.get_knowledge(key).await? {
            if existing.value != value {
                bail!("evidence WORM violation: update attempt on existing key '{}'", key);
            }
            bail!("evidence WORM violation: replay/duplicate write on key '{}'", key);
        }

        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            if let Ok(Some(existing)) = mirror.postgres.get_knowledge(key).await {
                if existing.value != value {
                    bail!(
                        "evidence WORM violation: mirror update attempt on existing key '{}'",
                        key
                    );
                }
                bail!(
                    "evidence WORM violation: mirror replay/duplicate write on key '{}'",
                    key
                );
            }
        }
        Ok(())
    }

    pub async fn upsert_json_knowledge<T: Serialize>(
        &self,
        key: impl Into<String>,
        value: &T,
        source: impl Into<String>,
    ) -> Result<KnowledgeRecord> {
        self.upsert_knowledge(
            key.into(),
            serde_json::to_string(value)?,
            source.into(),
        )
        .await
    }

    pub async fn atomic_write_relation_bundle(&self, input: AtomicRelationWriteInput) -> Result<String> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            bail!("atomic relation write requires postgres mirror");
        };
        mirror
            .postgres
            .atomic_write_relation_bundle(PostgresAtomicRelationWriteInput {
                session_id: input.session_id,
                trace_id: input.trace_id,
                state_key: input.state_key,
                state_payload: input.state_payload,
                relation_event_key: input.relation_event_key,
                relation_event_payload: input.relation_event_payload,
                evidence_key: input.evidence_key,
                evidence_payload: input.evidence_payload,
                write_proof_key: input.write_proof_key,
                write_proof_payload: input.write_proof_payload,
                source: input.source,
                edge_current: input
                    .edge_current
                    .map(|value| convert_struct(value))
                    .transpose()?,
                event_append: input
                    .event_append
                    .map(|value| convert_struct(value))
                    .transpose()?,
                hot_index_entries: input
                    .hot_index_entries
                    .into_iter()
                    .map(convert_struct)
                    .collect::<Result<Vec<_>>>()?,
            })
            .await
    }

    pub async fn list_relation_edges_current(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<RelationEdgeCurrentRecord>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            let mut fallback = self
                .fallback_kv_list::<serde_json::Value>(&format!("relation:state:{session_id}:"))
                .await?
                .into_iter()
                .filter(|value| {
                    value.get("kind").and_then(serde_json::Value::as_str) == Some("relation_edge")
                })
                .filter_map(|value| {
                    let payload = value.get("edge")?.clone();
                    Some(RelationEdgeCurrentRecord {
                        session_id: value
                            .get("session_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(session_id)
                            .to_string(),
                        trace_id: value
                            .get("trace_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        edge_id: payload
                            .get("edge_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        from_node: payload
                            .get("from_node")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        to_node: payload
                            .get("to_node")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        edge_type: payload
                            .get("edge_type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        payload,
                        evidence_ref: value
                            .get("evidence_ref")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        updated_at_ms: value
                            .get("updated_at_ms")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0),
                    })
                })
                .collect::<Vec<_>>();
            fallback.sort_by(|left, right| right.updated_at_ms.cmp(&left.updated_at_ms));
            fallback.truncate(limit.max(1));
            return Ok(fallback);
        };

        let rows = mirror.postgres.list_relation_edges(session_id, limit).await?;
        rows.into_iter().map(convert_struct).collect()
    }

    pub async fn list_relation_events(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<RelationEventRecord>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            let mut fallback = self
                .fallback_kv_list::<serde_json::Value>(&format!("relation:state:{session_id}:"))
                .await?
                .into_iter()
                .filter(|value| {
                    value.get("kind").and_then(serde_json::Value::as_str) == Some("relation_event")
                })
                .filter_map(|value| {
                    let payload = value.get("event")?.clone();
                    Some(RelationEventRecord {
                        session_id: value
                            .get("session_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or(session_id)
                            .to_string(),
                        trace_id: value
                            .get("trace_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        event_id: payload
                            .get("event_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        event_type: payload
                            .get("event_type")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        payload,
                        evidence_ref: value
                            .get("evidence_ref")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        created_at_ms: value
                            .get("updated_at_ms")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0),
                    })
                })
                .collect::<Vec<_>>();
            fallback.sort_by(|left, right| right.created_at_ms.cmp(&left.created_at_ms));
            fallback.truncate(limit.max(1));
            return Ok(fallback);
        };

        let rows = mirror.postgres.list_relation_events(session_id, limit).await?;
        rows.into_iter().map(convert_struct).collect()
    }

    pub async fn list_relation_hot_index(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<RelationHotIndexEntry>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            let mut fallback = self
                .fallback_kv_list::<RelationHotIndexEntry>(&format!("relation:hot-index:{session_id}:"))
                .await?;
            fallback.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
            });
            fallback.truncate(limit.max(1));
            return Ok(fallback);
        };

        let rows = mirror.postgres.list_relation_hot_index(session_id, limit).await?;
        rows.into_iter().map(convert_struct).collect()
    }

    pub async fn get_knowledge(&self, key: &str) -> Result<Option<KnowledgeRecord>> {
        let primary = self.repo.get_knowledge(key).await?;
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(primary);
        };

        let effective_read_preference = self.effective_shadow_read_preference(&mirror, key);
        match effective_read_preference {
            KnowledgeReadPreference::PrimaryStore => Ok(primary),
            KnowledgeReadPreference::Postgres => match mirror.postgres.get_knowledge(key).await {
                Ok(Some(record)) => Ok(Some(KnowledgeRecord {
                    id: record.id,
                    key: record.key,
                    value: record.value,
                    source: record.source,
                })),
                Ok(None) => {
                    if matches!(mirror.mode, KnowledgeMirrorMode::Enforced) {
                        Ok(None)
                    } else {
                        Ok(primary)
                    }
                }
                Err(error) => {
                    if matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
                        Ok(primary)
                    } else {
                        Err(error)
                    }
                }
            },
        }
    }

    pub async fn list_knowledge_by_prefix(&self, prefix: &str) -> Result<Vec<KnowledgeRecord>> {
        let primary = self.repo.list_knowledge_by_prefix(prefix).await?;
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(primary);
        };

        let effective_read_preference = self.effective_shadow_read_preference(&mirror, prefix);
        match effective_read_preference {
            KnowledgeReadPreference::PrimaryStore => Ok(primary),
            KnowledgeReadPreference::Postgres => {
                match mirror.postgres.list_knowledge_by_prefix(prefix).await {
                    Ok(records) => Ok(records
                        .into_iter()
                        .map(|record| KnowledgeRecord {
                            id: record.id,
                            key: record.key,
                            value: record.value,
                            source: record.source,
                        })
                        .collect()),
                    Err(error) => {
                        if matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
                            Ok(primary)
                        } else {
                            Err(error)
                        }
                    }
                }
            }
        }
    }

    fn effective_shadow_read_preference(
        &self,
        mirror: &KnowledgeMirrorConfig,
        target: &str,
    ) -> KnowledgeReadPreference {
        if !matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
            return mirror.read_preference;
        }
        if !matches!(mirror.read_preference, KnowledgeReadPreference::PrimaryStore) {
            return mirror.read_preference;
        }
        if mirror.shadow_read_rollout_percent == 0 {
            return KnowledgeReadPreference::PrimaryStore;
        }

        let bucket = stable_percent_bucket(target);
        if bucket < mirror.shadow_read_rollout_percent {
            KnowledgeReadPreference::Postgres
        } else {
            KnowledgeReadPreference::PrimaryStore
        }
    }

    pub async fn compare_shadow_get_knowledge(
        &self,
        key: &str,
    ) -> Result<Option<ShadowReadDiffReport>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(None);
        };
        if !matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
            return Ok(None);
        }

        let primary = self.repo.get_knowledge(key).await?;
        let primary_count = usize::from(primary.is_some());
        match mirror.postgres.get_knowledge(key).await {
            Ok(mirror_record) => {
                let mirror_count = usize::from(mirror_record.is_some());
                let mut missing_in_mirror = Vec::new();
                let mut extra_in_mirror = Vec::new();
                let mut value_mismatches = Vec::new();
                let mismatch = match (primary.as_ref(), mirror_record.as_ref()) {
                    (Some(left), Some(right)) => {
                        let same = left.key == right.key
                            && left.value == right.value
                            && left.source == right.source;
                        if !same {
                            value_mismatches.push(left.key.clone());
                        }
                        !same
                    }
                    (Some(left), None) => {
                        missing_in_mirror.push(left.key.clone());
                        true
                    }
                    (None, Some(right)) => {
                        extra_in_mirror.push(right.key.clone());
                        true
                    }
                    (None, None) => false,
                };

                let reason = if mismatch {
                    Some("state_store/postgres record mismatch".to_string())
                } else {
                    None
                };
                let diff_reason = reason
                    .clone()
                    .unwrap_or_else(|| "shadow read consistent".to_string());
                let diff_classes = classify_shadow_diff(
                    &missing_in_mirror,
                    &extra_in_mirror,
                    &value_mismatches,
                    reason.as_deref(),
                );
                Ok(Some(ShadowReadDiffReport {
                    domain: "knowledge".into(),
                    probe: "get_knowledge".into(),
                    target: key.to_string(),
                    mismatch,
                    reason,
                    diff_reason,
                    diff_classes,
                    old: serde_json::json!({
                        "store": "state_store",
                        "primary_count": primary_count,
                        "missing": missing_in_mirror,
                        "extra": extra_in_mirror,
                        "value_mismatches": value_mismatches,
                    }),
                    new: serde_json::json!({
                        "store": "postgres",
                        "mirror_count": mirror_count,
                    }),
                    evidence_ref: make_shadow_evidence_ref("knowledge", "get_knowledge", key),
                    primary_count,
                    mirror_count: Some(mirror_count),
                    missing_in_mirror,
                    extra_in_mirror,
                    value_mismatches,
                    generated_at_ms: shadow_now_ms(),
                }))
            }
            Err(error) => {
                let reason = Some(format!("postgres read failed: {error}"));
                let diff_classes = classify_shadow_diff(&[], &[], &[], reason.as_deref());
                Ok(Some(ShadowReadDiffReport {
                domain: "knowledge".into(),
                probe: "get_knowledge".into(),
                target: key.to_string(),
                mismatch: true,
                diff_reason: reason.clone().unwrap_or_else(|| "shadow read failed".into()),
                reason,
                diff_classes,
                old: serde_json::json!({
                    "store": "state_store",
                    "primary_count": primary_count,
                }),
                new: serde_json::json!({
                    "store": "postgres",
                }),
                evidence_ref: make_shadow_evidence_ref("knowledge", "get_knowledge", key),
                primary_count,
                mirror_count: None,
                missing_in_mirror: Vec::new(),
                extra_in_mirror: Vec::new(),
                value_mismatches: Vec::new(),
                generated_at_ms: shadow_now_ms(),
            }))
            }
        }
    }

    pub async fn compare_shadow_list_knowledge_by_prefix(
        &self,
        prefix: &str,
    ) -> Result<Option<ShadowReadDiffReport>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(None);
        };
        if !matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
            return Ok(None);
        }

        let primary = self.repo.list_knowledge_by_prefix(prefix).await?;
        let primary_count = primary.len();
        match mirror.postgres.list_knowledge_by_prefix(prefix).await {
            Ok(mirror_records) => {
                let mirror_count = mirror_records.len();
                let primary_map = primary
                    .into_iter()
                    .map(|record| (record.key, (record.value, record.source)))
                    .collect::<HashMap<_, _>>();
                let mirror_map = mirror_records
                    .into_iter()
                    .map(|record| (record.key, (record.value, record.source)))
                    .collect::<HashMap<_, _>>();

                let mut missing_in_mirror = primary_map
                    .keys()
                    .filter(|key| !mirror_map.contains_key(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                let mut extra_in_mirror = mirror_map
                    .keys()
                    .filter(|key| !primary_map.contains_key(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                let mut value_mismatches = primary_map
                    .iter()
                    .filter_map(|(key, left)| {
                        mirror_map.get(key).and_then(|right| {
                            if left == right {
                                None
                            } else {
                                Some(key.clone())
                            }
                        })
                    })
                    .collect::<Vec<_>>();

                missing_in_mirror.sort();
                extra_in_mirror.sort();
                value_mismatches.sort();

                let mismatch = !missing_in_mirror.is_empty()
                    || !extra_in_mirror.is_empty()
                    || !value_mismatches.is_empty();
                let reason = if mismatch {
                    Some("state_store/postgres prefix listing mismatch".to_string())
                } else {
                    None
                };
                let diff_reason = reason
                    .clone()
                    .unwrap_or_else(|| "shadow read consistent".to_string());
                let diff_classes = classify_shadow_diff(
                    &missing_in_mirror,
                    &extra_in_mirror,
                    &value_mismatches,
                    reason.as_deref(),
                );
                Ok(Some(ShadowReadDiffReport {
                    domain: "knowledge".into(),
                    probe: "list_knowledge_by_prefix".into(),
                    target: prefix.to_string(),
                    mismatch,
                    reason,
                    diff_reason,
                    diff_classes,
                    old: serde_json::json!({
                        "store": "state_store",
                        "primary_count": primary_count,
                        "missing": missing_in_mirror,
                        "extra": extra_in_mirror,
                        "value_mismatches": value_mismatches,
                    }),
                    new: serde_json::json!({
                        "store": "postgres",
                        "mirror_count": mirror_count,
                    }),
                    evidence_ref: make_shadow_evidence_ref("knowledge", "list_knowledge_by_prefix", prefix),
                    primary_count,
                    mirror_count: Some(mirror_count),
                    missing_in_mirror,
                    extra_in_mirror,
                    value_mismatches,
                    generated_at_ms: shadow_now_ms(),
                }))
            }
            Err(error) => {
                let reason = Some(format!("postgres list failed: {error}"));
                let diff_classes = classify_shadow_diff(&[], &[], &[], reason.as_deref());
                Ok(Some(ShadowReadDiffReport {
                domain: "knowledge".into(),
                probe: "list_knowledge_by_prefix".into(),
                target: prefix.to_string(),
                mismatch: true,
                reason: reason.clone(),
                diff_reason: reason.unwrap_or_else(|| "shadow read failed".into()),
                diff_classes,
                old: serde_json::json!({
                    "store": "state_store",
                    "primary_count": primary_count,
                }),
                new: serde_json::json!({
                    "store": "postgres",
                }),
                evidence_ref: make_shadow_evidence_ref("knowledge", "list_knowledge_by_prefix", prefix),
                primary_count,
                mirror_count: None,
                missing_in_mirror: Vec::new(),
                extra_in_mirror: Vec::new(),
                value_mismatches: Vec::new(),
                generated_at_ms: shadow_now_ms(),
            }))
            }
        }
    }

    pub async fn compare_shadow_schedule_events(
        &self,
        session_id: &str,
    ) -> Result<Option<ShadowReadDiffReport>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(None);
        };
        if !matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
            return Ok(None);
        }

        let primary = self.repo.list_schedule_events(session_id).await?;
        let primary_count = primary.len();
        match mirror.postgres.list_schedule_events(session_id).await {
            Ok(mirror_events) => {
                let mirror_count = mirror_events.len();
                let primary_map = primary
                    .iter()
                    .map(|event| (event.id.to_string(), event))
                    .collect::<HashMap<_, _>>();
                let mirror_map = mirror_events
                    .iter()
                    .map(|event| (event.id.to_string(), event))
                    .collect::<HashMap<_, _>>();

                let mut missing_in_mirror = primary_map
                    .keys()
                    .filter(|key| !mirror_map.contains_key(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                let mut extra_in_mirror = mirror_map
                    .keys()
                    .filter(|key| !primary_map.contains_key(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                let mut value_mismatches = primary_map
                    .keys()
                    .filter_map(|key| {
                        primary_map.get(key).and_then(|left| {
                            mirror_map.get(key).and_then(|right| {
                                let same = left.session_id == right.session_id
                                    && left.topic == right.topic
                                    && left.tool_name == right.tool_name
                                    && left.payload == right.payload
                                    && left.actor_id == right.actor_id
                                    && left.status == right.status;
                                if same { None } else { Some(key.clone()) }
                            })
                        })
                    })
                    .collect::<Vec<_>>();

                missing_in_mirror.sort();
                extra_in_mirror.sort();
                value_mismatches.sort();
                let mismatch = !missing_in_mirror.is_empty()
                    || !extra_in_mirror.is_empty()
                    || !value_mismatches.is_empty();

                let reason = if mismatch {
                    Some("state_store/postgres schedule events mismatch".to_string())
                } else {
                    None
                };
                let diff_reason = reason
                    .clone()
                    .unwrap_or_else(|| "shadow read consistent".to_string());
                let diff_classes = classify_shadow_diff(
                    &missing_in_mirror,
                    &extra_in_mirror,
                    &value_mismatches,
                    reason.as_deref(),
                );
                Ok(Some(ShadowReadDiffReport {
                    domain: "scheduler".into(),
                    probe: "schedule_events".into(),
                    target: session_id.to_string(),
                    mismatch,
                    reason,
                    diff_reason,
                    diff_classes,
                    old: serde_json::json!({
                        "store": "state_store",
                        "primary_count": primary_count,
                        "missing": missing_in_mirror,
                        "extra": extra_in_mirror,
                        "value_mismatches": value_mismatches,
                    }),
                    new: serde_json::json!({
                        "store": "postgres",
                        "mirror_count": mirror_count,
                    }),
                    evidence_ref: make_shadow_evidence_ref("scheduler", "schedule_events", session_id),
                    primary_count,
                    mirror_count: Some(mirror_count),
                    missing_in_mirror,
                    extra_in_mirror,
                    value_mismatches,
                    generated_at_ms: shadow_now_ms(),
                }))
            }
            Err(error) => {
                let reason = Some(format!("postgres schedule read failed: {error}"));
                let diff_classes = classify_shadow_diff(&[], &[], &[], reason.as_deref());
                Ok(Some(ShadowReadDiffReport {
                domain: "scheduler".into(),
                probe: "schedule_events".into(),
                target: session_id.to_string(),
                mismatch: true,
                reason: reason.clone(),
                diff_reason: reason.unwrap_or_else(|| "shadow read failed".into()),
                diff_classes,
                old: serde_json::json!({
                    "store": "state_store",
                    "primary_count": primary_count,
                }),
                new: serde_json::json!({
                    "store": "postgres",
                }),
                evidence_ref: make_shadow_evidence_ref("scheduler", "schedule_events", session_id),
                primary_count,
                mirror_count: None,
                missing_in_mirror: Vec::new(),
                extra_in_mirror: Vec::new(),
                value_mismatches: Vec::new(),
                generated_at_ms: shadow_now_ms(),
            }))
            }
        }
    }

    pub async fn compare_shadow_agent_state(
        &self,
        session_id: &str,
    ) -> Result<Option<ShadowReadDiffReport>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(None);
        };
        if !matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
            return Ok(None);
        }

        let primary = self.repo.get_agent_state(session_id).await?;
        let primary_count = usize::from(primary.is_some());
        match mirror.postgres.get_agent_state(session_id).await {
            Ok(mirror_state) => {
                let mirror_count = usize::from(mirror_state.is_some());
                let mut missing_in_mirror = Vec::new();
                let mut extra_in_mirror = Vec::new();
                let mut value_mismatches = Vec::new();
                let mismatch = match (primary.as_ref(), mirror_state.as_ref()) {
                    (Some(left), Some(right)) => {
                        let same = left.session_id == right.session_id
                            && left.last_user_message == right.last_user_message
                            && left.last_assistant_message == right.last_assistant_message;
                        if !same {
                            value_mismatches.push(session_id.to_string());
                        }
                        !same
                    }
                    (Some(_), None) => {
                        missing_in_mirror.push(session_id.to_string());
                        true
                    }
                    (None, Some(_)) => {
                        extra_in_mirror.push(session_id.to_string());
                        true
                    }
                    (None, None) => false,
                };
                let reason = if mismatch {
                    Some("state_store/postgres agent_state mismatch".to_string())
                } else {
                    None
                };
                let diff_reason = reason
                    .clone()
                    .unwrap_or_else(|| "shadow read consistent".to_string());
                let diff_classes = classify_shadow_diff(
                    &missing_in_mirror,
                    &extra_in_mirror,
                    &value_mismatches,
                    reason.as_deref(),
                );
                Ok(Some(ShadowReadDiffReport {
                    domain: "scheduler".into(),
                    probe: "agent_state".into(),
                    target: session_id.to_string(),
                    mismatch,
                    reason,
                    diff_reason,
                    diff_classes,
                    old: serde_json::json!({
                        "store": "state_store",
                        "primary_count": primary_count,
                        "missing": missing_in_mirror,
                        "extra": extra_in_mirror,
                        "value_mismatches": value_mismatches,
                    }),
                    new: serde_json::json!({
                        "store": "postgres",
                        "mirror_count": mirror_count,
                    }),
                    evidence_ref: make_shadow_evidence_ref("scheduler", "agent_state", session_id),
                    primary_count,
                    mirror_count: Some(mirror_count),
                    missing_in_mirror,
                    extra_in_mirror,
                    value_mismatches,
                    generated_at_ms: shadow_now_ms(),
                }))
            }
            Err(error) => {
                let reason = Some(format!("postgres agent_state read failed: {error}"));
                let diff_classes = classify_shadow_diff(&[], &[], &[], reason.as_deref());
                Ok(Some(ShadowReadDiffReport {
                domain: "scheduler".into(),
                probe: "agent_state".into(),
                target: session_id.to_string(),
                mismatch: true,
                reason: reason.clone(),
                diff_reason: reason.unwrap_or_else(|| "shadow read failed".into()),
                diff_classes,
                old: serde_json::json!({
                    "store": "state_store",
                    "primary_count": primary_count,
                }),
                new: serde_json::json!({
                    "store": "postgres",
                }),
                evidence_ref: make_shadow_evidence_ref("scheduler", "agent_state", session_id),
                primary_count,
                mirror_count: None,
                missing_in_mirror: Vec::new(),
                extra_in_mirror: Vec::new(),
                value_mismatches: Vec::new(),
                generated_at_ms: shadow_now_ms(),
            }))
            }
        }
    }

    pub async fn compare_shadow_session_lease(
        &self,
        session_id: &str,
    ) -> Result<Option<ShadowReadDiffReport>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(None);
        };
        if !matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
            return Ok(None);
        }

        let primary = self
            .repo
            .get_knowledge(&format!("identity:session-lease:{session_id}"))
            .await?
            .and_then(|record| serde_json::from_str::<SessionLease>(&record.value).ok());
        let primary_count = usize::from(primary.is_some());
        match mirror.postgres.get_session_lease(session_id).await {
            Ok(mirror_state) => {
                let mirror_count = usize::from(mirror_state.is_some());
                let mut missing_in_mirror = Vec::new();
                let mut extra_in_mirror = Vec::new();
                let mut value_mismatches = Vec::new();
                let mismatch = match (primary.as_ref(), mirror_state.as_ref()) {
                    (Some(left), Some(right)) => {
                        let right_local = session_lease_from_postgres(right.clone())?;
                        let same = left.lease_token == right_local.lease_token
                            && left.session_id == right_local.session_id
                            && left.tenant_id == right_local.tenant_id
                            && left.principal_id == right_local.principal_id
                            && left.policy_id == right_local.policy_id
                            && left.expires_at_ms == right_local.expires_at_ms
                            && left.issued_at_ms == right_local.issued_at_ms;
                        if !same {
                            value_mismatches.push(session_id.to_string());
                        }
                        !same
                    }
                    (Some(_), None) => {
                        missing_in_mirror.push(session_id.to_string());
                        true
                    }
                    (None, Some(_)) => {
                        extra_in_mirror.push(session_id.to_string());
                        true
                    }
                    (None, None) => false,
                };
                let reason = if mismatch {
                    Some("state_store/postgres session_lease mismatch".to_string())
                } else {
                    None
                };
                let diff_reason = reason
                    .clone()
                    .unwrap_or_else(|| "shadow read consistent".to_string());
                let diff_classes = classify_shadow_diff(
                    &missing_in_mirror,
                    &extra_in_mirror,
                    &value_mismatches,
                    reason.as_deref(),
                );
                Ok(Some(ShadowReadDiffReport {
                    domain: "identity".into(),
                    probe: "session_lease".into(),
                    target: session_id.to_string(),
                    mismatch,
                    reason,
                    diff_reason,
                    diff_classes,
                    old: serde_json::json!({
                        "store": "state_store",
                        "primary_count": primary_count,
                        "missing": missing_in_mirror,
                        "extra": extra_in_mirror,
                        "value_mismatches": value_mismatches,
                    }),
                    new: serde_json::json!({
                        "store": "postgres",
                        "mirror_count": mirror_count,
                    }),
                    evidence_ref: make_shadow_evidence_ref("identity", "session_lease", session_id),
                    primary_count,
                    mirror_count: Some(mirror_count),
                    missing_in_mirror,
                    extra_in_mirror,
                    value_mismatches,
                    generated_at_ms: shadow_now_ms(),
                }))
            }
            Err(error) => {
                let reason = Some(format!("postgres session_lease read failed: {error}"));
                let diff_classes = classify_shadow_diff(&[], &[], &[], reason.as_deref());
                Ok(Some(ShadowReadDiffReport {
                domain: "identity".into(),
                probe: "session_lease".into(),
                target: session_id.to_string(),
                mismatch: true,
                reason: reason.clone(),
                diff_reason: reason.unwrap_or_else(|| "shadow read failed".into()),
                diff_classes,
                old: serde_json::json!({
                    "store": "state_store",
                    "primary_count": primary_count,
                }),
                new: serde_json::json!({
                    "store": "postgres",
                }),
                evidence_ref: make_shadow_evidence_ref("identity", "session_lease", session_id),
                primary_count,
                mirror_count: None,
                missing_in_mirror: Vec::new(),
                extra_in_mirror: Vec::new(),
                value_mismatches: Vec::new(),
                generated_at_ms: shadow_now_ms(),
            }))
            }
        }
    }

    pub async fn compare_shadow_cost_attribution_by_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<Option<ShadowReadDiffReport>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return Ok(None);
        };
        if !matches!(mirror.mode, KnowledgeMirrorMode::Shadow) {
            return Ok(None);
        }

        let prefix = format!("billing:cost-attribution:{tenant_id}:{session_id}:");
        let primary = self
            .repo
            .list_knowledge_by_prefix(&prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<CostAttribution>(&record.value).ok())
            .collect::<Vec<_>>();
        let primary_count = primary.len();
        match mirror
            .postgres
            .list_cost_attribution_by_session(tenant_id, session_id)
            .await
        {
            Ok(mirror_items) => {
                let mirror_count = mirror_items.len();
                let primary_map = primary
                    .iter()
                    .map(|item| (item.attribution_id.clone(), item))
                    .collect::<HashMap<_, _>>();
                let mirror_map = mirror_items
                    .iter()
                    .map(|item| (item.attribution_id.clone(), item))
                    .collect::<HashMap<_, _>>();

                let mut missing_in_mirror = primary_map
                    .keys()
                    .filter(|key| !mirror_map.contains_key(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                let mut extra_in_mirror = mirror_map
                    .keys()
                    .filter(|key| !primary_map.contains_key(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                let mut value_mismatches = primary_map
                    .keys()
                    .filter_map(|key| {
                        primary_map.get(key).and_then(|left| {
                            mirror_map.get(key).and_then(|right| {
                                let right_local = cost_attribution_from_postgres((*right).clone()).ok()?;
                                let same = left.tenant_id == right_local.tenant_id
                                    && left.principal_id == right_local.principal_id
                                    && left.policy_id == right_local.policy_id
                                    && left.session_id == right_local.session_id
                                    && left.trace_id == right_local.trace_id
                                    && left.task_id == right_local.task_id
                                    && left.capability_id == right_local.capability_id
                                    && left.provider_tokens == right_local.provider_tokens
                                    && left.tool_invocations == right_local.tool_invocations
                                    && left.duration_ms == right_local.duration_ms
                                    && left.token_cost_micros == right_local.token_cost_micros
                                    && left.tool_cost_micros == right_local.tool_cost_micros
                                    && left.duration_cost_micros == right_local.duration_cost_micros
                                    && left.total_cost_micros == right_local.total_cost_micros
                                    && left.settled_at_ms == right_local.settled_at_ms;
                                if same { None } else { Some(key.clone()) }
                            })
                        })
                    })
                    .collect::<Vec<_>>();

                missing_in_mirror.sort();
                extra_in_mirror.sort();
                value_mismatches.sort();
                let mismatch = !missing_in_mirror.is_empty()
                    || !extra_in_mirror.is_empty()
                    || !value_mismatches.is_empty();

                let reason = if mismatch {
                    Some("state_store/postgres cost attribution mismatch".to_string())
                } else {
                    None
                };
                let diff_reason = reason
                    .clone()
                    .unwrap_or_else(|| "shadow read consistent".to_string());
                let diff_classes = classify_shadow_diff(
                    &missing_in_mirror,
                    &extra_in_mirror,
                    &value_mismatches,
                    reason.as_deref(),
                );
                Ok(Some(ShadowReadDiffReport {
                    domain: "billing".into(),
                    probe: "cost_attribution".into(),
                    target: format!("{tenant_id}:{session_id}"),
                    mismatch,
                    reason,
                    diff_reason,
                    diff_classes,
                    old: serde_json::json!({
                        "store": "state_store",
                        "primary_count": primary_count,
                        "missing": missing_in_mirror,
                        "extra": extra_in_mirror,
                        "value_mismatches": value_mismatches,
                    }),
                    new: serde_json::json!({
                        "store": "postgres",
                        "mirror_count": mirror_count,
                    }),
                    evidence_ref: make_shadow_evidence_ref(
                        "billing",
                        "cost_attribution",
                        &format!("{tenant_id}:{session_id}"),
                    ),
                    primary_count,
                    mirror_count: Some(mirror_count),
                    missing_in_mirror,
                    extra_in_mirror,
                    value_mismatches,
                    generated_at_ms: shadow_now_ms(),
                }))
            }
            Err(error) => {
                let reason = Some(format!("postgres cost_attribution read failed: {error}"));
                let diff_classes = classify_shadow_diff(&[], &[], &[], reason.as_deref());
                Ok(Some(ShadowReadDiffReport {
                domain: "billing".into(),
                probe: "cost_attribution".into(),
                target: format!("{tenant_id}:{session_id}"),
                mismatch: true,
                reason: reason.clone(),
                diff_reason: reason.unwrap_or_else(|| "shadow read failed".into()),
                diff_classes,
                old: serde_json::json!({
                    "store": "state_store",
                    "primary_count": primary_count,
                }),
                new: serde_json::json!({
                    "store": "postgres",
                }),
                evidence_ref: make_shadow_evidence_ref(
                    "billing",
                    "cost_attribution",
                    &format!("{tenant_id}:{session_id}"),
                ),
                primary_count,
                mirror_count: None,
                missing_in_mirror: Vec::new(),
                extra_in_mirror: Vec::new(),
                value_mismatches: Vec::new(),
                generated_at_ms: shadow_now_ms(),
            }))
            }
        }
    }

    pub async fn grant_permissions(
        &self,
        actor_id: impl Into<String>,
        permissions: Vec<PermissionAction>,
    ) -> Result<PermissionGrant> {
        let actor_id = actor_id.into();
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            let permissions_pg = permissions
                .clone()
                .into_iter()
                .map(permission_action_to_postgres)
                .collect::<Result<Vec<_>>>()?;
            match mirror
                .postgres
                .grant_permissions(actor_id.clone(), permissions_pg)
                .await
            {
                Ok(grant) => return Ok(permission_grant_from_postgres(grant)?),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "identity",
                        "grant_permissions",
                        &actor_id,
                        &error,
                    )
                    .await?;
                }
            }
        }
        self.repo.grant_permissions(actor_id, permissions).await
    }

    async fn fallback_kv_get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        Ok(self
            .repo
            .get_knowledge(key)
            .await?
            .and_then(|record| serde_json::from_str::<T>(&record.value).ok()))
    }

    async fn fallback_kv_list<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<T>> {
        Ok(self
            .repo
            .list_knowledge_by_prefix(prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<T>(&record.value).ok())
            .collect())
    }

    pub async fn upsert_tenant(&self, tenant: Tenant) -> Result<Tenant> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_tenant(tenant_to_postgres(&tenant)?)
                .await
            {
                Ok(record) => return tenant_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "identity",
                        "upsert_tenant",
                        &tenant.tenant_id,
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.upsert_json_knowledge(format!("identity:tenant:{}", tenant.tenant_id), &tenant, "identity")
            .await?;
        Ok(tenant)
    }

    pub async fn get_tenant(&self, tenant_id: &str) -> Result<Option<Tenant>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self
                .fallback_kv_get(&format!("identity:tenant:{tenant_id}"))
                .await;
        };
        match mirror.postgres.get_tenant(tenant_id).await {
            Ok(Some(record)) => Ok(Some(tenant_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&format!("identity:tenant:{tenant_id}")).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_principal(&self, principal: Principal) -> Result<Principal> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_principal(principal_to_postgres(&principal)?)
                .await
            {
                Ok(record) => return principal_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "identity",
                        "upsert_principal",
                        &format!("{}:{}", principal.tenant_id, principal.principal_id),
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.upsert_json_knowledge(
            format!("identity:principal:{}:{}", principal.tenant_id, principal.principal_id),
            &principal,
            "identity",
        )
        .await?;
        Ok(principal)
    }

    pub async fn get_principal(
        &self,
        tenant_id: &str,
        principal_id: &str,
    ) -> Result<Option<Principal>> {
        let key = format!("identity:principal:{tenant_id}:{principal_id}");
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.fallback_kv_get(&key).await;
        };
        match mirror.postgres.get_principal(tenant_id, principal_id).await {
            Ok(Some(record)) => Ok(Some(principal_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&key).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_role_binding(&self, binding: RoleBinding) -> Result<RoleBinding> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_role_binding(role_binding_to_postgres(&binding)?)
                .await
            {
                Ok(record) => return role_binding_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "identity",
                        "upsert_role_binding",
                        &format!("{}:{}", binding.tenant_id, binding.principal_id),
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.upsert_json_knowledge(
            format!(
                "identity:role-binding:{}:{}",
                binding.tenant_id, binding.principal_id
            ),
            &binding,
            "identity",
        )
        .await?;
        Ok(binding)
    }

    pub async fn get_role_binding(
        &self,
        tenant_id: &str,
        principal_id: &str,
    ) -> Result<Option<RoleBinding>> {
        let key = format!("identity:role-binding:{tenant_id}:{principal_id}");
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.fallback_kv_get(&key).await;
        };
        match mirror.postgres.get_role_binding(tenant_id, principal_id).await {
            Ok(Some(record)) => Ok(Some(role_binding_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&key).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_policy_binding(&self, binding: PolicyBinding) -> Result<PolicyBinding> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_policy_binding(policy_binding_to_postgres(&binding)?)
                .await
            {
                Ok(record) => return policy_binding_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "identity",
                        "upsert_policy_binding",
                        &format!("{}:{}", binding.tenant_id, binding.policy_id),
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.upsert_json_knowledge(
            format!("identity:policy-binding:{}:{}", binding.tenant_id, binding.policy_id),
            &binding,
            "identity",
        )
        .await?;
        Ok(binding)
    }

    pub async fn get_policy_binding(
        &self,
        tenant_id: &str,
        policy_id: &str,
    ) -> Result<Option<PolicyBinding>> {
        let key = format!("identity:policy-binding:{tenant_id}:{policy_id}");
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.fallback_kv_get(&key).await;
        };
        match mirror.postgres.get_policy_binding(tenant_id, policy_id).await {
            Ok(Some(record)) => Ok(Some(policy_binding_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&key).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_session_lease(&self, lease: SessionLease) -> Result<SessionLease> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_session_lease(session_lease_to_postgres(&lease)?)
                .await
            {
                Ok(record) => return session_lease_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "identity",
                        "upsert_session_lease",
                        &lease.session_id,
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.upsert_json_knowledge(
            format!("identity:session-lease:{}", lease.session_id),
            &lease,
            "identity",
        )
        .await?;
        Ok(lease)
    }

    pub async fn get_session_lease(&self, session_id: &str) -> Result<Option<SessionLease>> {
        let key = format!("identity:session-lease:{session_id}");
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.fallback_kv_get(&key).await;
        };
        match mirror.postgres.get_session_lease(session_id).await {
            Ok(Some(record)) => Ok(Some(session_lease_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&key).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_budget_account(&self, account: BudgetAccount) -> Result<BudgetAccount> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_budget_account(budget_account_to_postgres(&account)?)
                .await
                {
                    Ok(record) => return budget_account_from_postgres(record),
                    Err(error) => {
                        if Self::parse_controlled_rollback_window().is_none() {
                            return Err(error);
                        }
                        self.record_controlled_rollback_event(
                            "billing",
                            "upsert_budget_account",
                            &format!("{}:{}", account.tenant_id, account.account_id),
                            &error,
                        )
                        .await?;
                    }
                }
        }

        self.upsert_json_knowledge(
            format!("billing:budget-account:{}:{}", account.tenant_id, account.account_id),
            &account,
            "billing",
        )
        .await?;
        Ok(account)
    }

    pub async fn get_budget_account(
        &self,
        tenant_id: &str,
        account_id: &str,
    ) -> Result<Option<BudgetAccount>> {
        let key = format!("billing:budget-account:{tenant_id}:{account_id}");
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.fallback_kv_get(&key).await;
        };
        match mirror.postgres.get_budget_account(tenant_id, account_id).await {
            Ok(Some(record)) => Ok(Some(budget_account_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&key).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn append_spend_ledger(&self, entry: SpendLedger) -> Result<SpendLedger> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .append_spend_ledger(spend_ledger_to_postgres(&entry)?)
                .await
            {
                Ok(record) => return spend_ledger_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "billing",
                        "append_spend_ledger",
                        &format!("{}:{}:{}", entry.tenant_id, entry.account_id, entry.ledger_id),
                        &error,
                    )
                    .await?;
                }
            }
        }

        let key = format!(
            "billing:spend-ledger:{}:{}:{}",
            entry.tenant_id, entry.account_id, entry.ledger_id
        );
        if self.get_knowledge(&key).await?.is_some() {
            bail!("spend ledger {} already exists", entry.ledger_id);
        }
        self.upsert_json_knowledge(key, &entry, "billing").await?;
        Ok(entry)
    }

    pub async fn list_spend_ledger(
        &self,
        tenant_id: &str,
        account_id: &str,
    ) -> Result<Vec<SpendLedger>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            let prefix = format!("billing:spend-ledger:{tenant_id}:{account_id}:");
            let mut fallback = self.fallback_kv_list::<SpendLedger>(&prefix).await?;
            fallback.sort_by_key(|entry| entry.created_at_ms);
            return Ok(fallback);
        };
        match mirror.postgres.list_spend_ledger(tenant_id, account_id).await {
            Ok(entries) => {
                let mut mapped = entries
                    .into_iter()
                    .filter_map(|entry| spend_ledger_from_postgres(entry).ok())
                    .collect::<Vec<_>>();
                mapped.sort_by_key(|entry| entry.created_at_ms);
                Ok(mapped)
            }
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    let prefix = format!("billing:spend-ledger:{tenant_id}:{account_id}:");
                    let mut fallback = self.fallback_kv_list::<SpendLedger>(&prefix).await?;
                    fallback.sort_by_key(|entry| entry.created_at_ms);
                    Ok(fallback)
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn list_spend_ledger_by_task(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<Vec<SpendLedger>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            let mut fallback = self.fallback_kv_list::<SpendLedger>("billing:spend-ledger:").await?;
            fallback.retain(|entry| entry.session_id == session_id && entry.task_id == task_id);
            fallback.sort_by_key(|entry| entry.created_at_ms);
            return Ok(fallback);
        };
        match mirror.postgres.list_spend_ledger_by_task(session_id, task_id).await {
            Ok(entries) => {
                let mut mapped = entries
                    .into_iter()
                    .filter_map(|entry| spend_ledger_from_postgres(entry).ok())
                    .collect::<Vec<_>>();
                mapped.sort_by_key(|entry| entry.created_at_ms);
                Ok(mapped)
            }
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    let mut fallback = self.fallback_kv_list::<SpendLedger>("billing:spend-ledger:").await?;
                    fallback.retain(|entry| entry.session_id == session_id && entry.task_id == task_id);
                    fallback.sort_by_key(|entry| entry.created_at_ms);
                    Ok(fallback)
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_quota_window(&self, window: QuotaWindow) -> Result<QuotaWindow> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_quota_window(quota_window_to_postgres(&window)?)
                .await
            {
                Ok(record) => return quota_window_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "billing",
                        "upsert_quota_window",
                        &format!("{}:{}:{}", window.tenant_id, window.account_id, window.window_id),
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.upsert_json_knowledge(
            format!(
                "billing:quota-window:{}:{}:{}",
                window.tenant_id, window.account_id, window.window_id
            ),
            &window,
            "billing",
        )
        .await?;
        Ok(window)
    }

    pub async fn get_quota_window(
        &self,
        tenant_id: &str,
        account_id: &str,
        window_id: &str,
    ) -> Result<Option<QuotaWindow>> {
        let key = format!("billing:quota-window:{tenant_id}:{account_id}:{window_id}");
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.fallback_kv_get(&key).await;
        };
        match mirror.postgres.get_quota_window(tenant_id, account_id, window_id).await {
            Ok(Some(record)) => Ok(Some(quota_window_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&key).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_cost_attribution(
        &self,
        attribution: CostAttribution,
    ) -> Result<CostAttribution> {
        if let Some(mirror) = self.knowledge_mirror_snapshot() {
            match mirror
                .postgres
                .upsert_cost_attribution(cost_attribution_to_postgres(&attribution)?)
                .await
            {
                Ok(record) => return cost_attribution_from_postgres(record),
                Err(error) => {
                    if Self::parse_controlled_rollback_window().is_none() {
                        return Err(error);
                    }
                    self.record_controlled_rollback_event(
                        "billing",
                        "upsert_cost_attribution",
                        &format!(
                            "{}:{}:{}",
                            attribution.tenant_id, attribution.session_id, attribution.attribution_id
                        ),
                        &error,
                    )
                    .await?;
                }
            }
        }

        self.upsert_json_knowledge(
            format!(
                "billing:cost-attribution:{}:{}:{}",
                attribution.tenant_id, attribution.session_id, attribution.attribution_id
            ),
            &attribution,
            "billing",
        )
        .await?;
        Ok(attribution)
    }

    pub async fn get_cost_attribution(
        &self,
        tenant_id: &str,
        session_id: &str,
        attribution_id: &str,
    ) -> Result<Option<CostAttribution>> {
        let key = format!("billing:cost-attribution:{tenant_id}:{session_id}:{attribution_id}");
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            return self.fallback_kv_get(&key).await;
        };
        match mirror
            .postgres
            .get_cost_attribution(tenant_id, session_id, attribution_id)
            .await
        {
            Ok(Some(record)) => Ok(Some(cost_attribution_from_postgres(record)?)),
            Ok(None) => Ok(None),
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    self.fallback_kv_get(&key).await
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn list_cost_attribution_by_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<Vec<CostAttribution>> {
        let Some(mirror) = self.knowledge_mirror_snapshot() else {
            let prefix = format!("billing:cost-attribution:{tenant_id}:{session_id}:");
            let mut fallback = self.fallback_kv_list::<CostAttribution>(&prefix).await?;
            fallback.sort_by_key(|record| record.settled_at_ms);
            return Ok(fallback);
        };
        match mirror
            .postgres
            .list_cost_attribution_by_session(tenant_id, session_id)
            .await
        {
            Ok(records) => {
                let mut mapped = records
                    .into_iter()
                    .filter_map(|record| cost_attribution_from_postgres(record).ok())
                    .collect::<Vec<_>>();
                mapped.sort_by_key(|record| record.settled_at_ms);
                Ok(mapped)
            }
            Err(error) => {
                if Self::fallback_allowed_on_error(mirror.mode) {
                    let prefix = format!("billing:cost-attribution:{tenant_id}:{session_id}:");
                    let mut fallback = self.fallback_kv_list::<CostAttribution>(&prefix).await?;
                    fallback.sort_by_key(|record| record.settled_at_ms);
                    Ok(fallback)
                } else {
                    Err(error)
                }
            }
        }
    }

    pub async fn upsert_reflexion_episode(
        &self,
        record: ReflexionEpisodeRecord,
    ) -> Result<ReflexionEpisodeRecord> {
        self.repo.upsert_reflexion_episode(record).await
    }

    pub async fn list_reflexion_episodes(&self, session_id: &str) -> Result<Vec<ReflexionEpisodeRecord>> {
        self.repo.list_reflexion_episodes(session_id).await
    }

    pub async fn upsert_skill_library_record(
        &self,
        record: SkillLibraryRecord,
    ) -> Result<SkillLibraryRecord> {
        self.repo.upsert_skill_library_record(record).await
    }

    pub async fn list_skill_library_records(&self, session_id: &str) -> Result<Vec<SkillLibraryRecord>> {
        self.repo.list_skill_library_records(session_id).await
    }

    pub async fn upsert_causal_edge_record(
        &self,
        record: CausalEdgeRecord,
    ) -> Result<CausalEdgeRecord> {
        self.repo.upsert_causal_edge_record(record).await
    }

    pub async fn list_causal_edge_records(&self, session_id: &str) -> Result<Vec<CausalEdgeRecord>> {
        self.repo.list_causal_edge_records(session_id).await
    }

    pub async fn upsert_learning_session_record(
        &self,
        record: LearningSessionRecord,
    ) -> Result<LearningSessionRecord> {
        self.repo.upsert_learning_session_record(record).await
    }

    pub async fn list_learning_session_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<LearningSessionRecord>> {
        self.repo.list_learning_session_records(session_id).await
    }

    pub async fn append_witness_log_record(
        &self,
        record: WitnessLogRecord,
    ) -> Result<WitnessLogRecord> {
        self.repo.append_witness_log_record(record).await
    }

    pub async fn list_witness_log_records(&self, session_id: &str) -> Result<Vec<WitnessLogRecord>> {
        self.repo.list_witness_log_records(session_id).await
    }
}

fn schedule_event_from_postgres(event: autoloop_postgres_adapter::ScheduleEvent) -> ScheduleEvent {
    ScheduleEvent {
        id: event.id,
        session_id: event.session_id,
        topic: event.topic,
        tool_name: event.tool_name,
        payload: event.payload,
        actor_id: event.actor_id,
        status: event.status,
    }
}

fn agent_state_from_postgres(state: autoloop_postgres_adapter::AgentState) -> AgentState {
    AgentState {
        id: state.id,
        session_id: state.session_id,
        last_user_message: state.last_user_message,
        last_assistant_message: state.last_assistant_message,
    }
}

fn permission_grant_from_postgres(
    value: autoloop_postgres_adapter::PermissionGrant,
) -> Result<PermissionGrant> {
    convert_struct(value)
}

fn permission_action_to_postgres(
    value: PermissionAction,
) -> Result<autoloop_postgres_adapter::PermissionAction> {
    convert_struct(value)
}

fn convert_struct<T, U>(value: T) -> Result<U>
where
    T: Serialize,
    U: DeserializeOwned,
{
    let json = serde_json::to_value(value)?;
    Ok(serde_json::from_value(json)?)
}

fn tenant_to_postgres(value: &Tenant) -> Result<autoloop_postgres_adapter::Tenant> {
    convert_struct(value.clone())
}

fn tenant_from_postgres(value: autoloop_postgres_adapter::Tenant) -> Result<Tenant> {
    convert_struct(value)
}

fn principal_to_postgres(value: &Principal) -> Result<autoloop_postgres_adapter::Principal> {
    convert_struct(value.clone())
}

fn principal_from_postgres(value: autoloop_postgres_adapter::Principal) -> Result<Principal> {
    convert_struct(value)
}

fn role_binding_to_postgres(
    value: &RoleBinding,
) -> Result<autoloop_postgres_adapter::RoleBinding> {
    convert_struct(value.clone())
}

fn role_binding_from_postgres(
    value: autoloop_postgres_adapter::RoleBinding,
) -> Result<RoleBinding> {
    convert_struct(value)
}

fn policy_binding_to_postgres(
    value: &PolicyBinding,
) -> Result<autoloop_postgres_adapter::PolicyBinding> {
    convert_struct(value.clone())
}

fn policy_binding_from_postgres(
    value: autoloop_postgres_adapter::PolicyBinding,
) -> Result<PolicyBinding> {
    convert_struct(value)
}

fn session_lease_to_postgres(
    value: &SessionLease,
) -> Result<autoloop_postgres_adapter::SessionLease> {
    convert_struct(value.clone())
}

fn session_lease_from_postgres(
    value: autoloop_postgres_adapter::SessionLease,
) -> Result<SessionLease> {
    convert_struct(value)
}

fn budget_account_to_postgres(
    value: &BudgetAccount,
) -> Result<autoloop_postgres_adapter::BudgetAccount> {
    convert_struct(value.clone())
}

fn budget_account_from_postgres(
    value: autoloop_postgres_adapter::BudgetAccount,
) -> Result<BudgetAccount> {
    convert_struct(value)
}

fn spend_ledger_to_postgres(
    value: &SpendLedger,
) -> Result<autoloop_postgres_adapter::SpendLedger> {
    convert_struct(value.clone())
}

fn spend_ledger_from_postgres(
    value: autoloop_postgres_adapter::SpendLedger,
) -> Result<SpendLedger> {
    convert_struct(value)
}

fn quota_window_to_postgres(value: &QuotaWindow) -> Result<autoloop_postgres_adapter::QuotaWindow> {
    convert_struct(value.clone())
}

fn quota_window_from_postgres(
    value: autoloop_postgres_adapter::QuotaWindow,
) -> Result<QuotaWindow> {
    convert_struct(value)
}

fn cost_attribution_to_postgres(
    value: &CostAttribution,
) -> Result<autoloop_postgres_adapter::CostAttribution> {
    convert_struct(value.clone())
}

fn cost_attribution_from_postgres(
    value: autoloop_postgres_adapter::CostAttribution,
) -> Result<CostAttribution> {
    convert_struct(value)
}

fn shadow_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn classify_shadow_diff(
    missing_in_mirror: &[String],
    extra_in_mirror: &[String],
    value_mismatches: &[String],
    reason: Option<&str>,
) -> Vec<String> {
    let mut classes = Vec::new();
    if !missing_in_mirror.is_empty() || !extra_in_mirror.is_empty() {
        classes.push("missing_diff".to_string());
    }
    if !value_mismatches.is_empty() {
        classes.push("value_diff".to_string());
    }
    if classes.is_empty() || reason.map(|r| r.contains("failed")).unwrap_or(false) {
        classes.push("schema_diff".to_string());
    }
    classes
}

fn make_shadow_evidence_ref(domain: &str, probe: &str, target: &str) -> String {
    format!(
        "shadow_diff:{domain}:{probe}:{target}:{}",
        shadow_now_ms()
    )
}

fn stable_percent_bucket(value: &str) -> u8 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    (hasher.finish() % 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_postgres_adapter::{PostgresDb, PostgresDbConfig};
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn shadow_rollout_bucket_is_stable_and_bounded() {
        let first = stable_percent_bucket("memory:session-a:consolidation");
        let second = stable_percent_bucket("memory:session-a:consolidation");
        assert_eq!(first, second);
        assert!(first < 100);
    }

    #[tokio::test]
    async fn state_store_crud_is_type_safe_and_thread_safe() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        db.grant_permissions("agent-1", vec![PermissionAction::Read, PermissionAction::Write])
            .await
            .expect("grant");
        db.enforce_permission("agent-1", PermissionAction::Read)
            .await
            .expect("read permission");

        let knowledge = db
            .upsert_knowledge(
                "anchor:rust".into(),
                "Rust is the systems substrate for AutoLoop.".into(),
                "test".into(),
            )
            .await
            .expect("knowledge upsert");

        assert_eq!(knowledge.key, "anchor:rust");
        assert!(db.get_knowledge("anchor:rust").await.expect("knowledge read").is_some());
    }

    #[tokio::test]
    async fn identity_schema_crud_works_for_tenant_principal_policy_and_lease() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let tenant = db
            .upsert_tenant(Tenant {
                tenant_id: "tenant-a".into(),
                name: "Tenant A".into(),
                status: "active".into(),
                created_at_ms: 1,
            })
            .await
            .expect("tenant");
        db.upsert_principal(Principal {
            principal_id: "principal-a".into(),
            tenant_id: tenant.tenant_id.clone(),
            principal_type: "user".into(),
            status: "active".into(),
            created_at_ms: 2,
        })
        .await
        .expect("principal");
        db.upsert_role_binding(RoleBinding {
            tenant_id: tenant.tenant_id.clone(),
            principal_id: "principal-a".into(),
            role: "operator".into(),
            updated_at_ms: 3,
        })
        .await
        .expect("role");
        db.upsert_policy_binding(PolicyBinding {
            policy_id: "policy-a".into(),
            tenant_id: tenant.tenant_id.clone(),
            role: "operator".into(),
            allowed_actions: vec![PermissionAction::Read, PermissionAction::Dispatch],
            capability_prefixes: vec!["provider:".into(), "mcp::local-mcp::".into()],
            max_memory_mb: 1024,
            max_tokens: 8000,
            updated_at_ms: 4,
        })
        .await
        .expect("policy");
        db.upsert_session_lease(SessionLease {
            lease_token: "lease-a".into(),
            session_id: "session-a".into(),
            tenant_id: tenant.tenant_id.clone(),
            principal_id: "principal-a".into(),
            policy_id: "policy-a".into(),
            expires_at_ms: 9_999,
            issued_at_ms: 5,
        })
        .await
        .expect("lease");

        assert!(db
            .get_tenant("tenant-a")
            .await
            .expect("tenant get")
            .is_some());
        assert!(db
            .get_principal("tenant-a", "principal-a")
            .await
            .expect("principal get")
            .is_some());
        assert!(db
            .get_policy_binding("tenant-a", "policy-a")
            .await
            .expect("policy get")
            .is_some());
        assert!(db
            .get_session_lease("session-a")
            .await
            .expect("lease get")
            .is_some());
    }

    #[tokio::test]
    async fn scheduler_and_agent_state_crud_work() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let event = db
            .create_schedule_event(
                "session-a".into(),
                "wake.plan.execute".into(),
                "planner".into(),
                "{\"intent\":\"test\"}".into(),
                "agent-a".into(),
            )
            .await
            .expect("schedule create");
        db.update_schedule_status(event.id, "done")
            .await
            .expect("schedule status");
        let listed = db
            .list_schedule_events("session-a")
            .await
            .expect("schedule list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, "done");

        db.upsert_agent_state(
            "session-a".into(),
            "hello".into(),
            Some("world".into()),
        )
        .await
        .expect("state upsert");
        let state = db
            .get_agent_state("session-a")
            .await
            .expect("state get")
            .expect("state exists");
        assert_eq!(state.last_user_message, "hello");
        assert_eq!(state.last_assistant_message.as_deref(), Some("world"));
    }

    #[tokio::test]
    async fn billing_schema_append_only_and_replay_work() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.upsert_budget_account(BudgetAccount {
            account_id: "account-a".into(),
            tenant_id: "tenant-a".into(),
            principal_id: "principal-a".into(),
            policy_id: "policy-a".into(),
            total_budget_micros: 100_000,
            reserved_micros: 20_000,
            spent_micros: 10_000,
            blocked_count: 0,
            updated_at_ms: 1,
        })
        .await
        .expect("budget account");
        db.append_spend_ledger(SpendLedger {
            ledger_id: "l1".into(),
            tenant_id: "tenant-a".into(),
            account_id: "account-a".into(),
            session_id: "session-a".into(),
            trace_id: "trace-a".into(),
            task_id: "task-a".into(),
            capability_id: "provider:default".into(),
            kind: SpendLedgerKind::Reserve,
            amount_micros: 5_000,
            token_cost_micros: 3_000,
            tool_cost_micros: 1_000,
            duration_cost_micros: 1_000,
            reason: "precharge".into(),
            created_at_ms: 2,
        })
        .await
        .expect("ledger reserve");
        db.append_spend_ledger(SpendLedger {
            ledger_id: "l2".into(),
            tenant_id: "tenant-a".into(),
            account_id: "account-a".into(),
            session_id: "session-a".into(),
            trace_id: "trace-a".into(),
            task_id: "task-a".into(),
            capability_id: "provider:default".into(),
            kind: SpendLedgerKind::Settle,
            amount_micros: 4_500,
            token_cost_micros: 3_000,
            tool_cost_micros: 500,
            duration_cost_micros: 1_000,
            reason: "settled".into(),
            created_at_ms: 3,
        })
        .await
        .expect("ledger settle");
        db.upsert_quota_window(QuotaWindow {
            window_id: "w1".into(),
            tenant_id: "tenant-a".into(),
            account_id: "account-a".into(),
            window_start_ms: 0,
            window_end_ms: 10_000,
            window_budget_micros: 50_000,
            consumed_micros: 4_500,
            blocked_count: 0,
            updated_at_ms: 3,
        })
        .await
        .expect("quota");
        db.upsert_cost_attribution(CostAttribution {
            attribution_id: "a1".into(),
            tenant_id: "tenant-a".into(),
            principal_id: "principal-a".into(),
            policy_id: "policy-a".into(),
            session_id: "session-a".into(),
            trace_id: "trace-a".into(),
            task_id: "task-a".into(),
            capability_id: "provider:default".into(),
            provider_tokens: 300,
            tool_invocations: 0,
            duration_ms: 120,
            token_cost_micros: 3_000,
            tool_cost_micros: 500,
            duration_cost_micros: 1_000,
            total_cost_micros: 4_500,
            settled_at_ms: 3,
        })
        .await
        .expect("attribution");

        let replay = db
            .list_spend_ledger("tenant-a", "account-a")
            .await
            .expect("ledger replay");
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].ledger_id, "l1");
        assert_eq!(replay[1].ledger_id, "l2");
        assert_eq!(replay[1].kind, SpendLedgerKind::Settle);
        assert!(db
            .append_spend_ledger(SpendLedger {
                ledger_id: "l2".into(),
                tenant_id: "tenant-a".into(),
                account_id: "account-a".into(),
                session_id: "session-a".into(),
                trace_id: "trace-a".into(),
                task_id: "task-a".into(),
                capability_id: "provider:default".into(),
                kind: SpendLedgerKind::Settle,
                amount_micros: 1,
                token_cost_micros: 0,
                tool_cost_micros: 0,
                duration_cost_micros: 0,
                reason: "duplicate".into(),
                created_at_ms: 4,
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn enforced_mode_blocks_fallback_writes_by_default() {
        let _guard = env_lock().lock().expect("env lock");
        unsafe {
            std::env::remove_var("AUTOLOOP_STORAGE_ROLLBACK_UNTIL_MS");
            std::env::remove_var("AUTOLOOP_STORAGE_ROLLBACK_TICKET");
            std::env::remove_var("AUTOLOOP_STORAGE_ROLLBACK_REASON");
        }
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.configure_knowledge_mirror(
            PostgresDb::new(PostgresDbConfig {
                enabled: true,
                uri: "postgres://postgres:123456@127.0.0.1:1/invalid".into(),
                schema: "public".into(),
                pool_size: 1,
                auto_migrate: false,
            }),
            KnowledgeMirrorMode::Enforced,
            KnowledgeReadPreference::Postgres,
            0,
        )
        .expect("configure mirror");

        let result = db
            .upsert_tenant(Tenant {
                tenant_id: "tenant-r4-block".into(),
                name: "tenant".into(),
                status: "active".into(),
                created_at_ms: 1,
            })
            .await;
        assert!(result.is_err(), "fallback write must be blocked in enforced mode");
    }

    #[tokio::test]
    async fn controlled_rollback_window_allows_short_lived_fallback_and_audits() {
        let _guard = env_lock().lock().expect("env lock");
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.configure_knowledge_mirror(
            PostgresDb::new(PostgresDbConfig {
                enabled: true,
                uri: "postgres://postgres:123456@127.0.0.1:1/invalid".into(),
                schema: "public".into(),
                pool_size: 1,
                auto_migrate: false,
            }),
            KnowledgeMirrorMode::Enforced,
            KnowledgeReadPreference::Postgres,
            0,
        )
        .expect("configure mirror");

        let until = shadow_now_ms() + 60_000;
        unsafe {
            std::env::set_var("AUTOLOOP_STORAGE_ROLLBACK_UNTIL_MS", until.to_string());
            std::env::set_var("AUTOLOOP_STORAGE_ROLLBACK_TICKET", "rollback-drill-r4");
            std::env::set_var("AUTOLOOP_STORAGE_ROLLBACK_REASON", "drill");
        }
        let tenant_id = "tenant-r4-rollback".to_string();
        let write = db
            .upsert_tenant(Tenant {
                tenant_id: tenant_id.clone(),
                name: "tenant".into(),
                status: "active".into(),
                created_at_ms: 1,
            })
            .await;
        unsafe {
            std::env::remove_var("AUTOLOOP_STORAGE_ROLLBACK_UNTIL_MS");
            std::env::remove_var("AUTOLOOP_STORAGE_ROLLBACK_TICKET");
            std::env::remove_var("AUTOLOOP_STORAGE_ROLLBACK_REASON");
        }
        assert!(write.is_ok(), "controlled rollback should allow temporary fallback write");

        let fallback_value = db
            .repo
            .get_knowledge(&format!("identity:tenant:{tenant_id}"))
            .await
            .expect("read fallback")
            .and_then(|record| serde_json::from_str::<Tenant>(&record.value).ok());
        assert!(fallback_value.is_some(), "fallback state write should exist");

        let audit_logs = db
            .repo
            .list_knowledge_by_prefix("storage:rollback:event:identity:upsert_tenant:")
            .await
            .expect("rollback audit logs");
        assert!(
            !audit_logs.is_empty(),
            "controlled rollback must emit auditable rollback event"
        );
    }
}

