pub mod coordinator;
pub mod global_planner;
pub mod lifecycle;
pub mod mode_selector;
pub mod requirement_spec;
pub mod review_graph;
pub mod task_intake;
pub mod task_tree;

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::contracts::org::{
    AgentPerformance, LifecycleAction, LifecycleDecision, LifecycleState, OrgSession,
    ProjectTaskNode, RetrospectiveReport, ReviewDecision, ReviewEdge, ReviewGraph, ReviewResult,
    SopPatch, SopRolloutScope, TaskIntakeReport, TaskNodeStatus, TaskTree,
};
use crate::contracts::version_a::E2rLimit;
use crate::orchestration::{ExecutionReport, RequirementBrief, SwarmTask, ValidationReport};

#[derive(Debug, Clone)]
pub struct TaskIntakeAnalyzer {
    complexity_threshold: f64,
}

impl TaskIntakeAnalyzer {
    pub fn new(complexity_threshold: f64) -> Self {
        Self {
            complexity_threshold,
        }
    }

    pub fn analyze(&self, session_id: &str, request: &str, brief: &RequirementBrief) -> TaskIntakeReport {
        let token_est = request.split_whitespace().count() as f64;
        let question_factor = brief.open_questions.len() as f64 * 2.0;
        let acceptance_factor = brief.acceptance_criteria.len() as f64 * 1.5;
        let complexity = token_est * 0.05 + question_factor + acceptance_factor;
        let risk_level = if complexity >= self.complexity_threshold * 1.5 {
            "high"
        } else if complexity >= self.complexity_threshold {
            "medium"
        } else {
            "low"
        };
        let org_mode = complexity >= self.complexity_threshold || brief.confirmation_required;
        let required_roles = if org_mode {
            vec![
                "ceo".to_string(),
                "planner".to_string(),
                "worker".to_string(),
                "reviewer".to_string(),
            ]
        } else {
            vec!["worker".to_string()]
        };
        TaskIntakeReport {
            task_id: format!("task-intake:{session_id}"),
            user_goal: brief.clarified_goal.clone(),
            task_type: "requirement_swarm".to_string(),
            complexity_score: complexity,
            risk_level: risk_level.to_string(),
            required_roles,
            org_mode_required: org_mode,
            budget_hint: if risk_level == "high" { 120_000 } else { 60_000 },
            evidence_required: org_mode,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RoleDesigner;

impl RoleDesigner {
    pub fn design_roles(report: &TaskIntakeReport) -> Vec<String> {
        if report.required_roles.is_empty() {
            vec!["worker".to_string()]
        } else {
            report.required_roles.clone()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskTreeBuilder;

impl TaskTreeBuilder {
    pub fn build(session_id: &str, tasks: &[SwarmTask]) -> TaskTree {
        let nodes = tasks
            .iter()
            .map(|task| ProjectTaskNode {
                node_id: task.task_id.clone(),
                parent_id: None,
                deps: task.depends_on.clone(),
                status: TaskNodeStatus::Pending,
                assigned_talent: Some(task.agent_name.clone()),
                supervisor: Some("reviewer".to_string()),
                artifacts: Vec::new(),
                evidence_ref: None,
                retry_count: 0,
                review_round: 0,
                review_result: None,
            })
            .collect::<Vec<_>>();
        TaskTree {
            tree_id: format!("task-tree:{session_id}"),
            nodes,
        }
    }

    pub fn validate_dag(tree: &TaskTree) -> Result<(), String> {
        let mut indegree: HashMap<&str, usize> = HashMap::new();
        let mut outgoing: HashMap<&str, Vec<&str>> = HashMap::new();
        for node in &tree.nodes {
            indegree.entry(node.node_id.as_str()).or_insert(0);
        }
        for node in &tree.nodes {
            for dep in &node.deps {
                if indegree.contains_key(dep.as_str()) {
                    *indegree.entry(node.node_id.as_str()).or_insert(0) += 1;
                    outgoing
                        .entry(dep.as_str())
                        .or_default()
                        .push(node.node_id.as_str());
                }
            }
        }
        let mut queue = VecDeque::new();
        for (id, deg) in &indegree {
            if *deg == 0 {
                queue.push_back(*id);
            }
        }
        let mut visited = 0usize;
        while let Some(id) = queue.pop_front() {
            visited += 1;
            if let Some(children) = outgoing.get(id) {
                for child in children {
                    if let Some(entry) = indegree.get_mut(child) {
                        *entry = entry.saturating_sub(1);
                        if *entry == 0 {
                            queue.push_back(child);
                        }
                    }
                }
            }
        }
        if visited == indegree.len() {
            Ok(())
        } else {
            Err("task tree dependency graph contains cycle".to_string())
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReviewGraphBuilder;

impl ReviewGraphBuilder {
    pub fn build(session_id: &str, report: &TaskIntakeReport, tree: &TaskTree) -> ReviewGraph {
        let reviewer_count = if report.risk_level.eq_ignore_ascii_case("high") {
            2
        } else {
            1
        };
        let mut edges = Vec::new();
        for node in &tree.nodes {
            for i in 0..reviewer_count {
                edges.push(ReviewEdge {
                    edge_id: format!("review-edge:{}:{}:{}", session_id, node.node_id, i),
                    reviewer: if i == 0 {
                        "reviewer-primary".to_string()
                    } else {
                        "reviewer-secondary".to_string()
                    },
                    target_task_node_id: node.node_id.clone(),
                    required: true,
                });
            }
        }
        ReviewGraph {
            graph_id: format!("review-graph:{session_id}"),
            edges,
        }
    }
}

pub fn build_org_session(session_id: &str, goal: &str, report: &TaskIntakeReport, tree: &TaskTree, review_graph: &ReviewGraph) -> OrgSession {
    OrgSession {
        session_id: session_id.to_string(),
        user_goal: goal.to_string(),
        org_mode: report.org_mode_required,
        task_tree_id: tree.tree_id.clone(),
        dependency_dag_id: format!("dep-dag:{session_id}"),
        review_graph_id: review_graph.graph_id.clone(),
        communication_graph_id: format!("comm-graph:{session_id}"),
        budget: report.budget_hint,
        status: "shadow_planned".to_string(),
    }
}

pub fn seed_review_result(task_node_id: &str, reviewer: &str, evidence_ref: &str) -> ReviewResult {
    ReviewResult {
        review_id: format!("review:{task_node_id}"),
        reviewer: reviewer.to_string(),
        target_agent: "worker".to_string(),
        task_node_id: task_node_id.to_string(),
        decision: ReviewDecision::Accept,
        reason_code: "shadow_seed".to_string(),
        required_fix: None,
        confidence: 80,
        evidence_refs: vec![evidence_ref.to_string()],
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RecruitRequest {
    pub task_node_id: String,
    pub required_role: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TalentMatchOutput {
    #[serde(default)]
    pub assigned_leaf_nodes: Vec<String>,
    #[serde(default)]
    pub recruit_requests: Vec<RecruitRequest>,
}

#[derive(Debug, Clone, Default)]
pub struct TalentMatcher;

impl TalentMatcher {
    pub fn match_or_recruit(tree: &TaskTree) -> TalentMatchOutput {
        let leaf_nodes = leaf_nodes(tree);
        let mut assigned_leaf_nodes = Vec::new();
        let mut recruit_requests = Vec::new();
        for leaf in leaf_nodes {
            if let Some(node) = tree.nodes.iter().find(|n| n.node_id == leaf) {
                if node
                    .assigned_talent
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_some()
                {
                    assigned_leaf_nodes.push(node.node_id.clone());
                } else {
                    recruit_requests.push(RecruitRequest {
                        task_node_id: node.node_id.clone(),
                        required_role: "worker".to_string(),
                        reason: "leaf node missing assigned_talent".to_string(),
                    });
                }
            }
        }
        TalentMatchOutput {
            assigned_leaf_nodes,
            recruit_requests,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct OrgMessageTrace {
    pub message_id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub message_type: String,
    pub task_node_id: String,
    pub emitted_at_ms: u64,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Default)]
pub struct OrgMessageRouter;

impl OrgMessageRouter {
    pub fn trace_messages(
        session_id: &str,
        tree: &TaskTree,
        review_graph: &ReviewGraph,
        talent: &TalentMatchOutput,
        now_ms: u64,
    ) -> Vec<OrgMessageTrace> {
        let mut traces = Vec::new();
        for node_id in &talent.assigned_leaf_nodes {
            traces.push(OrgMessageTrace {
                message_id: format!("msg:{session_id}:assign:{node_id}"),
                from_agent: "planner".to_string(),
                to_agent: "worker".to_string(),
                message_type: "task_request".to_string(),
                task_node_id: node_id.clone(),
                emitted_at_ms: now_ms,
                evidence_ref: format!("evidence:orgmsg:{session_id}:assign:{node_id}:{now_ms}"),
            });
        }
        for req in &talent.recruit_requests {
            traces.push(OrgMessageTrace {
                message_id: format!("msg:{session_id}:recruit:{}", req.task_node_id),
                from_agent: "planner".to_string(),
                to_agent: "hr".to_string(),
                message_type: "escalation".to_string(),
                task_node_id: req.task_node_id.clone(),
                emitted_at_ms: now_ms,
                evidence_ref: format!(
                    "evidence:orgmsg:{session_id}:recruit:{}:{now_ms}",
                    req.task_node_id
                ),
            });
        }
        for edge in &review_graph.edges {
            if tree.nodes.iter().any(|node| node.node_id == edge.target_task_node_id) {
                traces.push(OrgMessageTrace {
                    message_id: format!("msg:{session_id}:review:{}", edge.edge_id),
                    from_agent: "worker".to_string(),
                    to_agent: edge.reviewer.clone(),
                    message_type: "review_request".to_string(),
                    task_node_id: edge.target_task_node_id.clone(),
                    emitted_at_ms: now_ms,
                    evidence_ref: format!(
                        "evidence:orgmsg:{session_id}:review:{}:{now_ms}",
                        edge.target_task_node_id
                    ),
                });
            }
        }
        traces
    }
}

pub fn leaf_nodes(tree: &TaskTree) -> Vec<String> {
    let mut depended = std::collections::HashSet::new();
    for node in &tree.nodes {
        for dep in &node.deps {
            depended.insert(dep.clone());
        }
    }
    tree.nodes
        .iter()
        .filter(|node| !depended.contains(&node.node_id))
        .map(|node| node.node_id.clone())
        .collect()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct E2rTaskDecision {
    pub task_node_id: String,
    pub from_status: TaskNodeStatus,
    pub to_status: TaskNodeStatus,
    pub decision: String,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct E2rGateOutcome {
    #[serde(default)]
    pub decisions: Vec<E2rTaskDecision>,
    #[serde(default)]
    pub accepted_task_ids: Vec<String>,
    #[serde(default)]
    pub committed_task_ids: Vec<String>,
    #[serde(default)]
    pub iterate_task_ids: Vec<String>,
    pub accepted_count: usize,
    pub committed_count: usize,
    pub rejected_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct E2rController;

impl E2rController {
    pub fn enforce(
        session_id: &str,
        tree: &TaskTree,
        review_graph: &ReviewGraph,
        execution_reports: &[ExecutionReport],
        limits: &E2rLimit,
        observed_cost_micros: u64,
        observed_time_ms: u64,
        now_ms: u64,
    ) -> E2rGateOutcome {
        let hard_limit_exceeded = (limits.max_cost_micros > 0 && observed_cost_micros > limits.max_cost_micros)
            || (limits.max_time_ms > 0 && observed_time_ms > limits.max_time_ms);
        let mut report_map = HashMap::new();
        for report in execution_reports {
            report_map.insert(report.task.task_id.clone(), report);
        }
        let mut node_map = HashMap::new();
        for node in &tree.nodes {
            node_map.insert(node.node_id.clone(), node);
        }

        let mut decisions = Vec::new();
        let mut accepted_task_ids = Vec::new();
        let mut committed_task_ids = Vec::new();
        let mut iterate_task_ids = Vec::new();
        let mut rejected_count = 0usize;
        let mut deferred_nodes = Vec::new();

        for node in &tree.nodes {
            let Some(report) = report_map.get(&node.node_id) else {
                iterate_task_ids.push(node.node_id.clone());
                rejected_count += 1;
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Pending,
                    to_status: TaskNodeStatus::Processing,
                    decision: "rejected->iterate(missing_execution_report)".to_string(),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:reject-missing-report:{}:{now_ms}",
                        node.node_id
                    ),
                });
                continue;
            };
            if hard_limit_exceeded {
                iterate_task_ids.push(node.node_id.clone());
                rejected_count += 1;
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Rejected,
                    to_status: TaskNodeStatus::Escalated,
                    decision: format!(
                        "rejected->escalate(e2r_hard_limit_exceeded:cost={observed_cost_micros}/{},time={observed_time_ms}/{})",
                        limits.max_cost_micros, limits.max_time_ms
                    ),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:hard-limit-escalate:{}:{now_ms}",
                        node.node_id
                    ),
                });
                continue;
            }

            if node.review_round >= limits.max_review_round || node.retry_count >= limits.max_retry {
                iterate_task_ids.push(node.node_id.clone());
                rejected_count += 1;
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Rejected,
                    to_status: TaskNodeStatus::Escalated,
                    decision: format!(
                        "rejected->escalate(e2r_hard_limit_node:review_round={}/{},retry={}/{})",
                        node.review_round, limits.max_review_round, node.retry_count, limits.max_retry
                    ),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:node-limit-escalate:{}:{now_ms}",
                        node.node_id
                    ),
                });
                continue;
            }

            let depends_on_known_nodes = node
                .deps
                .iter()
                .filter(|dep| node_map.contains_key(*dep))
                .cloned()
                .collect::<Vec<_>>();
            if !depends_on_known_nodes.is_empty()
                && depends_on_known_nodes
                    .iter()
                    .any(|dep| !committed_task_ids.contains(dep))
            {
                deferred_nodes.push(node.node_id.clone());
                continue;
            }

            let has_reviewer = review_graph
                .edges
                .iter()
                .any(|edge| edge.target_task_node_id == node.node_id && edge.required);
            let execution_pass = report.outcome_score >= 0
                && !report.guard_decision.eq_ignore_ascii_case("block");
            let review_pass = report.outcome_score >= 3 && has_reviewer;

            let global_gate_pass = global_gate_proof_present(report);
            if execution_pass && review_pass && global_gate_pass {
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Completed,
                    to_status: TaskNodeStatus::Reviewing,
                    decision: "completed->reviewing".to_string(),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:completed-reviewing:{}:{now_ms}",
                        node.node_id
                    ),
                });
                accepted_task_ids.push(node.node_id.clone());
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Reviewing,
                    to_status: TaskNodeStatus::Accepted,
                    decision: "reviewing->accepted(review_pass_with_evidence)".to_string(),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:reviewing-accepted:{}:{now_ms}",
                        node.node_id
                    ),
                });
                committed_task_ids.push(node.node_id.clone());
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Accepted,
                    to_status: TaskNodeStatus::Committed,
                    decision: "accepted->committed(global_gate_pass)".to_string(),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:accepted-committed:{}:{now_ms}",
                        node.node_id
                    ),
                });
            } else {
                iterate_task_ids.push(node.node_id.clone());
                rejected_count += 1;
                let reason = if !execution_pass {
                    "execution_fail"
                } else if !has_reviewer {
                    "missing_reviewer"
                } else if !global_gate_pass {
                    "missing_global_gate_proof"
                } else {
                    "review_reject"
                };
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Completed,
                    to_status: TaskNodeStatus::Reviewing,
                    decision: "completed->reviewing".to_string(),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:completed-reviewing:{}:{now_ms}",
                        node.node_id
                    ),
                });
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Reviewing,
                    to_status: TaskNodeStatus::Rejected,
                    decision: format!("reviewing->rejected({reason})"),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:reviewing-rejected:{}:{now_ms}",
                        node.node_id
                    ),
                });
                decisions.push(E2rTaskDecision {
                    task_node_id: node.node_id.clone(),
                    from_status: TaskNodeStatus::Rejected,
                    to_status: TaskNodeStatus::Processing,
                    decision: format!("rejected->iterate({reason})"),
                    evidence_ref: format!(
                        "evidence:e2r:{session_id}:rejected-iterate:{}:{now_ms}",
                        node.node_id
                    ),
                });
            }
        }
        for node_id in deferred_nodes {
            iterate_task_ids.push(node_id.clone());
            rejected_count += 1;
            decisions.push(E2rTaskDecision {
                task_node_id: node_id.clone(),
                from_status: TaskNodeStatus::Completed,
                to_status: TaskNodeStatus::Reviewing,
                decision: "deferred->wait_for_dependencies_accepted".to_string(),
                evidence_ref: format!(
                    "evidence:e2r:{session_id}:deferred-dependency:{}:{now_ms}",
                    node_id
                ),
            });
        }

        E2rGateOutcome {
            accepted_count: accepted_task_ids.len(),
            committed_count: committed_task_ids.len(),
            rejected_count,
            decisions,
            accepted_task_ids,
            committed_task_ids,
            iterate_task_ids,
        }
    }
}

fn global_gate_proof_present(report: &ExecutionReport) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&report.output) else {
        return false;
    };
    let evidence_ref = value
        .get("evidence_ref")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("relation_evidence_ref").and_then(|v| v.as_str()))
        .unwrap_or_default()
        .trim();
    if evidence_ref.is_empty() {
        return false;
    }

    let write_proof_obj = value
        .get("write_proof")
        .or_else(|| value.get("relation_write_proof"));
    if let Some(obj) = write_proof_obj {
        let hash = obj
            .get("sha256")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("hash").and_then(|v| v.as_str()))
            .unwrap_or_default()
            .trim();
        let proof_evidence = obj
            .get("evidence_ref")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim();
        if !hash.is_empty() && !proof_evidence.is_empty() {
            return true;
        }
    }

    value
        .get("relation_write_proof_ref")
        .and_then(|v| v.as_str())
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PerformanceInput {
    pub agent_id: String,
    pub success_rate: f64,
    pub review_pass_rate: f64,
    pub rework_caused: u32,
    pub risk_score: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct LifecycleEvaluation {
    pub performance: AgentPerformance,
    pub perf_3: f64,
    pub lifecycle_decision: LifecycleDecision,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Timeout,
    ToolFailed,
    VerifierFailed,
    WalFailed,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureDomain {
    Execution,
    Tooling,
    Verification,
    Persistence,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CircuitBreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPolicy {
    RetryBounded,
    ReplanIterate,
    BlockDownstream,
    RollbackReadonly,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct FailureRecoveryRecord {
    pub failure_id: String,
    pub session_id: String,
    pub failure_kind: FailureKind,
    pub failure_domain: FailureDomain,
    pub breaker_state: CircuitBreakerState,
    pub recovery_policy: RecoveryPolicy,
    pub isolated: bool,
    pub rollback_prepared: bool,
    pub reason: String,
    pub evidence_ref: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CircuitBreakerWindow {
    pub session_id: String,
    pub domain: FailureDomain,
    pub state: CircuitBreakerState,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
    pub failure_count: u32,
    pub cooldown_until_ms: u64,
}

impl CircuitBreakerWindow {
    pub fn bump(
        prev: Option<Self>,
        session_id: &str,
        domain: FailureDomain,
        now_ms: u64,
        threshold: u32,
        window_ms: u64,
        cooldown_ms: u64,
    ) -> Self {
        let mut next = prev.unwrap_or(Self {
            session_id: session_id.to_string(),
            domain,
            state: CircuitBreakerState::Closed,
            window_start_ms: now_ms,
            window_end_ms: now_ms.saturating_add(window_ms),
            failure_count: 0,
            cooldown_until_ms: 0,
        });

        if now_ms > next.window_end_ms {
            next.window_start_ms = now_ms;
            next.window_end_ms = now_ms.saturating_add(window_ms);
            next.failure_count = 0;
            if matches!(next.state, CircuitBreakerState::Open) && now_ms >= next.cooldown_until_ms {
                next.state = CircuitBreakerState::HalfOpen;
            }
        }

        next.failure_count = next.failure_count.saturating_add(1);
        if next.failure_count >= threshold {
            next.state = CircuitBreakerState::Open;
            next.cooldown_until_ms = now_ms.saturating_add(cooldown_ms);
        } else if matches!(next.state, CircuitBreakerState::HalfOpen) {
            next.state = CircuitBreakerState::Closed;
            next.cooldown_until_ms = 0;
        }
        next
    }
}

#[derive(Debug, Clone, Default)]
pub struct FailureControlPlane {
    injections: std::collections::HashSet<FailureKind>,
}

impl FailureControlPlane {
    pub fn from_request(request: &str) -> Self {
        let lower = request.to_ascii_lowercase();
        let mut injections = std::collections::HashSet::new();
        if lower.contains("timeout") && lower.contains("inject") {
            injections.insert(FailureKind::Timeout);
        }
        if lower.contains("tool_failed") || lower.contains("inject:tool_failed") {
            injections.insert(FailureKind::ToolFailed);
        }
        if lower.contains("verifier_failed") || lower.contains("inject:verifier_failed") {
            injections.insert(FailureKind::VerifierFailed);
        }
        if lower.contains("wal_failed") || lower.contains("inject:wal_failed") {
            injections.insert(FailureKind::WalFailed);
        }
        Self { injections }
    }

    pub fn should_inject(&self, kind: FailureKind) -> bool {
        self.injections.contains(&kind)
    }

    pub fn classify(
        session_id: &str,
        kind: FailureKind,
        reason: impl Into<String>,
        evidence_ref: impl Into<String>,
        now_ms: u64,
    ) -> FailureRecoveryRecord {
        let (failure_domain, recovery_policy, breaker_state, rollback_prepared) = match kind {
            FailureKind::Timeout => (
                FailureDomain::Execution,
                RecoveryPolicy::ReplanIterate,
                CircuitBreakerState::Open,
                true,
            ),
            FailureKind::ToolFailed => (
                FailureDomain::Tooling,
                RecoveryPolicy::RetryBounded,
                CircuitBreakerState::HalfOpen,
                true,
            ),
            FailureKind::VerifierFailed => (
                FailureDomain::Verification,
                RecoveryPolicy::BlockDownstream,
                CircuitBreakerState::Open,
                true,
            ),
            FailureKind::WalFailed => (
                FailureDomain::Persistence,
                RecoveryPolicy::RollbackReadonly,
                CircuitBreakerState::Open,
                true,
            ),
        };
        FailureRecoveryRecord {
            failure_id: format!("failure:{session_id}:{kind:?}:{now_ms}"),
            session_id: session_id.to_string(),
            failure_kind: kind,
            failure_domain,
            breaker_state,
            recovery_policy,
            isolated: true,
            rollback_prepared,
            reason: reason.into(),
            evidence_ref: evidence_ref.into(),
            created_at_ms: now_ms,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LifecycleEvaluator;

impl LifecycleEvaluator {
    pub fn evaluate(
        agent_id: &str,
        current: PerformanceInput,
        recent_scores: &[f64],
        previous_state: LifecycleState,
        evidence_ref: &str,
    ) -> LifecycleEvaluation {
        let score = current.success_rate * 0.35
            + current.review_pass_rate * 0.35
            - current.risk_score * 0.15
            - (current.rework_caused as f64) * 0.05;

        let prev_ema = recent_scores.last().copied().unwrap_or(score);
        let ema_score = 0.7 * prev_ema + 0.3 * score;
        let mut perf_series = recent_scores.to_vec();
        perf_series.push(score);
        let perf_3 = perf_series.iter().rev().take(3).copied().sum::<f64>()
            / (perf_series.iter().rev().take(3).count().max(1) as f64);

        let performance = AgentPerformance {
            agent_id: agent_id.to_string(),
            score,
            ema_score,
            success_rate: current.success_rate,
            review_pass_rate: current.review_pass_rate,
            false_accept_rate: (1.0 - current.review_pass_rate).clamp(0.0, 1.0),
            rework_caused: current.rework_caused,
            risk_score: current.risk_score,
        };

        let (decision, next_state, reason) = if current.risk_score >= 0.9 {
            (
                LifecycleAction::Quarantine,
                LifecycleState::Quarantined,
                "risk_score_above_quarantine_threshold".to_string(),
            )
        } else if previous_state == LifecycleState::Pip && perf_3 < 0.25 {
            (
                LifecycleAction::Retire,
                LifecycleState::Retired,
                "pip_failed_with_low_perf3".to_string(),
            )
        } else if perf_3 < 0.45 || current.success_rate < 0.55 {
            (
                LifecycleAction::Pip,
                LifecycleState::Pip,
                "performance_below_pip_threshold".to_string(),
            )
        } else if previous_state == LifecycleState::Candidate {
            (
                LifecycleAction::Promote,
                LifecycleState::Active,
                "candidate_promoted_after_stable_metrics".to_string(),
            )
        } else {
            (
                LifecycleAction::Restore,
                LifecycleState::Active,
                "metrics_stable".to_string(),
            )
        };

        let lifecycle_decision = LifecycleDecision {
            agent_id: agent_id.to_string(),
            decision,
            reason,
            previous_state,
            next_state,
            working_principles_patch: None,
            evidence_ref: evidence_ref.to_string(),
        };

        LifecycleEvaluation {
            performance,
            perf_3,
            lifecycle_decision,
        }
    }
}

pub fn validate_sop_patch_scope(patch: &SopPatch) -> Result<(), String> {
    match patch.rollout_scope {
        SopRolloutScope::Shadow | SopRolloutScope::Canary => Ok(()),
    }
}

pub fn build_retro_and_sop_patches(
    session_id: &str,
    brief: &RequirementBrief,
    execution_reports: &[ExecutionReport],
    validation: &ValidationReport,
    lifecycle: &[LifecycleEvaluation],
    evidence_ref: &str,
) -> (RetrospectiveReport, Vec<SopPatch>) {
    let mut rejection_reasons = execution_reports
        .iter()
        .filter(|report| report.outcome_score < 0)
        .map(|report| format!("{}:{}", report.task.task_id, report.guard_decision))
        .collect::<Vec<_>>();
    rejection_reasons.sort();
    rejection_reasons.dedup();

    let avg_score = if execution_reports.is_empty() {
        0.0
    } else {
        execution_reports
            .iter()
            .map(|item| item.outcome_score as f64)
            .sum::<f64>()
            / execution_reports.len() as f64
    };
    let objective_metrics = serde_json::json!({
        "execution_report_count": execution_reports.len(),
        "avg_outcome_score": avg_score,
        "validation_ready": validation.ready,
        "validation_summary": validation.summary,
        "lifecycle_decisions": lifecycle.iter().map(|item| serde_json::json!({
            "agent_id": item.lifecycle_decision.agent_id,
            "decision": item.lifecycle_decision.decision,
            "next_state": item.lifecycle_decision.next_state,
            "perf_3": item.perf_3,
        })).collect::<Vec<_>>(),
    });

    let patch_rollout_scope = if validation.ready {
        SopRolloutScope::Canary
    } else {
        SopRolloutScope::Shadow
    };
    let sop_patch = SopPatch {
        patch_id: format!("sop-patch:{session_id}:{}", current_millis()),
        trigger_pattern: if rejection_reasons.is_empty() {
            "general_quality_tuning".to_string()
        } else {
            "reject_or_iterate_pattern".to_string()
        },
        new_rule: "increase reviewer depth and tighten acceptance criteria for repeated rejection paths".to_string(),
        affected_roles: vec!["coo".to_string(), "worker".to_string(), "reviewer".to_string()],
        evidence_refs: vec![evidence_ref.to_string()],
        rollout_scope: patch_rollout_scope,
    };

    let retrospective = RetrospectiveReport {
        project_id: format!("project:{session_id}"),
        self_assessments: vec![
            format!("goal: {}", brief.clarified_goal),
            format!("scope: {}", brief.frozen_scope),
        ],
        objective_metrics,
        rejection_reasons,
        sop_patches: vec![sop_patch.patch_id.clone()],
        talent_gap: lifecycle
            .iter()
            .find(|item| item.lifecycle_decision.next_state == LifecycleState::Pip)
            .map(|item| format!("{} requires capability uplift", item.lifecycle_decision.agent_id)),
        evidence_refs: vec![evidence_ref.to_string()],
    };
    (retrospective, vec![sop_patch])
}

fn current_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn to_shadow_snapshot(
    intake: &TaskIntakeReport,
    roles: &[String],
    session: &OrgSession,
    tree: &TaskTree,
    review_graph: &ReviewGraph,
    talent: &TalentMatchOutput,
    messages: &[OrgMessageTrace],
) -> serde_json::Value {
    let mut meta = BTreeMap::new();
    meta.insert("phase".to_string(), "phase1a-shadow".to_string());
    meta.insert("mode".to_string(), "org-kernel-shadow".to_string());
    serde_json::json!({
        "meta": meta,
        "intake": intake,
        "roles": roles,
        "org_session": session,
        "task_tree": tree,
        "review_graph": review_graph,
        "talent_matching": talent,
        "org_messages": messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org_kernel::coordinator::Coordinator;
    use crate::org_kernel::global_planner::GlobalPlanner;
    use crate::org_kernel::mode_selector::AgentModeSelector;
    use crate::org_kernel::requirement_spec::LeadAnalyst;
    use crate::orchestration::RequirementTurn;

    #[test]
    fn dag_validation_rejects_cycle() {
        let tree = TaskTree {
            tree_id: "task-tree:test".to_string(),
            nodes: vec![
                ProjectTaskNode {
                    node_id: "a".to_string(),
                    parent_id: None,
                    deps: vec!["b".to_string()],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: None,
                    supervisor: None,
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
                ProjectTaskNode {
                    node_id: "b".to_string(),
                    parent_id: None,
                    deps: vec!["a".to_string()],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: None,
                    supervisor: None,
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
            ],
        };
        let err = TaskTreeBuilder::validate_dag(&tree).expect_err("cycle must fail");
        assert!(err.contains("cycle"));
    }

    #[test]
    fn complex_task_triggers_org_mode_and_builds_shadow_outputs() {
        let analyzer = TaskIntakeAnalyzer::new(8.0);
        let brief = RequirementBrief {
            anchor_id: "anchor:1".to_string(),
            original_request: "build a complex multi-step platform".to_string(),
            clarified_goal: "deliver multi-module platform".to_string(),
            frozen_scope: "mvp".to_string(),
            open_questions: vec!["q1".to_string(), "q2".to_string(), "q3".to_string()],
            acceptance_criteria: vec![
                "has ui".to_string(),
                "has api".to_string(),
                "has tests".to_string(),
                "has docs".to_string(),
            ],
            clarification_turns: vec![RequirementTurn {
                turn_index: 1,
                question: "q1".to_string(),
                inferred_answer: "a1".to_string(),
                resolved: true,
            }],
            confirmation_required: true,
        };
        let intake = analyzer.analyze("session-x", &brief.original_request, &brief);
        assert!(intake.org_mode_required);
        let roles = RoleDesigner::design_roles(&intake);
        let tasks = vec![
            SwarmTask {
                task_id: "t1".to_string(),
                agent_name: "worker-a".to_string(),
                role: "Worker".to_string(),
                objective: "build api".to_string(),
                depends_on: vec![],
            },
            SwarmTask {
                task_id: "t2".to_string(),
                agent_name: "worker-b".to_string(),
                role: "Worker".to_string(),
                objective: "build ui".to_string(),
                depends_on: vec!["t1".to_string()],
            },
        ];
        let tree = TaskTreeBuilder::build("session-x", &tasks);
        TaskTreeBuilder::validate_dag(&tree).expect("dag must pass");
        let review_graph = ReviewGraphBuilder::build("session-x", &intake, &tree);
        let talent = TalentMatcher::match_or_recruit(&tree);
        let messages = OrgMessageRouter::trace_messages(
            "session-x",
            &tree,
            &review_graph,
            &talent,
            1_717_000_000_100,
        );
        let session = build_org_session(
            "session-x",
            &brief.clarified_goal,
            &intake,
            &tree,
            &review_graph,
        );
        let snap = to_shadow_snapshot(
            &intake,
            &roles,
            &session,
            &tree,
            &review_graph,
            &talent,
            &messages,
        );
        assert!(snap.get("org_session").is_some());
        assert!(snap.get("task_tree").is_some());
        assert!(snap.get("review_graph").is_some());
        assert!(snap.get("talent_matching").is_some());
        assert!(snap.get("org_messages").is_some());
        assert!(talent.recruit_requests.is_empty());
        assert!(!messages.is_empty());
    }

    #[test]
    fn each_leaf_has_assignee_or_recruit_request() {
        let tree = TaskTree {
            tree_id: "task-tree:leaf-check".to_string(),
            nodes: vec![
                ProjectTaskNode {
                    node_id: "root".to_string(),
                    parent_id: None,
                    deps: vec![],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: Some("agent:root".to_string()),
                    supervisor: None,
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
                ProjectTaskNode {
                    node_id: "leaf-a".to_string(),
                    parent_id: Some("root".to_string()),
                    deps: vec!["root".to_string()],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: None,
                    supervisor: None,
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
            ],
        };
        let out = TalentMatcher::match_or_recruit(&tree);
        assert_eq!(out.assigned_leaf_nodes.len(), 0);
        assert_eq!(out.recruit_requests.len(), 1);
        assert_eq!(out.recruit_requests[0].task_node_id, "leaf-a");
    }

    #[test]
    fn e2r_gate_enforces_rejected_iterate_and_accept_commit() {
        let tree = TaskTree {
            tree_id: "task-tree:e2r".to_string(),
            nodes: vec![
                ProjectTaskNode {
                    node_id: "t-ok".to_string(),
                    parent_id: None,
                    deps: vec![],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: Some("agent:a".to_string()),
                    supervisor: Some("reviewer".to_string()),
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
                ProjectTaskNode {
                    node_id: "t-bad".to_string(),
                    parent_id: None,
                    deps: vec![],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: Some("agent:b".to_string()),
                    supervisor: Some("reviewer".to_string()),
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
            ],
        };
        let review_graph = ReviewGraph {
            graph_id: "review-graph:e2r".to_string(),
            edges: vec![
                ReviewEdge {
                    edge_id: "e1".to_string(),
                    reviewer: "reviewer-primary".to_string(),
                    target_task_node_id: "t-ok".to_string(),
                    required: true,
                },
                ReviewEdge {
                    edge_id: "e2".to_string(),
                    reviewer: "reviewer-primary".to_string(),
                    target_task_node_id: "t-bad".to_string(),
                    required: true,
                },
            ],
        };
        let reports = vec![
            ExecutionReport {
                task: SwarmTask {
                    task_id: "t-ok".to_string(),
                    agent_name: "agent:a".to_string(),
                    role: "Worker".to_string(),
                    objective: "ok".to_string(),
                    depends_on: vec![],
                },
                output: serde_json::json!({
                    "evidence_ref": "evidence:task:t-ok",
                    "relation_write_proof_ref": "relation:write_proof:task:t-ok:1"
                })
                .to_string(),
                tool_used: None,
                mcp_server: None,
                invocation_payload: None,
                outcome_score: 5,
                route_variant: "control".to_string(),
                control_score: 5,
                treatment_score: 5,
                guard_decision: "Allow".to_string(),
            },
            ExecutionReport {
                task: SwarmTask {
                    task_id: "t-bad".to_string(),
                    agent_name: "agent:b".to_string(),
                    role: "Worker".to_string(),
                    objective: "bad".to_string(),
                    depends_on: vec![],
                },
                output: "bad".to_string(),
                tool_used: None,
                mcp_server: None,
                invocation_payload: None,
                outcome_score: -1,
                route_variant: "control".to_string(),
                control_score: -1,
                treatment_score: -1,
                guard_decision: "Block".to_string(),
            },
        ];
        let out = E2rController::enforce(
            "session-e2r",
            &tree,
            &review_graph,
            &reports,
            &E2rLimit::default(),
            10_000,
            50,
            100,
        );
        assert_eq!(out.committed_count, 1);
        assert_eq!(out.rejected_count, 1);
        assert!(out.committed_task_ids.contains(&"t-ok".to_string()));
        assert!(out.iterate_task_ids.contains(&"t-bad".to_string()));
    }

    #[test]
    fn e2r_gate_enforces_cost_and_time_hard_limits() {
        let tree = TaskTree {
            tree_id: "task-tree:e2r-hard-limit".to_string(),
            nodes: vec![ProjectTaskNode {
                node_id: "t-1".to_string(),
                parent_id: None,
                deps: vec![],
                status: TaskNodeStatus::Pending,
                assigned_talent: Some("agent:a".to_string()),
                supervisor: Some("reviewer".to_string()),
                artifacts: Vec::new(),
                evidence_ref: None,
                retry_count: 0,
                review_round: 0,
                review_result: None,
            }],
        };
        let review_graph = ReviewGraph {
            graph_id: "review-graph:e2r-hard-limit".to_string(),
            edges: vec![ReviewEdge {
                edge_id: "e1".to_string(),
                reviewer: "reviewer-primary".to_string(),
                target_task_node_id: "t-1".to_string(),
                required: true,
            }],
        };
        let reports = vec![ExecutionReport {
            task: SwarmTask {
                task_id: "t-1".to_string(),
                agent_name: "agent:a".to_string(),
                role: "Worker".to_string(),
                objective: "obj".to_string(),
                depends_on: vec![],
            },
            output: serde_json::json!({
                "evidence_ref": "evidence:task:t-1",
                "relation_write_proof_ref": "relation:write_proof:task:t-1:1"
            })
            .to_string(),
            tool_used: None,
            mcp_server: None,
            invocation_payload: None,
            outcome_score: 5,
            route_variant: "control".to_string(),
            control_score: 5,
            treatment_score: 5,
            guard_decision: "Allow".to_string(),
        }];
        let limits = E2rLimit {
            max_review_round: 3,
            max_retry: 3,
            max_cost_micros: 50,
            max_time_ms: 10,
        };
        let out = E2rController::enforce(
            "session-e2r-hard-limit",
            &tree,
            &review_graph,
            &reports,
            &limits,
            500,
            200,
            100,
        );
        assert_eq!(out.committed_count, 0);
        assert_eq!(out.rejected_count, 1);
        assert!(out
            .decisions
            .iter()
            .any(|d| d.decision.contains("e2r_hard_limit_exceeded")));
    }

    #[test]
    fn d12_local_success_without_global_gate_proof_cannot_commit() {
        let tree = TaskTree {
            tree_id: "task-tree:d12-proof".to_string(),
            nodes: vec![ProjectTaskNode {
                node_id: "t-proof".to_string(),
                parent_id: None,
                deps: vec![],
                status: TaskNodeStatus::Pending,
                assigned_talent: Some("agent:proof".to_string()),
                supervisor: Some("reviewer".to_string()),
                artifacts: Vec::new(),
                evidence_ref: None,
                retry_count: 0,
                review_round: 0,
                review_result: None,
            }],
        };
        let review_graph = ReviewGraph {
            graph_id: "review-graph:d12-proof".to_string(),
            edges: vec![ReviewEdge {
                edge_id: "edge-proof".to_string(),
                reviewer: "reviewer-primary".to_string(),
                target_task_node_id: "t-proof".to_string(),
                required: true,
            }],
        };
        let reports = vec![ExecutionReport {
            task: SwarmTask {
                task_id: "t-proof".to_string(),
                agent_name: "agent:proof".to_string(),
                role: "Worker".to_string(),
                objective: "local pass only".to_string(),
                depends_on: vec![],
            },
            output: "ok-local-only".to_string(),
            tool_used: None,
            mcp_server: None,
            invocation_payload: None,
            outcome_score: 5,
            route_variant: "control".to_string(),
            control_score: 5,
            treatment_score: 5,
            guard_decision: "Allow".to_string(),
        }];
        let out = E2rController::enforce(
            "session-d12-proof",
            &tree,
            &review_graph,
            &reports,
            &E2rLimit::default(),
            10_000,
            50,
            100,
        );
        assert_eq!(out.committed_count, 0);
        assert!(out.iterate_task_ids.contains(&"t-proof".to_string()));
        assert!(out
            .decisions
            .iter()
            .any(|item| item.decision.contains("missing_global_gate_proof")));
    }

    #[test]
    fn e2r_gate_requires_dependencies_accepted_before_commit() {
        let tree = TaskTree {
            tree_id: "task-tree:e2r-deps".to_string(),
            nodes: vec![
                ProjectTaskNode {
                    node_id: "parent".to_string(),
                    parent_id: None,
                    deps: vec!["child".to_string()],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: Some("agent:p".to_string()),
                    supervisor: Some("reviewer".to_string()),
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
                ProjectTaskNode {
                    node_id: "child".to_string(),
                    parent_id: None,
                    deps: vec![],
                    status: TaskNodeStatus::Pending,
                    assigned_talent: Some("agent:c".to_string()),
                    supervisor: Some("reviewer".to_string()),
                    artifacts: Vec::new(),
                    evidence_ref: None,
                    retry_count: 0,
                    review_round: 0,
                    review_result: None,
                },
            ],
        };
        let review_graph = ReviewGraph {
            graph_id: "review-graph:e2r-deps".to_string(),
            edges: vec![
                ReviewEdge {
                    edge_id: "e-parent".to_string(),
                    reviewer: "reviewer-primary".to_string(),
                    target_task_node_id: "parent".to_string(),
                    required: true,
                },
                ReviewEdge {
                    edge_id: "e-child".to_string(),
                    reviewer: "reviewer-primary".to_string(),
                    target_task_node_id: "child".to_string(),
                    required: true,
                },
            ],
        };
        let reports = vec![
            ExecutionReport {
                task: SwarmTask {
                    task_id: "parent".to_string(),
                    agent_name: "agent:p".to_string(),
                    role: "Worker".to_string(),
                    objective: "parent".to_string(),
                    depends_on: vec!["child".to_string()],
                },
                output: serde_json::json!({
                    "evidence_ref": "evidence:task:parent",
                    "relation_write_proof_ref": "relation:write_proof:task:parent:1"
                })
                .to_string(),
                tool_used: None,
                mcp_server: None,
                invocation_payload: None,
                outcome_score: 5,
                route_variant: "control".to_string(),
                control_score: 5,
                treatment_score: 5,
                guard_decision: "Allow".to_string(),
            },
            ExecutionReport {
                task: SwarmTask {
                    task_id: "child".to_string(),
                    agent_name: "agent:c".to_string(),
                    role: "Worker".to_string(),
                    objective: "child".to_string(),
                    depends_on: vec![],
                },
                output: serde_json::json!({
                    "evidence_ref": "evidence:task:child",
                    "relation_write_proof_ref": "relation:write_proof:task:child:1"
                })
                .to_string(),
                tool_used: None,
                mcp_server: None,
                invocation_payload: None,
                outcome_score: 5,
                route_variant: "control".to_string(),
                control_score: 5,
                treatment_score: 5,
                guard_decision: "Allow".to_string(),
            },
        ];
        let out = E2rController::enforce(
            "session-e2r-deps",
            &tree,
            &review_graph,
            &reports,
            &E2rLimit::default(),
            10_000,
            50,
            100,
        );
        assert!(out.committed_task_ids.contains(&"child".to_string()));
        assert!(
            out.iterate_task_ids.contains(&"parent".to_string())
                || out
                    .decisions
                    .iter()
                    .any(|d| d.decision.contains("wait_for_dependencies_accepted"))
        );
    }

    #[test]
    fn lifecycle_evaluator_can_trigger_pip_retire_quarantine() {
        let pip = LifecycleEvaluator::evaluate(
            "agent:pip",
            PerformanceInput {
                agent_id: "agent:pip".to_string(),
                success_rate: 0.4,
                review_pass_rate: 0.45,
                rework_caused: 3,
                risk_score: 0.2,
            },
            &[0.3, 0.35],
            LifecycleState::Active,
            "evidence:lifecycle:pip:1",
        );
        assert_eq!(pip.lifecycle_decision.next_state, LifecycleState::Pip);

        let retire = LifecycleEvaluator::evaluate(
            "agent:retire",
            PerformanceInput {
                agent_id: "agent:retire".to_string(),
                success_rate: 0.2,
                review_pass_rate: 0.25,
                rework_caused: 5,
                risk_score: 0.2,
            },
            &[0.2, 0.22],
            LifecycleState::Pip,
            "evidence:lifecycle:retire:1",
        );
        assert_eq!(retire.lifecycle_decision.next_state, LifecycleState::Retired);

        let quarantine = LifecycleEvaluator::evaluate(
            "agent:quarantine",
            PerformanceInput {
                agent_id: "agent:quarantine".to_string(),
                success_rate: 0.9,
                review_pass_rate: 0.9,
                rework_caused: 0,
                risk_score: 0.95,
            },
            &[0.8, 0.82],
            LifecycleState::Active,
            "evidence:lifecycle:quarantine:1",
        );
        assert_eq!(
            quarantine.lifecycle_decision.next_state,
            LifecycleState::Quarantined
        );
    }

    #[test]
    fn sop_patch_scope_rejects_full_rollout_by_type_system() {
        let patch_shadow = SopPatch {
            patch_id: "p1".to_string(),
            trigger_pattern: "x".to_string(),
            new_rule: "y".to_string(),
            affected_roles: vec![],
            evidence_refs: vec!["evidence:1".to_string()],
            rollout_scope: SopRolloutScope::Shadow,
        };
        let patch_canary = SopPatch {
            patch_id: "p2".to_string(),
            trigger_pattern: "x".to_string(),
            new_rule: "y".to_string(),
            affected_roles: vec![],
            evidence_refs: vec!["evidence:2".to_string()],
            rollout_scope: SopRolloutScope::Canary,
        };
        assert!(validate_sop_patch_scope(&patch_shadow).is_ok());
        assert!(validate_sop_patch_scope(&patch_canary).is_ok());
    }

    #[test]
    fn retrospective_builds_sop_patch_with_shadow_or_canary_only() {
        let brief = RequirementBrief {
            anchor_id: "anchor:r".to_string(),
            original_request: "build x".to_string(),
            clarified_goal: "build x clarified".to_string(),
            frozen_scope: "mvp".to_string(),
            open_questions: vec![],
            acceptance_criteria: vec!["a".to_string()],
            clarification_turns: vec![],
            confirmation_required: false,
        };
        let reports = vec![ExecutionReport {
            task: SwarmTask {
                task_id: "t1".to_string(),
                agent_name: "worker".to_string(),
                role: "Worker".to_string(),
                objective: "obj".to_string(),
                depends_on: vec![],
            },
            output: "ok".to_string(),
            tool_used: None,
            mcp_server: None,
            invocation_payload: None,
            outcome_score: 5,
            route_variant: "control".to_string(),
            control_score: 5,
            treatment_score: 5,
            guard_decision: "Allow".to_string(),
        }];
        let validation = ValidationReport {
            ready: true,
            summary: "ok".to_string(),
            follow_up_tasks: vec![],
            verifier_summary: "pass".to_string(),
        };
        let (retro, patches) = build_retro_and_sop_patches(
            "session-r",
            &brief,
            &reports,
            &validation,
            &[],
            "evidence:retro:1",
        );
        assert_eq!(retro.project_id, "project:session-r");
        assert!(!patches.is_empty());
        assert!(matches!(
            patches[0].rollout_scope,
            SopRolloutScope::Shadow | SopRolloutScope::Canary
        ));
        assert!(validate_sop_patch_scope(&patches[0]).is_ok());
    }

    #[test]
    fn failure_control_plane_classifies_domain_and_recovery_policy() {
        let control = FailureControlPlane::from_request(
            "please inject timeout + inject:tool_failed + inject:verifier_failed + inject:wal_failed",
        );
        assert!(control.should_inject(FailureKind::Timeout));
        assert!(control.should_inject(FailureKind::ToolFailed));
        assert!(control.should_inject(FailureKind::VerifierFailed));
        assert!(control.should_inject(FailureKind::WalFailed));

        let timeout = FailureControlPlane::classify(
            "session-failure",
            FailureKind::Timeout,
            "timeout injected",
            "evidence:failure:timeout",
            100,
        );
        assert_eq!(timeout.failure_domain, FailureDomain::Execution);
        assert_eq!(timeout.recovery_policy, RecoveryPolicy::ReplanIterate);
        assert_eq!(timeout.breaker_state, CircuitBreakerState::Open);
        assert!(timeout.isolated);
        assert!(timeout.rollback_prepared);
    }

    #[test]
    fn circuit_breaker_window_opens_after_threshold_and_tracks_cooldown() {
        let mut win = CircuitBreakerWindow::bump(
            None,
            "session-cb",
            FailureDomain::Tooling,
            100,
            3,
            60_000,
            30_000,
        );
        assert_eq!(win.state, CircuitBreakerState::Closed);
        win = CircuitBreakerWindow::bump(
            Some(win),
            "session-cb",
            FailureDomain::Tooling,
            200,
            3,
            60_000,
            30_000,
        );
        assert_eq!(win.state, CircuitBreakerState::Closed);
        win = CircuitBreakerWindow::bump(
            Some(win),
            "session-cb",
            FailureDomain::Tooling,
            300,
            3,
            60_000,
            30_000,
        );
        assert_eq!(win.state, CircuitBreakerState::Open);
        assert!(win.cooldown_until_ms >= 30_300);
    }

    #[test]
    fn single_planning_chain_enforces_unique_global_plan() {
        let brief = RequirementBrief {
            anchor_id: "anchor:chain".to_string(),
            original_request: "ship governed feature".to_string(),
            clarified_goal: "ship governed feature".to_string(),
            frozen_scope: "one pipeline".to_string(),
            open_questions: vec![],
            acceptance_criteria: vec!["build".to_string(), "test".to_string()],
            clarification_turns: vec![],
            confirmation_required: false,
        };
        let requirement = LeadAnalyst::produce_requirement_spec(
            "session-chain",
            &brief,
            "medium",
            "evidence:req:1",
        )
        .expect("requirement");
        let mode = AgentModeSelector::decide(&requirement, "evidence:mode:1");
        assert_eq!(mode.task_id, requirement.task_id);

        let tasks = vec![SwarmTask {
            task_id: "task-a".to_string(),
            agent_name: "worker-a".to_string(),
            role: "worker".to_string(),
            objective: "implement".to_string(),
            depends_on: vec![],
        }];
        let tree = TaskTreeBuilder::build("session-chain", &tasks);
        let review = ReviewGraphBuilder::build(
            "session-chain",
            &TaskIntakeReport {
                task_id: "task-intake:session-chain".to_string(),
                user_goal: "ship governed feature".to_string(),
                task_type: "requirement_swarm".to_string(),
                complexity_score: 4.0,
                risk_level: "medium".to_string(),
                required_roles: vec!["worker".to_string(), "reviewer".to_string()],
                org_mode_required: true,
                budget_hint: 10_000,
                evidence_required: true,
            },
            &tree,
        );
        let mut plan = GlobalPlanner::produce_execution_plan(
            &requirement,
            &tree,
            &review,
            "evidence:plan:1",
        )
        .expect("plan");
        let dispatch = Coordinator::dispatch_from_single_plan(&plan).expect("dispatch");
        assert_eq!(dispatch.items.len(), 1);

        // duplicate task nodes imply competing global plan fragments
        plan.work_packages.push(plan.work_packages[0].clone());
        assert!(Coordinator::dispatch_from_single_plan(&plan).is_err());
    }

    #[test]
    fn d7_worker_can_only_write_lease_paths() {
        let plan = crate::contracts::org::ExecutionPlan {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            plan_id: "plan-d7-worker".to_string(),
            requirement_ref: "evidence:req:d7".to_string(),
            task_tree_id: "tree-d7".to_string(),
            work_packages: vec![crate::contracts::org::WorkPackage {
                api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
                task_node_id: "node-d7-worker".to_string(),
                description: "implement runtime change".to_string(),
                owned_paths: vec!["src/runtime".to_string()],
                read_only_paths: vec!["src/contracts".to_string()],
                dependencies: vec![],
                acceptance_criteria: vec!["cargo test --lib".to_string()],
                local_test_command: Some("cargo test --lib".to_string()),
                risk_level: "medium".to_string(),
            }],
            file_ownership: BTreeMap::from([(
                "src/runtime".to_string(),
                "worker-a".to_string(),
            )]),
            dependency_dag_id: "dag-d7-worker".to_string(),
            review_plan_ref: "review-d7-worker".to_string(),
            integration_plan_ref: "code/integration/task-d7-worker".to_string(),
            evidence_ref: "evidence:plan:d7-worker".to_string(),
        };
        let lease = Coordinator::issue_work_package_lease(
            &plan,
            "worker-a",
            "node-d7-worker",
            100,
            1_000,
        )
        .expect("issue worker lease");
        crate::org_kernel::coordinator::BranchLeaseManager::worker_write_allowed(
            &lease,
            "src/runtime/mod.rs",
            200,
        )
        .expect("write in owned path should pass");
        assert!(
            crate::org_kernel::coordinator::BranchLeaseManager::worker_write_allowed(
                &lease,
                "src/agent/mod.rs",
                200,
            )
            .is_err(),
            "write outside lease paths must be rejected"
        );
    }

    #[test]
    fn d7_integrator_only_allowed_on_integration_branch() {
        let plan = crate::contracts::org::ExecutionPlan {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            plan_id: "plan-d7-integrator".to_string(),
            requirement_ref: "evidence:req:d7-integrator".to_string(),
            task_tree_id: "tree-d7-integrator".to_string(),
            work_packages: vec![crate::contracts::org::WorkPackage {
                api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
                task_node_id: "node-d7-integrator".to_string(),
                description: "integrate".to_string(),
                owned_paths: vec!["src/runtime".to_string()],
                read_only_paths: vec![],
                dependencies: vec![],
                acceptance_criteria: vec![],
                local_test_command: None,
                risk_level: "medium".to_string(),
            }],
            file_ownership: BTreeMap::from([(
                "src/runtime".to_string(),
                "worker-a".to_string(),
            )]),
            dependency_dag_id: "dag-d7-integrator".to_string(),
            review_plan_ref: "review-d7-integrator".to_string(),
            integration_plan_ref: "code/integration/task-d7-integrator".to_string(),
            evidence_ref: "evidence:plan:d7-integrator".to_string(),
        };
        crate::org_kernel::coordinator::BranchLeaseManager::integrator_branch_allowed(
            &plan,
            "integrator",
            "code/integration/task-d7-integrator",
        )
        .expect("integrator should be allowed on integration branch");
        assert!(
            crate::org_kernel::coordinator::BranchLeaseManager::integrator_branch_allowed(
                &plan,
                "worker",
                "code/integration/task-d7-integrator",
            )
            .is_err(),
            "non-integrator must be rejected"
        );
        assert!(
            crate::org_kernel::coordinator::BranchLeaseManager::integrator_branch_allowed(
                &plan,
                "integrator",
                "code/agent-a/node-d7-integrator",
            )
            .is_err(),
            "integrator must not write non-integration branch"
        );
    }

    #[test]
    fn d8_worker_write_requires_lease_and_is_traceable() {
        let plan = crate::contracts::org::ExecutionPlan {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            plan_id: "plan-d8-worker-write".to_string(),
            requirement_ref: "evidence:req:d8".to_string(),
            task_tree_id: "tree-d8".to_string(),
            work_packages: vec![crate::contracts::org::WorkPackage {
                api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
                task_node_id: "node-d8-write".to_string(),
                description: "write generated file".to_string(),
                owned_paths: vec![std::env::temp_dir()
                    .join("ontoloop-d8")
                    .to_string_lossy()
                    .replace('\\', "/")],
                read_only_paths: vec![],
                dependencies: vec![],
                acceptance_criteria: vec![],
                local_test_command: None,
                risk_level: "medium".to_string(),
            }],
            file_ownership: BTreeMap::new(),
            dependency_dag_id: "dag-d8".to_string(),
            review_plan_ref: "review-d8".to_string(),
            integration_plan_ref: "code/integration/task-d8".to_string(),
            evidence_ref: "evidence:plan:d8".to_string(),
        };
        let lease = Coordinator::issue_work_package_lease(
            &plan,
            "worker-a",
            "node-d8-write",
            100,
            10_000,
        )
        .expect("issue worker lease");
        let target = std::env::temp_dir()
            .join("ontoloop-d8")
            .join("artifact.txt")
            .to_string_lossy()
            .replace('\\', "/");

        let denied = crate::org_kernel::coordinator::BranchLeaseManager::gate_worker_write(
            None,
            &target,
            200,
        );
        assert_eq!(denied.decision, crate::org_kernel::coordinator::LeaseGateDecision::Block);
        assert!(denied.reason.contains("lease_missing"));
        assert!(denied.evidence_ref.contains("evidence:lease-gate:"));

        let bound = crate::org_kernel::coordinator::BranchLeaseManager::compile_time_guard_bind_worker_path(
            Some(&lease),
            &target,
            200,
        )
        .expect("bind path with lease");
        let proof = crate::org_kernel::coordinator::BranchLeaseManager::write_worker_file_with_lease(
            Some(&lease),
            &bound,
            "onto",
            210,
        )
        .expect("write with lease");
        assert!(proof.evidence_ref.contains("evidence:lease-write:"));
        assert_eq!(proof.bytes_written, 4);
    }
}
