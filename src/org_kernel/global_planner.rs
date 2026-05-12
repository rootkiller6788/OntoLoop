use anyhow::{Result, bail};
use std::collections::BTreeMap;

use crate::contracts::org::{ExecutionPlan, RequirementSpec, ReviewGraph, TaskTree, WorkPackage};

pub use super::{E2rController, E2rGateOutcome, E2rTaskDecision};

#[derive(Debug, Clone, Default)]
pub struct GlobalPlanner;

impl GlobalPlanner {
    pub fn produce_execution_plan(
        requirement: &RequirementSpec,
        tree: &TaskTree,
        review_graph: &ReviewGraph,
        evidence_ref: &str,
    ) -> Result<ExecutionPlan> {
        if evidence_ref.trim().is_empty() {
            bail!("global planner requires non-empty evidence_ref");
        }
        let work_packages = tree
            .nodes
            .iter()
            .map(|node| WorkPackage {
                api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
                task_node_id: node.node_id.clone(),
                description: format!("execute task node {}", node.node_id),
                owned_paths: vec![format!("artifact/code/{}", node.node_id)],
                read_only_paths: vec!["context/".to_string()],
                dependencies: node.deps.clone(),
                acceptance_criteria: vec!["review_pass".to_string(), "evidence_bound".to_string()],
                local_test_command: Some("cargo test --lib".to_string()),
                risk_level: requirement.risk_level.clone(),
            })
            .collect::<Vec<_>>();

        let file_ownership = work_packages
            .iter()
            .map(|wp| {
                (
                    wp.owned_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| format!("artifact/code/{}", wp.task_node_id)),
                    wp.task_node_id.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        Ok(ExecutionPlan {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            plan_id: format!("execution-plan:{}", requirement.task_id),
            requirement_ref: requirement.task_id.clone(),
            task_tree_id: tree.tree_id.clone(),
            work_packages,
            file_ownership,
            dependency_dag_id: format!("dep-dag:{}", tree.tree_id),
            review_plan_ref: review_graph.graph_id.clone(),
            integration_plan_ref: format!("code/integration/{}", requirement.task_id),
            evidence_ref: evidence_ref.to_string(),
        })
    }
}
