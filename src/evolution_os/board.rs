use serde::{Deserialize, Serialize};

use crate::contracts::evolution_os::{PromotionDecision, RealitySnapshot, WorldlineScore};

use super::proposal_hub::{EvolutionProposal, ProposalKind};

#[derive(Debug, Clone)]
pub struct PromotionBoardInput<'a> {
    pub reality: &'a RealitySnapshot,
    pub proposals: &'a [EvolutionProposal],
    pub scores: &'a [WorldlineScore],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoutStage {
    pub recurring_sources: Vec<(String, usize)>,
    pub best_score: f32,
    pub low_confidence_candidates: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchStage {
    pub shortlisted_ids: Vec<String>,
    pub proposal_only_enforced: bool,
    pub patch_summaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JudgeStage {
    pub policy_compliant: bool,
    pub verifier_supported: bool,
    pub replay_supported: bool,
    pub regression_safe: bool,
    pub max_verifier_confidence: f32,
    pub max_instability_penalty: f32,
    pub avg_risk_penalty: f32,
    pub max_governance_violation_penalty: f32,
    pub verdict_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivistStage {
    pub target_path: String,
    pub apply_immediately: bool,
    pub record_key: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionBoardOutcome {
    pub decision: PromotionDecision,
    pub reason: String,
    pub scout: ScoutStage,
    pub patch: PatchStage,
    pub judge: JudgeStage,
    pub archivist: ArchivistStage,
}

#[derive(Debug, Clone, Default)]
pub struct PromotionGovernanceBoard;

impl PromotionGovernanceBoard {
    pub fn decide(&self, input: PromotionBoardInput<'_>) -> PromotionBoardOutcome {
        let scout = self.run_scout_stage(input.proposals, input.scores);
        let patch = self.run_patch_stage(input.proposals);
        let judge = self.run_judge_stage(input.scores, &patch);
        let (decision, reason) = self.evaluate_decision_matrix(
            input.reality,
            input.proposals,
            &scout,
            &patch,
            &judge,
        );
        let archivist = self.run_archivist_stage(input.reality, &decision, input.proposals);

        PromotionBoardOutcome {
            decision,
            reason,
            scout,
            patch,
            judge,
            archivist,
        }
    }

    fn run_scout_stage(&self, proposals: &[EvolutionProposal], scores: &[WorldlineScore]) -> ScoutStage {
        let mut source_counts = std::collections::BTreeMap::<String, usize>::new();
        for proposal in proposals {
            *source_counts.entry(proposal.source_bus.clone()).or_insert(0) += 1;
        }
        let recurring_sources = source_counts.into_iter().collect::<Vec<_>>();
        let best_score = scores
            .iter()
            .map(|score| score.total_score)
            .fold(f32::NEG_INFINITY, f32::max);
        let best_score = if best_score.is_finite() { best_score } else { 0.0 };
        let low_confidence_candidates = scores
            .iter()
            .filter(|score| score.verifier_confidence < 0.6)
            .count();

        ScoutStage {
            recurring_sources,
            best_score,
            low_confidence_candidates,
            notes: vec![
                "scout clustered proposal sources and confidence hotspots".to_string(),
                "stage is advisory only and does not mutate production state".to_string(),
            ],
        }
    }

    fn run_patch_stage(&self, proposals: &[EvolutionProposal]) -> PatchStage {
        let mut shortlisted = proposals
            .iter()
            .map(|proposal| (proposal.proposal_id.clone(), proposal.expected_lift))
            .collect::<Vec<_>>();
        shortlisted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let shortlisted_ids = shortlisted
            .into_iter()
            .take(5)
            .map(|item| item.0)
            .collect::<Vec<_>>();
        let proposal_only_enforced = proposals.iter().all(|proposal| proposal.proposal_only);
        let patch_summaries = proposals
            .iter()
            .take(5)
            .map(|proposal| format!("{} [{}]", proposal.summary, proposal.source_bus))
            .collect::<Vec<_>>();

        PatchStage {
            shortlisted_ids,
            proposal_only_enforced,
            patch_summaries,
        }
    }

    fn run_judge_stage(&self, scores: &[WorldlineScore], patch: &PatchStage) -> JudgeStage {
        if scores.is_empty() {
            return JudgeStage {
                policy_compliant: patch.proposal_only_enforced,
                verifier_supported: false,
                replay_supported: false,
                regression_safe: false,
                max_verifier_confidence: 0.0,
                max_instability_penalty: 1.0,
                avg_risk_penalty: 1.0,
                max_governance_violation_penalty: 1.0,
                verdict_notes: vec!["judge found no score evidence to evaluate".to_string()],
            };
        }

        let max_verifier = scores
            .iter()
            .map(|score| score.verifier_confidence)
            .fold(0.0_f32, f32::max);
        let replay_risk_max = scores
            .iter()
            .map(|score| score.instability_penalty)
            .fold(0.0_f32, f32::max);
        let risk_avg = scores
            .iter()
            .map(|score| score.risk_penalty)
            .sum::<f32>()
            / scores.len() as f32;
        let governance_violation_max = scores
            .iter()
            .map(|score| score.governance_violation_penalty)
            .fold(0.0_f32, f32::max);

        let verifier_supported = max_verifier >= 0.55;
        let replay_supported = replay_risk_max <= 0.75;
        let regression_safe = risk_avg <= 0.75;
        let policy_compliant = patch.proposal_only_enforced;

        JudgeStage {
            policy_compliant,
            verifier_supported,
            replay_supported,
            regression_safe,
            max_verifier_confidence: max_verifier,
            max_instability_penalty: replay_risk_max,
            avg_risk_penalty: risk_avg,
            max_governance_violation_penalty: governance_violation_max,
            verdict_notes: vec![
                format!("max_verifier_confidence={max_verifier:.3}"),
                format!("max_instability_penalty={replay_risk_max:.3}"),
                format!("avg_risk_penalty={risk_avg:.3}"),
                format!("max_governance_violation_penalty={governance_violation_max:.3}"),
            ],
        }
    }

    fn evaluate_decision_matrix(
        &self,
        reality: &RealitySnapshot,
        proposals: &[EvolutionProposal],
        scout: &ScoutStage,
        patch: &PatchStage,
        judge: &JudgeStage,
    ) -> (PromotionDecision, String) {
        if proposals.is_empty() {
            return (
                PromotionDecision::LogOnly,
                "matrix: no proposals -> LOG_ONLY".to_string(),
            );
        }
        if !patch.proposal_only_enforced || !judge.policy_compliant {
            return (
                PromotionDecision::Discard,
                "matrix: proposal-only guard violated -> DISCARD".to_string(),
            );
        }
        if judge.max_governance_violation_penalty >= 0.35 || judge.max_instability_penalty >= 0.95 {
            return (
                PromotionDecision::Rollback,
                format!(
                    "matrix: governance/instability critical (gov={:.2}, instability={:.2}) -> ROLLBACK",
                    judge.max_governance_violation_penalty, judge.max_instability_penalty
                ),
            );
        }
        if !judge.verifier_supported || !judge.replay_supported {
            return (
                PromotionDecision::EscalateHumanReview,
                "matrix: verifier/replay support insufficient -> ESCALATE".to_string(),
            );
        }
        if scout.best_score < 0.05 {
            return (
                PromotionDecision::LogOnly,
                format!("matrix: best_score {:.3} too low -> LOG_ONLY", scout.best_score),
            );
        }
        if scout.best_score < 0.2 || !judge.regression_safe {
            return (
                PromotionDecision::Localize,
                format!(
                    "matrix: bounded promotion only (best_score={:.3}, regression_safe={}) -> LOCALIZE",
                    scout.best_score, judge.regression_safe
                ),
            );
        }
        if !reality.runtime_mode.eq_ignore_ascii_case("shadow") {
            return (
                PromotionDecision::EscalateHumanReview,
                "matrix: non-shadow runtime requires operator review -> ESCALATE".to_string(),
            );
        }

        let dominant_source = dominant_source_bus(proposals);
        let dominant_kind = dominant_kind(proposals);

        if dominant_source.contains("plugin.lifecycle")
            || matches!(dominant_kind, ProposalKind::VerifierPlacementPatch)
        {
            return (
                PromotionDecision::PromoteGovernanceContract,
                "matrix: plugin/governance-heavy proposal -> PROMOTE_GOVERNANCE_CONTRACT"
                    .to_string(),
            );
        }
        if dominant_source.contains("foundry")
            || matches!(dominant_kind, ProposalKind::PromptPatch | ProposalKind::ReusableSubgraphPatch)
        {
            return (
                PromotionDecision::PromoteTemplate,
                "matrix: foundry/template-ready proposal -> PROMOTE_TEMPLATE".to_string(),
            );
        }
        if dominant_source.contains("patch.review") {
            return (
                PromotionDecision::CrystallizeMemoryRule,
                "matrix: patch review signal dominates -> CRYSTALLIZE_MEMORY_RULE".to_string(),
            );
        }

        (
            PromotionDecision::PromoteRuntimeUpdate,
            "matrix: runtime graph/budget signal dominates -> PROMOTE_RUNTIME_UPDATE".to_string(),
        )
    }

    fn run_archivist_stage(
        &self,
        reality: &RealitySnapshot,
        decision: &PromotionDecision,
        proposals: &[EvolutionProposal],
    ) -> ArchivistStage {
        let target_path = match decision {
            PromotionDecision::PromoteRuntimeUpdate => "9A.runtime_update_queue",
            PromotionDecision::PromoteTemplate => "9B.template_frontier",
            PromotionDecision::PromoteGovernanceContract => "9C.governance_update_queue",
            PromotionDecision::CrystallizeMemoryRule => "9B.memory_rule_crystal",
            PromotionDecision::Localize => "9D.local_only_patch",
            PromotionDecision::LogOnly => "8.board_log_only",
            PromotionDecision::Discard => "8.board_discard",
            PromotionDecision::Rollback => "9D.rollback_queue",
            PromotionDecision::EscalateHumanReview => "8.human_review_queue",
        }
        .to_string();

        ArchivistStage {
            target_path,
            apply_immediately: false,
            record_key: format!(
                "evo:board:{}:{}:{}",
                reality.session_id,
                reality.trace_id,
                proposals.len()
            ),
            notes: vec![
                "archivist records structured board outputs only".to_string(),
                "no direct production mutation is allowed in this stage".to_string(),
            ],
        }
    }
}

fn dominant_source_bus(proposals: &[EvolutionProposal]) -> String {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for proposal in proposals {
        *counts.entry(proposal.source_bus.clone()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1))
        .map(|item| item.0)
        .unwrap_or_else(|| "mod.worldline".to_string())
}

fn dominant_kind(proposals: &[EvolutionProposal]) -> ProposalKind {
    let mut counts = std::collections::BTreeMap::<String, (ProposalKind, usize)>::new();
    for proposal in proposals {
        let key = format!("{:?}", proposal.kind);
        let entry = counts.entry(key).or_insert((proposal.kind.clone(), 0));
        entry.1 += 1;
    }
    counts
        .into_values()
        .max_by(|left, right| left.1.cmp(&right.1))
        .map(|item| item.0)
        .unwrap_or(ProposalKind::GraphPatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_reality(mode: &str) -> RealitySnapshot {
        RealitySnapshot {
            snapshot_id: "reality:test:1".to_string(),
            session_id: "session:test".to_string(),
            trace_id: "trace:test".to_string(),
            tenant_id: "tenant:test".to_string(),
            policy_version: "policy:v1".to_string(),
            runtime_mode: mode.to_string(),
            available_tools: vec![],
            memory_refs: vec![],
            graph_refs: vec![],
            budget_micros: 100_000,
            latency_budget_ms: 3000,
            repo_refs: vec!["repo://test".to_string()],
            policy_refs: vec!["policy:test".to_string()],
            tool_refs: vec![],
            budget_profile: std::collections::BTreeMap::new(),
            repo_digest: "repo:test".to_string(),
            memory_digest: "memory:test".to_string(),
            graph_digest: "graph:test".to_string(),
            policy_digest: "policy:test".to_string(),
            tool_digest: "tool:test".to_string(),
            budget_digest: "budget:test".to_string(),
            reality_fingerprint: "fp:test".to_string(),
            created_at_ms: 1,
        }
    }

    fn base_score() -> WorldlineScore {
        WorldlineScore {
            candidate_id: "g1".to_string(),
            task_success: 0.9,
            robustness: 0.88,
            reuse_gain: 0.73,
            verifier_confidence: 0.84,
            cost_penalty: 0.12,
            latency_penalty: 0.09,
            risk_penalty: 0.2,
            instability_penalty: 0.21,
            governance_violation_penalty: 0.0,
            total_score: 2.03,
            reasons: vec![],
            scored_at_ms: 1,
        }
    }

    fn proposal(source_bus: &str, kind: ProposalKind) -> EvolutionProposal {
        EvolutionProposal {
            proposal_id: format!("proposal:{source_bus}"),
            candidate_id: "g1".to_string(),
            kind,
            source_bus: source_bus.to_string(),
            proposal_only: true,
            summary: "candidate selected".to_string(),
            expected_lift: 1.1,
        }
    }

    #[test]
    fn board_runs_four_structured_stages_and_keeps_proposal_only() {
        let board = PromotionGovernanceBoard;
        let reality = base_reality("shadow");
        let proposals = vec![proposal("mod.worldline", ProposalKind::GraphPatch)];
        let scores = vec![base_score()];

        let outcome = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &proposals,
            scores: &scores,
        });

        assert!(matches!(
            outcome.decision,
            PromotionDecision::PromoteRuntimeUpdate
        ));
        assert!(outcome.patch.proposal_only_enforced);
        assert!(!outcome.patch.shortlisted_ids.is_empty());
        assert!(!outcome.scout.recurring_sources.is_empty());
        assert!(!outcome.judge.verdict_notes.is_empty());
        assert!(!outcome.archivist.apply_immediately);
    }

    #[test]
    fn decision_matrix_promotes_template_for_foundry_dominance() {
        let board = PromotionGovernanceBoard;
        let reality = base_reality("shadow");
        let proposals = vec![proposal("foundry.promotion", ProposalKind::PromptPatch)];
        let scores = vec![base_score()];
        let outcome = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &proposals,
            scores: &scores,
        });
        assert!(matches!(outcome.decision, PromotionDecision::PromoteTemplate));
    }

    #[test]
    fn decision_matrix_promotes_governance_for_plugin_lifecycle_dominance() {
        let board = PromotionGovernanceBoard;
        let reality = base_reality("shadow");
        let proposals = vec![proposal(
            "plugin.lifecycle",
            ProposalKind::VerifierPlacementPatch,
        )];
        let scores = vec![base_score()];
        let outcome = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &proposals,
            scores: &scores,
        });
        assert!(matches!(
            outcome.decision,
            PromotionDecision::PromoteGovernanceContract
        ));
    }

    #[test]
    fn decision_matrix_crystallizes_memory_rule_for_patch_review_dominance() {
        let board = PromotionGovernanceBoard;
        let reality = base_reality("shadow");
        let proposals = vec![proposal("patch.review", ProposalKind::GraphPatch)];
        let scores = vec![base_score()];
        let outcome = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &proposals,
            scores: &scores,
        });
        assert!(matches!(
            outcome.decision,
            PromotionDecision::CrystallizeMemoryRule
        ));
    }

    #[test]
    fn decision_matrix_rolls_back_when_instability_is_critical() {
        let board = PromotionGovernanceBoard;
        let reality = base_reality("shadow");
        let proposals = vec![proposal("mod.worldline", ProposalKind::GraphPatch)];
        let mut score = base_score();
        score.instability_penalty = 0.97;
        let scores = vec![score];
        let outcome = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &proposals,
            scores: &scores,
        });
        assert!(matches!(outcome.decision, PromotionDecision::Rollback));
    }

    #[test]
    fn decision_matrix_joint_policy_replay_regression_gates() {
        let board = PromotionGovernanceBoard;
        let reality = base_reality("shadow");

        let mut policy_broken = proposal("mod.worldline", ProposalKind::GraphPatch);
        policy_broken.proposal_only = false;
        let discard = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &[policy_broken],
            scores: &[base_score()],
        });
        assert!(matches!(discard.decision, PromotionDecision::Discard));

        let mut replay_bad = base_score();
        replay_bad.instability_penalty = 0.80;
        let escalate = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &[proposal("mod.worldline", ProposalKind::GraphPatch)],
            scores: &[replay_bad],
        });
        assert!(matches!(
            escalate.decision,
            PromotionDecision::EscalateHumanReview
        ));

        let mut regression_bad = base_score();
        regression_bad.risk_penalty = 0.88;
        let localize = board.decide(PromotionBoardInput {
            reality: &reality,
            proposals: &[proposal("mod.worldline", ProposalKind::GraphPatch)],
            scores: &[regression_bad],
        });
        assert!(matches!(localize.decision, PromotionDecision::Localize));
    }

    #[test]
    fn decision_matrix_covers_all_nine_decisions() {
        let board = PromotionGovernanceBoard;
        let shadow = base_reality("shadow");
        let active = base_reality("normal");

        let mut seen = std::collections::BTreeSet::new();

        let runtime = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[proposal("mod.worldline", ProposalKind::GraphPatch)],
            scores: &[base_score()],
        });
        seen.insert(format!("{:?}", runtime.decision));

        let template = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[proposal("foundry.promotion", ProposalKind::PromptPatch)],
            scores: &[base_score()],
        });
        seen.insert(format!("{:?}", template.decision));

        let governance = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[proposal("plugin.lifecycle", ProposalKind::VerifierPlacementPatch)],
            scores: &[base_score()],
        });
        seen.insert(format!("{:?}", governance.decision));

        let crystal = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[proposal("patch.review", ProposalKind::GraphPatch)],
            scores: &[base_score()],
        });
        seen.insert(format!("{:?}", crystal.decision));

        let mut rollback_score = base_score();
        rollback_score.instability_penalty = 0.97;
        let rollback = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[proposal("mod.worldline", ProposalKind::GraphPatch)],
            scores: &[rollback_score],
        });
        seen.insert(format!("{:?}", rollback.decision));

        let mut discard_proposal = proposal("mod.worldline", ProposalKind::GraphPatch);
        discard_proposal.proposal_only = false;
        let discard = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[discard_proposal],
            scores: &[base_score()],
        });
        seen.insert(format!("{:?}", discard.decision));

        let log_only = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[],
            scores: &[base_score()],
        });
        seen.insert(format!("{:?}", log_only.decision));

        let mut localize_score = base_score();
        localize_score.risk_penalty = 0.90;
        let localize = board.decide(PromotionBoardInput {
            reality: &shadow,
            proposals: &[proposal("mod.worldline", ProposalKind::GraphPatch)],
            scores: &[localize_score],
        });
        seen.insert(format!("{:?}", localize.decision));

        let escalate = board.decide(PromotionBoardInput {
            reality: &active,
            proposals: &[proposal("mod.worldline", ProposalKind::GraphPatch)],
            scores: &[base_score()],
        });
        seen.insert(format!("{:?}", escalate.decision));

        let expected = std::collections::BTreeSet::from([
            "Discard".to_string(),
            "LogOnly".to_string(),
            "Localize".to_string(),
            "PromoteRuntimeUpdate".to_string(),
            "PromoteTemplate".to_string(),
            "PromoteGovernanceContract".to_string(),
            "CrystallizeMemoryRule".to_string(),
            "Rollback".to_string(),
            "EscalateHumanReview".to_string(),
        ]);
        assert_eq!(seen, expected);
    }
}


