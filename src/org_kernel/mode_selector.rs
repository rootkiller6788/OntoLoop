use crate::contracts::org::{AgentMode, AgentModeDecision, RequirementSpec};

pub use super::RoleDesigner;

#[derive(Debug, Clone, Default)]
pub struct AgentModeSelector;

impl AgentModeSelector {
    pub fn decide(requirement: &RequirementSpec, evidence_ref: &str) -> AgentModeDecision {
        let risk = requirement.risk_level.to_ascii_lowercase();
        let mode = if risk == "high" {
            AgentMode::DebateReview
        } else if requirement.acceptance_criteria.len() > 3 {
            AgentMode::ParallelWorkers
        } else if requirement.scope.split_whitespace().count() > 8 {
            AgentMode::PlannerWorker
        } else {
            AgentMode::SingleAgent
        };
        AgentModeDecision {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            task_id: requirement.task_id.clone(),
            mode,
            reason: "single_requirement_truth_mode_selection".to_string(),
            risk_level: requirement.risk_level.clone(),
            expected_parallelism: if risk == "high" { 3 } else { 1 },
            evidence_ref: evidence_ref.to_string(),
        }
    }
}
