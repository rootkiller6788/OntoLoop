pub mod acl_fine_policy;
pub mod atomic_renderer;
pub mod canonical_repo;
pub mod compiler_core;
pub mod conflict_manager;
pub mod context_core;
pub mod episode_ledger;
pub mod formal_checker;
pub mod gateway_core;
pub mod graph_export;
pub mod graph_health;
pub mod heal_proposal;
pub mod hot_index_updater;
pub mod ingest_validator;
pub mod incremental_compiler;
pub mod ontology_enhancer;
pub mod patch_core;
pub mod patch_review_queue;
pub mod protocol;
pub mod provenance_core;
pub mod recall_core;
pub mod recall_plugin_router;
pub mod repo_core;
pub mod schema_registry;
pub mod semantic_edges;
pub mod semantic_lint;
pub mod signed_commit_chain;
pub mod supermemory_export_mirror;
pub mod view_plane;
use acl_fine_policy::{AclDecision, FineGrainedAclPolicy};
use anyhow::{Result, bail};
use atomic_renderer::AtomicRenderer;
use autoloop_state_adapter::StateStore;
use canonical_repo::{CanonicalRepo, CanonicalWriteReceipt};
use conflict_manager::{ConflictManager, ConflictRecord};
use episode_ledger::{EpisodeLedger, EpisodeStage};
use formal_checker::{FormalCheckMode, FormalCheckReport, FormalChecker};
use gateway_core::{GatewayDecision, GatewayInput};
use heal_proposal::{HealProposalRecord, HealProposalRequest, HealProposalService};
use hot_index_updater::{HotIndexUpdateReport, HotIndexUpdater, RefreshPlanMode, SourceRefreshPlan};
use ingest_validator::{IngestValidationMode, IngestValidationReport, IngestValidator};
use incremental_compiler::{IncrementalCompileReport, IncrementalCompiler};
use ontology_enhancer::{OntologyEnhancement, OntologyEnhancer};
use patch_core::PatchPlan;
use patch_review_queue::{PatchReviewQueue, PatchReviewStatus};
use protocol::{CorePackageHealth, CorePackageKind, CorePackageManifest, CorePackageStatus};
use provenance_core::{ProvenanceCore, ProvenanceLineageRecord, ProvenanceSegmentRef};
use recall_core::RecallPlan;
use recall_plugin_router::RecallPluginRouter;
use signed_commit_chain::SignedCommitChain;
use supermemory_export_mirror::{
    MirrorExportReceipt, MirrorExportRequest, SupermemoryExportMirror,
};
use view_plane::{ViewPlane, ViewPlaneReceipt, ViewPlaneSnapshot};

pub trait CorePackage: Send + Sync {
    fn manifest(&self) -> &CorePackageManifest;
    fn health(&self) -> CorePackageHealth;
}

#[derive(Clone)]
pub struct GitmemoryCoreKernel {
    manifests: Vec<CorePackageManifest>,
}

impl GitmemoryCoreKernel {
    pub fn new() -> Self {
        let manifests = vec![
            gateway_core::GatewayCore::manifest_frozen(),
            recall_core::RecallCore::manifest_frozen(),
            patch_core::PatchCore::manifest_frozen(),
            repo_core::RepoCore::manifest_frozen(),
            compiler_core::CompilerCore::manifest_frozen(),
            provenance_core::ProvenanceCore::manifest_frozen(),
            context_core::ContextCore::manifest_frozen(),
        ];
        Self { manifests }
    }

    pub fn manifests(&self) -> &[CorePackageManifest] {
        &self.manifests
    }

    pub fn health(&self) -> Vec<CorePackageHealth> {
        self.manifests
            .iter()
            .map(|manifest| CorePackageHealth {
                kind: manifest.kind.clone(),
                status: CorePackageStatus::Ready,
                message: "core package skeleton ready".to_string(),
            })
            .collect()
    }

    pub fn has_exactly_seven_packages(&self) -> bool {
        self.manifests.len() == 7
            && self.manifests.iter().all(|manifest| manifest.facade_only)
            && self
                .manifests
                .iter()
                .map(|manifest| &manifest.kind)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 7
    }

    pub async fn run_gateway_recall_patch(
        &self,
        db: &StateStore,
        session_id: &str,
        tenant_id: &str,
        intent: &str,
        actor: &str,
    ) -> Result<GatewayRecallPatchRun> {
        let trace_id = format!("episode:{}:{}", session_id, current_time_ms());
        let gateway_input = GatewayInput {
            session_id: session_id.to_string(),
            tenant_id: tenant_id.to_string(),
            intent: intent.to_string(),
            actor: actor.to_string(),
        };
        let gateway = gateway_core::GatewayCore::decide(&gateway_input);
        if !gateway.accepted {
            bail!("gateway rejected intent: {}", gateway.reason);
        }
        let gateway_key = EpisodeLedger::append(
            db,
            session_id,
            &trace_id,
            EpisodeStage::Gateway,
            serde_json::to_value(&gateway)?,
        )
        .await?;

        let route =
            RecallPluginRouter::route(db, session_id, tenant_id, &gateway.normalized_intent)
                .await?;
        let seed_hits = route
            .lexical_fallback
            .as_ref()
            .map(|item| item.hits.clone())
            .unwrap_or_default();
        let recall = recall_core::RecallCore::plan_with_graph_expansion(
            &gateway,
            &route.strategy,
            route.selected_sources.clone(),
            seed_hits,
            route.graph_enabled,
            route.neighbor_threshold,
            route.max_neighbors,
        );
        let recall_key = EpisodeLedger::append(
            db,
            session_id,
            &trace_id,
            EpisodeStage::Recall,
            serde_json::json!({
                "recall": recall,
                "route": route,
            }),
        )
        .await?;

        let patch = patch_core::PatchCore::build(&recall);
        let review = PatchReviewQueue::enqueue(db, session_id, &trace_id, &patch).await?;
        let patch_key = EpisodeLedger::append(
            db,
            session_id,
            &trace_id,
            EpisodeStage::Patch,
            serde_json::json!({
                "patch": patch,
                "review_id": review.review_id,
                "status": review.status,
                "decision": review.decision,
            }),
        )
        .await?;

        db.upsert_json_knowledge(
            format!("memory:episode:run:{}:{}", session_id, current_time_ms()),
            &serde_json::json!({
                "session_id": session_id,
                "tenant_id": tenant_id,
                "trace_id": trace_id,
                "gateway_ref": gateway_key,
                "recall_ref": recall_key,
                "patch_ref": patch_key,
            }),
            "episode-ledger",
        )
        .await?;

        Ok(GatewayRecallPatchRun {
            trace_id,
            gateway,
            recall,
            patch,
            ledger_refs: vec![gateway_key, recall_key, patch_key],
        })
    }

    pub async fn run_day56_commit_chain(
        &self,
        db: &StateStore,
        repo_root: &std::path::Path,
        session_id: &str,
        signer: &str,
        run: &GatewayRecallPatchRun,
    ) -> Result<Day56CommitRun> {
        let rendered = AtomicRenderer::render(session_id, &run.trace_id, &run.patch);
        let ingest_validation = IngestValidator::validate(
            repo_root,
            &rendered.relative_path,
            &rendered.markdown,
            IngestValidationMode::Enforced,
        )?;
        if !ingest_validation.passed {
            bail!(
                "ingest validation failed: broken_links={} unindexed={}",
                ingest_validation.broken_links.len(),
                ingest_validation.unindexed.len()
            );
        }
        let write =
            CanonicalRepo::write_atomic(repo_root, &rendered.relative_path, &rendered.markdown)?;
        let tree_digest = crate::observability::event_stream::digest_value(&serde_json::json!({
            "relative_path": write.relative_path,
            "bytes": write.bytes,
        }));
        let commit = SignedCommitChain::append(
            db,
            session_id,
            &run.trace_id,
            &tree_digest,
            &serde_json::json!({
                "gateway": run.gateway,
                "recall": run.recall,
                "patch": run.patch,
                "rendered_path": write.relative_path,
            }),
            signer,
        )
        .await?;
        let chain_ref = format!(
            "memory:commit-chain:{}:{}",
            session_id, commit.created_at_ms
        );
        EpisodeLedger::append(
            db,
            session_id,
            &run.trace_id,
            EpisodeStage::Patch,
            serde_json::json!({
                "day56": {
                    "ingest_validation": ingest_validation,
                    "canonical_write": write,
                    "commit_id": commit.commit_id,
                    "signature": commit.signature,
                    "commit_chain_ref": chain_ref,
                }
            }),
        )
        .await?;

        Ok(Day56CommitRun {
            trace_id: run.trace_id.clone(),
            ingest_validation,
            canonical_write: write,
            commit_id: commit.commit_id,
            commit_signature: commit.signature,
            commit_chain_ref: chain_ref,
        })
    }

    pub fn validate_day56_ingest_only(
        &self,
        repo_root: &std::path::Path,
        session_id: &str,
        run: &GatewayRecallPatchRun,
    ) -> Result<IngestValidationReport> {
        let rendered = AtomicRenderer::render(session_id, &run.trace_id, &run.patch);
        IngestValidator::validate(
            repo_root,
            &rendered.relative_path,
            &rendered.markdown,
            IngestValidationMode::ValidateOnly,
        )
    }

    pub async fn run_heal_proposal(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        request: HealProposalRequest,
    ) -> Result<DayHealProposalRun> {
        let proposal = HealProposalService::propose(db, session_id, trace_id, request).await?;
        let ledger_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::Patch,
            serde_json::json!({
                "heal_proposal_id": proposal.proposal_id,
                "review_id": proposal.review_id,
                "review_status": proposal.review_status,
                "patch": proposal.patch,
            }),
        )
        .await?;
        Ok(DayHealProposalRun {
            trace_id: trace_id.to_string(),
            proposal,
            ledger_refs: vec![ledger_ref],
        })
    }

    pub async fn execute_approved_heal_proposal(
        &self,
        db: &StateStore,
        repo_root: &std::path::Path,
        session_id: &str,
        trace_id: &str,
        review_id: &str,
        signer: &str,
    ) -> Result<DayHealCommitRun> {
        let review = PatchReviewQueue::list(db, session_id)
            .await?
            .into_iter()
            .find(|item| item.review_id == review_id)
            .ok_or_else(|| anyhow::anyhow!("heal review not found: {}", review_id))?;
        if review.review_kind != "heal_proposal" {
            bail!("review is not a heal proposal: {}", review_id);
        }
        if review.status != PatchReviewStatus::Approved {
            bail!("heal proposal not approved yet: {}", review_id);
        }

        let rendered = AtomicRenderer::render(session_id, trace_id, &review.patch);
        let ingest_validation = IngestValidator::validate(
            repo_root,
            &rendered.relative_path,
            &rendered.markdown,
            IngestValidationMode::Enforced,
        )?;
        if !ingest_validation.passed {
            bail!(
                "ingest validation failed: broken_links={} unindexed={}",
                ingest_validation.broken_links.len(),
                ingest_validation.unindexed.len()
            );
        }
        let write =
            CanonicalRepo::write_atomic(repo_root, &rendered.relative_path, &rendered.markdown)?;
        let tree_digest = crate::observability::event_stream::digest_value(&serde_json::json!({
            "relative_path": write.relative_path,
            "bytes": write.bytes,
            "review_id": review_id,
        }));
        let commit = SignedCommitChain::append(
            db,
            session_id,
            trace_id,
            &tree_digest,
            &serde_json::json!({
                "heal_review_id": review_id,
                "patch": review.patch,
                "rendered_path": write.relative_path,
            }),
            signer,
        )
        .await?;
        let commit_chain_ref = format!(
            "memory:commit-chain:{}:{}",
            session_id, commit.created_at_ms
        );
        let ledger_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::Patch,
            serde_json::json!({
                "heal_review_id": review_id,
                "ingest_validation": ingest_validation,
                "canonical_write": write,
                "commit_id": commit.commit_id,
                "signature": commit.signature,
                "commit_chain_ref": commit_chain_ref,
            }),
        )
        .await?;

        Ok(DayHealCommitRun {
            trace_id: trace_id.to_string(),
            review_id: review_id.to_string(),
            ingest_validation,
            canonical_write: write,
            commit_id: commit.commit_id,
            commit_signature: commit.signature,
            commit_chain_ref,
            ledger_refs: vec![ledger_ref],
        })
    }

    pub async fn run_day78_incremental(
        &self,
        db: &StateStore,
        repo_root: &std::path::Path,
        session_id: &str,
        trace_id: &str,
        changed_files: &[String],
    ) -> Result<Day78IncrementalRun> {
        let refresh_plan =
            HotIndexUpdater::plan_refresh(repo_root, changed_files, RefreshPlanMode::Detect)?;
        let effective_changed_files = refresh_plan.effective_changed_files.clone();
        let compile = IncrementalCompiler::rebuild_changed(repo_root, &effective_changed_files)?;
        let compile_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::Compiler,
            serde_json::to_value(&compile)?,
        )
        .await?;

        let hot_index = HotIndexUpdater::update(repo_root, &compile)?;
        let hot_index_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::HotIndex,
            serde_json::to_value(&hot_index)?,
        )
        .await?;

        db.upsert_json_knowledge(
            format!("memory:compiler:run:{}:{}", session_id, current_time_ms()),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "changed_files": changed_files,
                "effective_changed_files": effective_changed_files,
                "refresh_plan": &refresh_plan,
                "compile_ref": compile_ref,
                "hot_index_ref": hot_index_ref,
            }),
            "episode-ledger",
        )
        .await?;

        Ok(Day78IncrementalRun {
            trace_id: trace_id.to_string(),
            changed_files: changed_files.to_vec(),
            refresh_plan,
            compile,
            hot_index,
            ledger_refs: vec![compile_ref, hot_index_ref],
        })
    }

    pub async fn run_day11_mirror_export(
        &self,
        db: &StateStore,
        session_id: &str,
        tenant_id: &str,
        trace_id: &str,
        approved: bool,
        compile_refs: &[String],
        trace_refs: &[String],
        payload: serde_json::Value,
    ) -> Result<Day11MirrorRun> {
        let request = MirrorExportRequest {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            tenant_id: tenant_id.to_string(),
            approved,
            compiled: !compile_refs.is_empty(),
            traceable: !trace_refs.is_empty(),
            approval_ref: if approved {
                Some(format!("approval:{session_id}:{trace_id}"))
            } else {
                None
            },
            compile_refs: compile_refs.to_vec(),
            trace_refs: trace_refs.to_vec(),
            payload,
        };
        let mirror = SupermemoryExportMirror::export(db, &request).await?;
        let mirror_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::MirrorExport,
            serde_json::json!({
                "mirror_export_ref": mirror.export_ref,
                "direction": mirror.direction,
                "policy": mirror.policy,
                "approved": request.approved,
                "compiled": request.compiled,
                "traceable": request.traceable,
                "compile_refs": request.compile_refs,
                "trace_refs": request.trace_refs,
            }),
        )
        .await?;

        Ok(Day11MirrorRun {
            trace_id: trace_id.to_string(),
            mirror,
            ledger_refs: vec![mirror_ref],
        })
    }

    pub async fn run_day12_replay_audit_provenance(
        &self,
        db: &StateStore,
        session_id: &str,
        tenant_id: &str,
        trace_id: &str,
        segments: &[ProvenanceSegmentRef],
        replay_refs: &[String],
        audit_refs: &[String],
    ) -> Result<Day12ProvenanceRun> {
        let lineage = ProvenanceCore::append_lineage(
            db,
            session_id,
            tenant_id,
            trace_id,
            segments,
            replay_refs,
            audit_refs,
        )
        .await?;
        let lineage_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::Provenance,
            serde_json::json!({
                "lineage_id": lineage.lineage_id,
                "lineage_digest": lineage.lineage_digest,
                "segment_count": lineage.segments.len(),
                "replay_refs": lineage.replay_refs,
                "audit_refs": lineage.audit_refs,
            }),
        )
        .await?;
        let event = crate::observability::event_stream::append_event(
            db,
            "provenance.lineage.committed",
            trace_id.to_string(),
            session_id.to_string(),
            None,
            Some("provenance-core".to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "lineage_id": lineage.lineage_id,
                "lineage_digest": lineage.lineage_digest,
                "segment_count": lineage.segments.len(),
                "tenant_id": tenant_id,
            }),
        )
        .await?;

        Ok(Day12ProvenanceRun {
            trace_id: trace_id.to_string(),
            lineage,
            ledger_refs: vec![lineage_ref],
            event_id: event.event_id,
        })
    }

    pub async fn run_phase3_source_view_plane(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<Phase3SourceViewRun> {
        let snapshot = ViewPlane::build(db, session_id, trace_id).await?;
        let receipt = ViewPlane::persist(db, session_id, &snapshot).await?;
        let view_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::View,
            serde_json::json!({
                "mindmap_ref": receipt.mindmap_ref,
                "explainer_ref": receipt.explainer_ref,
                "plane_ref": receipt.plane_ref,
            }),
        )
        .await?;
        Ok(Phase3SourceViewRun {
            trace_id: trace_id.to_string(),
            snapshot,
            receipt,
            ledger_refs: vec![view_ref],
        })
    }

    pub async fn run_phase4_advanced_governance(
        &self,
        db: &StateStore,
        repo_root: &std::path::Path,
        session_id: &str,
        tenant_id: &str,
        trace_id: &str,
        phase: GovernancePhase,
        actor: &str,
        action: &str,
        namespace: &str,
        sensitivity: &str,
    ) -> Result<Phase4GovernanceRun> {
        let acl =
            FineGrainedAclPolicy::evaluate(db, tenant_id, actor, action, namespace, sensitivity)
                .await?;
        let conflicts = ConflictManager::analyze(db, session_id, trace_id).await?;
        let ontology = OntologyEnhancer::enhance(db, repo_root, session_id, trace_id).await?;
        let formal =
            FormalChecker::check_with_mode(db, session_id, trace_id, phase.to_formal_mode())
                .await?;

        let allowed = acl.allowed && formal.passed && conflicts.is_empty();
        let summary = if allowed {
            "phase4 governance passed".to_string()
        } else {
            "phase4 governance blocked by acl/formal/conflict checks".to_string()
        };
        let rule_id = if !acl.allowed {
            Some(
                acl.matched_rule_id
                    .clone()
                    .unwrap_or_else(|| "acl:no_match".to_string()),
            )
        } else if let Some(conflict) = conflicts.first() {
            Some(conflict.conflict_id.clone())
        } else {
            formal
                .rule_results
                .iter()
                .find(|rule| !rule.passed)
                .map(|rule| rule.rule_id.clone())
        };
        let policy_version = acl.policy_version.clone();
        let replay_fp = format!("replay-fp:{session_id}:{trace_id}");
        let governance_ref = EpisodeLedger::append(
            db,
            session_id,
            trace_id,
            EpisodeStage::Governance,
            serde_json::json!({
                "allowed": allowed,
                "phase": phase,
                "summary": summary,
                "rule_id": rule_id,
                "policy_version": policy_version,
                "replay_fp": replay_fp,
                "acl": acl,
                "conflict_count": conflicts.len(),
                "formal_passed": formal.passed,
                "ontology_version": ontology.taxonomy_version,
            }),
        )
        .await?;

        Ok(Phase4GovernanceRun {
            trace_id: trace_id.to_string(),
            allowed,
            summary,
            rule_id,
            policy_version,
            replay_fp,
            acl,
            conflicts,
            ontology,
            formal,
            ledger_refs: vec![governance_ref],
        })
    }
}

impl Default for GitmemoryCoreKernel {
    fn default() -> Self {
        Self::new()
    }
}

pub fn frozen_manifest(id: &str, kind: CorePackageKind, owner: &str) -> CorePackageManifest {
    CorePackageManifest::frozen_v2(id, kind, owner)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayRecallPatchRun {
    pub trace_id: String,
    pub gateway: GatewayDecision,
    pub recall: RecallPlan,
    pub patch: PatchPlan,
    pub ledger_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Day56CommitRun {
    pub trace_id: String,
    pub ingest_validation: IngestValidationReport,
    pub canonical_write: CanonicalWriteReceipt,
    pub commit_id: String,
    pub commit_signature: String,
    pub commit_chain_ref: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DayHealProposalRun {
    pub trace_id: String,
    pub proposal: HealProposalRecord,
    pub ledger_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DayHealCommitRun {
    pub trace_id: String,
    pub review_id: String,
    pub ingest_validation: IngestValidationReport,
    pub canonical_write: CanonicalWriteReceipt,
    pub commit_id: String,
    pub commit_signature: String,
    pub commit_chain_ref: String,
    pub ledger_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Day78IncrementalRun {
    pub trace_id: String,
    pub changed_files: Vec<String>,
    pub refresh_plan: SourceRefreshPlan,
    pub compile: IncrementalCompileReport,
    pub hot_index: HotIndexUpdateReport,
    pub ledger_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Day11MirrorRun {
    pub trace_id: String,
    pub mirror: MirrorExportReceipt,
    pub ledger_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Day12ProvenanceRun {
    pub trace_id: String,
    pub lineage: ProvenanceLineageRecord,
    pub ledger_refs: Vec<String>,
    pub event_id: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Phase3SourceViewRun {
    pub trace_id: String,
    pub snapshot: ViewPlaneSnapshot,
    pub receipt: ViewPlaneReceipt,
    pub ledger_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Phase4GovernanceRun {
    pub trace_id: String,
    pub allowed: bool,
    pub summary: String,
    pub rule_id: Option<String>,
    pub policy_version: String,
    pub replay_fp: String,
    pub acl: AclDecision,
    pub conflicts: Vec<ConflictRecord>,
    pub ontology: OntologyEnhancement,
    pub formal: FormalCheckReport,
    pub ledger_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GovernancePhase {
    Admission,
    Verify,
}

impl GovernancePhase {
    fn to_formal_mode(self) -> FormalCheckMode {
        match self {
            Self::Admission => FormalCheckMode::Admission,
            Self::Verify => FormalCheckMode::Verify,
        }
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::heal_proposal::HealProposalRequest;
    use super::patch_core::PatchOpKind;
    use super::patch_review_queue::PatchReviewQueue;
    use super::recall_plugin_router::RecallPluginRouter;
    use super::{GitmemoryCoreKernel, GovernancePhase, ProvenanceSegmentRef};
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[test]
    fn core_kernel_registers_seven_frozen_packages() {
        let kernel = GitmemoryCoreKernel::new();
        assert!(kernel.has_exactly_seven_packages());
    }

    #[tokio::test]
    async fn gateway_recall_patch_writes_unified_episode_ledger() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = GitmemoryCoreKernel::new();
        let run = kernel
            .run_gateway_recall_patch(
                &db,
                "session:day3-4",
                "tenant:day3-4",
                "sync memory control plane",
                "principal:tester",
            )
            .await
            .expect("run");
        assert_eq!(run.ledger_refs.len(), 3);

        let records = db
            .list_knowledge_by_prefix("memory:episode:session:day3-4")
            .await
            .expect("list prefix");
        assert!(
            records.len() >= 3,
            "expected at least gateway/recall/patch records, got {}",
            records.len()
        );
    }

    #[tokio::test]
    async fn day56_writes_canonical_markdown_and_signed_commit_chain() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let stage = kernel
            .run_gateway_recall_patch(
                &db,
                "session:day5-6",
                "tenant:day5-6",
                "render atomic markdown",
                "principal:tester",
            )
            .await
            .expect("stage run");

        let temp =
            std::env::temp_dir().join(format!("autoloop-day56-{}", super::current_time_ms()));
        std::fs::create_dir_all(&temp).expect("mkdir temp");
        let commit = kernel
            .run_day56_commit_chain(&db, &temp, "session:day5-6", "principal:signer", &stage)
            .await
            .expect("day56 run");

        assert!(!commit.commit_id.is_empty());
        assert!(!commit.commit_signature.is_empty());

        let rendered = std::fs::read_to_string(&commit.canonical_write.absolute_path)
            .expect("rendered markdown");
        assert!(rendered.contains("# Atomic Patch"));
        assert!(rendered.contains("namespace:"));

        let commit_record = db
            .get_knowledge(&commit.commit_chain_ref)
            .await
            .expect("db")
            .expect("commit record");
        assert!(commit_record.value.contains("commit_id"));
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn heal_proposal_requires_approval_before_canonical_write() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let temp = std::env::temp_dir().join(format!(
            "autoloop-heal-proposal-{}",
            super::current_time_ms()
        ));
        std::fs::create_dir_all(&temp).expect("mkdir temp");

        let proposal = kernel
            .run_heal_proposal(
                &db,
                "session:heal",
                "trace:heal",
                HealProposalRequest {
                    namespace: "tenant:heal".to_string(),
                    target: "canonical/memory/heal.md".to_string(),
                    reason: "repair stale graph neighborhood".to_string(),
                    op_kind: PatchOpKind::Delete,
                },
            )
            .await
            .expect("heal proposal");
        assert_eq!(proposal.ledger_refs.len(), 1);

        let canonical_dir = temp.join("canonical");
        assert!(
            !canonical_dir.exists(),
            "proposal stage must not write canonical files"
        );

        let pre_approval = kernel
            .execute_approved_heal_proposal(
                &db,
                &temp,
                "session:heal",
                "trace:heal",
                &proposal.proposal.review_id,
                "principal:signer",
            )
            .await;
        assert!(pre_approval.is_err(), "unapproved heal must not commit");

        PatchReviewQueue::approve(
            &db,
            "session:heal",
            &proposal.proposal.review_id,
            "operator:reviewer",
            "manual heal approval",
        )
        .await
        .expect("approve heal review");

        let committed = kernel
            .execute_approved_heal_proposal(
                &db,
                &temp,
                "session:heal",
                "trace:heal",
                &proposal.proposal.review_id,
                "principal:signer",
            )
            .await
            .expect("commit approved heal");
        assert!(std::path::Path::new(&committed.canonical_write.absolute_path).exists());
        assert!(!committed.commit_id.is_empty());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn day78_incremental_compiler_rebuilds_changed_files_only() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = GitmemoryCoreKernel::new();
        let temp =
            std::env::temp_dir().join(format!("autoloop-day78-{}", super::current_time_ms()));
        std::fs::create_dir_all(temp.join("memory")).expect("mkdir memory");
        std::fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
        std::fs::write(
            temp.join("memory").join("MEMORY.md"),
            "# Memory\n\n- alpha fact\n- beta fact\n",
        )
        .expect("write memory");
        std::fs::write(
            temp.join("docs").join("notes.md"),
            "# Notes\n\n- untouched\n",
        )
        .expect("write docs");

        let changed_files = vec![
            "memory/MEMORY.md".to_string(),
            "missing/nope.md".to_string(),
        ];
        let run = kernel
            .run_day78_incremental(&db, &temp, "session:day7-8", "trace:day7-8", &changed_files)
            .await
            .expect("day78 run");

        assert_eq!(run.changed_files, changed_files);
        assert_eq!(
            run.refresh_plan.effective_changed_files,
            changed_files
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert_eq!(run.compile.compiled_files.len(), 1);
        assert_eq!(
            run.compile.compiled_files[0].source_file,
            "memory/MEMORY.md"
        );
        assert_eq!(
            run.compile.skipped_missing_files,
            vec!["missing/nope.md".to_string()]
        );
        assert_eq!(
            run.hot_index.touched_files,
            vec!["memory/MEMORY.md".to_string()]
        );
        assert!(run.hot_index.total_entries >= 1);
        assert_eq!(run.ledger_refs.len(), 2);

        let graph_projection = temp
            .join(".gitmemory")
            .join("projections")
            .join("graph")
            .join("memory_MEMORY_md.json");
        let vector_projection = temp
            .join(".gitmemory")
            .join("projections")
            .join("vector")
            .join("memory_MEMORY_md.json");
        let search_projection = temp
            .join(".gitmemory")
            .join("projections")
            .join("search")
            .join("memory_MEMORY_md.json");
        assert!(graph_projection.exists());
        assert!(vector_projection.exists());
        assert!(search_projection.exists());

        let index_path = temp.join(".gitmemory").join("hot_index.json");
        assert!(index_path.exists());
        let index_content = std::fs::read_to_string(index_path).expect("read index");
        assert!(index_content.contains("memory/MEMORY.md"));
        assert!(!index_content.contains("docs/notes.md"));
        let semantic_cache = temp
            .join(".gitmemory")
            .join("semantic")
            .join("edge_cache.json");
        let semantic_checkpoint = temp
            .join(".gitmemory")
            .join("semantic")
            .join("checkpoint.jsonl");
        assert!(semantic_cache.exists());
        assert!(semantic_checkpoint.exists());
        assert!(!run.compile.inference_cache_entries.is_empty());
        assert!(!run.compile.inference_checkpoint_records.is_empty());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn day78_refresh_plan_expands_stale_sources_before_compile() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let temp = std::env::temp_dir().join(format!(
            "autoloop-day78-stale-{}",
            super::current_time_ms()
        ));
        std::fs::create_dir_all(temp.join("memory")).expect("mkdir memory");
        std::fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
        std::fs::write(temp.join("memory").join("MEMORY.md"), "# Memory\n- alpha\n")
            .expect("write memory");
        std::fs::write(temp.join("docs").join("notes.md"), "# Notes\n- original\n")
            .expect("write notes");

        let first = kernel
            .run_day78_incremental(
                &db,
                &temp,
                "session:day7-8-stale",
                "trace:day7-8-stale:first",
                &["memory/MEMORY.md".to_string(), "docs/notes.md".to_string()],
            )
            .await
            .expect("first run");
        assert_eq!(first.compile.compiled_files.len(), 2);

        std::fs::write(temp.join("docs").join("notes.md"), "# Notes\n- updated\n")
            .expect("update notes");
        let second = kernel
            .run_day78_incremental(
                &db,
                &temp,
                "session:day7-8-stale",
                "trace:day7-8-stale:second",
                &[],
            )
            .await
            .expect("second run");

        assert!(second.refresh_plan.stale_files.contains(&"docs/notes.md".to_string()));
        assert_eq!(
            second.refresh_plan.effective_changed_files,
            vec!["docs/notes.md".to_string()]
        );
        assert_eq!(second.compile.compiled_files.len(), 1);
        assert_eq!(second.compile.compiled_files[0].source_file, "docs/notes.md");
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn day78_unchanged_sources_do_not_rebuild_projections() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let temp = std::env::temp_dir().join(format!(
            "autoloop-day78-unchanged-{}",
            super::current_time_ms()
        ));
        std::fs::create_dir_all(temp.join("memory")).expect("mkdir memory");
        std::fs::write(temp.join("memory").join("MEMORY.md"), "# Memory\n- alpha\n")
            .expect("write memory");

        let first = kernel
            .run_day78_incremental(
                &db,
                &temp,
                "session:day7-8-unchanged",
                "trace:day7-8-unchanged:first",
                &["memory/MEMORY.md".to_string()],
            )
            .await
            .expect("first run");
        assert_eq!(first.compile.compiled_files.len(), 1);

        let second = kernel
            .run_day78_incremental(
                &db,
                &temp,
                "session:day7-8-unchanged",
                "trace:day7-8-unchanged:second",
                &[],
            )
            .await
            .expect("second run");
        assert!(second.refresh_plan.stale_files.is_empty());
        assert!(second.refresh_plan.effective_changed_files.is_empty());
        assert!(second.compile.compiled_files.is_empty());
        assert!(second.hot_index.touched_files.is_empty());
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn day11_exports_mirror_only_when_approved_compiled_traceable() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let run = kernel
            .run_day11_mirror_export(
                &db,
                "session:day11",
                "tenant:day11",
                "trace:day11",
                true,
                &["compile:ref:1".to_string()],
                &["trace:ref:1".to_string()],
                serde_json::json!({"items":[{"memory":"ok"}]}),
            )
            .await
            .expect("day11 run");
        assert_eq!(run.ledger_refs.len(), 1);
        let record = db
            .get_knowledge(&run.mirror.export_ref)
            .await
            .expect("db")
            .expect("record");
        assert!(record.value.contains("\"direction\":\"outbound_only\""));
        assert!(record.value.contains("\"approved\":true"));
    }

    #[tokio::test]
    async fn day11_rejects_mirror_export_without_approval() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let result = kernel
            .run_day11_mirror_export(
                &db,
                "session:day11-reject",
                "tenant:day11-reject",
                "trace:day11-reject",
                false,
                &["compile:ref:1".to_string()],
                &["trace:ref:1".to_string()],
                serde_json::json!({"items":[{"memory":"blocked"}]}),
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .err()
                .expect("err")
                .to_string()
                .contains("approval is required")
        );
    }

    #[tokio::test]
    async fn day12_connects_replay_audit_with_full_provenance_lineage() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let segments = vec![
            ProvenanceSegmentRef {
                stage: "gateway".to_string(),
                reference: "memory:episode:gateway:1".to_string(),
            },
            ProvenanceSegmentRef {
                stage: "patch".to_string(),
                reference: "memory:episode:patch:1".to_string(),
            },
            ProvenanceSegmentRef {
                stage: "compiler".to_string(),
                reference: "memory:compiler:run:1".to_string(),
            },
            ProvenanceSegmentRef {
                stage: "mirror_export".to_string(),
                reference: "memory:supermemory:mirror:export:1".to_string(),
            },
        ];
        let replay_refs = vec!["replay:snapshot:session-day12".to_string()];
        let audit_refs = vec!["audit:event:session-day12".to_string()];

        let run = kernel
            .run_day12_replay_audit_provenance(
                &db,
                "session:day12",
                "tenant:day12",
                "trace:day12",
                &segments,
                &replay_refs,
                &audit_refs,
            )
            .await
            .expect("day12 run");

        assert_eq!(run.ledger_refs.len(), 1);
        assert_eq!(run.lineage.segments.len(), 4);
        assert_eq!(run.lineage.replay_refs, replay_refs);
        assert_eq!(run.lineage.audit_refs, audit_refs);

        let prov_records = db
            .list_knowledge_by_prefix("memory:provenance:session:day12:trace:day12:")
            .await
            .expect("provenance records");
        assert!(
            !prov_records.is_empty(),
            "expected persisted provenance record"
        );

        let events = crate::observability::event_stream::list_session_events(&db, "session:day12")
            .await
            .expect("events");
        assert!(
            events
                .iter()
                .any(|event| event.kind == "provenance.lineage.committed"),
            "expected provenance lineage committed event"
        );
    }

    #[tokio::test]
    async fn day13_full_chain_and_regression_surface_end_to_end() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let session_id = "session:day13";
        let tenant_id = "tenant:day13";

        let day34 = kernel
            .run_gateway_recall_patch(
                &db,
                session_id,
                tenant_id,
                "run full day13 chain",
                "principal:day13",
            )
            .await
            .expect("day3-4");

        let temp =
            std::env::temp_dir().join(format!("autoloop-day13-{}", super::current_time_ms()));
        std::fs::create_dir_all(&temp).expect("mkdir temp");
        let day56 = kernel
            .run_day56_commit_chain(&db, &temp, session_id, "principal:signer", &day34)
            .await
            .expect("day5-6");

        let day78 = kernel
            .run_day78_incremental(
                &db,
                &temp,
                session_id,
                &day34.trace_id,
                &[day56.canonical_write.relative_path.clone()],
            )
            .await
            .expect("day7-8");

        let day11 = kernel
            .run_day11_mirror_export(
                &db,
                session_id,
                tenant_id,
                &day34.trace_id,
                true,
                &day78.ledger_refs,
                &[day56.commit_chain_ref.clone()],
                serde_json::json!({
                    "session_id": session_id,
                    "compile_refs": day78.ledger_refs,
                }),
            )
            .await
            .expect("day11");

        let segments = vec![
            ProvenanceSegmentRef {
                stage: "gateway".to_string(),
                reference: day34.ledger_refs[0].clone(),
            },
            ProvenanceSegmentRef {
                stage: "recall".to_string(),
                reference: day34.ledger_refs[1].clone(),
            },
            ProvenanceSegmentRef {
                stage: "patch".to_string(),
                reference: day34.ledger_refs[2].clone(),
            },
            ProvenanceSegmentRef {
                stage: "compiler".to_string(),
                reference: day78.ledger_refs[0].clone(),
            },
            ProvenanceSegmentRef {
                stage: "hot_index".to_string(),
                reference: day78.ledger_refs[1].clone(),
            },
            ProvenanceSegmentRef {
                stage: "mirror_export".to_string(),
                reference: day11.ledger_refs[0].clone(),
            },
        ];
        let day12 = kernel
            .run_day12_replay_audit_provenance(
                &db,
                session_id,
                tenant_id,
                &day34.trace_id,
                &segments,
                &[format!("replay:snapshot:{session_id}:{}", day34.trace_id)],
                &[format!("audit:event:{session_id}:{}", day34.trace_id)],
            )
            .await
            .expect("day12");

        let view = crate::observability::query_plane::persist_unified_query_view(
            &db,
            session_id,
            Some(&day34.trace_id),
        )
        .await
        .expect("query view");

        assert!(view.ledger.is_object());
        assert!(view.replay.is_object());
        assert_eq!(day12.ledger_refs.len(), 1);

        let mirror = db
            .get_knowledge(&day11.mirror.export_ref)
            .await
            .expect("db")
            .expect("mirror");
        assert!(mirror.value.contains("\"direction\":\"outbound_only\""));

        let events = crate::observability::event_stream::list_session_events(&db, session_id)
            .await
            .expect("events");
        assert!(
            events
                .iter()
                .any(|event| event.kind == "provenance.lineage.committed")
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn phase1_patch_review_queue_is_materialized() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let _ = kernel
            .run_gateway_recall_patch(
                &db,
                "session:phase1",
                "tenant:phase1",
                "create memory patch",
                "principal:phase1",
            )
            .await
            .expect("run");
        let review = db
            .get_knowledge("memory:patch:review:session:phase1:latest")
            .await
            .expect("db")
            .expect("review");
        assert!(review.value.contains("risk_score"));
        assert!(review.value.contains("status"));
    }

    #[tokio::test]
    async fn phase2_recall_router_prefers_enabled_plugins() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.upsert_json_knowledge(
            "plugin:lifecycle:index".to_string(),
            &serde_json::json!([
                {"plugin_id":"plugin:graph-projection","state":"enabled"},
                {"plugin_id":"plugin:vector-projection","state":"enabled"},
                {"plugin_id":"plugin:search-projection","state":"enabled"},
                {"plugin_id":"plugin:other","state":"disabled"}
            ]),
            "plugin-lifecycle",
        )
        .await
        .expect("seed");

        let route =
            RecallPluginRouter::route(&db, "session:phase2", "tenant:phase2", "find memory")
                .await
                .expect("route");
        assert!(route.pluginized);
        assert!(route.strategy.contains("plugin-recall-router"));
        assert!(
            route
                .plugin_ids
                .iter()
                .any(|id| id == "plugin:graph-projection")
        );
    }

    #[tokio::test]
    async fn phase3_source_view_plane_materializes_mindmap_and_explainer() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let session_id = "session:phase3";
        let run = kernel
            .run_gateway_recall_patch(
                &db,
                session_id,
                "tenant:phase3",
                "materialize source/view plane",
                "principal:phase3",
            )
            .await
            .expect("day3-4");

        let temp =
            std::env::temp_dir().join(format!("autoloop-phase3-view-{}", super::current_time_ms()));
        std::fs::create_dir_all(temp.join("memory")).expect("mkdir memory");
        let changed = "memory/MEMORY.md".to_string();
        std::fs::write(temp.join(&changed), "# Memory\n\n- phase3 source\n").expect("write source");

        let _day78 = kernel
            .run_day78_incremental(
                &db,
                &temp,
                session_id,
                &run.trace_id,
                std::slice::from_ref(&changed),
            )
            .await
            .expect("day7-8");

        let view = kernel
            .run_phase3_source_view_plane(&db, session_id, &run.trace_id)
            .await
            .expect("phase3");
        assert_eq!(view.ledger_refs.len(), 1);
        assert!(view.snapshot.mindmap.nodes.len() >= 2);
        assert!(!view.snapshot.explainer.sections.is_empty());

        let mindmap_latest = db
            .get_knowledge(&format!("memory:view:mindmap:{session_id}:latest"))
            .await
            .expect("db")
            .expect("mindmap latest");
        let explainer_latest = db
            .get_knowledge(&format!("memory:view:explainer:{session_id}:latest"))
            .await
            .expect("db")
            .expect("explainer latest");
        assert!(mindmap_latest.value.contains("memory:view:mindmap"));
        assert!(explainer_latest.value.contains("memory:view:explainer"));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[tokio::test]
    async fn phase4_advanced_governance_materializes_acl_conflict_ontology_formal() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let kernel = GitmemoryCoreKernel::new();
        let session_id = "session:phase4";
        let tenant_id = "tenant:phase4";
        let run = kernel
            .run_gateway_recall_patch(
                &db,
                session_id,
                tenant_id,
                "prepare governance hardening",
                "principal:phase4",
            )
            .await
            .expect("day3-4");

        db.upsert_json_knowledge(
            format!("memory:provenance:{session_id}:{}:latest", run.trace_id),
            &serde_json::json!({
                "lineage_id": "lineage:phase4",
                "trace_id": run.trace_id,
            }),
            "provenance-core",
        )
        .await
        .expect("seed provenance");

        let governance = kernel
            .run_phase4_advanced_governance(
                &db,
                std::path::Path::new("D:\\AutoLoop\\autoloop-app"),
                session_id,
                tenant_id,
                &run.trace_id,
                GovernancePhase::Verify,
                "principal:phase4",
                "write",
                "memory:episodes",
                "low",
            )
            .await
            .expect("phase4 run");

        assert!(governance.allowed);
        assert_eq!(governance.ledger_refs.len(), 1);
        assert!(governance.formal.passed);
        assert!(governance.conflicts.is_empty());
        assert!(!governance.policy_version.is_empty());
        assert!(governance.replay_fp.contains("replay-fp"));

        let governance_record = db
            .get_knowledge(&governance.ledger_refs[0])
            .await
            .expect("db")
            .expect("governance ledger");
        assert!(governance_record.value.contains("governance"));
    }
}

