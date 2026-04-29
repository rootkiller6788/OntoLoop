use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use autoloop_state_adapter::{
    CausalEdgeRecord, KnowledgeRecord, LearningSessionRecord, ReflexionEpisodeRecord,
    SkillLibraryRecord, WitnessLogRecord,
};
use serde::{Deserialize, Serialize};

use crate::tools::ForgedMcpToolManifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LearningEventKind {
    Failure,
    Success,
    ToolCall,
    RouteDecision,
    Audit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningEvent {
    pub event_kind: LearningEventKind,
    pub session_id: String,
    pub source: String,
    pub summary: String,
    pub score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LearningAssetKind {
    ReflexionEpisode,
    Skill,
    CausalEdge,
    LearningSession,
    WitnessLog,
    ForgedToolManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningDocument {
    pub id: String,
    pub session_id: String,
    pub asset_kind: LearningAssetKind,
    pub text: String,
    pub score: f32,
    pub created_at_ms: u64,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LearningFilter {
    pub session_id: Option<String>,
    pub asset_kinds: Vec<LearningAssetKind>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalEvidence {
    pub document: LearningDocument,
    pub similarity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JointRoutingEvidence {
    pub retrieved: Vec<RetrievalEvidence>,
    pub skill_success_rate: f32,
    pub causal_confidence: f32,
    pub tool_success_rate: f32,
}

#[async_trait]
pub trait LearningRepository: Send + Sync {
    async fn upsert_reflexion_episode(
        &self,
        record: ReflexionEpisodeRecord,
    ) -> Result<ReflexionEpisodeRecord>;
    async fn list_reflexion_episodes(
        &self,
        session_id: &str,
    ) -> Result<Vec<ReflexionEpisodeRecord>>;
    async fn upsert_skill_library_record(
        &self,
        record: SkillLibraryRecord,
    ) -> Result<SkillLibraryRecord>;
    async fn list_skill_library_records(&self, session_id: &str)
    -> Result<Vec<SkillLibraryRecord>>;
    async fn upsert_causal_edge_record(&self, record: CausalEdgeRecord)
    -> Result<CausalEdgeRecord>;
    async fn list_causal_edge_records(&self, session_id: &str) -> Result<Vec<CausalEdgeRecord>>;
    async fn upsert_learning_session_record(
        &self,
        record: LearningSessionRecord,
    ) -> Result<LearningSessionRecord>;
    async fn list_learning_session_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<LearningSessionRecord>>;
    async fn append_witness_log_record(&self, record: WitnessLogRecord)
    -> Result<WitnessLogRecord>;
    async fn list_witness_log_records(&self, session_id: &str) -> Result<Vec<WitnessLogRecord>>;
}

pub trait LearningScorer: Send + Sync {
    fn score_route(&self, evidence: &JointRoutingEvidence) -> f32;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
}

#[derive(Debug, Clone)]
pub struct HashEmbeddingProvider {
    dimensions: usize,
}

impl HashEmbeddingProvider {
    pub fn new(dimensions: usize) -> Self {
        Self { dimensions }
    }
}

#[async_trait]
impl EmbeddingProvider for HashEmbeddingProvider {
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| embed_hash(text, self.dimensions))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[derive(Debug, Clone, Default)]
pub struct WeightedLearningScorer;

impl LearningScorer for WeightedLearningScorer {
    fn score_route(&self, evidence: &JointRoutingEvidence) -> f32 {
        let retrieval_score = if evidence.retrieved.is_empty() {
            0.0
        } else {
            evidence
                .retrieved
                .iter()
                .map(|item| item.similarity.max(0.0))
                .sum::<f32>()
                / evidence.retrieved.len() as f32
        };

        retrieval_score * 0.35
            + evidence.skill_success_rate * 0.25
            + evidence.causal_confidence * 0.20
            + evidence.tool_success_rate * 0.20
    }
}

pub fn embed_hash(text: &str, dimensions: usize) -> Vec<f32> {
    let mut vector = vec![0.0f32; dimensions.max(8)];

    for token in text.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if token.is_empty() {
            continue;
        }

        let normalized = token.to_ascii_lowercase();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        normalized.hash(&mut hasher);
        let index = (hasher.finish() as usize) % vector.len();
        vector[index] += 1.0;
    }

    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in &mut vector {
            *value /= norm;
        }
    }

    vector
}

pub fn document_from_episode(record: &ReflexionEpisodeRecord) -> LearningDocument {
    let mut metadata = HashMap::new();
    metadata.insert("hypothesis".into(), record.hypothesis.clone());
    metadata.insert("status".into(), record.status.clone());
    LearningDocument {
        id: record.id.clone(),
        session_id: record.session_id.clone(),
        asset_kind: LearningAssetKind::ReflexionEpisode,
        text: format!("{} {} {}", record.objective, record.outcome, record.lesson),
        score: record.score,
        created_at_ms: record.created_at_ms,
        metadata,
    }
}

pub fn document_from_skill(record: &SkillLibraryRecord) -> LearningDocument {
    let mut metadata = HashMap::new();
    metadata.insert("trigger".into(), record.trigger.clone());
    metadata.insert("success_rate".into(), format!("{:.3}", record.success_rate));
    LearningDocument {
        id: record.id.clone(),
        session_id: record.session_id.clone(),
        asset_kind: LearningAssetKind::Skill,
        text: format!("{} {}", record.name, record.procedure),
        score: record.confidence,
        created_at_ms: record.updated_at_ms,
        metadata,
    }
}

pub fn document_from_causal_edge(record: &CausalEdgeRecord) -> LearningDocument {
    let mut metadata = HashMap::new();
    metadata.insert("cause".into(), record.cause.clone());
    metadata.insert("effect".into(), record.effect.clone());
    LearningDocument {
        id: record.id.clone(),
        session_id: record.session_id.clone(),
        asset_kind: LearningAssetKind::CausalEdge,
        text: format!("{} {} {}", record.cause, record.effect, record.evidence),
        score: record.confidence,
        created_at_ms: record.created_at_ms,
        metadata,
    }
}

pub fn document_from_learning_session(record: &LearningSessionRecord) -> LearningDocument {
    let mut metadata = HashMap::new();
    metadata.insert("status".into(), record.status.clone());
    LearningDocument {
        id: record.id.clone(),
        session_id: record.session_id.clone(),
        asset_kind: LearningAssetKind::LearningSession,
        text: format!("{} {}", record.objective, record.summary),
        score: record.priority,
        created_at_ms: record.completed_at_ms.unwrap_or(record.started_at_ms),
        metadata,
    }
}

pub fn document_from_witness(record: &WitnessLogRecord) -> LearningDocument {
    let mut metadata = HashMap::new();
    metadata.insert("event_type".into(), format!("{:?}", record.event_type));
    metadata.insert("source".into(), record.source.clone());
    LearningDocument {
        id: record.id.clone(),
        session_id: record.session_id.clone(),
        asset_kind: LearningAssetKind::WitnessLog,
        text: format!("{} {}", record.detail, record.metadata_json),
        score: record.score,
        created_at_ms: record.created_at_ms,
        metadata,
    }
}

pub fn document_from_forged_tool_manifest(
    record: &KnowledgeRecord,
    manifest: &ForgedMcpToolManifest,
) -> LearningDocument {
    let mut metadata = HashMap::new();
    metadata.insert("server".into(), manifest.server.clone());
    metadata.insert("tool_name".into(), manifest.registered_tool_name.clone());
    metadata.insert("capability_name".into(), manifest.capability_name.clone());
    metadata.insert("delegate_tool".into(), manifest.delegate_tool_name.clone());
    LearningDocument {
        id: record.key.clone(),
        session_id: "global".into(),
        asset_kind: LearningAssetKind::ForgedToolManifest,
        text: format!(
            "{} {} {} {} {}",
            manifest.capability_name,
            manifest.purpose,
            manifest.command_template,
            manifest.help_text,
            manifest.examples.join(" ")
        ),
        score: 0.95,
        created_at_ms: 0,
        metadata,
    }
}

pub type SharedEmbeddingProvider = Arc<dyn EmbeddingProvider>;

