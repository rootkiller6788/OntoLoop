use std::collections::BTreeMap;

use autoloop::{
    contracts::org::{ExecutionPlan, ORG_EXECUTION_CONTRACT_VERSION, WorkPackage},
    org_kernel::coordinator::{BranchLeaseManager, Coordinator, LeaseGateDecision},
};

#[test]
fn d8_no_lease_write_is_blocked_and_traced() {
    let target = std::env::temp_dir()
        .join("ontoloop-d8-e2e")
        .join("no-lease.txt")
        .to_string_lossy()
        .replace('\\', "/");
    let gate = BranchLeaseManager::gate_worker_write(None, &target, 100);
    assert_eq!(gate.decision, LeaseGateDecision::Block);
    assert!(gate.reason.contains("lease_missing"));
    assert!(gate.evidence_ref.contains("evidence:lease-gate:"));
}

#[test]
fn d8_lease_write_allows_worker_owned_path_only() {
    let owned_root = std::env::temp_dir()
        .join("ontoloop-d8-e2e")
        .join("owned")
        .to_string_lossy()
        .replace('\\', "/");
    let owned_file = format!("{owned_root}/out.txt");
    let plan = ExecutionPlan {
        api_version: ORG_EXECUTION_CONTRACT_VERSION.to_string(),
        plan_id: "plan-d8-e2e".to_string(),
        requirement_ref: "evidence:req:d8-e2e".to_string(),
        task_tree_id: "tree-d8-e2e".to_string(),
        work_packages: vec![WorkPackage {
            api_version: ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            task_node_id: "node-d8-e2e".to_string(),
            description: "write file".to_string(),
            owned_paths: vec![owned_root.clone()],
            read_only_paths: vec![],
            dependencies: vec![],
            acceptance_criteria: vec![],
            local_test_command: None,
            risk_level: "medium".to_string(),
        }],
        file_ownership: BTreeMap::from([(owned_root.clone(), "worker-e2e".to_string())]),
        dependency_dag_id: "dag-d8-e2e".to_string(),
        review_plan_ref: "review-d8-e2e".to_string(),
        integration_plan_ref: "code/integration/task-d8-e2e".to_string(),
        evidence_ref: "evidence:plan:d8-e2e".to_string(),
    };
    let lease = Coordinator::issue_work_package_lease(
        &plan,
        "worker-e2e",
        "node-d8-e2e",
        100,
        10_000,
    )
    .expect("lease");

    let bound = BranchLeaseManager::compile_time_guard_bind_worker_path(
        Some(&lease),
        &owned_file,
        120,
    )
    .expect("bind");
    let proof = BranchLeaseManager::write_worker_file_with_lease(
        Some(&lease),
        &bound,
        "d8",
        125,
    )
    .expect("write");
    assert_eq!(proof.bytes_written, 2);
    assert!(proof.evidence_ref.contains("evidence:lease-write:"));

    let forbidden = std::env::temp_dir()
        .join("ontoloop-d8-e2e")
        .join("forbidden")
        .join("out.txt")
        .to_string_lossy()
        .replace('\\', "/");
    assert!(
        BranchLeaseManager::compile_time_guard_bind_worker_path(Some(&lease), &forbidden, 130)
            .is_err(),
        "path outside lease must be rejected"
    );
}
