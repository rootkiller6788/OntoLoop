use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorePromotionDecision {
    Discard,
    LogOnly,
    Localize,
    PromoteRuntimeUpdate,
    PromoteTemplate,
    PromoteGovernanceContract,
    CrystallizeMemoryRule,
    Rollback,
    EscalateHumanReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoreRolloutStage {
    Shadow,
    Canary10,
    Canary30,
    Full,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstitutionAuditRecord {
    pub evidence_ref: String,
    pub audit_source: String,
    pub policy_allow: bool,
    pub policy_version: String,
    pub decision_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionWriteInput {
    pub board_decision: CorePromotionDecision,
    pub rollout_stage: CoreRolloutStage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionWriteDecision {
    pub board_decision: CorePromotionDecision,
    pub rollout_stage: CoreRolloutStage,
    pub policy_allow: bool,
    pub evidence_ref: String,
    pub production_write_allowed: bool,
    pub deny_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionConstitutionState {
    decision: ProductionWriteDecision,
    state_hash: String,
}

impl ExecutionConstitutionState {
    pub fn decision(&self) -> &ProductionWriteDecision {
        &self.decision
    }

    pub fn state_hash(&self) -> &str {
        &self.state_hash
    }
}

pub fn transition_with_audit(
    previous_state_hash: Option<&str>,
    input: &ProductionWriteInput,
    audit: &ConstitutionAuditRecord,
) -> ExecutionConstitutionState {
    let decision_allows = matches!(
        input.board_decision,
        CorePromotionDecision::PromoteRuntimeUpdate
            | CorePromotionDecision::PromoteTemplate
            | CorePromotionDecision::PromoteGovernanceContract
            | CorePromotionDecision::CrystallizeMemoryRule
    );
    let rollout_is_full = matches!(input.rollout_stage, CoreRolloutStage::Full);

    let (production_write_allowed, deny_reason) = if !decision_allows {
        (false, "decision_not_production_promotable")
    } else if !audit.policy_allow {
        (false, "policy_denied")
    } else if audit.evidence_ref.trim().is_empty() {
        (false, "missing_evidence_ref")
    } else if !rollout_is_full {
        (false, "rollout_stage_not_full")
    } else {
        (true, "allowed")
    };

    let decision = ProductionWriteDecision {
        board_decision: input.board_decision.clone(),
        rollout_stage: input.rollout_stage.clone(),
        policy_allow: audit.policy_allow,
        evidence_ref: audit.evidence_ref.clone(),
        production_write_allowed,
        deny_reason: deny_reason.to_string(),
    };

    let state_hash = digest_core_state(previous_state_hash.unwrap_or("genesis"), &decision, audit);

    ExecutionConstitutionState {
        decision,
        state_hash,
    }
}

fn digest_core_state(
    previous_state_hash: &str,
    decision: &ProductionWriteDecision,
    audit: &ConstitutionAuditRecord,
) -> String {
    let payload = format!(
        "core-constitution/v1::{previous_state_hash}::{:?}::{:?}::{}::{}::{}::{}::{}::{}",
        decision.board_decision,
        decision.rollout_stage,
        decision.policy_allow,
        decision.evidence_ref,
        decision.production_write_allowed,
        decision.deny_reason,
        audit.policy_version,
        audit.decision_hash
    );
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in payload.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("constitution:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_requires_full_and_audited_inputs() {
        let audit = ConstitutionAuditRecord {
            evidence_ref: "evidence:1".into(),
            audit_source: "test".into(),
            policy_allow: true,
            policy_version: "policy-v2".into(),
            decision_hash: "hash-1".into(),
        };
        let canary = transition_with_audit(
            None,
            &ProductionWriteInput {
                board_decision: CorePromotionDecision::PromoteGovernanceContract,
                rollout_stage: CoreRolloutStage::Canary10,
            },
            &audit,
        );
        assert!(!canary.decision.production_write_allowed);
        assert_eq!(canary.decision.deny_reason, "rollout_stage_not_full");

        let full = transition_with_audit(
            Some(canary.state_hash()),
            &ProductionWriteInput {
                board_decision: CorePromotionDecision::PromoteGovernanceContract,
                rollout_stage: CoreRolloutStage::Full,
            },
            &audit,
        );
        assert!(full.decision.production_write_allowed);
        assert_eq!(full.decision.deny_reason, "allowed");
        assert_ne!(full.state_hash(), canary.state_hash());
    }
}

