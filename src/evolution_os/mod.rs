use anyhow::Result;
use serde_json::{Value, json};

use crate::contracts::evolution_os::{
    CandidateGraph, PromotionDecision, RealitySnapshot, TrustedPriorSnapshot, WorldlineScore,
};

pub mod board;
pub mod graph_compiler;
pub mod ingest;
pub mod path_executor;
pub mod prior;
pub mod proposal_hub;
pub mod replay;
pub mod resynthesis;
pub mod rollout;
pub mod worldline;

pub use board::{PromotionBoardInput, PromotionBoardOutcome, PromotionGovernanceBoard};
pub use graph_compiler::{DynamicRuntimeGraphCompiler, GraphCompileInput};
pub use ingest::{CanonicalRealityIngestor, IngestInput};
pub use path_executor::{PathExecutionPlan, PromotionPath, PromotionPathExecutor};
pub use prior::{CrystalPriorDecision, CrystalPriorLayer, CrystalPriorSelection, PriorHitReason};
pub use proposal_hub::{
    ControlledProposalHub, EvolutionProposal, ExternalProposalSignals, ProposalKind,
    ProposalSelectionInput,
};
pub use resynthesis::CanonicalRealityResynthesizer;
pub use rollout::{RolloutPlan, RolloutStage, TrustedRolloutEngine};
pub use worldline::{
    TelemetryReplaySnapshot, WorldlineEvaluator, WorldlineRecommendation, WorldlineWeights,
};

const EVIDENCE_REPLAY_SCHEMA_VERSION: &str = "evolution-evidence-replay/v1";
const EVIDENCE_REPLAY_SEED_VERSION: &str = "evolution-evidence-seed/v1";
const EVIDENCE_REPLAY_VERSION: &str = "evolution-evidence-replay-contract/v1";
const REPLAY_CHAIN_SCHEMA_VERSION: &str = "evolution-replay-chain/v1";
const REPLAY_CHAIN_SEED_VERSION: &str = "evolution-replay-chain-seed/v1";
const REPLAY_CHAIN_VERSION: &str = "evolution-replay-chain-contract/v1";

#[derive(Debug, Clone)]
pub struct EvolutionOsKernel {
    pub ingest: CanonicalRealityIngestor,
    pub priors: CrystalPriorLayer,
    pub graph_compiler: DynamicRuntimeGraphCompiler,
    pub worldline: WorldlineEvaluator,
    pub proposals: ControlledProposalHub,
    pub board: PromotionGovernanceBoard,
    pub path_executor: PromotionPathExecutor,
    pub resynthesis: CanonicalRealityResynthesizer,
    pub rollout: TrustedRolloutEngine,
}

impl Default for EvolutionOsKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl EvolutionOsKernel {
    pub fn new() -> Self {
        Self {
            ingest: CanonicalRealityIngestor::default(),
            priors: CrystalPriorLayer::default(),
            graph_compiler: DynamicRuntimeGraphCompiler::default(),
            worldline: WorldlineEvaluator::default(),
            proposals: ControlledProposalHub::default(),
            board: PromotionGovernanceBoard::default(),
            path_executor: PromotionPathExecutor::default(),
            resynthesis: CanonicalRealityResynthesizer::default(),
            rollout: TrustedRolloutEngine::default(),
        }
    }

    pub fn run_shadow_cycle(&self, input: IngestInput) -> Result<EvolutionShadowCycle> {
        let telemetry_replay = input.telemetry_replay.clone();
        let proposal_signals = input.proposal_signals.clone().unwrap_or_default();
        let reality = self.ingest.ingest(input)?;
        let prior_decision = self.priors.select_with_trace(&reality);
        let priors = prior_decision.selection.clone();
        let candidates = self.graph_compiler.compile(GraphCompileInput {
            reality: &reality,
            priors: &priors,
        });

        let mut scores = Vec::<WorldlineScore>::new();
        for candidate in &candidates {
            scores.push(
                self.worldline
                    .score_with_snapshot(candidate, telemetry_replay.as_ref()),
            );
        }

        let recommendation = self.worldline.recommend(&scores, telemetry_replay.as_ref());

        let proposals = self.proposals.select(ProposalSelectionInput {
            reality: &reality,
            candidates: &candidates,
            scores: &scores,
            external_signals: Some(&proposal_signals),
        });

        let board_outcome = self.board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &proposals,
            scores: &scores,
        });

        let mut trusted_prior = self
            .resynthesis
            .synthesize(&reality, &board_outcome, priors.template_refs.clone());
        trusted_prior.promoted_candidate_id = recommendation
            .as_ref()
            .map(|item| item.recommended_candidate_id.clone());

        let rollout = self
            .rollout
            .plan(&trusted_prior, board_outcome.decision.clone());
        let path_plan = self
            .path_executor
            .plan(&trusted_prior, &board_outcome, &proposals);

        Ok(EvolutionShadowCycle {
            reality,
            prior_decision,
            candidates,
            scores,
            recommendation,
            proposal_signals,
            proposals,
            board_decision: board_outcome.decision.clone(),
            board_outcome,
            path_plan,
            trusted_prior,
            rollout,
        })
    }
}

#[derive(Debug, Clone)]
pub struct EvolutionShadowCycle {
    pub reality: RealitySnapshot,
    pub prior_decision: CrystalPriorDecision,
    pub candidates: Vec<CandidateGraph>,
    pub scores: Vec<WorldlineScore>,
    pub recommendation: Option<WorldlineRecommendation>,
    pub proposal_signals: ExternalProposalSignals,
    pub proposals: Vec<EvolutionProposal>,
    pub board_decision: PromotionDecision,
    pub board_outcome: PromotionBoardOutcome,
    pub path_plan: PathExecutionPlan,
    pub trusted_prior: TrustedPriorSnapshot,
    pub rollout: RolloutPlan,
}

impl EvolutionShadowCycle {
    pub fn to_evidence_json(
        &self,
        entrypoint: &str,
        prompt_excerpt: &str,
        outcome: &str,
        outcome_detail: &str,
    ) -> Value {
        let base = json!({
            "entrypoint": entrypoint,
            "outcome": outcome,
            "outcome_detail": outcome_detail,
            "prompt_excerpt": prompt_excerpt.chars().take(220).collect::<String>(),
            "reality": self.reality,
            "prior_decision": self.prior_decision,
            "candidates": self.candidates,
            "scores": self.scores,
            "recommendation": self.recommendation,
            "proposal_signals": self.proposal_signals,
            "proposals": self.proposals,
            "board_decision": self.board_decision,
            "board_outcome": self.board_outcome,
            "path_plan": self.path_plan,
            "trusted_prior": self.trusted_prior,
            "rollout": self.rollout,
        });
        let evidence_replay_payload = json!({
            "entrypoint": entrypoint,
            "outcome": outcome,
            "outcome_detail": outcome_detail,
            "prompt_excerpt": prompt_excerpt.chars().take(220).collect::<String>(),
            "board_decision": format!("{:?}", self.board_decision),
            "components": {
                "reality_fingerprint": self.reality.reality_fingerprint,
                "prior_fingerprint": self.prior_decision.replay_fingerprint,
                "path_fingerprint": self.path_plan.replay_fingerprint,
                "rollout_fingerprint": self.rollout.replay_fingerprint,
            }
        });
        let evidence_replay_fingerprint = crate::evolution_os::replay::build_fingerprint(
            "evidencefp",
            EVIDENCE_REPLAY_SCHEMA_VERSION,
            EVIDENCE_REPLAY_SEED_VERSION,
            EVIDENCE_REPLAY_VERSION,
            &evidence_replay_payload,
        );
        let replay_chain_fingerprint = crate::evolution_os::replay::build_chain_fingerprint(
            REPLAY_CHAIN_SCHEMA_VERSION,
            REPLAY_CHAIN_SEED_VERSION,
            REPLAY_CHAIN_VERSION,
            &[
                ("reality", &self.reality.reality_fingerprint),
                ("prior", &self.prior_decision.replay_fingerprint),
                ("path", &self.path_plan.replay_fingerprint),
                ("rollout", &self.rollout.replay_fingerprint),
                ("evidence", &evidence_replay_fingerprint),
            ],
        );

        let mut enriched = base;
        if let Value::Object(ref mut map) = enriched {
            map.insert(
                "replay_contract".to_string(),
                json!({
                    "evidence": {
                        "schema_version": EVIDENCE_REPLAY_SCHEMA_VERSION,
                        "seed_version": EVIDENCE_REPLAY_SEED_VERSION,
                        "replay_version": EVIDENCE_REPLAY_VERSION,
                        "fingerprint": evidence_replay_fingerprint,
                    },
                    "components": {
                        "reality_fingerprint": self.reality.reality_fingerprint,
                        "prior_fingerprint": self.prior_decision.replay_fingerprint,
                        "prior_schema_version": self.prior_decision.replay_schema_version,
                        "prior_seed_version": self.prior_decision.replay_seed_version,
                        "prior_replay_version": self.prior_decision.replay_version,
                        "path_fingerprint": self.path_plan.replay_fingerprint,
                        "path_schema_version": self.path_plan.replay_schema_version,
                        "path_seed_version": self.path_plan.replay_seed_version,
                        "path_replay_version": self.path_plan.replay_version,
                        "rollout_fingerprint": self.rollout.replay_fingerprint,
                        "rollout_schema_version": self.rollout.replay_schema_version,
                        "rollout_seed_version": self.rollout.replay_seed_version,
                        "rollout_replay_version": self.rollout.replay_version,
                    },
                    "chain": {
                        "schema_version": REPLAY_CHAIN_SCHEMA_VERSION,
                        "seed_version": REPLAY_CHAIN_SEED_VERSION,
                        "replay_version": REPLAY_CHAIN_VERSION,
                        "fingerprint": replay_chain_fingerprint,
                    },
                    "drift_explainer": [
                        crate::evolution_os::replay::default_version_drift_explainer("prior", &self.prior_decision.replay_version),
                        crate::evolution_os::replay::default_version_drift_explainer("path", &self.path_plan.replay_version),
                        crate::evolution_os::replay::default_version_drift_explainer("rollout", &self.rollout.replay_version),
                        crate::evolution_os::replay::default_version_drift_explainer("evidence", EVIDENCE_REPLAY_VERSION),
                    ],
                }),
            );
        }
        enriched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(now_ms: u64) -> IngestInput {
        IngestInput {
            session_id: "session:evo-test".into(),
            trace_id: "trace:evo-test".into(),
            tenant_id: "tenant:evo".into(),
            policy_version: "policy-v2".into(),
            runtime_mode: "shadow".into(),
            available_tools: vec!["tool:planner".into(), "tool:verifier".into()],
            memory_refs: vec!["memory:latest".into()],
            graph_refs: vec!["graph:latest".into()],
            repo_refs: vec!["repo://autoloop-app".into()],
            policy_refs: vec!["policy:tenant-evo:default".into()],
            tool_refs: vec![],
            budget_micros: 100_000,
            latency_budget_ms: 3_000,
            budget_profile: std::collections::BTreeMap::new(),
            now_ms,
            telemetry_replay: Some(TelemetryReplaySnapshot {
                verifier_score: 0.81,
                provider_retry_count: 1,
                tool_retry_count: 1,
                replay_mismatch_rate: 0.12,
                deterministic_boundary_respected: true,
                latency_p95_ms: 1400,
                worldline_weights: None,
                weights_version: None,
            }),
            proposal_signals: Some(ExternalProposalSignals {
                foundry_promotion_hints: vec![serde_json::json!({
                    "hint_id": "hint-test",
                    "reason": "test foundry hint",
                })],
                patch_reviews: vec![serde_json::json!({
                    "review_id": "review-test",
                    "status": "queued",
                    "decision": {"risk_score": 0.7}
                })],
                plugin_lifecycle_updates: vec![serde_json::json!({
                    "plugin_id": "plugin:test",
                    "state": "enabled",
                    "verified": true
                })],
            }),
        }
    }

    #[test]
    fn shadow_cycle_builds_full_pipeline_outputs() {
        let kernel = EvolutionOsKernel::new();
        let cycle = kernel
            .run_shadow_cycle(sample_input(1_710_000_000_000))
            .expect("shadow cycle");

        assert!(!cycle.candidates.is_empty());
        assert_eq!(cycle.candidates.len(), cycle.scores.len());
        assert!(!cycle.trusted_prior.prior_id.is_empty());
        assert_eq!(cycle.prior_decision.catalog_version, "prior-catalog:v1");
        assert!(!cycle.prior_decision.replay_fingerprint.is_empty());
        assert!(
            cycle
                .prior_decision
                .hit_reasons
                .iter()
                .any(|item| item.domain == "promotion_policy")
        );
        assert!(!cycle.reality.reality_fingerprint.is_empty());
        assert!(!cycle.reality.repo_digest.is_empty());
        assert!(!cycle.reality.memory_digest.is_empty());
        assert!(!cycle.reality.graph_digest.is_empty());
        assert!(!cycle.reality.policy_digest.is_empty());
        assert!(!cycle.reality.tool_digest.is_empty());
        assert!(!cycle.reality.budget_digest.is_empty());
        assert!(
            cycle.trusted_prior.promoted_candidate_id.is_some(),
            "trusted prior snapshot should carry promoted candidate id"
        );
        assert!(cycle.recommendation.is_some());
        assert!(
            cycle
                .proposals
                .iter()
                .any(|item| item.source_bus == "foundry.promotion")
        );
        assert!(
            cycle.board_outcome.patch.proposal_only_enforced,
            "board should preserve proposal-only semantics"
        );
        assert!(
            !cycle.path_plan.actions.is_empty(),
            "path executor should emit at least one action"
        );
        let evidence = cycle.to_evidence_json("process_requirement_swarm", "prompt", "success", "ok");
        assert_eq!(evidence.get("entrypoint").and_then(Value::as_str), Some("process_requirement_swarm"));
        assert!(evidence.get("recommendation").is_some());
        assert!(evidence.get("board_outcome").is_some());
        assert!(evidence.get("path_plan").is_some());
        assert!(evidence.get("replay_contract").is_some());
    }

    #[test]
    fn full_chain_replay_fingerprint_stable_for_same_input_version() {
        let kernel = EvolutionOsKernel::new();
        let first = kernel
            .run_shadow_cycle(sample_input(1_710_000_000_100))
            .expect("first cycle");
        let second = kernel
            .run_shadow_cycle(sample_input(1_710_000_000_900))
            .expect("second cycle");
        let first_evidence = first.to_evidence_json("process_direct", "prompt", "ok", "stable");
        let second_evidence = second.to_evidence_json("process_direct", "prompt", "ok", "stable");

        assert_eq!(
            first.prior_decision.replay_fingerprint,
            second.prior_decision.replay_fingerprint
        );
        assert_eq!(first.path_plan.replay_fingerprint, second.path_plan.replay_fingerprint);
        assert_eq!(first.rollout.replay_fingerprint, second.rollout.replay_fingerprint);
        assert_eq!(
            first_evidence["replay_contract"]["chain"]["fingerprint"],
            second_evidence["replay_contract"]["chain"]["fingerprint"]
        );
    }

    #[test]
    fn version_change_produces_explainable_replay_drift() {
        let payload = serde_json::json!({"component":"prior","policy":"v2"});
        let first = crate::evolution_os::replay::build_fingerprint(
            "priorfp",
            "prior-replay/v1",
            "prior-seed/v1",
            "prior-replay-contract/v1",
            &payload,
        );
        let second = crate::evolution_os::replay::build_fingerprint(
            "priorfp",
            "prior-replay/v1",
            "prior-seed/v1",
            "prior-replay-contract/v2",
            &payload,
        );
        assert_ne!(first, second);
        assert!(
            crate::evolution_os::replay::default_version_drift_explainer(
                "prior",
                "prior-replay-contract/v2"
            )
            .contains("replay_version")
        );
    }
}




