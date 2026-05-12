use anyhow::{Result, bail};

use crate::contracts::org::RequirementSpec;
use crate::orchestration::RequirementBrief;

pub use super::{build_org_session, seed_review_result};

#[derive(Debug, Clone, Default)]
pub struct LeadAnalyst;

impl LeadAnalyst {
    pub fn produce_requirement_spec(
        session_id: &str,
        brief: &RequirementBrief,
        risk_level: &str,
        evidence_ref: &str,
    ) -> Result<RequirementSpec> {
        if evidence_ref.trim().is_empty() {
            bail!("lead analyst requires non-empty evidence_ref");
        }
        Ok(RequirementSpec {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            task_id: format!("task:{session_id}"),
            goal: brief.clarified_goal.clone(),
            scope: brief.frozen_scope.clone(),
            non_goals: Vec::new(),
            assumptions: vec!["single_authority_requirement_truth".to_string()],
            unknowns: brief.open_questions.clone(),
            acceptance_criteria: brief.acceptance_criteria.clone(),
            risk_level: risk_level.to_string(),
            evidence_ref: evidence_ref.to_string(),
        })
    }
}
