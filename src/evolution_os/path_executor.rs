use serde::{Deserialize, Serialize};

use crate::contracts::evolution_os::PromotionDecision;
use crate::evolution_os::replay;

use super::{EvolutionProposal, PromotionBoardOutcome, TrustedPriorSnapshot};

const PATH_REPLAY_SCHEMA_VERSION: &str = "path-replay/v1";
const PATH_REPLAY_SEED_VERSION: &str = "path-seed/v1";
const PATH_REPLAY_VERSION: &str = "path-replay-contract/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum PromotionPath {
    Path9A_RuntimeUpdate,
    Path9B_Crystalization,
    Path9C_GovernanceUpdate,
    Path9D_LocalOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathAction {
    pub executor: String,
    pub operation: String,
    pub target: String,
    pub proposal_only: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathExecutionPlan {
    pub selected_path: PromotionPath,
    pub decision: PromotionDecision,
    pub actions: Vec<PathAction>,
    #[serde(default)]
    pub replay_fingerprint: String,
    #[serde(default)]
    pub replay_schema_version: String,
    #[serde(default)]
    pub replay_seed_version: String,
    #[serde(default)]
    pub replay_version: String,
}

#[derive(Debug, Clone, Default)]
pub struct PromotionPathExecutor;

impl PromotionPathExecutor {
    pub fn expected_path_for_decision(decision: &PromotionDecision) -> PromotionPath {
        match decision {
            PromotionDecision::PromoteRuntimeUpdate => PromotionPath::Path9A_RuntimeUpdate,
            PromotionDecision::PromoteTemplate | PromotionDecision::CrystallizeMemoryRule => {
                PromotionPath::Path9B_Crystalization
            }
            PromotionDecision::PromoteGovernanceContract => PromotionPath::Path9C_GovernanceUpdate,
            PromotionDecision::Discard
            | PromotionDecision::LogOnly
            | PromotionDecision::Localize
            | PromotionDecision::Rollback
            | PromotionDecision::EscalateHumanReview => PromotionPath::Path9D_LocalOnly,
        }
    }

    pub fn path_code(path: &PromotionPath) -> &'static str {
        match path {
            PromotionPath::Path9A_RuntimeUpdate => "9A",
            PromotionPath::Path9B_Crystalization => "9B",
            PromotionPath::Path9C_GovernanceUpdate => "9C",
            PromotionPath::Path9D_LocalOnly => "9D",
        }
    }

    pub fn plan(
        &self,
        prior: &TrustedPriorSnapshot,
        board: &PromotionBoardOutcome,
        proposals: &[EvolutionProposal],
    ) -> PathExecutionPlan {
        let selected_path = Self::expected_path_for_decision(&board.decision);

        let mut actions = Vec::<PathAction>::new();
        match selected_path {
            PromotionPath::Path9A_RuntimeUpdate => {
                let plugin_target = proposals
                    .iter()
                    .find(|proposal| proposal.source_bus == "plugin.lifecycle")
                    .map(|proposal| proposal.candidate_id.clone())
                    .unwrap_or_else(|| "plugin:graph-projection".to_string());
                actions.push(PathAction {
                    executor: "plugins.lifecycle".to_string(),
                    operation: "rollout_canary10".to_string(),
                    target: plugin_target,
                    proposal_only: true,
                    notes: vec![
                        format!("prior_ref={}", prior.prior_id),
                        "runtime path uses lifecycle rollout surface".to_string(),
                    ],
                });
            }
            PromotionPath::Path9B_Crystalization => {
                actions.push(PathAction {
                    executor: "gitmemory.patch_review_queue".to_string(),
                    operation: "enqueue_patch_review".to_string(),
                    target: "memory:patch:review".to_string(),
                    proposal_only: true,
                    notes: vec![
                        "crystalization path enqueues patch plan for approval".to_string(),
                        format!("board_target={}", board.archivist.target_path),
                    ],
                });
                actions.push(PathAction {
                    executor: "gitmemory.patch_review_queue".to_string(),
                    operation: "await_approve_or_reject".to_string(),
                    target: "memory:patch:review:*".to_string(),
                    proposal_only: true,
                    notes: vec![
                        "closure step enforces explicit review decision before apply".to_string(),
                    ],
                });
                actions.push(PathAction {
                    executor: "gitmemory.patch_review_queue".to_string(),
                    operation: "apply_if_approved".to_string(),
                    target: "memory:patch:apply".to_string(),
                    proposal_only: true,
                    notes: vec![
                        "apply step is gated by approved review status only".to_string(),
                    ],
                });
            }
            PromotionPath::Path9C_GovernanceUpdate => {
                actions.push(PathAction {
                    executor: "governance.config_surface".to_string(),
                    operation: "propose_policy_patch".to_string(),
                    target: "policy:evolution:governance".to_string(),
                    proposal_only: true,
                    notes: vec![
                        format!("contract_refs={}", prior.governance_contract_refs.len()),
                        "governance update stays proposal-only before approval".to_string(),
                    ],
                });
                actions.push(PathAction {
                    executor: "governance.config_surface".to_string(),
                    operation: "version_governance_config".to_string(),
                    target: "policy:evolution:governance:version:*".to_string(),
                    proposal_only: true,
                    notes: vec![
                        "versioned governance record allows deterministic replay and rollback"
                            .to_string(),
                    ],
                });
                actions.push(PathAction {
                    executor: "governance.config_surface".to_string(),
                    operation: "activate_after_canary".to_string(),
                    target: "policy:evolution:governance:active".to_string(),
                    proposal_only: true,
                    notes: vec![
                        "activation remains gated by rollout stage and governance approval"
                            .to_string(),
                    ],
                });
            }
            PromotionPath::Path9D_LocalOnly => {
                actions.push(PathAction {
                    executor: "local.experiment".to_string(),
                    operation: "record_localized_experiment".to_string(),
                    target: "settings.local:evolution".to_string(),
                    proposal_only: true,
                    notes: vec![
                        "local-only path avoids org-wide promotion".to_string(),
                        format!("decision={:?}", board.decision),
                    ],
                });
                actions.push(PathAction {
                    executor: "local.experiment".to_string(),
                    operation: "isolate_local_scope".to_string(),
                    target: "settings.local:evolution:isolation".to_string(),
                    proposal_only: true,
                    notes: vec![
                        "local-only changes are isolated from org-wide runtime state".to_string(),
                    ],
                });
                actions.push(PathAction {
                    executor: "local.experiment".to_string(),
                    operation: "prepare_local_rollback".to_string(),
                    target: "settings.local:evolution:rollback".to_string(),
                    proposal_only: true,
                    notes: vec!["rollback contract is prepared for fast local revert".to_string()],
                });
            }
        }

        let replay_payload = serde_json::json!({
            "selected_path": Self::path_code(&selected_path),
            "decision": format!("{:?}", board.decision),
            "proposal_count": proposals.len(),
            "actions": actions.iter().map(|action| serde_json::json!({
                "executor": action.executor,
                "operation": action.operation,
                "target": action.target,
                "proposal_only": action.proposal_only,
                "notes": action.notes.iter().filter(|note| {
                    !note.starts_with("prior_ref=")
                }).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "prior_context": {
                "decision": format!("{:?}", prior.decision),
                "trusted_boundary_version": prior.trusted_boundary_version,
                "template_refs": prior.template_refs,
                "governance_contract_refs": prior.governance_contract_refs,
                "routing_priors": prior.routing_priors,
                "verifier_priors": prior.verifier_priors,
            }
        });
        let replay_fingerprint = replay::build_fingerprint(
            "pathfp",
            PATH_REPLAY_SCHEMA_VERSION,
            PATH_REPLAY_SEED_VERSION,
            PATH_REPLAY_VERSION,
            &replay_payload,
        );

        PathExecutionPlan {
            selected_path,
            decision: board.decision.clone(),
            actions,
            replay_fingerprint,
            replay_schema_version: PATH_REPLAY_SCHEMA_VERSION.to_string(),
            replay_seed_version: PATH_REPLAY_SEED_VERSION.to_string(),
            replay_version: PATH_REPLAY_VERSION.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::evolution_os::{PromotionDecision, TrustedPriorSnapshot};

    fn prior(decision: PromotionDecision) -> TrustedPriorSnapshot {
        TrustedPriorSnapshot {
            prior_id: "prior:test".to_string(),
            based_on_reality_snapshot: "reality:test".to_string(),
            promoted_candidate_id: None,
            decision,
            template_refs: vec![],
            reusable_subgraph_refs: vec![],
            governance_contract_refs: vec!["contract:test".to_string()],
            routing_priors: vec![],
            verifier_priors: vec![],
            budget_priors: std::collections::BTreeMap::new(),
            trusted_boundary_version: "v1".to_string(),
            created_by: "test".to_string(),
            created_at_ms: 1,
        }
    }

    fn board_outcome(decision: PromotionDecision) -> PromotionBoardOutcome {
        PromotionBoardOutcome {
            decision,
            reason: "test".to_string(),
            scout: crate::evolution_os::board::ScoutStage {
                recurring_sources: vec![],
                best_score: 1.0,
                low_confidence_candidates: 0,
                notes: vec![],
            },
            patch: crate::evolution_os::board::PatchStage {
                shortlisted_ids: vec!["proposal".to_string()],
                proposal_only_enforced: true,
                patch_summaries: vec![],
            },
            judge: crate::evolution_os::board::JudgeStage {
                policy_compliant: true,
                verifier_supported: true,
                replay_supported: true,
                regression_safe: true,
                max_verifier_confidence: 0.8,
                max_instability_penalty: 0.2,
                avg_risk_penalty: 0.2,
                max_governance_violation_penalty: 0.0,
                verdict_notes: vec![],
            },
            archivist: crate::evolution_os::board::ArchivistStage {
                target_path: "9A.runtime_update_queue".to_string(),
                apply_immediately: false,
                record_key: "evo:test".to_string(),
                notes: vec![],
            },
        }
    }

    #[test]
    fn expected_path_for_all_decisions_is_stable() {
        let expectations = vec![
            (PromotionDecision::Discard, PromotionPath::Path9D_LocalOnly),
            (PromotionDecision::LogOnly, PromotionPath::Path9D_LocalOnly),
            (PromotionDecision::Localize, PromotionPath::Path9D_LocalOnly),
            (
                PromotionDecision::PromoteRuntimeUpdate,
                PromotionPath::Path9A_RuntimeUpdate,
            ),
            (PromotionDecision::PromoteTemplate, PromotionPath::Path9B_Crystalization),
            (
                PromotionDecision::PromoteGovernanceContract,
                PromotionPath::Path9C_GovernanceUpdate,
            ),
            (
                PromotionDecision::CrystallizeMemoryRule,
                PromotionPath::Path9B_Crystalization,
            ),
            (PromotionDecision::Rollback, PromotionPath::Path9D_LocalOnly),
            (
                PromotionDecision::EscalateHumanReview,
                PromotionPath::Path9D_LocalOnly,
            ),
        ];

        for (decision, expected_path) in expectations {
            let actual = PromotionPathExecutor::expected_path_for_decision(&decision);
            assert_eq!(actual, expected_path);
            assert!(
                ["9A", "9B", "9C", "9D"].contains(&PromotionPathExecutor::path_code(&actual)),
                "path code must stay within 9A/9B/9C/9D"
            );
        }
    }
    #[test]
    fn path_executor_maps_decisions_to_9a_9b_9c_9d() {
        let exec = PromotionPathExecutor;
        let proposals = vec![EvolutionProposal {
            proposal_id: "proposal:plugin".to_string(),
            candidate_id: "plugin:graph-projection".to_string(),
            kind: crate::evolution_os::proposal_hub::ProposalKind::GraphPatch,
            source_bus: "plugin.lifecycle".to_string(),
            proposal_only: true,
            summary: "x".to_string(),
            expected_lift: 1.0,
        }];

        let p9a = exec.plan(
            &prior(PromotionDecision::PromoteRuntimeUpdate),
            &board_outcome(PromotionDecision::PromoteRuntimeUpdate),
            &proposals,
        );
        assert_eq!(p9a.selected_path, PromotionPath::Path9A_RuntimeUpdate);
        assert_eq!(p9a.replay_schema_version, "path-replay/v1");
        assert_eq!(p9a.replay_seed_version, "path-seed/v1");
        assert_eq!(p9a.replay_version, "path-replay-contract/v1");
        assert!(!p9a.replay_fingerprint.is_empty());

        let p9b = exec.plan(
            &prior(PromotionDecision::PromoteTemplate),
            &board_outcome(PromotionDecision::PromoteTemplate),
            &proposals,
        );
        assert_eq!(p9b.selected_path, PromotionPath::Path9B_Crystalization);
        let p9b_ops = p9b
            .actions
            .iter()
            .map(|action| action.operation.as_str())
            .collect::<Vec<_>>();
        assert!(p9b_ops.contains(&"enqueue_patch_review"));
        assert!(p9b_ops.contains(&"await_approve_or_reject"));
        assert!(p9b_ops.contains(&"apply_if_approved"));

        let p9c = exec.plan(
            &prior(PromotionDecision::PromoteGovernanceContract),
            &board_outcome(PromotionDecision::PromoteGovernanceContract),
            &proposals,
        );
        assert_eq!(p9c.selected_path, PromotionPath::Path9C_GovernanceUpdate);
        let p9c_ops = p9c
            .actions
            .iter()
            .map(|action| action.operation.as_str())
            .collect::<Vec<_>>();
        assert!(p9c_ops.contains(&"propose_policy_patch"));
        assert!(p9c_ops.contains(&"version_governance_config"));
        assert!(p9c_ops.contains(&"activate_after_canary"));

        let p9d = exec.plan(
            &prior(PromotionDecision::LogOnly),
            &board_outcome(PromotionDecision::LogOnly),
            &proposals,
        );
        assert_eq!(p9d.selected_path, PromotionPath::Path9D_LocalOnly);
        let p9d_ops = p9d
            .actions
            .iter()
            .map(|action| action.operation.as_str())
            .collect::<Vec<_>>();
        assert!(p9d_ops.contains(&"record_localized_experiment"));
        assert!(p9d_ops.contains(&"isolate_local_scope"));
        assert!(p9d_ops.contains(&"prepare_local_rollback"));
        assert!(!p9d.replay_fingerprint.is_empty());
    }
}


