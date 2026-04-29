use std::collections::BTreeMap;

use crate::contracts::evolution_os::{PromotionDecision, RealitySnapshot, TrustedPriorSnapshot};

use super::board::PromotionBoardOutcome;

#[derive(Debug, Clone, Default)]
pub struct CanonicalRealityResynthesizer;

impl CanonicalRealityResynthesizer {
    pub fn synthesize(
        &self,
        reality: &RealitySnapshot,
        board: &PromotionBoardOutcome,
        template_refs: Vec<String>,
    ) -> TrustedPriorSnapshot {
        TrustedPriorSnapshot {
            prior_id: format!("trusted-prior:{}:{}", reality.session_id, reality.created_at_ms),
            based_on_reality_snapshot: reality.snapshot_id.clone(),
            promoted_candidate_id: None,
            decision: board.decision.clone(),
            template_refs,
            reusable_subgraph_refs: vec!["subgraph:baseline:v1".into()],
            governance_contract_refs: vec![format!("contract:{}", reality.policy_version)],
            routing_priors: vec!["routing:governed-default".into()],
            verifier_priors: vec!["verifier:strict-default".into()],
            budget_priors: BTreeMap::from([("default".to_string(), reality.budget_micros)]),
            trusted_boundary_version: format!(
                "{}::{}",
                "trusted-boundary-v1",
                crate::contracts::version::EVOLUTION_OS_CONTRACT_VERSION
            ),
            created_by: "evolution-os".into(),
            created_at_ms: reality.created_at_ms,
        }
    }
}

#[allow(dead_code)]
fn _decision_hint(decision: &PromotionDecision) -> &'static str {
    match decision {
        PromotionDecision::Discard => "discard",
        PromotionDecision::LogOnly => "log-only",
        PromotionDecision::Localize => "localize",
        PromotionDecision::PromoteRuntimeUpdate => "promote-runtime",
        PromotionDecision::PromoteTemplate => "promote-template",
        PromotionDecision::PromoteGovernanceContract => "promote-governance",
        PromotionDecision::CrystallizeMemoryRule => "crystallize-memory",
        PromotionDecision::Rollback => "rollback",
        PromotionDecision::EscalateHumanReview => "escalate-human",
    }
}
