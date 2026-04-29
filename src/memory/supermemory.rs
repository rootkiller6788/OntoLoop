use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, bail};
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use crate::contracts::context::{MemoryScopeContract, MemoryScopeSpec};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingStatus {
    Queued,
    Extracting,
    Chunking,
    Embedding,
    Indexing,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionJob {
    pub job_id: String,
    pub session_id: String,
    pub tenant_id: String,
    pub source: String,
    pub document_id: String,
    pub metadata: BTreeMap<String, String>,
    pub document_date: Option<String>,
    pub event_date: Option<String>,
    pub queued_at_ms: u64,
    pub status: ProcessingStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedContent {
    pub job_id: String,
    pub document_id: String,
    pub content_type: String,
    pub raw_text: String,
    pub normalized_text: String,
    pub token_estimate: u32,
    pub metadata: BTreeMap<String, String>,
    pub extracted_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticChunk {
    pub chunk_id: String,
    pub job_id: String,
    pub document_id: String,
    pub sequence: usize,
    pub text: String,
    pub token_estimate: u32,
    pub start_offset: usize,
    pub end_offset: usize,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedChunk {
    pub chunk_id: String,
    pub model: String,
    pub dimensions: usize,
    pub vector: Vec<f32>,
    pub embedded_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicMemory {
    pub memory_id: String,
    pub session_id: String,
    pub job_id: String,
    pub document_id: String,
    pub chunk_id: String,
    pub statement: String,
    pub confidence: f32,
    pub is_static: bool,
    pub source_ref: String,
    pub tags: Vec<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRelationType {
    Updates,
    Extends,
    Derives,
    IsLatest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRelation {
    pub relation_id: String,
    pub session_id: String,
    pub job_id: String,
    pub from_memory_id: String,
    pub to_memory_id: String,
    pub relation_type: MemoryRelationType,
    pub weight: f32,
    pub is_latest: bool,
    pub valid: bool,
    pub rationale: String,
    pub invalidated_at_ms: Option<u64>,
    pub invalidation_reason: Option<String>,
    pub created_at_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalGrounding {
    pub session_id: String,
    pub job_id: String,
    pub document_id: String,
    pub document_date: Option<String>,
    pub event_date: Option<String>,
    pub ingested_at_ms: u64,
    pub last_updated_at_ms: u64,
    pub freshness_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfileSnapshot {
    pub session_id: String,
    pub static_facts: Vec<String>,
    pub dynamic_context: Vec<String>,
    pub memory_count: usize,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchHit {
    pub memory_id: String,
    pub chunk_id: String,
    pub score: f32,
    pub memory_text: String,
    pub chunk_text: String,
    pub source_ref: String,
    pub reason: String,
    pub tags: Vec<String>,
    pub document_date: Option<String>,
    pub event_date: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextAssembly {
    pub session_id: String,
    pub query: String,
    pub profile: UserProfileSnapshot,
    pub hits: Vec<HybridSearchHit>,
    pub source_evidence_refs: Vec<String>,
    pub document_refs: Vec<String>,
    pub assembled_at_ms: u64,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryWriteAction {
    #[serde(rename = "ADD")]
    Add,
    #[serde(rename = "UPDATE")]
    Update,
    #[serde(rename = "DELETE")]
    Delete,
    #[serde(rename = "NONE")]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryWriteDecision {
    pub decision_id: String,
    pub session_id: String,
    pub job_id: String,
    pub candidate_text: String,
    pub action: MemoryWriteAction,
    pub target_memory_id: Option<String>,
    pub resulting_memory_id: Option<String>,
    pub reason: String,
    pub old_memory: Option<String>,
    pub new_memory: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MemoryScopeIndexRecord {
    memory_id: String,
    session_id: String,
    user_id: Option<String>,
    agent_id: Option<String>,
    run_id: Option<String>,
    actor_id: Option<String>,
    updated_at_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHistoryRecord {
    pub history_id: String,
    pub session_id: String,
    pub job_id: String,
    pub trace_id: String,
    pub memory_id: String,
    pub old_memory: Option<String>,
    pub new_memory: Option<String>,
    pub event: String,
    pub actor_id: Option<String>,
    pub role: Option<String>,
    pub is_deleted: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

struct HybridSearchPhaseMemory {
    memory: AtomicMemory,
    memory_score: f32,
}

#[derive(Debug, Clone)]
pub struct SupermemoryKernel {
    pub chunk_char_target: usize,
    pub chunk_overlap_chars: usize,
    pub embedding_dimensions: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OcrBackend {
    None,
    Http,
    Command,
}

#[derive(Debug, Clone)]
struct OcrConfig {
    backend: OcrBackend,
    http_url: Option<String>,
    http_token: Option<String>,
    command: Option<String>,
    timeout_ms: u64,
}

impl OcrConfig {
    fn from_env() -> Self {
        let backend = match std::env::var("AUTOLOOP_OCR_BACKEND")
            .unwrap_or_else(|_| "none".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "http" => OcrBackend::Http,
            "command" | "cmd" => OcrBackend::Command,
            _ => OcrBackend::None,
        };
        let timeout_ms = std::env::var("AUTOLOOP_OCR_TIMEOUT_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(8_000);
        Self {
            backend,
            http_url: std::env::var("AUTOLOOP_OCR_HTTP_URL").ok(),
            http_token: std::env::var("AUTOLOOP_OCR_HTTP_TOKEN").ok(),
            command: std::env::var("AUTOLOOP_OCR_COMMAND").ok(),
            timeout_ms,
        }
    }
}

impl Default for SupermemoryKernel {
    fn default() -> Self {
        Self {
            chunk_char_target: 560,
            chunk_overlap_chars: 72,
            embedding_dimensions: 64,
        }
    }
}

impl SupermemoryKernel {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn run_pipeline(
        &self,
        db: &StateStore,
        session_id: &str,
        tenant_id: &str,
        source: &str,
        content: &str,
        metadata: BTreeMap<String, String>,
        document_date: Option<String>,
        event_date: Option<String>,
        query: &str,
        top_k: usize,
    ) -> Result<ContextAssembly> {
        let job = self
            .queue_ingestion(
                db,
                session_id,
                tenant_id,
                source,
                metadata,
                document_date,
                event_date,
            )
            .await?;
        self.persist_ingestion_payload(db, &job, content).await?;
        self.process_ingestion_job(db, job, content, query, top_k)
            .await
    }

    async fn process_ingestion_job(
        &self,
        db: &StateStore,
        mut job: IngestionJob,
        content: &str,
        query: &str,
        top_k: usize,
    ) -> Result<ContextAssembly> {
        let scope_contract =
            self.build_scope_contract(&job.session_id, &job.metadata, &BTreeMap::new())?;

        job.status = ProcessingStatus::Extracting;
        self.persist_job(db, &job).await?;

        let extracted = self.extract_content_with_ocr(&job, content).await?;

        job.status = ProcessingStatus::Chunking;
        self.persist_job(db, &job).await?;
        let chunks = self.chunk_semantic(&extracted);

        job.status = ProcessingStatus::Embedding;
        self.persist_job(db, &job).await?;
        let embeddings = self.embed_chunks(&chunks);

        let candidate_memories = self.generate_atomic_memories(&job.session_id, &job, &chunks);
        let existing_memories = self
            .read_scoped_atomic_memories(db, &job.session_id, &scope_contract)
            .await?;
        let (atomic_memories, context_memories, write_decisions) = self.reconcile_memory_writes(
            &job,
            &scope_contract,
            &existing_memories,
            &candidate_memories,
        );

        job.status = ProcessingStatus::Indexing;
        self.persist_job(db, &job).await?;
        let relation_source = if context_memories.len() >= 2 {
            context_memories.as_slice()
        } else {
            atomic_memories.as_slice()
        };
        let relations = self.build_relationships(&job.session_id, &job.job_id, relation_source);
        let temporal = self.temporal_grounding(&job);

        let profile = self.build_user_profile(&job.session_id, &context_memories);
        let hits =
            self.hybrid_search_from_records(query, top_k, &context_memories, &chunks, &temporal);
        let context = self.assemble_context(&job.session_id, query, profile, hits);

        self.persist_pipeline(
            db,
            &job,
            &extracted,
            &chunks,
            &embeddings,
            &atomic_memories,
            &relations,
            &temporal,
            &context.profile,
            &context,
            &write_decisions,
            &scope_contract,
        )
        .await?;

        job.status = ProcessingStatus::Done;
        self.persist_job(db, &job).await?;
        Ok(context)
    }

    pub async fn queue_ingestion(
        &self,
        db: &StateStore,
        session_id: &str,
        tenant_id: &str,
        source: &str,
        metadata: BTreeMap<String, String>,
        document_date: Option<String>,
        event_date: Option<String>,
    ) -> Result<IngestionJob> {
        let scope_contract = self.build_scope_contract(session_id, &metadata, &BTreeMap::new())?;
        let mut scoped_metadata = scope_contract.metadata_template;
        if let Some(actor_id) = scope_contract.scope.actor_id {
            scoped_metadata.insert("scope_actor_filter".to_string(), actor_id);
        }

        let now = current_time_ms();
        let job_id = format!("job:{session_id}:{now}");
        let document_id = format!("doc:{session_id}:{now}");
        let job = IngestionJob {
            job_id,
            session_id: session_id.to_string(),
            tenant_id: tenant_id.to_string(),
            source: source.to_string(),
            document_id,
            metadata: scoped_metadata,
            document_date,
            event_date,
            queued_at_ms: now,
            status: ProcessingStatus::Queued,
        };
        self.persist_job(db, &job).await?;
        Ok(job)
    }
    pub fn extract_content(&self, job: &IngestionJob, raw_content: &str) -> ExtractedContent {
        let content_type = detect_content_type(raw_content, &job.metadata);
        let extracted_text = extract_by_content_type(&content_type, raw_content, &job.metadata);
        let normalized = normalize_whitespace(&extracted_text);

        let mut metadata = job.metadata.clone();
        metadata.insert("source".into(), job.source.clone());
        metadata.insert("tenant_id".into(), job.tenant_id.clone());

        ExtractedContent {
            job_id: job.job_id.clone(),
            document_id: job.document_id.clone(),
            content_type,
            raw_text: extracted_text,
            normalized_text: normalized.clone(),
            token_estimate: estimate_tokens(&normalized),
            metadata,
            extracted_at_ms: current_time_ms(),
        }
    }

    async fn extract_content_with_ocr(
        &self,
        job: &IngestionJob,
        raw_content: &str,
    ) -> Result<ExtractedContent> {
        let content_type = detect_content_type(raw_content, &job.metadata);
        let extracted_text = self
            .extract_by_content_type_async(&content_type, raw_content, &job.metadata)
            .await?;
        let normalized = normalize_whitespace(&extracted_text);

        let mut metadata = job.metadata.clone();
        metadata.insert("source".into(), job.source.clone());
        metadata.insert("tenant_id".into(), job.tenant_id.clone());

        Ok(ExtractedContent {
            job_id: job.job_id.clone(),
            document_id: job.document_id.clone(),
            content_type,
            raw_text: extracted_text,
            normalized_text: normalized.clone(),
            token_estimate: estimate_tokens(&normalized),
            metadata,
            extracted_at_ms: current_time_ms(),
        })
    }

    async fn extract_by_content_type_async(
        &self,
        content_type: &str,
        raw: &str,
        metadata: &BTreeMap<String, String>,
    ) -> Result<String> {
        match content_type {
            "ocr" | "image" => {
                if let Some(text) = metadata.get("ocr_text") {
                    return Ok(text.clone());
                }
                let config = OcrConfig::from_env();
                if let Some(text) = try_external_ocr(raw, metadata, &config).await? {
                    Ok(text)
                } else {
                    Ok(format!("OCR_PENDING {}", raw))
                }
            }
            _ => Ok(extract_by_content_type(content_type, raw, metadata)),
        }
    }
    pub fn chunk_semantic(&self, extracted: &ExtractedContent) -> Vec<SemanticChunk> {
        let mut chunks = Vec::new();
        let mut sequence = 0usize;
        let mut cursor = 0usize;

        for block in split_semantic_blocks(&extracted.normalized_text, self.chunk_char_target) {
            let start = cursor;
            let end = start.saturating_add(block.len());
            cursor = end.saturating_sub(self.chunk_overlap_chars.min(block.len()));

            let mut metadata = extracted.metadata.clone();
            metadata.insert("chunk_sequence".into(), sequence.to_string());

            chunks.push(SemanticChunk {
                chunk_id: format!("chunk:{}:{}", extracted.job_id, sequence),
                job_id: extracted.job_id.clone(),
                document_id: extracted.document_id.clone(),
                sequence,
                text: block.clone(),
                token_estimate: estimate_tokens(&block),
                start_offset: start,
                end_offset: end,
                metadata,
            });
            sequence = sequence.saturating_add(1);
        }

        if chunks.is_empty() {
            let text = extracted.normalized_text.clone();
            chunks.push(SemanticChunk {
                chunk_id: format!("chunk:{}:0", extracted.job_id),
                job_id: extracted.job_id.clone(),
                document_id: extracted.document_id.clone(),
                sequence: 0,
                text: text.clone(),
                token_estimate: estimate_tokens(&text),
                start_offset: 0,
                end_offset: text.len(),
                metadata: extracted.metadata.clone(),
            });
        }

        chunks
    }

    pub fn embed_chunks(&self, chunks: &[SemanticChunk]) -> Vec<EmbeddedChunk> {
        chunks
            .iter()
            .map(|chunk| EmbeddedChunk {
                chunk_id: chunk.chunk_id.clone(),
                model: "supermemory-hash-embedding-v1".into(),
                dimensions: self.embedding_dimensions,
                vector: self.hash_embedding(&chunk.text),
                embedded_at_ms: current_time_ms(),
            })
            .collect()
    }

    pub fn generate_atomic_memories(
        &self,
        session_id: &str,
        job: &IngestionJob,
        chunks: &[SemanticChunk],
    ) -> Vec<AtomicMemory> {
        chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                let statement = summarize_chunk_statement(&chunk.text);
                let tags = derive_tags(&chunk.text, &chunk.metadata);
                let is_static = tags.iter().any(|tag| {
                    matches!(
                        tag.as_str(),
                        "identity" | "profile" | "preference" | "stable_fact"
                    )
                }) || statement.to_ascii_lowercase().contains("prefers");

                AtomicMemory {
                    memory_id: format!("memory:{}:{}", job.job_id, index),
                    session_id: session_id.to_string(),
                    job_id: job.job_id.clone(),
                    document_id: job.document_id.clone(),
                    chunk_id: chunk.chunk_id.clone(),
                    statement,
                    confidence: 0.7,
                    is_static,
                    source_ref: format!(
                        "memory:supermemory:documents:{}:{}",
                        session_id, job.job_id
                    ),
                    tags,
                    created_at_ms: current_time_ms(),
                }
            })
            .collect()
    }

    pub fn build_relationships(
        &self,
        session_id: &str,
        job_id: &str,
        memories: &[AtomicMemory],
    ) -> Vec<MemoryRelation> {
        if memories.is_empty() {
            return Vec::new();
        }

        let now = current_time_ms();
        let mut relations = Vec::new();

        for pair in memories.windows(2) {
            if let [left, right] = pair {
                relations.push(MemoryRelation {
                    relation_id: format!(
                        "rel:{job_id}:extends:{}->{}",
                        left.memory_id, right.memory_id
                    ),
                    session_id: session_id.to_string(),
                    job_id: job_id.to_string(),
                    from_memory_id: left.memory_id.clone(),
                    to_memory_id: right.memory_id.clone(),
                    relation_type: MemoryRelationType::Extends,
                    weight: 0.72,
                    is_latest: false,
                    valid: true,
                    rationale: "adjacent semantic progression".into(),
                    invalidated_at_ms: None,
                    invalidation_reason: None,
                    created_at_ms: now,
                });

                if lexical_overlap(&left.statement, &right.statement) >= 2 {
                    relations.push(MemoryRelation {
                        relation_id: format!(
                            "rel:{job_id}:updates:{}->{}",
                            left.memory_id, right.memory_id
                        ),
                        session_id: session_id.to_string(),
                        job_id: job_id.to_string(),
                        from_memory_id: left.memory_id.clone(),
                        to_memory_id: right.memory_id.clone(),
                        relation_type: MemoryRelationType::Updates,
                        weight: 0.84,
                        is_latest: false,
                        valid: true,
                        rationale: "new statement supersedes earlier statement context".into(),
                        invalidated_at_ms: None,
                        invalidation_reason: None,
                        created_at_ms: now,
                    });
                }
            }
        }

        if let Some(root) = memories.first() {
            for derived in memories.iter().skip(2).step_by(2) {
                relations.push(MemoryRelation {
                    relation_id: format!(
                        "rel:{job_id}:derives:{}->{}",
                        root.memory_id, derived.memory_id
                    ),
                    session_id: session_id.to_string(),
                    job_id: job_id.to_string(),
                    from_memory_id: root.memory_id.clone(),
                    to_memory_id: derived.memory_id.clone(),
                    relation_type: MemoryRelationType::Derives,
                    weight: 0.66,
                    is_latest: false,
                    valid: true,
                    rationale: "cross-chunk inferred theme continuity".into(),
                    invalidated_at_ms: None,
                    invalidation_reason: None,
                    created_at_ms: now,
                });
            }
        }

        if let Some(latest) = memories.last() {
            for old in memories.iter().take(memories.len().saturating_sub(1)) {
                relations.push(MemoryRelation {
                    relation_id: format!(
                        "rel:{job_id}:isLatest:{}->{}",
                        old.memory_id, latest.memory_id
                    ),
                    session_id: session_id.to_string(),
                    job_id: job_id.to_string(),
                    from_memory_id: old.memory_id.clone(),
                    to_memory_id: latest.memory_id.clone(),
                    relation_type: MemoryRelationType::IsLatest,
                    weight: 1.0,
                    is_latest: true,
                    valid: true,
                    rationale: "latest memory in this ingestion chain".into(),
                    invalidated_at_ms: None,
                    invalidation_reason: None,
                    created_at_ms: now,
                });
            }
        }

        relations
    }

    pub fn temporal_grounding(&self, job: &IngestionJob) -> TemporalGrounding {
        let now = current_time_ms();
        let freshness_score = match (&job.document_date, &job.event_date) {
            (Some(_), Some(_)) => 0.92,
            (Some(_), None) | (None, Some(_)) => 0.78,
            (None, None) => 0.55,
        };

        TemporalGrounding {
            session_id: job.session_id.clone(),
            job_id: job.job_id.clone(),
            document_id: job.document_id.clone(),
            document_date: job.document_date.clone(),
            event_date: job.event_date.clone(),
            ingested_at_ms: now,
            last_updated_at_ms: now,
            freshness_score,
        }
    }

    pub fn build_user_profile(
        &self,
        session_id: &str,
        memories: &[AtomicMemory],
    ) -> UserProfileSnapshot {
        let mut static_facts = Vec::new();
        let mut dynamic_context = Vec::new();

        for memory in memories {
            if memory.is_static {
                static_facts.push(memory.statement.clone());
            } else {
                dynamic_context.push(memory.statement.clone());
            }
        }

        static_facts.truncate(12);
        dynamic_context.truncate(16);

        UserProfileSnapshot {
            session_id: session_id.to_string(),
            static_facts,
            dynamic_context,
            memory_count: memories.len(),
            generated_at_ms: current_time_ms(),
        }
    }

    pub fn hybrid_search_from_records(
        &self,
        query: &str,
        top_k: usize,
        memories: &[AtomicMemory],
        chunks: &[SemanticChunk],
        temporal: &TemporalGrounding,
    ) -> Vec<HybridSearchHit> {
        let chunk_map = chunks
            .iter()
            .map(|chunk| (chunk.chunk_id.clone(), chunk))
            .collect::<HashMap<_, _>>();

        let query_terms = tokenize(query);
        let mut ranked_memories = memories
            .iter()
            .map(|memory| HybridSearchPhaseMemory {
                memory: memory.clone(),
                memory_score: memory_phase_score(&query_terms, memory, temporal),
            })
            .filter(|phase| phase.memory_score > 0.0)
            .collect::<Vec<_>>();
        ranked_memories.sort_by(|left, right| {
            right
                .memory_score
                .partial_cmp(&left.memory_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if ranked_memories.is_empty() {
            ranked_memories = memories
                .iter()
                .map(|memory| HybridSearchPhaseMemory {
                    memory: memory.clone(),
                    memory_score: (memory.confidence + temporal.freshness_score).max(0.01),
                })
                .collect::<Vec<_>>();
        }

        let mut hits = ranked_memories
            .into_iter()
            .take(top_k.max(1))
            .map(|phase| {
                let chunk_ref = chunk_map.get(&phase.memory.chunk_id);
                let chunk_text = chunk_ref
                    .map(|chunk| chunk.text.clone())
                    .unwrap_or_default();
                let chunk_score = chunk_ref
                    .map(|chunk| chunk_inject_score(&query_terms, chunk))
                    .unwrap_or(0.0);
                let score = phase.memory_score + chunk_score;

                HybridSearchHit {
                    memory_id: phase.memory.memory_id.clone(),
                    chunk_id: phase.memory.chunk_id.clone(),
                    score,
                    memory_text: phase.memory.statement.clone(),
                    chunk_text,
                    source_ref: phase.memory.source_ref.clone(),
                    reason: format!(
                        "memory-first score {:.2} + source-chunk-inject {:.2}",
                        phase.memory_score, chunk_score
                    ),
                    tags: phase.memory.tags.clone(),
                    document_date: temporal.document_date.clone(),
                    event_date: temporal.event_date.clone(),
                }
            })
            .collect::<Vec<_>>();

        hits.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits
    }

    pub async fn hybrid_search(
        &self,
        db: &StateStore,
        session_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<HybridSearchHit>> {
        self.hybrid_search_scoped(
            db,
            session_id,
            query,
            top_k,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .await
    }

    pub async fn hybrid_search_scoped(
        &self,
        db: &StateStore,
        session_id: &str,
        query: &str,
        top_k: usize,
        input_metadata: &BTreeMap<String, String>,
        input_filters: &BTreeMap<String, String>,
    ) -> Result<Vec<HybridSearchHit>> {
        let scope_contract =
            self.build_scope_contract(session_id, input_metadata, input_filters)?;
        let memories = self
            .read_scoped_atomic_memories(db, session_id, &scope_contract)
            .await?;
        let chunks = db
            .list_knowledge_by_prefix(&format!("memory:supermemory:chunks:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<SemanticChunk>(&record.value).ok())
            .collect::<Vec<_>>();
        let chunks = dedupe_semantic_chunks(chunks);

        let temporal = latest_temporal(db, session_id)
            .await?
            .unwrap_or(TemporalGrounding {
                session_id: session_id.to_string(),
                job_id: "job:unknown".into(),
                document_id: "doc:unknown".into(),
                document_date: None,
                event_date: None,
                ingested_at_ms: current_time_ms(),
                last_updated_at_ms: current_time_ms(),
                freshness_score: 0.5,
            });

        Ok(self.hybrid_search_from_records(query, top_k, &memories, &chunks, &temporal))
    }
    pub fn assemble_context(
        &self,
        session_id: &str,
        query: &str,
        profile: UserProfileSnapshot,
        hits: Vec<HybridSearchHit>,
    ) -> ContextAssembly {
        let source_evidence_refs = hits
            .iter()
            .map(|hit| hit.source_ref.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let document_refs = hits
            .iter()
            .map(|hit| format!("doc-ref:{}", hit.memory_id))
            .collect::<Vec<_>>();

        ContextAssembly {
            session_id: session_id.to_string(),
            query: query.to_string(),
            profile,
            hits: hits.clone(),
            source_evidence_refs,
            document_refs,
            assembled_at_ms: current_time_ms(),
            summary: format!(
                "Assembled supermemory context with {} hits and {} profile facts",
                hits.len(),
                hits.iter().filter(|item| item.score > 1.0).count()
            ),
        }
    }

    fn build_scope_contract(
        &self,
        session_id: &str,
        input_metadata: &BTreeMap<String, String>,
        input_filters: &BTreeMap<String, String>,
    ) -> Result<MemoryScopeContract> {
        let mut metadata_template = input_metadata.clone();
        let mut query_filters = input_filters.clone();

        let metadata_filter_dsl = {
            let dsl_raw = metadata_template
                .remove("metadata_filter_dsl")
                .or_else(|| query_filters.remove("metadata_filter_dsl"));
            match dsl_raw {
                Some(raw) => Some(serde_json::from_str::<serde_json::Value>(&raw).map_err(
                    |error| anyhow::anyhow!("invalid metadata_filter_dsl JSON: {error}"),
                )?),
                None => None,
            }
        };

        let user_id = metadata_template
            .get("user_id")
            .cloned()
            .or_else(|| query_filters.get("user_id").cloned());
        let agent_id = metadata_template
            .get("agent_id")
            .cloned()
            .or_else(|| query_filters.get("agent_id").cloned());
        let run_id = metadata_template
            .get("run_id")
            .cloned()
            .or_else(|| query_filters.get("run_id").cloned())
            .or_else(|| Some(session_id.to_string()));

        if user_id.is_none() && agent_id.is_none() && run_id.is_none() {
            bail!("supermemory scope contract requires one of user_id/agent_id/run_id");
        }

        if let Some(value) = &user_id {
            metadata_template.insert("user_id".to_string(), value.clone());
            query_filters.insert("user_id".to_string(), value.clone());
        }
        if let Some(value) = &agent_id {
            metadata_template.insert("agent_id".to_string(), value.clone());
            query_filters.insert("agent_id".to_string(), value.clone());
        }
        if let Some(value) = &run_id {
            metadata_template.insert("run_id".to_string(), value.clone());
            query_filters.insert("run_id".to_string(), value.clone());
        }

        let actor_id = metadata_template
            .get("actor_id")
            .cloned()
            .or_else(|| metadata_template.get("actor_filter").cloned())
            .or_else(|| metadata_template.get("scope_actor_filter").cloned())
            .or_else(|| query_filters.get("actor_id").cloned());

        metadata_template.remove("actor_id");
        metadata_template.remove("actor_filter");
        metadata_template.remove("scope_actor_filter");
        if let Some(value) = &actor_id {
            query_filters.insert("actor_id".to_string(), value.clone());
        }

        metadata_template.insert(
            "scope_contract".to_string(),
            "user_agent_run_actor_v1".to_string(),
        );

        Ok(MemoryScopeContract {
            scope: MemoryScopeSpec {
                user_id,
                agent_id,
                run_id,
                actor_id,
            },
            metadata_template,
            query_filters,
            metadata_filter_dsl,
        })
    }
    async fn read_scoped_atomic_memories(
        &self,
        db: &StateStore,
        session_id: &str,
        scope_contract: &MemoryScopeContract,
    ) -> Result<Vec<AtomicMemory>> {
        let memories = db
            .list_knowledge_by_prefix(&format!("memory:supermemory:atomic:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<AtomicMemory>(&record.value).ok())
            .collect::<Vec<_>>();
        let memories = dedupe_atomic_memories(memories);
        let scope_map = self.load_scope_index_map(db, session_id).await?;
        let decision_map = self.load_decision_state_map(db, session_id).await?;

        Ok(memories
            .into_iter()
            .filter(|memory| {
                !matches!(
                    decision_map.get(&memory.memory_id),
                    Some(MemoryWriteAction::Delete)
                ) && self.memory_matches_scope(
                    memory,
                    scope_map.get(&memory.memory_id),
                    scope_contract,
                ) && self.memory_matches_metadata_filter(
                    memory,
                    scope_map.get(&memory.memory_id),
                    scope_contract.metadata_filter_dsl.as_ref(),
                )
            })
            .collect())
    }

    async fn load_scope_index_map(
        &self,
        db: &StateStore,
        session_id: &str,
    ) -> Result<HashMap<String, MemoryScopeIndexRecord>> {
        let mut map = HashMap::new();
        for record in db
            .list_knowledge_by_prefix(&format!("memory:supermemory:scope-index:{session_id}:"))
            .await?
        {
            if let Ok(value) = serde_json::from_str::<MemoryScopeIndexRecord>(&record.value) {
                map.insert(value.memory_id.clone(), value);
            }
        }
        Ok(map)
    }

    async fn load_decision_state_map(
        &self,
        db: &StateStore,
        session_id: &str,
    ) -> Result<HashMap<String, MemoryWriteAction>> {
        let mut map = HashMap::new();
        for record in db
            .list_knowledge_by_prefix(&format!("memory:supermemory:decision-state:{session_id}:"))
            .await?
        {
            if let Ok(value) = serde_json::from_str::<MemoryWriteDecision>(&record.value) {
                if let Some(memory_id) = value
                    .target_memory_id
                    .clone()
                    .or_else(|| value.resulting_memory_id.clone())
                {
                    map.insert(memory_id, value.action);
                }
            }
        }
        Ok(map)
    }

    fn memory_matches_scope(
        &self,
        memory: &AtomicMemory,
        scope_index: Option<&MemoryScopeIndexRecord>,
        scope_contract: &MemoryScopeContract,
    ) -> bool {
        for (key, value) in &scope_contract.query_filters {
            let candidate = match key.as_str() {
                "user_id" => scope_index.and_then(|item| item.user_id.as_deref()),
                "agent_id" => scope_index.and_then(|item| item.agent_id.as_deref()),
                "run_id" => scope_index
                    .and_then(|item| item.run_id.as_deref())
                    .or(Some(memory.session_id.as_str())),
                "actor_id" => scope_index.and_then(|item| item.actor_id.as_deref()),
                _ => continue,
            };
            if candidate != Some(value.as_str()) {
                return false;
            }
        }
        true
    }

    fn memory_matches_metadata_filter(
        &self,
        memory: &AtomicMemory,
        scope_index: Option<&MemoryScopeIndexRecord>,
        metadata_filter_dsl: Option<&serde_json::Value>,
    ) -> bool {
        let Some(filter) = metadata_filter_dsl else {
            return true;
        };
        let fields = self.build_memory_filter_fields(memory, scope_index);
        evaluate_filter_dsl(filter, &fields)
    }

    fn build_memory_filter_fields(
        &self,
        memory: &AtomicMemory,
        scope_index: Option<&MemoryScopeIndexRecord>,
    ) -> BTreeMap<String, serde_json::Value> {
        let mut fields = BTreeMap::new();
        fields.insert("memory_id".to_string(), serde_json::json!(memory.memory_id));
        fields.insert(
            "session_id".to_string(),
            serde_json::json!(memory.session_id),
        );
        fields.insert("job_id".to_string(), serde_json::json!(memory.job_id));
        fields.insert(
            "document_id".to_string(),
            serde_json::json!(memory.document_id),
        );
        fields.insert("chunk_id".to_string(), serde_json::json!(memory.chunk_id));
        fields.insert("statement".to_string(), serde_json::json!(memory.statement));
        fields.insert(
            "confidence".to_string(),
            serde_json::json!(memory.confidence),
        );
        fields.insert("is_static".to_string(), serde_json::json!(memory.is_static));
        fields.insert(
            "source_ref".to_string(),
            serde_json::json!(memory.source_ref),
        );
        fields.insert("tags".to_string(), serde_json::json!(memory.tags));
        fields.insert(
            "created_at_ms".to_string(),
            serde_json::json!(memory.created_at_ms),
        );

        if let Some(scope) = scope_index {
            if let Some(user_id) = &scope.user_id {
                fields.insert("user_id".to_string(), serde_json::json!(user_id));
            }
            if let Some(agent_id) = &scope.agent_id {
                fields.insert("agent_id".to_string(), serde_json::json!(agent_id));
            }
            if let Some(run_id) = &scope.run_id {
                fields.insert("run_id".to_string(), serde_json::json!(run_id));
            }
            if let Some(actor_id) = &scope.actor_id {
                fields.insert("actor_id".to_string(), serde_json::json!(actor_id));
            }
        }

        fields
    }
    fn reconcile_memory_writes(
        &self,
        job: &IngestionJob,
        _scope_contract: &MemoryScopeContract,
        existing_memories: &[AtomicMemory],
        candidate_memories: &[AtomicMemory],
    ) -> (
        Vec<AtomicMemory>,
        Vec<AtomicMemory>,
        Vec<MemoryWriteDecision>,
    ) {
        let mut active_by_id = existing_memories
            .iter()
            .map(|memory| (memory.memory_id.clone(), memory.clone()))
            .collect::<HashMap<_, _>>();

        let mut to_persist = Vec::new();
        let mut decisions = Vec::new();
        let mut seen_candidate_statements = HashSet::new();

        for (index, candidate) in candidate_memories.iter().enumerate() {
            let normalized_candidate = normalize_fact_statement(&candidate.statement);
            if normalized_candidate.is_empty() {
                decisions.push(self.build_write_decision(
                    job,
                    index,
                    candidate,
                    MemoryWriteAction::None,
                    None,
                    None,
                    Some("empty candidate statement".to_string()),
                ));
                continue;
            }

            if !seen_candidate_statements.insert(normalized_candidate.clone()) {
                decisions.push(self.build_write_decision(
                    job,
                    index,
                    candidate,
                    MemoryWriteAction::None,
                    None,
                    None,
                    Some("duplicate candidate statement".to_string()),
                ));
                continue;
            }

            let best_match_id = find_best_memory_match_id(candidate, &active_by_id);
            if let Some(memory_id) = best_match_id {
                if let Some(existing) = active_by_id.get(&memory_id).cloned() {
                    let normalized_existing = normalize_fact_statement(&existing.statement);
                    if normalized_existing == normalized_candidate {
                        decisions.push(self.build_write_decision(
                            job,
                            index,
                            candidate,
                            MemoryWriteAction::None,
                            Some(existing.clone()),
                            Some(existing.clone()),
                            Some("same statement already stored".to_string()),
                        ));
                        continue;
                    }

                    if is_contradictory_statement(&existing.statement, &candidate.statement) {
                        active_by_id.remove(&memory_id);
                        decisions.push(self.build_write_decision(
                            job,
                            index,
                            candidate,
                            MemoryWriteAction::Delete,
                            Some(existing),
                            None,
                            Some("contradictory statement detected".to_string()),
                        ));
                        continue;
                    }

                    if should_update_memory(&existing.statement, &candidate.statement) {
                        let mut updated = existing.clone();
                        updated.job_id = job.job_id.clone();
                        updated.document_id = job.document_id.clone();
                        updated.chunk_id = candidate.chunk_id.clone();
                        updated.statement = candidate.statement.clone();
                        updated.confidence = updated.confidence.max(candidate.confidence);
                        updated.tags = merge_tags(&existing.tags, &candidate.tags);
                        updated.source_ref = candidate.source_ref.clone();
                        updated.created_at_ms = current_time_ms();

                        active_by_id.insert(updated.memory_id.clone(), updated.clone());
                        to_persist.push(updated.clone());
                        decisions.push(self.build_write_decision(
                            job,
                            index,
                            candidate,
                            MemoryWriteAction::Update,
                            Some(existing),
                            Some(updated),
                            Some("overlapping statement upgraded".to_string()),
                        ));
                        continue;
                    }
                }
            }

            active_by_id.insert(candidate.memory_id.clone(), candidate.clone());
            to_persist.push(candidate.clone());
            decisions.push(self.build_write_decision(
                job,
                index,
                candidate,
                MemoryWriteAction::Add,
                None,
                Some(candidate.clone()),
                Some("new statement".to_string()),
            ));
        }

        let mut active_memories = active_by_id.into_values().collect::<Vec<_>>();
        active_memories.sort_by(|left, right| {
            left.created_at_ms
                .cmp(&right.created_at_ms)
                .then_with(|| left.memory_id.cmp(&right.memory_id))
        });

        (to_persist, active_memories, decisions)
    }

    fn build_write_decision(
        &self,
        job: &IngestionJob,
        index: usize,
        candidate: &AtomicMemory,
        action: MemoryWriteAction,
        old_memory: Option<AtomicMemory>,
        new_memory: Option<AtomicMemory>,
        reason: Option<String>,
    ) -> MemoryWriteDecision {
        let now = current_time_ms();
        MemoryWriteDecision {
            decision_id: format!("decision:{}:{}:{}", job.job_id, index, now),
            session_id: job.session_id.clone(),
            job_id: job.job_id.clone(),
            candidate_text: candidate.statement.clone(),
            action,
            target_memory_id: old_memory.as_ref().map(|item| item.memory_id.clone()),
            resulting_memory_id: new_memory.as_ref().map(|item| item.memory_id.clone()),
            reason: reason.unwrap_or_else(|| "decision applied".to_string()),
            old_memory: old_memory.as_ref().map(|item| item.statement.clone()),
            new_memory: new_memory.as_ref().map(|item| item.statement.clone()),
            created_at_ms: now,
        }
    }

    async fn persist_scope_index(
        &self,
        db: &StateStore,
        session_id: &str,
        memory_id: &str,
        scope: &MemoryScopeSpec,
    ) -> Result<()> {
        let record = MemoryScopeIndexRecord {
            memory_id: memory_id.to_string(),
            session_id: session_id.to_string(),
            user_id: scope.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            run_id: scope.run_id.clone(),
            actor_id: scope.actor_id.clone(),
            updated_at_ms: current_time_ms(),
        };
        self.persist_record_by_key(
            db,
            &record,
            "supermemory-kernel",
            format!("memory:supermemory:scope-index:{session_id}:{memory_id}"),
        )
        .await
    }

    async fn persist_decisions(
        &self,
        db: &StateStore,
        job: &IngestionJob,
        decisions: &[MemoryWriteDecision],
    ) -> Result<()> {
        for decision in decisions {
            self.persist_record_by_key(
                db,
                decision,
                "supermemory-kernel",
                format!(
                    "memory:supermemory:decisions:{}:{}:{}",
                    job.session_id, job.job_id, decision.decision_id
                ),
            )
            .await?;
            if let Some(memory_id) = decision
                .target_memory_id
                .clone()
                .or_else(|| decision.resulting_memory_id.clone())
            {
                self.persist_record_by_key(
                    db,
                    decision,
                    "supermemory-kernel",
                    format!(
                        "memory:supermemory:decision-state:{}:{}",
                        job.session_id, memory_id
                    ),
                )
                .await?;
            }
        }
        Ok(())
    }
    async fn persist_relation_soft_deletes(
        &self,
        db: &StateStore,
        job: &IngestionJob,
        decisions: &[MemoryWriteDecision],
    ) -> Result<()> {
        let mut invalidation_targets = HashMap::new();
        for decision in decisions {
            if !matches!(
                decision.action,
                MemoryWriteAction::Delete | MemoryWriteAction::Update
            ) {
                continue;
            }
            if let Some(memory_id) = decision.target_memory_id.clone() {
                invalidation_targets.insert(memory_id, decision.clone());
            }
        }

        if invalidation_targets.is_empty() {
            return Ok(());
        }

        let relation_records = db
            .list_knowledge_by_prefix(&format!("memory:supermemory:relations:{}:", job.session_id))
            .await?;

        for record in relation_records {
            if !record.key.ends_with(":latest") {
                continue;
            }

            let Ok(mut relation) = serde_json::from_str::<MemoryRelation>(&record.value) else {
                continue;
            };

            if !relation.valid {
                continue;
            }

            let decision = invalidation_targets
                .get(&relation.from_memory_id)
                .or_else(|| invalidation_targets.get(&relation.to_memory_id));
            let Some(decision) = decision else {
                continue;
            };

            let now = current_time_ms();
            relation.valid = false;
            relation.is_latest = false;
            relation.invalidated_at_ms = Some(now);
            relation.invalidation_reason = Some(format!(
                "soft-delete:{}:{}:{}",
                memory_write_action_label(decision.action.clone()),
                decision.decision_id,
                decision.reason
            ));

            let append_suffix = format!(
                "{}:softdelete:{}:{}",
                job.job_id, decision.decision_id, relation.relation_id
            );
            self.persist_append_record(
                db,
                "relations",
                &job.session_id,
                &append_suffix,
                &relation,
                "supermemory-kernel",
            )
            .await?;
            self.persist_record_by_key(db, &relation, "supermemory-kernel", record.key.clone())
                .await?;
            self.persist_update_record(
                db,
                "relations",
                &job.session_id,
                &relation,
                "supermemory-kernel",
            )
            .await?;
        }

        Ok(())
    }

    async fn persist_memory_history_subledger(
        &self,
        db: &StateStore,
        job: &IngestionJob,
        decisions: &[MemoryWriteDecision],
        scope_contract: &MemoryScopeContract,
    ) -> Result<()> {
        let actor_id = scope_contract
            .scope
            .actor_id
            .clone()
            .or_else(|| job.metadata.get("actor_id").cloned())
            .or_else(|| job.metadata.get("actor_filter").cloned())
            .or_else(|| job.metadata.get("scope_actor_filter").cloned());
        let role = job
            .metadata
            .get("role")
            .cloned()
            .or_else(|| Some("memory-kernel".to_string()));

        for (index, decision) in decisions.iter().enumerate() {
            let memory_id = decision
                .target_memory_id
                .clone()
                .or_else(|| decision.resulting_memory_id.clone())
                .unwrap_or_else(|| format!("memory:unknown:{}", index));
            let now = current_time_ms();
            let event = memory_write_action_label(decision.action.clone());
            let history = MemoryHistoryRecord {
                history_id: format!("history:{}:{}:{}", job.job_id, index, now),
                session_id: job.session_id.clone(),
                job_id: job.job_id.clone(),
                trace_id: format!("trace:{}:{}", job.session_id, job.job_id),
                memory_id: memory_id.clone(),
                old_memory: decision.old_memory.clone(),
                new_memory: decision.new_memory.clone(),
                event: event.clone(),
                actor_id: actor_id.clone(),
                role: role.clone(),
                is_deleted: decision.action == MemoryWriteAction::Delete,
                created_at_ms: now,
                updated_at_ms: now,
            };

            self.persist_record_by_key(
                db,
                &history,
                "supermemory-kernel",
                format!(
                    "memory:supermemory:history:{}:{}:{}",
                    job.session_id, job.job_id, history.history_id
                ),
            )
            .await?;

            self.persist_record_by_key(
                db,
                &history,
                "supermemory-kernel",
                format!(
                    "memory:supermemory:history:{}:{}:latest",
                    job.session_id, memory_id
                ),
            )
            .await?;

            self.persist_record_by_key(
                db,
                &history,
                "evidence-ledger",
                format!("evidence:memory:{}:{}", job.session_id, history.history_id),
            )
            .await?;
        }

        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn persist_pipeline(
        &self,
        db: &StateStore,
        job: &IngestionJob,
        extracted: &ExtractedContent,
        chunks: &[SemanticChunk],
        embeddings: &[EmbeddedChunk],
        memories: &[AtomicMemory],
        relations: &[MemoryRelation],
        temporal: &TemporalGrounding,
        profile: &UserProfileSnapshot,
        context: &ContextAssembly,
        decisions: &[MemoryWriteDecision],
        scope_contract: &MemoryScopeContract,
    ) -> Result<()> {
        self.persist_job(db, job).await?;

        self.persist_append_record(
            db,
            "documents",
            &job.session_id,
            &job.job_id,
            extracted,
            "supermemory-kernel",
        )
        .await?;
        self.persist_update_record(
            db,
            "documents",
            &job.session_id,
            extracted,
            "supermemory-kernel",
        )
        .await?;

        for chunk in chunks {
            let append_suffix = format!("{}:{}", job.job_id, chunk.sequence);
            self.persist_append_record(
                db,
                "chunks",
                &job.session_id,
                &append_suffix,
                chunk,
                "supermemory-kernel",
            )
            .await?;
            self.persist_update_record(db, "chunks", &job.session_id, chunk, "supermemory-kernel")
                .await?;
        }

        for embedding in embeddings {
            db.upsert_json_knowledge(
                format!(
                    "memory:supermemory:embeddings:{}:{}",
                    job.session_id, embedding.chunk_id
                ),
                embedding,
                "supermemory-kernel",
            )
            .await?;
        }

        for memory in memories {
            let append_suffix = format!("{}:{}", job.job_id, memory.memory_id);
            self.persist_append_record(
                db,
                "atomic",
                &job.session_id,
                &append_suffix,
                memory,
                "supermemory-kernel",
            )
            .await?;
            self.persist_update_record(db, "atomic", &job.session_id, memory, "supermemory-kernel")
                .await?;
            self.persist_scope_index(
                db,
                &job.session_id,
                &memory.memory_id,
                &scope_contract.scope,
            )
            .await?;
        }

        for relation in relations {
            let append_suffix = format!("{}:{}", job.job_id, relation.relation_id);
            self.persist_append_record(
                db,
                "relations",
                &job.session_id,
                &append_suffix,
                relation,
                "supermemory-kernel",
            )
            .await?;
            self.persist_update_record(
                db,
                "relations",
                &job.session_id,
                relation,
                "supermemory-kernel",
            )
            .await?;
        }

        db.upsert_json_knowledge(
            format!(
                "memory:supermemory:temporal:{}:{}",
                job.session_id, job.job_id
            ),
            temporal,
            "supermemory-kernel",
        )
        .await?;

        self.persist_append_record(
            db,
            "profile",
            &job.session_id,
            &job.job_id,
            profile,
            "supermemory-kernel",
        )
        .await?;
        self.persist_update_record(
            db,
            "profile",
            &job.session_id,
            profile,
            "supermemory-kernel",
        )
        .await?;

        self.persist_append_record(
            db,
            "context",
            &job.session_id,
            &job.job_id,
            context,
            "supermemory-kernel",
        )
        .await?;
        self.persist_update_record(
            db,
            "context",
            &job.session_id,
            context,
            "supermemory-kernel",
        )
        .await?;

        self.persist_decisions(db, job, decisions).await?;
        self.persist_relation_soft_deletes(db, job, decisions)
            .await?;
        self.persist_memory_history_subledger(db, job, decisions, scope_contract)
            .await?;

        Ok(())
    }

    async fn persist_job(&self, db: &StateStore, job: &IngestionJob) -> Result<()> {
        self.persist_append_record(
            db,
            "queue",
            &job.session_id,
            &job.job_id,
            job,
            "supermemory-kernel",
        )
        .await?;
        self.persist_update_record(db, "queue", &job.session_id, job, "supermemory-kernel")
            .await?;
        Ok(())
    }

    pub async fn run_queue_worker_once(
        &self,
        db: &StateStore,
        session_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Option<ContextAssembly>> {
        let mut queued = db
            .list_knowledge_by_prefix(&format!("memory:supermemory:queue:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<IngestionJob>(&record.value).ok())
            .filter(|job| job.status == ProcessingStatus::Queued)
            .collect::<Vec<_>>();
        queued.sort_by_key(|job| job.queued_at_ms);

        let Some(job) = queued.into_iter().next() else {
            return Ok(None);
        };
        let payload = self
            .load_ingestion_payload(db, &job)
            .await?
            .unwrap_or_else(|| job.source.clone());
        let context = self
            .process_ingestion_job(db, job, &payload, query, top_k)
            .await?;
        Ok(Some(context))
    }

    async fn persist_ingestion_payload(
        &self,
        db: &StateStore,
        job: &IngestionJob,
        content: &str,
    ) -> Result<()> {
        db.upsert_knowledge(
            format!(
                "memory:supermemory:ingest-payload:{}:{}",
                job.session_id, job.job_id
            ),
            content.to_string(),
            "supermemory-kernel".into(),
        )
        .await?;
        Ok(())
    }

    async fn load_ingestion_payload(
        &self,
        db: &StateStore,
        job: &IngestionJob,
    ) -> Result<Option<String>> {
        Ok(db
            .get_knowledge(&format!(
                "memory:supermemory:ingest-payload:{}:{}",
                job.session_id, job.job_id
            ))
            .await?
            .map(|record| record.value))
    }
    async fn persist_append_record<T: Serialize>(
        &self,
        db: &StateStore,
        domain: &str,
        session_id: &str,
        append_suffix: &str,
        payload: &T,
        source: &str,
    ) -> Result<()> {
        let key = format!("memory:supermemory:{domain}:{session_id}:{append_suffix}");
        self.persist_record_by_key(db, payload, source, key).await
    }

    async fn persist_update_record<T: Serialize>(
        &self,
        db: &StateStore,
        domain: &str,
        session_id: &str,
        payload: &T,
        source: &str,
    ) -> Result<()> {
        let key = format!("memory:supermemory:{domain}:{session_id}:latest");
        self.persist_record_by_key(db, payload, source, key).await
    }

    async fn persist_record_by_key<T: Serialize>(
        &self,
        db: &StateStore,
        payload: &T,
        source: &str,
        key: String,
    ) -> Result<()> {
        db.upsert_json_knowledge(key, payload, source).await?;
        Ok(())
    }

    fn hash_embedding(&self, text: &str) -> Vec<f32> {
        let dims = self.embedding_dimensions.max(8);
        let mut vector = vec![0.0f32; dims];
        let bytes = text.as_bytes();

        for (index, byte) in bytes.iter().enumerate() {
            let slot = index % dims;
            vector[slot] += (*byte as f32 / 255.0) * ((index % 13 + 1) as f32 / 13.0);
        }

        let norm = vector.iter().map(|item| item * item).sum::<f32>().sqrt();
        if norm > 0.0 {
            for value in &mut vector {
                *value /= norm;
            }
        }
        vector
    }
}

async fn latest_temporal(db: &StateStore, session_id: &str) -> Result<Option<TemporalGrounding>> {
    let records = db
        .list_knowledge_by_prefix(&format!("memory:supermemory:temporal:{session_id}:"))
        .await?;
    let mut latest: Option<TemporalGrounding> = None;
    for record in records {
        if let Ok(item) = serde_json::from_str::<TemporalGrounding>(&record.value) {
            let replace = latest
                .as_ref()
                .map(|current| item.last_updated_at_ms > current.last_updated_at_ms)
                .unwrap_or(true);
            if replace {
                latest = Some(item);
            }
        }
    }
    Ok(latest)
}

fn split_semantic_blocks(text: &str, target_size: usize) -> Vec<String> {
    let mut output = Vec::new();
    let blocks = text
        .split("\n\n")
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    for block in blocks {
        if block.len() <= target_size {
            output.push(block.to_string());
            continue;
        }
        let mut current = String::new();
        for sentence in split_sentences(block) {
            let candidate_len = current
                .len()
                .saturating_add(sentence.len())
                .saturating_add(1);
            if candidate_len > target_size && !current.trim().is_empty() {
                output.push(current.trim().to_string());
                current.clear();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(sentence);
        }
        if !current.trim().is_empty() {
            output.push(current.trim().to_string());
        }
    }

    if output.is_empty() {
        vec![text.trim().to_string()]
    } else {
        output
    }
}

fn split_sentences(text: &str) -> Vec<&str> {
    text.split_terminator(['.', '!', '?'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
}

async fn try_external_ocr(
    raw: &str,
    metadata: &BTreeMap<String, String>,
    config: &OcrConfig,
) -> Result<Option<String>> {
    match config.backend {
        OcrBackend::None => Ok(None),
        OcrBackend::Http => run_http_ocr(raw, metadata, config).await,
        OcrBackend::Command => run_command_ocr(raw, config).await,
    }
}

async fn run_http_ocr(
    raw: &str,
    metadata: &BTreeMap<String, String>,
    config: &OcrConfig,
) -> Result<Option<String>> {
    let Some(url) = config.http_url.as_deref() else {
        return Ok(None);
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(config.timeout_ms.max(500)))
        .build()?;
    let mut request = client.post(url).json(&serde_json::json!({
        "input": raw,
        "metadata": metadata,
    }));
    if let Some(token) = config
        .http_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
    {
        request = request.bearer_auth(token);
    }

    let mut last_error: Option<anyhow::Error> = None;
    for _ in 0..2 {
        match request
            .try_clone()
            .ok_or_else(|| anyhow::anyhow!("ocr request clone failed"))?
            .send()
            .await
        {
            Ok(response) => {
                if !response.status().is_success() {
                    last_error = Some(anyhow::anyhow!("ocr http status {}", response.status()));
                    continue;
                }
                let body: serde_json::Value = response
                    .json()
                    .await
                    .unwrap_or_else(|_| serde_json::json!({}));
                if let Some(text) = body.get("ocr_text").and_then(serde_json::Value::as_str) {
                    return Ok(Some(text.to_string()));
                }
                if let Some(text) = body.get("text").and_then(serde_json::Value::as_str) {
                    return Ok(Some(text.to_string()));
                }
                if let Some(text) = body.get("result").and_then(serde_json::Value::as_str) {
                    return Ok(Some(text.to_string()));
                }
                return Ok(None);
            }
            Err(error) => {
                last_error = Some(error.into());
            }
        }
    }
    if let Some(error) = last_error {
        bail!("ocr http backend failed: {}", error);
    }
    Ok(None)
}

async fn run_command_ocr(raw: &str, config: &OcrConfig) -> Result<Option<String>> {
    let Some(command_line) = config.command.as_deref() else {
        return Ok(None);
    };
    let mut parts = command_line.split_whitespace();
    let Some(program) = parts.next() else {
        return Ok(None);
    };
    let mut command = Command::new(program);
    for arg in parts {
        command.arg(arg);
    }
    command.arg(raw);

    let output = timeout(
        Duration::from_millis(config.timeout_ms.max(500)),
        command.output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("ocr command timed out"))??;

    if !output.status.success() {
        bail!("ocr command failed with status {}", output.status);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}
fn detect_content_type(raw: &str, metadata: &BTreeMap<String, String>) -> String {
    if let Some(kind) = metadata.get("content_type") {
        let lowered = kind.to_ascii_lowercase();
        if !lowered.trim().is_empty() {
            return lowered;
        }
    }
    let trimmed = raw.trim_start();
    if trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html") {
        "html".into()
    } else if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        "json".into()
    } else if raw.contains(',') && raw.lines().count() > 1 {
        "csv".into()
    } else if raw.contains("```") || raw.contains("# ") {
        "markdown".into()
    } else if raw.trim_start().starts_with("http") {
        "url".into()
    } else if metadata
        .get("file_ext")
        .map(|v| v.eq_ignore_ascii_case("png") || v.eq_ignore_ascii_case("jpg"))
        .unwrap_or(false)
    {
        "ocr".into()
    } else {
        "text".into()
    }
}

fn extract_by_content_type(
    content_type: &str,
    raw: &str,
    metadata: &BTreeMap<String, String>,
) -> String {
    match content_type {
        "html" => strip_html_tags(raw),
        "json" => flatten_json_text(raw).unwrap_or_else(|| raw.to_string()),
        "csv" => normalize_csv(raw),
        "markdown" | "code_or_markdown" => strip_markdown(raw),
        "ocr" | "image" => metadata
            .get("ocr_text")
            .cloned()
            .unwrap_or_else(|| format!("OCR_PENDING {}", raw)),
        _ => raw.to_string(),
    }
}

fn strip_html_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn flatten_json_text(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut fields = Vec::<String>::new();
    flatten_json_value("", &value, &mut fields);
    if fields.is_empty() {
        None
    } else {
        Some(fields.join("\n"))
    }
}

fn flatten_json_value(prefix: &str, value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let next = if prefix.is_empty() {
                    k.to_string()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_json_value(&next, v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                let next = format!("{prefix}[{idx}]");
                flatten_json_value(&next, item, out);
            }
        }
        _ => {
            let value_text = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            out.push(format!("{prefix}: {value_text}"));
        }
    }
}

fn normalize_csv(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.split(',')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn strip_markdown(text: &str) -> String {
    text.lines()
        .map(|line| {
            line.trim_start_matches('#')
                .trim_start_matches('-')
                .trim_start_matches('*')
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
fn summarize_chunk_statement(text: &str) -> String {
    let first_sentence = split_sentences(text)
        .into_iter()
        .next()
        .unwrap_or(text)
        .trim()
        .to_string();
    if first_sentence.chars().count() <= 220 {
        first_sentence
    } else {
        let head = first_sentence.chars().take(220).collect::<String>();
        format!("{head}...")
    }
}

fn derive_tags(text: &str, metadata: &BTreeMap<String, String>) -> Vec<String> {
    let mut tags = HashSet::new();
    for token in tokenize(text).into_iter().filter(|token| token.len() > 3) {
        if tags.len() >= 8 {
            break;
        }
        tags.insert(token);
    }

    if let Some(extra) = metadata.get("tags") {
        for tag in extra
            .split(',')
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
        {
            tags.insert(tag.to_ascii_lowercase());
        }
    }

    tags.into_iter().collect::<Vec<_>>()
}

fn memory_write_action_label(action: MemoryWriteAction) -> String {
    match action {
        MemoryWriteAction::Add => "ADD".to_string(),
        MemoryWriteAction::Update => "UPDATE".to_string(),
        MemoryWriteAction::Delete => "DELETE".to_string(),
        MemoryWriteAction::None => "NONE".to_string(),
    }
}

fn evaluate_filter_dsl(
    filter: &serde_json::Value,
    fields: &BTreeMap<String, serde_json::Value>,
) -> bool {
    match filter {
        serde_json::Value::Object(map) => {
            for (key, condition) in map {
                match key.as_str() {
                    "AND" => {
                        let Some(items) = condition.as_array() else {
                            return false;
                        };
                        if !items.iter().all(|item| evaluate_filter_dsl(item, fields)) {
                            return false;
                        }
                    }
                    "OR" => {
                        let Some(items) = condition.as_array() else {
                            return false;
                        };
                        if !items.iter().any(|item| evaluate_filter_dsl(item, fields)) {
                            return false;
                        }
                    }
                    "NOT" => {
                        if let Some(items) = condition.as_array() {
                            if items.iter().any(|item| evaluate_filter_dsl(item, fields)) {
                                return false;
                            }
                        } else if evaluate_filter_dsl(condition, fields) {
                            return false;
                        }
                    }
                    field_name => {
                        let field_value = fields
                            .get(field_name)
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        if !evaluate_filter_condition(&field_value, condition) {
                            return false;
                        }
                    }
                }
            }
            true
        }
        _ => false,
    }
}

fn evaluate_filter_condition(
    field_value: &serde_json::Value,
    condition: &serde_json::Value,
) -> bool {
    if !condition.is_object() {
        if condition.as_str() == Some("*") {
            return !field_value.is_null();
        }
        return field_value == condition;
    }

    let Some(map) = condition.as_object() else {
        return false;
    };

    for (op, expected) in map {
        let passed = match op.as_str() {
            "eq" => field_value == expected,
            "ne" => field_value != expected,
            "gt" => numeric_compare(field_value, expected, |a, b| a > b),
            "gte" => numeric_compare(field_value, expected, |a, b| a >= b),
            "lt" => numeric_compare(field_value, expected, |a, b| a < b),
            "lte" => numeric_compare(field_value, expected, |a, b| a <= b),
            "in" => contains_value(expected, field_value),
            "nin" => !contains_value(expected, field_value),
            "contains" => field_contains(field_value, expected, false),
            "icontains" => field_contains(field_value, expected, true),
            _ => false,
        };
        if !passed {
            return false;
        }
    }

    true
}

fn numeric_compare(
    field_value: &serde_json::Value,
    expected: &serde_json::Value,
    comparator: impl Fn(f64, f64) -> bool,
) -> bool {
    let Some(left) = field_value.as_f64() else {
        return false;
    };
    let Some(right) = expected.as_f64() else {
        return false;
    };
    comparator(left, right)
}

fn contains_value(container: &serde_json::Value, expected_member: &serde_json::Value) -> bool {
    container
        .as_array()
        .map(|items| items.iter().any(|item| item == expected_member))
        .unwrap_or(false)
}

fn field_contains(
    field_value: &serde_json::Value,
    expected: &serde_json::Value,
    case_insensitive: bool,
) -> bool {
    let expected_text = value_to_filter_text(expected, case_insensitive);

    if let Some(items) = field_value.as_array() {
        return items.iter().any(|item| {
            let item_text = value_to_filter_text(item, case_insensitive);
            if case_insensitive {
                item_text.contains(&expected_text)
            } else {
                item_text.contains(expected_text.as_str())
            }
        });
    }

    let field_text = value_to_filter_text(field_value, case_insensitive);
    if case_insensitive {
        field_text.contains(&expected_text)
    } else {
        field_text.contains(expected_text.as_str())
    }
}

fn value_to_filter_text(value: &serde_json::Value, case_insensitive: bool) -> String {
    let mut text = match value {
        serde_json::Value::String(s) => s.clone(),
        _ => value.to_string(),
    };
    if case_insensitive {
        text = text.to_ascii_lowercase();
    }
    text
}
fn normalize_fact_statement(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn token_set(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| part.len() >= 2)
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

fn jaccard_similarity(left: &str, right: &str) -> f32 {
    let left_set = token_set(left);
    let right_set = token_set(right);
    if left_set.is_empty() || right_set.is_empty() {
        return 0.0;
    }
    let intersection = left_set.intersection(&right_set).count() as f32;
    let union = left_set.union(&right_set).count() as f32;
    if union <= f32::EPSILON {
        0.0
    } else {
        intersection / union
    }
}

fn find_best_memory_match_id(
    candidate: &AtomicMemory,
    active_by_id: &HashMap<String, AtomicMemory>,
) -> Option<String> {
    active_by_id
        .iter()
        .filter_map(|(memory_id, existing)| {
            let overlap = lexical_overlap(&candidate.statement, &existing.statement) as f32;
            let jaccard = jaccard_similarity(&candidate.statement, &existing.statement);
            if overlap < 2.0 && jaccard < 0.58 {
                return None;
            }
            let score = overlap + jaccard;
            Some((memory_id.clone(), score))
        })
        .max_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(memory_id, _)| memory_id)
}

fn should_update_memory(existing: &str, candidate: &str) -> bool {
    let overlap = lexical_overlap(existing, candidate);
    let similarity = jaccard_similarity(existing, candidate);
    overlap >= 2 || similarity >= 0.42
}

fn contains_negation_markers(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    [
        " not ",
        " never ",
        " cannot ",
        " can't ",
        " wont ",
        " won't ",
        " no ",
        " without ",
        " dislike",
        " hate",
        " avoid",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

fn is_contradictory_statement(existing: &str, candidate: &str) -> bool {
    let overlap = lexical_overlap(existing, candidate);
    if overlap == 0 {
        return false;
    }
    contains_negation_markers(existing) != contains_negation_markers(candidate)
}

fn merge_tags(existing: &[String], incoming: &[String]) -> Vec<String> {
    let mut tags = existing
        .iter()
        .chain(incoming.iter())
        .map(|tag| tag.to_ascii_lowercase())
        .filter(|tag| !tag.trim().is_empty())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags
}
fn lexical_overlap(left: &str, right: &str) -> usize {
    let left_set = tokenize(left);
    let right_set = tokenize(right);
    left_set.intersection(&right_set).count()
}

fn overlap_count(query_terms: &HashSet<String>, text: &str) -> usize {
    let text_terms = tokenize(text);
    query_terms.intersection(&text_terms).count()
}

fn memory_phase_score(
    query_terms: &HashSet<String>,
    memory: &AtomicMemory,
    temporal: &TemporalGrounding,
) -> f32 {
    let statement_score = overlap_count(query_terms, &memory.statement) as f32 * 2.1;
    let tag_score = memory
        .tags
        .iter()
        .filter(|tag| query_terms.contains(&tag.to_ascii_lowercase()))
        .count() as f32
        * 1.2;
    statement_score + tag_score + memory.confidence + temporal.freshness_score * 0.4
}

fn chunk_inject_score(query_terms: &HashSet<String>, chunk: &SemanticChunk) -> f32 {
    let overlap = overlap_count(query_terms, &chunk.text) as f32;
    let token_norm = (chunk.token_estimate as f32 / 96.0).min(1.0);
    overlap + token_norm * 0.25
}

fn dedupe_atomic_memories(memories: Vec<AtomicMemory>) -> Vec<AtomicMemory> {
    let mut by_id = HashMap::<String, AtomicMemory>::new();
    for memory in memories {
        let replace = by_id
            .get(&memory.memory_id)
            .map(|current| memory.created_at_ms > current.created_at_ms)
            .unwrap_or(true);
        if replace {
            by_id.insert(memory.memory_id.clone(), memory);
        }
    }
    by_id.into_values().collect()
}

fn dedupe_semantic_chunks(chunks: Vec<SemanticChunk>) -> Vec<SemanticChunk> {
    let mut by_id = HashMap::<String, SemanticChunk>::new();
    for chunk in chunks {
        let replace = by_id
            .get(&chunk.chunk_id)
            .map(|current| chunk.sequence >= current.sequence)
            .unwrap_or(true);
        if replace {
            by_id.insert(chunk.chunk_id.clone(), chunk);
        }
    }
    by_id.into_values().collect()
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| part.len() >= 2)
        .collect::<HashSet<_>>()
}

fn normalize_whitespace(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\n\n\n", "\n\n")
}

fn estimate_tokens(text: &str) -> u32 {
    ((text.chars().count() as f32) / 4.0).ceil() as u32
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn supermemory_pipeline_persists_and_retrieves_context() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = SupermemoryKernel::default();
        let content = "AutoLoop keeps a governed runtime and verifier gate.\n\nThe system stores execution evidence and graph memory context.\n\nThe operator prefers strict policy and replayable traces.";

        let context = kernel
            .run_pipeline(
                &db,
                "session-supermemory",
                "tenant-supermemory",
                "unit-test",
                content,
                BTreeMap::from([(
                    String::from("tags"),
                    String::from("governance,memory,policy"),
                )]),
                Some("2026-03-23".into()),
                Some("2026-03-23".into()),
                "policy memory replay",
                5,
            )
            .await
            .expect("pipeline");

        assert!(!context.hits.is_empty());
        assert!(context.summary.contains("Assembled supermemory context"));

        let queue = db
            .list_knowledge_by_prefix("memory:supermemory:queue:session-supermemory:")
            .await
            .expect("queue");
        let chunks = db
            .list_knowledge_by_prefix("memory:supermemory:chunks:session-supermemory:")
            .await
            .expect("chunks");
        let atomic = db
            .list_knowledge_by_prefix("memory:supermemory:atomic:session-supermemory:")
            .await
            .expect("atomic");
        let relations = db
            .list_knowledge_by_prefix("memory:supermemory:relations:session-supermemory:")
            .await
            .expect("relations");
        let documents = db
            .list_knowledge_by_prefix("memory:supermemory:documents:session-supermemory:")
            .await
            .expect("documents");
        let profile = db
            .list_knowledge_by_prefix("memory:supermemory:profile:session-supermemory:")
            .await
            .expect("profile");
        let context_records = db
            .list_knowledge_by_prefix("memory:supermemory:context:session-supermemory:")
            .await
            .expect("context");

        assert!(!queue.is_empty());
        assert!(!chunks.is_empty());
        assert!(!atomic.is_empty());
        assert!(!relations.is_empty());
        assert!(!documents.is_empty());
        assert!(!profile.is_empty());
        assert!(!context_records.is_empty());
        assert!(queue.iter().any(|record| record.key.ends_with(":latest")));
        assert!(
            documents
                .iter()
                .any(|record| record.key.ends_with(":latest"))
        );
        assert!(chunks.iter().any(|record| record.key.ends_with(":latest")));
        assert!(atomic.iter().any(|record| record.key.ends_with(":latest")));
        assert!(
            relations
                .iter()
                .any(|record| record.key.ends_with(":latest"))
        );
        assert!(profile.iter().any(|record| record.key.ends_with(":latest")));
        assert!(
            context_records
                .iter()
                .any(|record| record.key.ends_with(":latest"))
        );

        let searched = kernel
            .hybrid_search(&db, "session-supermemory", "replay policy", 3)
            .await
            .expect("search");
        assert!(!searched.is_empty());
        assert!(searched.iter().all(|hit| !hit.chunk_text.is_empty()));
        assert!(
            searched
                .iter()
                .all(|hit| hit.reason.contains("memory-first")
                    && hit.reason.contains("source-chunk-inject"))
        );
    }

    #[test]
    fn chunking_is_stable_for_same_input() {
        let kernel = SupermemoryKernel {
            chunk_char_target: 72,
            chunk_overlap_chars: 12,
            embedding_dimensions: 16,
        };
        let job = IngestionJob {
            job_id: "job:stable".into(),
            session_id: "session-stable".into(),
            tenant_id: "tenant-stable".into(),
            source: "unit-test".into(),
            document_id: "doc:stable".into(),
            metadata: BTreeMap::new(),
            document_date: None,
            event_date: None,
            queued_at_ms: 1,
            status: ProcessingStatus::Queued,
        };
        let extracted = kernel.extract_content(
            &job,
            "Policy memory replay chain keeps evidence linked. Another sentence adds context. Third sentence verifies stability.",
        );

        let first = kernel.chunk_semantic(&extracted);
        let second = kernel.chunk_semantic(&extracted);

        let first_signature = first
            .iter()
            .map(|chunk| {
                (
                    chunk.sequence,
                    chunk.text.clone(),
                    chunk.start_offset,
                    chunk.end_offset,
                )
            })
            .collect::<Vec<_>>();
        let second_signature = second
            .iter()
            .map(|chunk| {
                (
                    chunk.sequence,
                    chunk.text.clone(),
                    chunk.start_offset,
                    chunk.end_offset,
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(first_signature, second_signature);
    }

    #[test]
    fn relationship_inference_covers_updates_extends_derives_and_is_latest() {
        let kernel = SupermemoryKernel::default();
        let memories = vec![
            AtomicMemory {
                memory_id: "m1".into(),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: "c1".into(),
                statement: "alpha beta gamma stable".into(),
                confidence: 0.8,
                is_static: false,
                source_ref: "src".into(),
                tags: vec!["alpha".into()],
                created_at_ms: 1,
            },
            AtomicMemory {
                memory_id: "m2".into(),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: "c2".into(),
                statement: "alpha beta delta details".into(),
                confidence: 0.8,
                is_static: false,
                source_ref: "src".into(),
                tags: vec!["beta".into()],
                created_at_ms: 2,
            },
            AtomicMemory {
                memory_id: "m3".into(),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: "c3".into(),
                statement: "epsilon zeta context".into(),
                confidence: 0.7,
                is_static: false,
                source_ref: "src".into(),
                tags: vec!["epsilon".into()],
                created_at_ms: 3,
            },
            AtomicMemory {
                memory_id: "m4".into(),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: "c4".into(),
                statement: "epsilon zeta context latest".into(),
                confidence: 0.9,
                is_static: false,
                source_ref: "src".into(),
                tags: vec!["zeta".into()],
                created_at_ms: 4,
            },
        ];

        let relations = kernel.build_relationships("s", "j", &memories);

        assert!(
            relations
                .iter()
                .any(|relation| relation.relation_type == MemoryRelationType::Updates)
        );
        assert!(
            relations
                .iter()
                .any(|relation| relation.relation_type == MemoryRelationType::Extends)
        );
        assert!(
            relations
                .iter()
                .any(|relation| relation.relation_type == MemoryRelationType::Derives)
        );
        assert!(
            relations
                .iter()
                .any(|relation| relation.relation_type == MemoryRelationType::IsLatest)
        );
        assert_eq!(
            relations
                .iter()
                .filter(|relation| relation.relation_type == MemoryRelationType::IsLatest)
                .count(),
            3
        );
    }

    #[test]
    fn hybrid_search_orders_by_memory_first_then_chunk_inject() {
        let kernel = SupermemoryKernel::default();
        let temporal = TemporalGrounding {
            session_id: "s".into(),
            job_id: "j".into(),
            document_id: "d".into(),
            document_date: Some("2026-03-24".into()),
            event_date: Some("2026-03-24".into()),
            ingested_at_ms: 1,
            last_updated_at_ms: 1,
            freshness_score: 0.9,
        };
        let memories = vec![
            AtomicMemory {
                memory_id: "memory-relevant".into(),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: "chunk-relevant".into(),
                statement: "policy memory replay enforcement".into(),
                confidence: 0.95,
                is_static: false,
                source_ref: "src:relevant".into(),
                tags: vec!["policy".into(), "memory".into(), "replay".into()],
                created_at_ms: 1,
            },
            AtomicMemory {
                memory_id: "memory-weak".into(),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: "chunk-weak".into(),
                statement: "weather forecast tomorrow".into(),
                confidence: 0.3,
                is_static: false,
                source_ref: "src:weak".into(),
                tags: vec!["weather".into()],
                created_at_ms: 1,
            },
        ];
        let chunks = vec![
            SemanticChunk {
                chunk_id: "chunk-relevant".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                sequence: 0,
                text: "policy replay memory source evidence".into(),
                token_estimate: 12,
                start_offset: 0,
                end_offset: 34,
                metadata: BTreeMap::new(),
            },
            SemanticChunk {
                chunk_id: "chunk-weak".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                sequence: 1,
                text: "sunny sky".into(),
                token_estimate: 4,
                start_offset: 35,
                end_offset: 43,
                metadata: BTreeMap::new(),
            },
        ];

        let hits =
            kernel.hybrid_search_from_records("policy replay", 2, &memories, &chunks, &temporal);

        assert_eq!(
            hits.first().map(|hit| hit.memory_id.as_str()),
            Some("memory-relevant")
        );
        assert!(
            hits.first()
                .map(|hit| hit.reason.contains("memory-first")
                    && hit.reason.contains("source-chunk-inject"))
                .unwrap_or(false)
        );
        assert!(hits[0].score >= hits[1].score);
    }

    #[test]
    fn profile_generation_truncates_static_and_dynamic_memories() {
        let kernel = SupermemoryKernel::default();
        let mut memories = Vec::new();
        for index in 0..15 {
            memories.push(AtomicMemory {
                memory_id: format!("static-{index}"),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: format!("c-static-{index}"),
                statement: format!("static fact {index}"),
                confidence: 0.8,
                is_static: true,
                source_ref: "src:static".into(),
                tags: vec!["profile".into()],
                created_at_ms: index as u64,
            });
        }
        for index in 0..20 {
            memories.push(AtomicMemory {
                memory_id: format!("dynamic-{index}"),
                session_id: "s".into(),
                job_id: "j".into(),
                document_id: "d".into(),
                chunk_id: format!("c-dynamic-{index}"),
                statement: format!("dynamic context {index}"),
                confidence: 0.6,
                is_static: false,
                source_ref: "src:dynamic".into(),
                tags: vec!["dynamic".into()],
                created_at_ms: (100 + index) as u64,
            });
        }

        let profile = kernel.build_user_profile("session-profile", &memories);

        assert_eq!(profile.memory_count, 35);
        assert_eq!(profile.static_facts.len(), 12);
        assert_eq!(profile.dynamic_context.len(), 16);
    }
    #[test]
    fn scope_contract_binds_run_and_actor_filter() {
        let kernel = SupermemoryKernel::default();
        let metadata = BTreeMap::from([
            ("actor_filter".to_string(), "planner-agent".to_string()),
            ("tags".to_string(), "memory,scope".to_string()),
        ]);

        let scope = kernel
            .build_scope_contract("session-scope", &metadata, &BTreeMap::new())
            .expect("scope contract");

        assert_eq!(scope.scope.run_id.as_deref(), Some("session-scope"));
        assert_eq!(
            scope.query_filters.get("actor_id").map(String::as_str),
            Some("planner-agent")
        );
        assert!(!scope.metadata_template.contains_key("actor_id"));
        assert!(!scope.metadata_template.contains_key("actor_filter"));
        assert_eq!(
            scope
                .metadata_template
                .get("scope_contract")
                .map(String::as_str),
            Some("user_agent_run_actor_v1")
        );
    }

    #[test]
    fn memory_write_decider_emits_add_update_delete_none() {
        let kernel = SupermemoryKernel::default();
        let metadata = BTreeMap::from([("run_id".to_string(), "run-decision".to_string())]);
        let scope = kernel
            .build_scope_contract("session-decision", &metadata, &BTreeMap::new())
            .expect("scope contract");

        let job = IngestionJob {
            job_id: "job:decision".to_string(),
            session_id: "session-decision".to_string(),
            tenant_id: "tenant-decision".to_string(),
            source: "unit-test".to_string(),
            document_id: "doc:decision".to_string(),
            metadata: metadata.clone(),
            document_date: None,
            event_date: None,
            queued_at_ms: 1,
            status: ProcessingStatus::Queued,
        };

        let existing = vec![
            AtomicMemory {
                memory_id: "memory-existing-profile".to_string(),
                session_id: "session-decision".to_string(),
                job_id: "job:old".to_string(),
                document_id: "doc:old".to_string(),
                chunk_id: "chunk:old:0".to_string(),
                statement: "User likes chess".to_string(),
                confidence: 0.8,
                is_static: false,
                source_ref: "memory:supermemory:documents:session-decision:job:old".to_string(),
                tags: vec!["chess".to_string()],
                created_at_ms: 1,
            },
            AtomicMemory {
                memory_id: "memory-existing-pizza".to_string(),
                session_id: "session-decision".to_string(),
                job_id: "job:old".to_string(),
                document_id: "doc:old".to_string(),
                chunk_id: "chunk:old:1".to_string(),
                statement: "User likes cheese pizza".to_string(),
                confidence: 0.8,
                is_static: false,
                source_ref: "memory:supermemory:documents:session-decision:job:old".to_string(),
                tags: vec!["pizza".to_string()],
                created_at_ms: 2,
            },
        ];

        let candidates = vec![
            AtomicMemory {
                memory_id: "memory:new:0".to_string(),
                session_id: "session-decision".to_string(),
                job_id: "job:decision".to_string(),
                document_id: "doc:decision".to_string(),
                chunk_id: "chunk:new:0".to_string(),
                statement: "User likes chess".to_string(),
                confidence: 0.7,
                is_static: false,
                source_ref: "memory:supermemory:documents:session-decision:job:decision"
                    .to_string(),
                tags: vec!["chess".to_string()],
                created_at_ms: 10,
            },
            AtomicMemory {
                memory_id: "memory:new:1".to_string(),
                session_id: "session-decision".to_string(),
                job_id: "job:decision".to_string(),
                document_id: "doc:decision".to_string(),
                chunk_id: "chunk:new:1".to_string(),
                statement: "User likes chess and strategy tournaments".to_string(),
                confidence: 0.9,
                is_static: false,
                source_ref: "memory:supermemory:documents:session-decision:job:decision"
                    .to_string(),
                tags: vec!["chess".to_string(), "strategy".to_string()],
                created_at_ms: 11,
            },
            AtomicMemory {
                memory_id: "memory:new:2".to_string(),
                session_id: "session-decision".to_string(),
                job_id: "job:decision".to_string(),
                document_id: "doc:decision".to_string(),
                chunk_id: "chunk:new:2".to_string(),
                statement: "User dislikes cheese pizza".to_string(),
                confidence: 0.85,
                is_static: false,
                source_ref: "memory:supermemory:documents:session-decision:job:decision"
                    .to_string(),
                tags: vec!["pizza".to_string()],
                created_at_ms: 12,
            },
            AtomicMemory {
                memory_id: "memory:new:3".to_string(),
                session_id: "session-decision".to_string(),
                job_id: "job:decision".to_string(),
                document_id: "doc:decision".to_string(),
                chunk_id: "chunk:new:3".to_string(),
                statement: "User builds Rust systems".to_string(),
                confidence: 0.88,
                is_static: false,
                source_ref: "memory:supermemory:documents:session-decision:job:decision"
                    .to_string(),
                tags: vec!["rust".to_string()],
                created_at_ms: 13,
            },
        ];

        let (to_persist, active_memories, decisions) =
            kernel.reconcile_memory_writes(&job, &scope, &existing, &candidates);

        let actions = decisions
            .iter()
            .map(|decision| decision.action.clone())
            .collect::<Vec<_>>();
        assert!(actions.contains(&MemoryWriteAction::None));
        assert!(actions.contains(&MemoryWriteAction::Update));
        assert!(actions.contains(&MemoryWriteAction::Delete));
        assert!(actions.contains(&MemoryWriteAction::Add));

        assert!(
            to_persist
                .iter()
                .any(|memory| memory.memory_id == "memory-existing-profile")
        );
        assert!(
            to_persist
                .iter()
                .any(|memory| memory.memory_id == "memory:new:3")
        );
        assert!(
            !active_memories
                .iter()
                .any(|memory| memory.memory_id == "memory-existing-pizza")
        );
        assert!(
            active_memories
                .iter()
                .any(|memory| memory.statement.contains("strategy tournaments"))
        );
    }
    #[tokio::test]
    async fn metadata_filter_dsl_supports_and_or_not_and_comparators() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = SupermemoryKernel::default();
        let session_id = "session-dsl-filter";
        kernel
            .run_pipeline(
                &db,
                session_id,
                "tenant-dsl",
                "dsl-test",
                "Policy replay evidence is maintained by planner agent with strict trust gate.",
                BTreeMap::from([
                    ("tags".to_string(), "policy,replay,trust".to_string()),
                    ("actor_filter".to_string(), "planner-agent".to_string()),
                    ("role".to_string(), "planner".to_string()),
                ]),
                Some("2026-04-05".to_string()),
                Some("2026-04-05".to_string()),
                "policy replay trust",
                5,
            )
            .await
            .expect("seed pipeline");

        let pass_filters = BTreeMap::from([(
            "metadata_filter_dsl".to_string(),
            serde_json::json!({
                "AND": [
                    {"confidence": {"gte": 0.6}},
                    {"tags": {"contains": "policy"}}
                ],
                "OR": [
                    {"statement": {"icontains": "replay"}},
                    {"actor_id": {"eq": "planner-agent"}}
                ],
                "NOT": [
                    {"source_ref": {"icontains": "forbidden"}}
                ]
            })
            .to_string(),
        )]);

        let pass_hits = kernel
            .hybrid_search_scoped(
                &db,
                session_id,
                "policy replay",
                5,
                &BTreeMap::new(),
                &pass_filters,
            )
            .await
            .expect("pass filter search");
        assert!(
            !pass_hits.is_empty(),
            "expected DSL pass filter to return hits"
        );

        let fail_filters = BTreeMap::from([(
            "metadata_filter_dsl".to_string(),
            serde_json::json!({
                "AND": [
                    {"actor_id": {"eq": "other-agent"}}
                ]
            })
            .to_string(),
        )]);

        let fail_hits = kernel
            .hybrid_search_scoped(
                &db,
                session_id,
                "policy replay",
                5,
                &BTreeMap::new(),
                &fail_filters,
            )
            .await
            .expect("fail filter search");
        assert!(
            fail_hits.is_empty(),
            "expected DSL fail filter to return no hits"
        );
    }

    #[tokio::test]
    async fn memory_history_subledger_persists_old_new_event_actor_role() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = SupermemoryKernel::default();
        let session_id = "session-history-ledger";
        kernel
            .run_pipeline(
                &db,
                session_id,
                "tenant-history",
                "history-test",
                "User likes cheese pizza and trusts replayable audit evidence.",
                BTreeMap::from([
                    ("tags".to_string(), "pizza,preference,audit".to_string()),
                    ("actor_filter".to_string(), "planner-agent".to_string()),
                    ("role".to_string(), "planner".to_string()),
                ]),
                None,
                None,
                "pizza audit",
                5,
            )
            .await
            .expect("history pipeline");

        let history_records = db
            .list_knowledge_by_prefix(&format!("memory:supermemory:history:{session_id}:"))
            .await
            .expect("history records");
        assert!(
            !history_records.is_empty(),
            "history sub-ledger should not be empty"
        );

        let parsed_history = history_records
            .iter()
            .find_map(|record| serde_json::from_str::<MemoryHistoryRecord>(&record.value).ok())
            .expect("parse history record");
        assert_eq!(parsed_history.actor_id.as_deref(), Some("planner-agent"));
        assert_eq!(parsed_history.role.as_deref(), Some("planner"));
        assert!(
            ["ADD", "UPDATE", "DELETE", "NONE"].contains(&parsed_history.event.as_str()),
            "unexpected event {}",
            parsed_history.event
        );

        let evidence_memory = db
            .list_knowledge_by_prefix(&format!("evidence:memory:{session_id}:"))
            .await
            .expect("evidence memory sub-ledger");
        assert!(
            !evidence_memory.is_empty(),
            "evidence memory sub-ledger should have append events"
        );
    }
    #[tokio::test]
    async fn relation_soft_delete_marks_valid_false_instead_of_physical_delete() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = SupermemoryKernel::default();
        let session_id = "session-relation-soft-delete";
        let relation = MemoryRelation {
            relation_id: "rel:legacy:isLatest:m-old->m-new".into(),
            session_id: session_id.into(),
            job_id: "job:legacy".into(),
            from_memory_id: "m-old".into(),
            to_memory_id: "m-new".into(),
            relation_type: MemoryRelationType::IsLatest,
            weight: 1.0,
            is_latest: true,
            valid: true,
            rationale: "latest relation before soft delete".into(),
            invalidated_at_ms: None,
            invalidation_reason: None,
            created_at_ms: current_time_ms(),
        };

        kernel
            .persist_record_by_key(
                &db,
                &relation,
                "supermemory-kernel",
                format!(
                    "memory:supermemory:relations:{session_id}:{}:latest",
                    relation.relation_id
                ),
            )
            .await
            .expect("seed latest relation");

        let job = IngestionJob {
            job_id: "job:softdelete".into(),
            session_id: session_id.into(),
            tenant_id: "tenant-softdelete".into(),
            source: "unit-test".into(),
            document_id: "doc:softdelete".into(),
            metadata: BTreeMap::new(),
            document_date: None,
            event_date: None,
            queued_at_ms: current_time_ms(),
            status: ProcessingStatus::Done,
        };

        let decisions = vec![MemoryWriteDecision {
            decision_id: "decision:softdelete:1".into(),
            session_id: session_id.into(),
            job_id: job.job_id.clone(),
            candidate_text: "replace old memory".into(),
            action: MemoryWriteAction::Delete,
            target_memory_id: Some("m-old".into()),
            resulting_memory_id: None,
            reason: "contradictory statement detected".into(),
            old_memory: Some("old".into()),
            new_memory: None,
            created_at_ms: current_time_ms(),
        }];

        kernel
            .persist_relation_soft_deletes(&db, &job, &decisions)
            .await
            .expect("soft delete relations");

        let latest = db
            .get_knowledge(&format!(
                "memory:supermemory:relations:{session_id}:{}:latest",
                relation.relation_id
            ))
            .await
            .expect("read latest")
            .expect("latest exists");
        let latest_relation: MemoryRelation =
            serde_json::from_str(&latest.value).expect("parse latest relation");
        assert!(!latest_relation.valid);
        assert!(!latest_relation.is_latest);
        assert!(latest_relation.invalidated_at_ms.is_some());
        assert!(
            latest_relation
                .invalidation_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("soft-delete:DELETE:decision:softdelete:1"))
        );

        let relation_records = db
            .list_knowledge_by_prefix(&format!("memory:supermemory:relations:{session_id}:"))
            .await
            .expect("relation records");
        assert!(
            relation_records
                .iter()
                .any(|record| record.key.contains(":softdelete:")),
            "expected append-only softdelete relation record"
        );
    }

    #[tokio::test]
    async fn queue_worker_processes_existing_job_without_requeue() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = SupermemoryKernel::default();
        let job = kernel
            .queue_ingestion(
                &db,
                "session-worker",
                "tenant-worker",
                "worker-source",
                BTreeMap::new(),
                None,
                None,
            )
            .await
            .expect("queue job");
        kernel
            .persist_ingestion_payload(&db, &job, "worker payload text for queue processing")
            .await
            .expect("persist payload");

        let context = kernel
            .run_queue_worker_once(&db, "session-worker", "worker payload", 3)
            .await
            .expect("run worker")
            .expect("context");
        assert!(!context.hits.is_empty());

        let queue_records = db
            .list_knowledge_by_prefix("memory:supermemory:queue:session-worker:")
            .await
            .expect("queue records");
        let queued_jobs = queue_records
            .iter()
            .filter_map(|record| serde_json::from_str::<IngestionJob>(&record.value).ok())
            .filter(|candidate| candidate.status == ProcessingStatus::Queued)
            .collect::<Vec<_>>();
        assert!(
            queued_jobs.is_empty(),
            "processed queue job should not be re-queued"
        );

        let done_jobs = queue_records
            .iter()
            .filter_map(|record| serde_json::from_str::<IngestionJob>(&record.value).ok())
            .filter(|candidate| candidate.status == ProcessingStatus::Done)
            .collect::<Vec<_>>();
        assert!(
            done_jobs
                .iter()
                .any(|candidate| candidate.job_id == job.job_id)
        );
    }
}

