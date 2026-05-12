use anyhow::{Result, bail};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::contracts::org::{BranchLease, ExecutionPlan, WorkPackage};

pub use super::{
    OrgMessageRouter, OrgMessageTrace, RecruitRequest, TalentMatchOutput, TalentMatcher,
    to_shadow_snapshot,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DispatchItem {
    pub task_node_id: String,
    pub owned_paths: Vec<String>,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DispatchPlan {
    pub plan_id: String,
    pub items: Vec<DispatchItem>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkPackageLease {
    pub task_node_id: String,
    pub worker_id: String,
    pub branch_lease: BranchLease,
    pub integration_branch: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum LeaseGateDecision {
    Allow,
    Block,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LeaseGateOutcome {
    pub decision: LeaseGateDecision,
    pub reason: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LeaseWriteProof {
    pub task_node_id: String,
    pub worker_id: String,
    pub write_path: String,
    pub bytes_written: usize,
    pub evidence_ref: String,
}

#[derive(Debug, Clone)]
pub struct LeaseBoundPath(String);

impl LeaseBoundPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Default)]
pub struct Coordinator;

impl Coordinator {
    pub fn dispatch_from_single_plan(plan: &ExecutionPlan) -> Result<DispatchPlan> {
        ensure_single_global_plan(plan)?;
        let items = plan
            .work_packages
            .iter()
            .map(to_dispatch_item)
            .collect::<Vec<_>>();
        Ok(DispatchPlan {
            plan_id: plan.plan_id.clone(),
            items,
        })
    }

    pub fn issue_work_package_lease(
        plan: &ExecutionPlan,
        worker_id: &str,
        task_node_id: &str,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<WorkPackageLease> {
        ensure_single_global_plan(plan)?;
        if worker_id.trim().is_empty() {
            bail!("branch lease requires non-empty worker_id");
        }
        let wp = plan
            .work_packages
            .iter()
            .find(|item| item.task_node_id == task_node_id)
            .ok_or_else(|| anyhow::anyhow!("work package not found: {task_node_id}"))?;
        if wp.owned_paths.is_empty() {
            bail!("work package owned_paths cannot be empty for lease issuance");
        }
        for path in &wp.owned_paths {
            if let Some(owner) = plan.file_ownership.get(path) {
                if !owner.trim().is_empty() && owner != worker_id {
                    bail!(
                        "file ownership mismatch for {path}: expected owner {}, got {}",
                        owner,
                        worker_id
                    );
                }
            }
        }
        let lease = BranchLease {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            branch_name: format!(
                "code/agent-{}/{}",
                worker_id.replace(':', "_"),
                task_node_id
            ),
            agent_id: worker_id.to_string(),
            writable_paths: wp.owned_paths.clone(),
            readonly_paths: wp.read_only_paths.clone(),
            expires_at_ms: now_ms.saturating_add(ttl_ms.max(1)),
            token_budget: 100_000,
            evidence_ref: format!(
                "evidence:branch-lease:{}:{}:{}",
                plan.plan_id, worker_id, task_node_id
            ),
        };
        Ok(WorkPackageLease {
            task_node_id: task_node_id.to_string(),
            worker_id: worker_id.to_string(),
            branch_lease: lease,
            integration_branch: plan.integration_plan_ref.clone(),
        })
    }
}

fn ensure_single_global_plan(plan: &ExecutionPlan) -> Result<()> {
    if plan.plan_id.trim().is_empty() {
        bail!("coordinator requires non-empty global plan id");
    }
    if plan.work_packages.is_empty() {
        bail!("coordinator requires non-empty work_packages");
    }
    let mut unique_nodes = BTreeSet::new();
    for wp in &plan.work_packages {
        if !unique_nodes.insert(wp.task_node_id.clone()) {
            bail!(
                "coordinator rejects duplicated task_node_id in global plan: {}",
                wp.task_node_id
            );
        }
    }
    Ok(())
}

fn to_dispatch_item(wp: &WorkPackage) -> DispatchItem {
    DispatchItem {
        task_node_id: wp.task_node_id.clone(),
        owned_paths: wp.owned_paths.clone(),
        dependencies: wp.dependencies.clone(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct BranchLeaseManager;

impl BranchLeaseManager {
    pub fn compile_time_guard_bind_worker_path(
        lease: Option<&WorkPackageLease>,
        write_path: &str,
        now_ms: u64,
    ) -> Result<LeaseBoundPath> {
        let lease = lease.ok_or_else(|| anyhow::anyhow!("branch lease required for worker write"))?;
        Self::worker_write_allowed(lease, write_path, now_ms)?;
        Ok(LeaseBoundPath(normalize_path(write_path)))
    }

    pub fn worker_write_allowed(lease: &WorkPackageLease, write_path: &str, now_ms: u64) -> Result<()> {
        if now_ms > lease.branch_lease.expires_at_ms {
            bail!("branch lease expired for task {}", lease.task_node_id);
        }
        let normalized = normalize_path(write_path);
        let allowed = lease
            .branch_lease
            .writable_paths
            .iter()
            .map(|path| normalize_path(path))
            .any(|prefix| path_within_prefix(&normalized, &prefix));
        if !allowed {
            bail!(
                "worker write path not covered by branch lease: {}",
                write_path
            );
        }
        Ok(())
    }

    pub fn gate_worker_write(
        lease: Option<&WorkPackageLease>,
        write_path: &str,
        now_ms: u64,
    ) -> LeaseGateOutcome {
        let (decision, reason, worker_id, task_node_id) = match lease {
            Some(bound) => match Self::worker_write_allowed(bound, write_path, now_ms) {
                Ok(_) => (
                    LeaseGateDecision::Allow,
                    "lease_path_allowed".to_string(),
                    bound.worker_id.clone(),
                    bound.task_node_id.clone(),
                ),
                Err(error) => (
                    LeaseGateDecision::Block,
                    format!("lease_path_blocked:{error}"),
                    bound.worker_id.clone(),
                    bound.task_node_id.clone(),
                ),
            },
            None => (
                LeaseGateDecision::Block,
                "lease_missing".to_string(),
                "unknown".to_string(),
                "unknown".to_string(),
            ),
        };
        LeaseGateOutcome {
            decision,
            reason,
            evidence_ref: format!(
                "evidence:lease-gate:{}:{}:{}:{}",
                task_node_id,
                worker_id,
                normalize_path(write_path),
                now_ms
            ),
        }
    }

    pub fn write_worker_file_with_lease(
        lease: Option<&WorkPackageLease>,
        target: &LeaseBoundPath,
        content: &str,
        now_ms: u64,
    ) -> Result<LeaseWriteProof> {
        let lease = lease.ok_or_else(|| anyhow::anyhow!("branch lease required for worker write"))?;
        Self::worker_write_allowed(lease, target.as_str(), now_ms)?;
        if let Some(parent) = Path::new(target.as_str()).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(target.as_str(), content.as_bytes())?;
        Ok(LeaseWriteProof {
            task_node_id: lease.task_node_id.clone(),
            worker_id: lease.worker_id.clone(),
            write_path: target.as_str().to_string(),
            bytes_written: content.len(),
            evidence_ref: format!(
                "evidence:lease-write:{}:{}:{}:{}",
                lease.task_node_id,
                lease.worker_id,
                target.as_str(),
                now_ms
            ),
        })
    }

    pub fn integrator_branch_allowed(plan: &ExecutionPlan, actor_role: &str, branch: &str) -> Result<()> {
        if !actor_role.eq_ignore_ascii_case("integrator") {
            bail!("only integrator can write integration branch");
        }
        if plan.integration_plan_ref.trim().is_empty() {
            bail!("integration plan branch is empty");
        }
        if !normalize_path(&plan.integration_plan_ref).starts_with("code/integration/") {
            bail!("integration plan branch must be under code/integration/*");
        }
        if normalize_path(branch) != normalize_path(&plan.integration_plan_ref) {
            bail!(
                "integrator branch mismatch: expected {}, got {}",
                plan.integration_plan_ref,
                branch
            );
        }
        Ok(())
    }
}

fn normalize_path(value: &str) -> String {
    value.trim().replace('\\', "/").trim_end_matches('/').to_string()
}

fn path_within_prefix(path: &str, prefix: &str) -> bool {
    path == prefix || path.starts_with(&(prefix.to_string() + "/"))
}
