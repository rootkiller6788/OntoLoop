pub mod learning;
mod sidecar;
pub mod supermemory;

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use anyhow::{Result, bail};
use autoloop_state_adapter::{
    CausalEdgeRecord, LearningEventKind as StoredLearningEventKind, LearningSessionRecord,
    ReflexionEpisodeRecord, SkillLibraryRecord, StateStore, WitnessLogRecord,
};
use serde::{Deserialize, Serialize};

use crate::{
    config::{LearningConfig, MemoryConfig},
    providers::ChatMessage,
};

use crate::tools::{ForgedMcpToolManifest, ToolRegistry};
pub use learning::{
    EmbeddingProvider, HashEmbeddingProvider, JointRoutingEvidence, LearningAssetKind,
    LearningDocument, LearningEvent, LearningEventKind, LearningFilter, LearningRepository,
    LearningScorer, RetrievalEvidence, WeightedLearningScorer, document_from_causal_edge,
    document_from_episode, document_from_forged_tool_manifest, document_from_learning_session,
    document_from_skill, document_from_witness,
};
use sidecar::{SidecarMemoryIndex, SidecarQuery};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTarget {
    Identity,
    LongTerm,
    History,
}

#[derive(Debug, Clone)]
pub struct MemorySnippet {
    pub source: MemoryTarget,
    pub label: String,
    pub content: String,
    pub tags: Vec<String>,
    pub priority: usize,
}

#[derive(Debug, Clone)]
pub struct MemoryContextRequest<'a> {
    pub user_input: &'a str,
    pub session_history: &'a [ChatMessage],
    pub max_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflexionEpisode {
    pub proposal_id: String,
    pub hypothesis: String,
    pub outcome: String,
    pub lesson: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRecord {
    pub name: String,
    pub trigger: String,
    pub procedure: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalEdge {
    pub cause: String,
    pub effect: String,
    pub evidence: String,
    pub strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WitnessLog {
    pub source: String,
    pub observation: String,
    pub metric_name: String,
    pub metric_value: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailurePatternCluster {
    pub cluster_id: String,
    pub pattern: String,
    pub frequency: usize,
    pub representative_examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalValidationSummary {
    pub validated_edges: usize,
    pub average_confidence: f32,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityImprovementProposal {
    pub tool_name: String,
    pub change_hint: String,
    pub rationale: String,
    pub priority: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningConsolidation {
    pub consolidated_skills: Vec<SkillRecord>,
    pub failure_clusters: Vec<FailurePatternCluster>,
    pub causal_validation: CausalValidationSummary,
    pub capability_improvements: Vec<CapabilityImprovementProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningProposal {
    pub proposal_id: String,
    pub session_id: String,
    pub anchor: String,
    pub hypothesis: String,
    pub reason: String,
    pub proposed_skill_name: String,
    pub proposed_confidence: f32,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePack {
    pub proposal_id: String,
    pub witness_ids: Vec<String>,
    pub episode_ids: Vec<String>,
    pub quality_score: f32,
    pub bias_flags: Vec<String>,
    pub counter_evidence_count: usize,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSignal {
    pub signal_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub source: String,
    pub evidence_ref: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LearningSignalRejectRecord {
    pub reject_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub source: String,
    pub target: String,
    pub reason: String,
    pub evidence_ref: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningGateVerdict {
    pub approved: bool,
    pub reason: String,
    pub canary_ratio: f32,
    pub rollback_window_ms: u64,
    pub risk_tags: Vec<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillPromotionStage {
    Canary,
    Promoted,
    Rejected,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPromotionRecord {
    pub promotion_id: String,
    pub proposal_id: String,
    pub session_id: String,
    pub skill_name: String,
    pub stage: SkillPromotionStage,
    pub reason: String,
    pub confidence: f32,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StrategyMemoryTier {
    HotSession,
    WarmValidated,
    ColdArchive,
}

#[derive(Debug, Clone)]
pub struct MemorySubsystem {
    targets: Vec<MemoryTarget>,
    sidecar: Option<Arc<SidecarMemoryIndex>>,
    default_top_k: usize,
    supermemory: supermemory::SupermemoryKernel,
}

impl MemorySubsystem {
    pub fn from_config(config: &MemoryConfig, learning: &LearningConfig) -> Self {
        let mut targets = Vec::new();
        if config.load_identity {
            targets.push(MemoryTarget::Identity);
        }
        if config.load_memory_md {
            targets.push(MemoryTarget::LongTerm);
        }
        if config.load_history_md {
            targets.push(MemoryTarget::History);
        }
        let sidecar = if learning.enabled && learning.sidecar_enabled {
            Some(Arc::new(SidecarMemoryIndex::new(
                learning.clone(),
                Arc::new(HashEmbeddingProvider::new(learning.embedding_dimensions)),
            )))
        } else {
            None
        };
        Self {
            targets,
            sidecar,
            default_top_k: learning.top_k.max(1),
            supermemory: supermemory::SupermemoryKernel::default(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.targets.is_empty() {
            bail!("at least one memory target should be enabled");
        }
        Ok(())
    }

    pub fn load_targets(&self) -> &[MemoryTarget] {
        &self.targets
    }

    pub fn supermemory_kernel(&self) -> &supermemory::SupermemoryKernel {
        &self.supermemory
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_supermemory_pipeline(
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
    ) -> Result<supermemory::ContextAssembly> {
        self.supermemory
            .run_pipeline(
                db,
                session_id,
                tenant_id,
                source,
                content,
                metadata,
                document_date,
                event_date,
                query,
                self.default_top_k.max(1),
            )
            .await
    }

    pub async fn run_supermemory_queue_worker_once(
        &self,
        db: &StateStore,
        session_id: &str,
        query: &str,
    ) -> Result<Option<supermemory::ContextAssembly>> {
        self.supermemory
            .run_queue_worker_once(db, session_id, query, self.default_top_k.max(1))
            .await
    }

    pub async fn retrieve_supermemory_hybrid_hits(
        &self,
        db: &StateStore,
        session_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<supermemory::HybridSearchHit>> {
        self.supermemory
            .hybrid_search(db, session_id, query, top_k.max(1))
            .await
    }

    pub fn build_memory_context(&self) -> String {
        self.build_memory_context_for("", &[])
    }

    pub fn build_memory_context_for(
        &self,
        user_input: &str,
        session_history: &[ChatMessage],
    ) -> String {
        let request = MemoryContextRequest {
            user_input,
            session_history,
            max_items: 5,
        };

        let mut selected = self.select_relevant_memories(&request);
        if selected.is_empty() {
            return "- No memory signals selected.".into();
        }

        selected.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.label.cmp(&right.label))
        });

        selected
            .into_iter()
            .map(|snippet| format!("- [{}] {}", snippet.label, snippet.content))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub async fn build_memory_context_with_learning(
        &self,
        db: &StateStore,
        session_id: &str,
        user_input: &str,
        session_history: &[ChatMessage],
    ) -> Result<String> {
        let base = self.build_memory_context_for(user_input, session_history);
        let retrieved = self
            .retrieve_learning_evidence(db, session_id, user_input, self.default_top_k)
            .await?;
        if retrieved.is_empty() {
            return Ok(base);
        }

        let learning_lines = retrieved
            .into_iter()
            .map(|item| {
                format!(
                    "- [LEARNING/{:?}/{:.2}] {}",
                    item.document.asset_kind, item.similarity, item.document.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(format!("{base}\n{learning_lines}"))
    }

    pub fn select_relevant_memories(
        &self,
        request: &MemoryContextRequest<'_>,
    ) -> Vec<MemorySnippet> {
        let query_terms = tokenize(request.user_input);
        let mut snippets = self.memory_candidates(request.session_history);
        let mut dedup = HashSet::new();

        snippets.retain(|snippet| dedup.insert(normalize(&snippet.content)));
        snippets.sort_by(|left, right| {
            score_snippet(right, &query_terms)
                .cmp(&score_snippet(left, &query_terms))
                .then_with(|| right.priority.cmp(&left.priority))
        });

        snippets
            .into_iter()
            .filter(|snippet| score_snippet(snippet, &query_terms) > 0)
            .take(request.max_items)
            .collect()
    }

    fn memory_candidates(&self, session_history: &[ChatMessage]) -> Vec<MemorySnippet> {
        let mut snippets = Vec::new();

        for target in &self.targets {
            match target {
                MemoryTarget::Identity => snippets.push(MemorySnippet {
                    source: MemoryTarget::Identity,
                    label: "IDENTITY".into(),
                    content: "Protect operator intent, maintain safe autonomy, and prefer verifiable actions over speculation.".into(),
                    tags: vec!["identity".into(), "safety".into(), "operator".into(), "autonomy".into()],
                    priority: 10,
                }),
                MemoryTarget::LongTerm => snippets.extend([
                    MemorySnippet {
                        source: MemoryTarget::LongTerm,
                        label: "MEMORY".into(),
                        content: "AutoLoop uses Rust-first runtime boundaries, StateStore state, and MCP-compatible provider/tool adapters.".into(),
                        tags: vec!["autoloop".into(), "rust".into(), "state_store".into(), "mcp".into()],
                        priority: 9,
                    },
                    MemorySnippet {
                        source: MemoryTarget::LongTerm,
                        label: "MEMORY".into(),
                        content: "Preserve durable facts such as anchors, permissions, and prior architecture decisions; skip repetitive summaries.".into(),
                        tags: vec!["anchor".into(), "knowledge".into(), "permission".into(), "history".into()],
                        priority: 8,
                    },
                ]),
                MemoryTarget::History => snippets.extend(
                    session_history
                        .iter()
                        .rev()
                        .take(6)
                        .filter(|message| !message.content.trim().is_empty())
                        .map(|message| MemorySnippet {
                            source: MemoryTarget::History,
                            label: format!("HISTORY/{}", message.role.to_uppercase()),
                            content: compress_message(&message.content),
                            tags: tokenize(&message.content).into_iter().collect(),
                            priority: if message.role == "user" { 7 } else { 5 },
                        }),
                ),
            }
        }

        snippets
    }

    pub async fn refresh_learning_sidecar(&self, db: &StateStore, session_id: &str) -> Result<()> {
        let Some(sidecar) = &self.sidecar else {
            return Ok(());
        };

        for episode in db.list_reflexion_episodes(session_id).await? {
            sidecar.upsert(document_from_episode(&episode)).await?;
        }
        for skill in db.list_skill_library_records(session_id).await? {
            sidecar.upsert(document_from_skill(&skill)).await?;
        }
        for edge in db.list_causal_edge_records(session_id).await? {
            sidecar.upsert(document_from_causal_edge(&edge)).await?;
        }
        for session in db.list_learning_session_records(session_id).await? {
            sidecar
                .upsert(document_from_learning_session(&session))
                .await?;
        }
        for witness in db.list_witness_log_records(session_id).await? {
            sidecar.upsert(document_from_witness(&witness)).await?;
        }
        for record in db
            .list_knowledge_by_prefix(ToolRegistry::FORGED_TOOL_PREFIX)
            .await?
        {
            if let Ok(manifest) = serde_json::from_str::<ForgedMcpToolManifest>(&record.value) {
                sidecar
                    .upsert(document_from_forged_tool_manifest(&record, &manifest))
                    .await?;
            }
        }

        sidecar.rebuild()?;
        Ok(())
    }

    pub async fn retrieve_learning_evidence(
        &self,
        db: &StateStore,
        session_id: &str,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<RetrievalEvidence>> {
        let Some(sidecar) = &self.sidecar else {
            return Ok(Vec::new());
        };
        self.refresh_learning_sidecar(db, session_id).await?;
        sidecar
            .search(SidecarQuery {
                query: query.to_string(),
                filter: LearningFilter {
                    session_id: Some(session_id.to_string()),
                    asset_kinds: Vec::new(),
                    metadata: Default::default(),
                },
                top_k,
            })
            .await
    }

    pub async fn persist_learning_event(
        &self,
        db: &StateStore,
        event: &LearningEvent,
    ) -> Result<WitnessLogRecord> {
        let record = WitnessLogRecord {
            id: format!(
                "{}:{}:{}",
                event.session_id,
                normalize(&event.source),
                current_time_ms()
            ),
            session_id: event.session_id.clone(),
            event_type: map_event_kind(event.event_kind),
            source: event.source.clone(),
            detail: event.summary.clone(),
            score: event.score,
            created_at_ms: current_time_ms(),
            metadata_json: serde_json::to_string(event)?,
        };
        db.append_witness_log_record(record).await
    }

    pub async fn persist_reflexion_episode(
        &self,
        db: &StateStore,
        session_id: &str,
        episode: &ReflexionEpisode,
    ) -> Result<()> {
        let record = ReflexionEpisodeRecord {
            id: format!("{session_id}:{}", episode.proposal_id),
            session_id: session_id.to_string(),
            objective: episode.hypothesis.clone(),
            hypothesis: episode.hypothesis.clone(),
            outcome: episode.outcome.clone(),
            lesson: episode.lesson.clone(),
            status: if episode.outcome.to_ascii_lowercase().contains("ready") {
                "success".into()
            } else {
                "failure".into()
            },
            score: if episode.outcome.to_ascii_lowercase().contains("ready") {
                1.0
            } else {
                0.0
            },
            created_at_ms: current_time_ms(),
        };
        db.upsert_reflexion_episode(record).await?;
        Ok(())
    }

    pub async fn persist_skill(
        &self,
        db: &StateStore,
        session_id: &str,
        skill: &SkillRecord,
        signal: &LearningSignal,
    ) -> Result<()> {
        self
            .validate_learning_signal(db, signal, session_id, "skill_registry.persist_skill")
            .await?;
        let record = SkillLibraryRecord {
            id: format!("{session_id}:{}", normalize(&skill.name)),
            session_id: session_id.to_string(),
            name: skill.name.clone(),
            trigger: skill.trigger.clone(),
            procedure: skill.procedure.clone(),
            confidence: skill.confidence,
            success_rate: skill.confidence,
            evidence_count: 1,
            created_at_ms: current_time_ms(),
            updated_at_ms: current_time_ms(),
        };
        db.upsert_skill_library_record(record).await?;
        Ok(())
    }

    pub async fn persist_causal_edge(
        &self,
        db: &StateStore,
        session_id: &str,
        edge: &CausalEdge,
    ) -> Result<()> {
        let record = CausalEdgeRecord {
            id: format!(
                "{session_id}:{}:{}",
                normalize(&edge.cause),
                normalize(&edge.effect)
            ),
            session_id: session_id.to_string(),
            cause: edge.cause.clone(),
            effect: edge.effect.clone(),
            evidence: edge.evidence.clone(),
            strength: edge.strength,
            confidence: edge.strength,
            created_at_ms: current_time_ms(),
        };
        db.upsert_causal_edge_record(record).await?;
        Ok(())
    }

    pub async fn persist_witness_log(
        &self,
        db: &StateStore,
        session_id: &str,
        witness: &WitnessLog,
    ) -> Result<()> {
        let record = WitnessLogRecord {
            id: format!(
                "{session_id}:{}:{}:{}",
                normalize(&witness.source),
                normalize(&witness.metric_name),
                current_time_ms()
            ),
            session_id: session_id.to_string(),
            event_type: StoredLearningEventKind::Audit,
            source: witness.source.clone(),
            detail: witness.observation.clone(),
            score: witness.metric_value,
            created_at_ms: current_time_ms(),
            metadata_json: serde_json::to_string(witness)?,
        };
        db.append_witness_log_record(record).await?;
        Ok(())
    }

    pub async fn persist_learning_session(
        &self,
        db: &StateStore,
        session: LearningSessionRecord,
    ) -> Result<()> {
        db.upsert_learning_session_record(session).await?;
        Ok(())
    }

    pub async fn consolidate_learning(
        &self,
        db: &StateStore,
        session_id: &str,
    ) -> Result<LearningConsolidation> {
        let episodes = db.list_reflexion_episodes(session_id).await?;
        let skills = db.list_skill_library_records(session_id).await?;
        let edges = db.list_causal_edge_records(session_id).await?;
        let witness = db.list_witness_log_records(session_id).await?;

        let consolidated_skills = consolidate_skills(&skills, &episodes);
        let failure_clusters = cluster_failures(&episodes, &witness, session_id);
        let causal_validation = validate_causal_edges(&edges);
        let capability_improvements =
            derive_capability_improvements(&failure_clusters, &witness, db).await?;

        Ok(LearningConsolidation {
            consolidated_skills,
            failure_clusters,
            causal_validation,
            capability_improvements,
        })
    }

    pub fn draft_learning_proposal(
        &self,
        session_id: &str,
        anchor: &str,
        reason: &str,
        assistant_response: &str,
    ) -> LearningProposal {
        let now = current_time_ms();
        let anchor_key = normalize(anchor).replace(' ', "-");
        let proposal_id = format!("{session_id}:{anchor_key}:{now}");
        LearningProposal {
            proposal_id: proposal_id.clone(),
            session_id: session_id.to_string(),
            anchor: anchor.to_string(),
            hypothesis: format!(
                "If we improve '{}' using anchored evidence, response quality should increase without violating safety.",
                anchor
            ),
            reason: reason.to_string(),
            proposed_skill_name: format!("skill-{}", anchor_key),
            proposed_confidence: if assistant_response.to_ascii_lowercase().contains("not sure") {
                0.45
            } else {
                0.72
            },
            created_at_ms: now,
        }
    }

    pub async fn collect_evidence_pack(
        &self,
        db: &StateStore,
        session_id: &str,
        proposal: &LearningProposal,
    ) -> Result<EvidencePack> {
        let witness = db.list_witness_log_records(session_id).await?;
        let episodes = db.list_reflexion_episodes(session_id).await?;
        let proposal_anchor = proposal.anchor.to_ascii_lowercase();
        let related_witness = witness
            .iter()
            .filter(|record| {
                let lower = format!(
                    "{} {}",
                    record.detail.to_ascii_lowercase(),
                    record.metadata_json.to_ascii_lowercase()
                );
                lower.contains(&proposal_anchor)
            })
            .collect::<Vec<_>>();
        let related_episodes = episodes
            .iter()
            .filter(|record| {
                let lower = format!(
                    "{} {} {}",
                    record.objective.to_ascii_lowercase(),
                    record.hypothesis.to_ascii_lowercase(),
                    record.lesson.to_ascii_lowercase()
                );
                lower.contains(&proposal_anchor)
            })
            .collect::<Vec<_>>();
        let witness_ids = related_witness
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        let episode_ids = related_episodes
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>();
        let negative = related_witness
            .iter()
            .filter(|record| record.score < 0.0)
            .count();
        let positive = related_witness
            .iter()
            .filter(|record| record.score >= 0.0)
            .count();
        let bias_flags = related_witness
            .iter()
            .filter_map(|record| {
                let lower = record.detail.to_ascii_lowercase();
                if lower.contains("always")
                    || lower.contains("never")
                    || lower.contains("only")
                    || lower.contains("bias")
                {
                    Some(record.detail.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let quality_base = if positive + negative == 0 {
            0.35
        } else {
            positive as f32 / (positive + negative) as f32
        };
        let diversity_bonus = (episode_ids.len().min(4) as f32) * 0.08;
        let bias_penalty = (bias_flags.len().min(3) as f32) * 0.12;
        let quality_score = (quality_base + diversity_bonus - bias_penalty).clamp(0.0, 1.0);
        Ok(EvidencePack {
            proposal_id: proposal.proposal_id.clone(),
            witness_ids,
            episode_ids,
            quality_score,
            bias_flags: bias_flags.clone(),
            counter_evidence_count: negative,
            summary: format!(
                "evidence quality {:.2}, positives={}, negatives={}, bias_flags={}",
                quality_score,
                positive,
                negative,
                bias_flags.len()
            ),
        })
    }

    pub async fn persist_learning_proposal(
        &self,
        db: &StateStore,
        proposal: &LearningProposal,
        evidence: &EvidencePack,
        verdict: &LearningGateVerdict,
        signal: &LearningSignal,
    ) -> Result<()> {
        self
            .validate_learning_signal(
                db,
                signal,
                &proposal.session_id,
                "mutable_memory_ledger.persist_learning_proposal",
            )
            .await?;
        db.upsert_json_knowledge(
            format!(
                "memory:{}:learning-proposal:{}",
                proposal.session_id, proposal.proposal_id
            ),
            &serde_json::json!({
                "proposal": proposal,
                "evidence": evidence,
                "verdict": verdict,
                "learning_signal": signal,
            }),
            "learning-gate",
        )
        .await?;
        let tier = if verdict.approved {
            StrategyMemoryTier::WarmValidated
        } else {
            StrategyMemoryTier::HotSession
        };
        self.persist_strategy_memory_tier(
            db,
            &proposal.session_id,
            tier,
            &proposal.proposal_id,
            &serde_json::json!({
                "anchor": proposal.anchor,
                "reason": proposal.reason,
                "verdict": verdict.reason,
                "quality_score": evidence.quality_score,
                "learning_signal": signal,
            }),
            signal,
        )
        .await?;
        Ok(())
    }

    pub async fn promote_skill_with_verdict(
        &self,
        db: &StateStore,
        proposal: &LearningProposal,
        verdict: &LearningGateVerdict,
        candidate: &SkillRecord,
        signal: &LearningSignal,
    ) -> Result<SkillPromotionRecord> {
        self
            .validate_learning_signal(
                db,
                signal,
                &proposal.session_id,
                "skill_registry.promote_skill_with_verdict",
            )
            .await?;
        let now = current_time_ms();
        let stage = if verdict.approved {
            SkillPromotionStage::Canary
        } else {
            SkillPromotionStage::Rejected
        };
        let confidence = if verdict.approved {
            candidate.confidence.clamp(0.45, 0.82)
        } else {
            0.0
        };
        if verdict.approved {
            self.persist_skill(
                db,
                &proposal.session_id,
                &SkillRecord {
                    confidence,
                    ..candidate.clone()
                },
                signal,
            )
            .await?;
        }
        let record = SkillPromotionRecord {
            promotion_id: format!(
                "promotion:{}:{}:{}",
                proposal.session_id,
                normalize(&candidate.name),
                now
            ),
            proposal_id: proposal.proposal_id.clone(),
            session_id: proposal.session_id.clone(),
            skill_name: candidate.name.clone(),
            stage: stage.clone(),
            reason: verdict.reason.clone(),
            confidence,
            created_at_ms: now,
        };
        db.upsert_json_knowledge(
            format!(
                "memory:{}:skill-promotion:{}",
                proposal.session_id, record.promotion_id
            ),
            &record,
            "learning-promotion",
        )
        .await?;
        self.persist_strategy_memory_tier(
            db,
            &proposal.session_id,
            if stage == SkillPromotionStage::Canary {
                StrategyMemoryTier::WarmValidated
            } else {
                StrategyMemoryTier::ColdArchive
            },
            &record.promotion_id,
            &record,
            signal,
        )
        .await?;
        Ok(record)
    }

    pub async fn rollback_skill_promotion(
        &self,
        db: &StateStore,
        session_id: &str,
        promotion: &SkillPromotionRecord,
        reason: &str,
        signal: &LearningSignal,
    ) -> Result<SkillPromotionRecord> {
        self
            .validate_learning_signal(
                db,
                signal,
                session_id,
                "skill_registry.rollback_skill_promotion",
            )
            .await?;
        let mut rolled = promotion.clone();
        rolled.stage = SkillPromotionStage::RolledBack;
        rolled.reason = reason.to_string();
        rolled.confidence = 0.0;
        rolled.created_at_ms = current_time_ms();
        db.upsert_json_knowledge(
            format!(
                "memory:{session_id}:skill-promotion:{}",
                rolled.promotion_id
            ),
            &rolled,
            "learning-promotion",
        )
        .await?;
        self.persist_strategy_memory_tier(
            db,
            session_id,
            StrategyMemoryTier::ColdArchive,
            &rolled.promotion_id,
            &rolled,
            signal,
        )
        .await?;
        // Rollback must not keep a promoted skill as long-term active memory.
        self.persist_skill(
            db,
            session_id,
            &SkillRecord {
                name: rolled.skill_name.clone(),
                trigger: "rolled-back".into(),
                procedure: format!("rolled_back:{}", reason),
                confidence: 0.0,
            },
            signal,
        )
        .await?;
        Ok(rolled)
    }

    pub async fn persist_strategy_memory_tier<T: Serialize>(
        &self,
        db: &StateStore,
        session_id: &str,
        tier: StrategyMemoryTier,
        key: &str,
        payload: &T,
        signal: &LearningSignal,
    ) -> Result<()> {
        self
            .validate_learning_signal(
                db,
                signal,
                session_id,
                "mutable_memory_ledger.persist_strategy_memory_tier",
            )
            .await?;
        let tier_key = match tier {
            StrategyMemoryTier::HotSession => "hot",
            StrategyMemoryTier::WarmValidated => "warm",
            StrategyMemoryTier::ColdArchive => "cold",
        };
        db.upsert_json_knowledge(
            format!("memory:strategy:{tier_key}:{session_id}:{}", normalize(key)),
            &serde_json::json!({
                "payload": payload,
                "learning_signal": signal,
            }),
            "strategy-memory",
        )
        .await?;
        Ok(())
    }

    async fn validate_learning_signal(
        &self,
        db: &StateStore,
        signal: &LearningSignal,
        expected_session_id: &str,
        target: &str,
    ) -> Result<()> {
        if signal.session_id.trim() != expected_session_id.trim() {
            self
                .persist_learning_signal_reject(
                    db,
                    expected_session_id,
                    &signal.trace_id,
                    &signal.source,
                    target,
                    "learning_signal.session_id_mismatch",
                    Some(signal.evidence_ref.as_str()),
                )
                .await?;
            bail!(
                "learning signal rejected for `{target}`: session mismatch (expected={}, actual={})",
                expected_session_id,
                signal.session_id
            );
        }
        if signal.evidence_ref.trim().is_empty() {
            self
                .persist_learning_signal_reject(
                    db,
                    expected_session_id,
                    &signal.trace_id,
                    &signal.source,
                    target,
                    "learning_signal.missing_evidence_ref",
                    None,
                )
                .await?;
            bail!("learning signal rejected for `{target}`: evidence_ref is required");
        }
        Ok(())
    }

    async fn persist_learning_signal_reject(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        source: &str,
        target: &str,
        reason: &str,
        evidence_ref: Option<&str>,
    ) -> Result<()> {
        let now = current_time_ms();
        let record = LearningSignalRejectRecord {
            reject_id: format!("learning-signal-reject:{session_id}:{now}:{}", normalize(target)),
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            reason: reason.to_string(),
            evidence_ref: evidence_ref.map(str::to_string),
            created_at_ms: now,
        };
        db.upsert_json_knowledge(
            format!(
                "evidence:memory:{session_id}:learning-signal-reject:{}",
                record.reject_id
            ),
            &record,
            "learning-signal-guard",
        )
        .await?;
        Ok(())
    }

    pub async fn strategy_memory_layers(
        &self,
        db: &StateStore,
        session_id: &str,
    ) -> Result<serde_json::Value> {
        let hot = db
            .list_knowledge_by_prefix(&format!("memory:strategy:hot:{session_id}:"))
            .await?;
        let warm = db
            .list_knowledge_by_prefix(&format!("memory:strategy:warm:{session_id}:"))
            .await?;
        let cold = db
            .list_knowledge_by_prefix(&format!("memory:strategy:cold:{session_id}:"))
            .await?;
        Ok(serde_json::json!({
            "session_id": session_id,
            "hot_count": hot.len(),
            "warm_count": warm.len(),
            "cold_count": cold.len(),
            "hot_keys": hot.iter().map(|r| r.key.clone()).take(10).collect::<Vec<_>>(),
            "warm_keys": warm.iter().map(|r| r.key.clone()).take(10).collect::<Vec<_>>(),
            "cold_keys": cold.iter().map(|r| r.key.clone()).take(10).collect::<Vec<_>>(),
        }))
    }
}

fn consolidate_skills(
    skills: &[SkillLibraryRecord],
    episodes: &[ReflexionEpisodeRecord],
) -> Vec<SkillRecord> {
    let mut consolidated = skills
        .iter()
        .map(|skill| SkillRecord {
            name: skill.name.clone(),
            trigger: skill.trigger.clone(),
            procedure: skill.procedure.clone(),
            confidence: ((skill.confidence + skill.success_rate) / 2.0).clamp(0.0, 1.0),
        })
        .collect::<Vec<_>>();

    if let Some(last_failure) = episodes
        .iter()
        .rev()
        .find(|episode| episode.status != "success")
    {
        consolidated.push(SkillRecord {
            name: "rollback-on-regression".into(),
            trigger: last_failure.objective.clone(),
            procedure: format!(
                "When immutable verification regresses, roll back and isolate the failing slice. Lesson: {}",
                last_failure.lesson
            ),
            confidence: 0.72,
        });
    }

    consolidated
}

fn cluster_failures(
    episodes: &[ReflexionEpisodeRecord],
    witness: &[WitnessLogRecord],
    session_id: &str,
) -> Vec<FailurePatternCluster> {
    let mut cluster_map: std::collections::HashMap<String, Vec<String>> = Default::default();
    for episode in episodes
        .iter()
        .filter(|episode| episode.status != "success")
    {
        let pattern = if episode.outcome.to_ascii_lowercase().contains("approval") {
            "approval-gated-failure"
        } else if episode.outcome.to_ascii_lowercase().contains("coverage") {
            "acceptance-coverage-gap"
        } else {
            "execution-regression"
        };
        cluster_map
            .entry(pattern.into())
            .or_default()
            .push(episode.lesson.clone());
    }
    for log in witness.iter().filter(|log| log.score < 0.0) {
        cluster_map
            .entry("negative-witness".into())
            .or_default()
            .push(log.detail.clone());
    }

    cluster_map
        .into_iter()
        .map(|(pattern, examples)| FailurePatternCluster {
            cluster_id: format!("{session_id}:{}", normalize(&pattern)),
            pattern,
            frequency: examples.len(),
            representative_examples: examples.into_iter().take(3).collect(),
        })
        .collect()
}

fn validate_causal_edges(edges: &[CausalEdgeRecord]) -> CausalValidationSummary {
    if edges.is_empty() {
        return CausalValidationSummary {
            validated_edges: 0,
            average_confidence: 0.0,
            summary: "No causal edges to validate yet.".into(),
        };
    }
    let average_confidence =
        edges.iter().map(|edge| edge.confidence).sum::<f32>() / edges.len() as f32;
    CausalValidationSummary {
        validated_edges: edges.len(),
        average_confidence,
        summary: format!(
            "Validated {} causal edges with average confidence {:.2}.",
            edges.len(),
            average_confidence
        ),
    }
}

async fn derive_capability_improvements(
    clusters: &[FailurePatternCluster],
    witness: &[WitnessLogRecord],
    db: &StateStore,
) -> Result<Vec<CapabilityImprovementProposal>> {
    let manifests = db
        .list_knowledge_by_prefix(ToolRegistry::FORGED_TOOL_PREFIX)
        .await?
        .into_iter()
        .filter_map(|record| serde_json::from_str::<ForgedMcpToolManifest>(&record.value).ok())
        .collect::<Vec<_>>();

    let negative_signals = witness
        .iter()
        .filter(|log| log.score < 0.0)
        .map(|log| log.detail.to_ascii_lowercase())
        .collect::<Vec<_>>();

    Ok(manifests
        .into_iter()
        .filter_map(|manifest| {
            let has_negative_signal = negative_signals.iter().any(|detail| {
                detail.contains(&manifest.server.to_ascii_lowercase())
                    || detail.contains(&manifest.capability_name.to_ascii_lowercase())
            });
            let related_cluster = clusters.iter().find(|cluster| {
                cluster.pattern.contains("execution") || cluster.pattern.contains("approval")
            });
            if has_negative_signal || related_cluster.is_some() {
                Some(CapabilityImprovementProposal {
                    tool_name: manifest.registered_tool_name.clone(),
                    change_hint: if manifest.requires_gate() {
                        "tighten scope or split a safer low-risk sub-capability".into()
                    } else {
                        "improve argument schema and success signal coverage".into()
                    },
                    rationale: related_cluster
                        .map(|cluster| format!("Observed cluster {}", cluster.pattern))
                        .unwrap_or_else(|| {
                            "Negative witness evidence suggests route instability".into()
                        }),
                    priority: if manifest.requires_gate() { 0.9 } else { 0.65 },
                })
            } else {
                None
            }
        })
        .collect())
}

fn map_event_kind(kind: LearningEventKind) -> StoredLearningEventKind {
    match kind {
        LearningEventKind::Failure => StoredLearningEventKind::Failure,
        LearningEventKind::Success => StoredLearningEventKind::Success,
        LearningEventKind::ToolCall => StoredLearningEventKind::ToolCall,
        LearningEventKind::RouteDecision => StoredLearningEventKind::RouteDecision,
        LearningEventKind::Audit => StoredLearningEventKind::Audit,
    }
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn score_snippet(snippet: &MemorySnippet, query_terms: &HashSet<String>) -> usize {
    let snippet_terms = tokenize(&format!("{} {}", snippet.content, snippet.tags.join(" ")));
    let overlap = snippet_terms.intersection(query_terms).count();
    snippet.priority + overlap * 3 + usize::from(snippet.source == MemoryTarget::History)
}

fn compress_message(content: &str) -> String {
    const MAX_LEN: usize = 140;
    let trimmed = content.trim();
    if trimmed.chars().count() <= MAX_LEN {
        return trimmed.to_string();
    }
    let prefix = trimmed.chars().take(MAX_LEN).collect::<String>();
    format!("{prefix}...")
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| part.len() >= 3)
        .map(|part| part.to_ascii_lowercase())
        .collect()
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::AppConfig,
        tools::{ForgedMcpToolManifest, ToolRegistry},
    };
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    fn sample_signal(session_id: &str) -> LearningSignal {
        LearningSignal {
            signal_id: format!("learning-signal:{session_id}:test"),
            session_id: session_id.to_string(),
            trace_id: format!("trace:{session_id}:learning:test"),
            source: "memory.tests".to_string(),
            evidence_ref: format!("eventlog:{session_id}:learning:test"),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn memory_prefers_high_relevance_and_skips_redundancy() {
        let memory = MemorySubsystem {
            targets: vec![
                MemoryTarget::Identity,
                MemoryTarget::LongTerm,
                MemoryTarget::History,
            ],
            sidecar: None,
            default_top_k: 4,
            supermemory: supermemory::SupermemoryKernel::default(),
        };
        let history = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: "We store anchors in StateStore for GraphRAG.".into(),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "assistant".into(),
                content: "We store anchors in StateStore for GraphRAG.".into(),
            },
        ];

        let selected = memory.select_relevant_memories(&MemoryContextRequest {
            user_input: "How should anchor memory be stored in StateStore?",
            session_history: &history,
            max_items: 4,
        });

        assert!(
            selected
                .iter()
                .any(|snippet| snippet.content.contains("StateStore"))
        );
        assert_eq!(
            selected
                .iter()
                .filter(|snippet| snippet.content == "We store anchors in StateStore for GraphRAG.")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn memory_retrieves_global_forged_tool_assets() {
        let config = AppConfig::default();
        let memory = MemorySubsystem::from_config(&config.memory, &config.learning);
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let manifest = ForgedMcpToolManifest {
            registered_tool_name: "mcp::local-mcp::diagram-export".into(),
            delegate_tool_name: "mcp::local-mcp::invoke".into(),
            server: "local-mcp".into(),
            capability_name: "diagram-export".into(),
            purpose: "Export diagrams through a reusable forged MCP tool".into(),
            executable: "diagram-cli".into(),
            command_template: "diagram-cli export --project {{project}}".into(),
            payload_template: serde_json::json!({"server":"local-mcp"}),
            output_mode: crate::tools::CliOutputMode::Json,
            working_directory: Some(".".into()),
            success_signal: Some("completed".into()),
            help_text: "diagram export help".into(),
            skill_markdown: "# diagram-export".into(),
            examples: vec!["diagram-cli export --project demo.drawio".into()],
            ..ForgedMcpToolManifest::default()
        };
        db.upsert_json_knowledge(
            format!(
                "{}{}",
                ToolRegistry::FORGED_TOOL_PREFIX,
                manifest.registered_tool_name
            ),
            &manifest,
            "cli-forge",
        )
        .await
        .expect("persist manifest");

        let results = memory
            .retrieve_learning_evidence(&db, "session-memory", "diagram export mcp tool", 4)
            .await
            .expect("retrieve");

        assert!(
            results
                .iter()
                .any(|item| item.document.asset_kind == LearningAssetKind::ForgedToolManifest)
        );
    }

    #[tokio::test]
    async fn memory_consolidates_learning_assets_into_improvement_signals() {
        let config = AppConfig::default();
        let memory = MemorySubsystem::from_config(&config.memory, &config.learning);
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        memory
            .persist_reflexion_episode(
                &db,
                "session-learn",
                &ReflexionEpisode {
                    proposal_id: "p1".into(),
                    hypothesis: "test".into(),
                    outcome: "execution failed due to approval".into(),
                    lesson: "split risky capability".into(),
                },
            )
            .await
            .expect("episode");
        memory
            .persist_witness_log(
                &db,
                "session-learn",
                &WitnessLog {
                    source: "mcp::local-mcp::deploy".into(),
                    observation: "approval blocked execution".into(),
                    metric_name: "outcome_score".into(),
                    metric_value: -6.0,
                },
            )
            .await
            .expect("witness");
        db.upsert_json_knowledge(
            format!(
                "{}{}",
                ToolRegistry::FORGED_TOOL_PREFIX,
                "mcp::local-mcp::deploy"
            ),
            &ForgedMcpToolManifest {
                registered_tool_name: "mcp::local-mcp::deploy".into(),
                delegate_tool_name: "mcp::local-mcp::invoke".into(),
                server: "local-mcp".into(),
                capability_name: "deploy".into(),
                purpose: "deploy over network".into(),
                executable: "deploy-cli".into(),
                command_template: "deploy-cli --target {{target}}".into(),
                payload_template: serde_json::json!({"server":"local-mcp"}),
                output_mode: crate::tools::CliOutputMode::Json,
                working_directory: Some(".".into()),
                success_signal: Some("completed".into()),
                help_text: "help".into(),
                skill_markdown: "# skill".into(),
                examples: vec![],
                ..ForgedMcpToolManifest::default()
            },
            "cli-forge",
        )
        .await
        .expect("manifest");

        let consolidation = memory
            .consolidate_learning(&db, "session-learn")
            .await
            .expect("consolidation");

        assert!(!consolidation.failure_clusters.is_empty());
        assert!(!consolidation.capability_improvements.is_empty());
    }

    #[tokio::test]
    async fn p12_promotion_pipeline_supports_canary_and_rollback_with_strategy_tiers() {
        let config = AppConfig::default();
        let memory = MemorySubsystem::from_config(&config.memory, &config.learning);
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let proposal = memory.draft_learning_proposal(
            "session-p12-promote",
            "routing",
            "improve route stability",
            "use verified evidence",
        );
        let approved = LearningGateVerdict {
            approved: true,
            reason: "approved".into(),
            canary_ratio: 0.1,
            rollback_window_ms: 900_000,
            risk_tags: vec![],
            created_at_ms: current_time_ms(),
        };
        let promotion = memory
            .promote_skill_with_verdict(
                &db,
                &proposal,
                &approved,
                &SkillRecord {
                    name: "skill-routing".into(),
                    trigger: "routing".into(),
                    procedure: "use evidence-aware route scoring".into(),
                    confidence: 0.77,
                },
                &sample_signal("session-p12-promote"),
            )
            .await
            .expect("promote");
        assert_eq!(promotion.stage, SkillPromotionStage::Canary);

        let rolled = memory
            .rollback_skill_promotion(
                &db,
                "session-p12-promote",
                &promotion,
                "regression detected",
                &sample_signal("session-p12-promote"),
            )
            .await
            .expect("rollback");
        assert_eq!(rolled.stage, SkillPromotionStage::RolledBack);

        let layers = memory
            .strategy_memory_layers(&db, "session-p12-promote")
            .await
            .expect("layers");
        assert!(layers["warm_count"].as_u64().unwrap_or(0) >= 1);
        assert!(layers["cold_count"].as_u64().unwrap_or(0) >= 1);
    }

    #[tokio::test]
    async fn learning_signal_without_evidence_ref_is_rejected_and_audited() {
        let config = AppConfig::default();
        let memory = MemorySubsystem::from_config(&config.memory, &config.learning);
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });

        let rejected = memory
            .persist_skill(
                &db,
                "session-learning-reject",
                &SkillRecord {
                    name: "skill-reject".into(),
                    trigger: "reject".into(),
                    procedure: "noop".into(),
                    confidence: 0.1,
                },
                &LearningSignal {
                    signal_id: "sig:reject".into(),
                    session_id: "session-learning-reject".into(),
                    trace_id: "trace:reject".into(),
                    source: "tests".into(),
                    evidence_ref: "".into(),
                    metadata: BTreeMap::new(),
                },
            )
            .await;

        assert!(rejected.is_err(), "missing evidence_ref must be rejected");
        let reject_records = db
            .list_knowledge_by_prefix("evidence:memory:session-learning-reject:learning-signal-reject:")
            .await
            .expect("list rejects");
        assert!(
            !reject_records.is_empty(),
            "rejection must be persisted for query-plane/audit visibility"
        );
        assert!(
            reject_records
                .first()
                .map(|item| item.value.contains("learning_signal.missing_evidence_ref"))
                .unwrap_or(false)
        );
    }
}

