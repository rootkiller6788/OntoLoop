use serde::{Deserialize, Serialize};

use crate::contracts::evolution_os::{PromotionDecision, TrustedPriorSnapshot};
use crate::evolution_os::replay;

const ROLLOUT_REPLAY_SCHEMA_VERSION: &str = "rollout-replay/v1";
const ROLLOUT_REPLAY_SEED_VERSION: &str = "rollout-seed/v1";
const ROLLOUT_REPLAY_VERSION: &str = "rollout-replay-contract/v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RolloutStage {
    Shadow,
    Canary10,
    Canary30,
    Full,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolloutPlan {
    pub prior_id: String,
    pub decision: PromotionDecision,
    pub stage: RolloutStage,
    pub activation_version: String,
    pub rollback_on_failure: bool,
    pub reason: String,
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
pub struct TrustedRolloutEngine;

impl TrustedRolloutEngine {
    pub fn plan(&self, prior: &TrustedPriorSnapshot, decision: PromotionDecision) -> RolloutPlan {
        let stage = match decision {
            PromotionDecision::Discard => RolloutStage::Rollback,
            PromotionDecision::LogOnly | PromotionDecision::Localize => RolloutStage::Shadow,
            PromotionDecision::PromoteRuntimeUpdate
            | PromotionDecision::PromoteTemplate
            | PromotionDecision::PromoteGovernanceContract
            | PromotionDecision::CrystallizeMemoryRule => RolloutStage::Canary10,
            PromotionDecision::Rollback => RolloutStage::Rollback,
            PromotionDecision::EscalateHumanReview => RolloutStage::Shadow,
        };
        let rollback_on_failure = matches!(
            stage,
            RolloutStage::Canary10 | RolloutStage::Canary30 | RolloutStage::Full
        );
        let activation_version = format!(
            "{}::{}::{}",
            crate::contracts::version::EVOLUTION_OS_CONTRACT_VERSION,
            prior.created_at_ms,
            prior.prior_id
        );
        let replay_payload = serde_json::json!({
            "decision": format!("{:?}", decision),
            "stage": format!("{:?}", stage),
            "rollback_on_failure": rollback_on_failure,
            "reason": "rollout plan emitted by trusted rollout engine",
            "prior_context": {
                "decision": format!("{:?}", prior.decision),
                "trusted_boundary_version": prior.trusted_boundary_version,
                "template_refs": prior.template_refs,
                "governance_contract_refs": prior.governance_contract_refs,
                "routing_priors": prior.routing_priors,
                "verifier_priors": prior.verifier_priors,
                "budget_priors": prior.budget_priors,
                "created_by": prior.created_by,
            }
        });
        let replay_fingerprint = replay::build_fingerprint(
            "rolloutfp",
            ROLLOUT_REPLAY_SCHEMA_VERSION,
            ROLLOUT_REPLAY_SEED_VERSION,
            ROLLOUT_REPLAY_VERSION,
            &replay_payload,
        );

        RolloutPlan {
            prior_id: prior.prior_id.clone(),
            decision,
            stage,
            activation_version,
            rollback_on_failure,
            reason: "rollout plan emitted by trusted rollout engine".into(),
            replay_fingerprint,
            replay_schema_version: ROLLOUT_REPLAY_SCHEMA_VERSION.to_string(),
            replay_seed_version: ROLLOUT_REPLAY_SEED_VERSION.to_string(),
            replay_version: ROLLOUT_REPLAY_VERSION.to_string(),
        }
    }

    pub fn should_auto_rollback_on_failure(&self, plan: &RolloutPlan, runtime_gate_pass: bool) -> bool {
        plan.rollback_on_failure && !runtime_gate_pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prior(decision: PromotionDecision) -> TrustedPriorSnapshot {
        TrustedPriorSnapshot {
            prior_id: "prior:test".to_string(),
            based_on_reality_snapshot: "reality:test".to_string(),
            promoted_candidate_id: Some("candidate:test".to_string()),
            decision,
            template_refs: vec![],
            reusable_subgraph_refs: vec![],
            governance_contract_refs: vec![],
            routing_priors: vec![],
            verifier_priors: vec![],
            budget_priors: std::collections::BTreeMap::new(),
            trusted_boundary_version: "v1".to_string(),
            created_by: "test".to_string(),
            created_at_ms: 100,
        }
    }

    #[test]
    fn rollout_canary_failure_triggers_auto_rollback() {
        let engine = TrustedRolloutEngine;
        let plan = engine.plan(&prior(PromotionDecision::PromoteRuntimeUpdate), PromotionDecision::PromoteRuntimeUpdate);
        assert!(matches!(plan.stage, RolloutStage::Canary10));
        assert!(plan.rollback_on_failure);
        assert!(plan.activation_version.starts_with("evolution-os/v1::"));
        assert_eq!(plan.replay_schema_version, "rollout-replay/v1");
        assert_eq!(plan.replay_seed_version, "rollout-seed/v1");
        assert_eq!(plan.replay_version, "rollout-replay-contract/v1");
        assert!(!plan.replay_fingerprint.is_empty());
        assert!(engine.should_auto_rollback_on_failure(&plan, false));
        assert!(!engine.should_auto_rollback_on_failure(&plan, true));
    }
}
