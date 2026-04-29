use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RealitySnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub tenant_id: String,
    pub policy_version: String,
    pub runtime_mode: String,
    pub available_tools: Vec<String>,
    pub memory_refs: Vec<String>,
    pub graph_refs: Vec<String>,
    pub budget_micros: u64,
    pub latency_budget_ms: u64,
    #[serde(default)]
    pub repo_refs: Vec<String>,
    #[serde(default)]
    pub policy_refs: Vec<String>,
    #[serde(default)]
    pub tool_refs: Vec<String>,
    #[serde(default)]
    pub budget_profile: BTreeMap<String, u64>,
    #[serde(default)]
    pub repo_digest: String,
    #[serde(default)]
    pub memory_digest: String,
    #[serde(default)]
    pub graph_digest: String,
    #[serde(default)]
    pub policy_digest: String,
    #[serde(default)]
    pub tool_digest: String,
    #[serde(default)]
    pub budget_digest: String,
    #[serde(default)]
    pub reality_fingerprint: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateGraph {
    pub candidate_id: String,
    pub reality_snapshot_id: String,
    pub graph_version: String,
    pub node_ids: Vec<String>,
    pub edges: Vec<(String, String)>,
    pub budget_allocation: BTreeMap<String, u64>,
    pub expected_cost_micros: u64,
    pub expected_latency_ms: u64,
    pub expected_risk_score: f32,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorldlineScore {
    pub candidate_id: String,
    pub task_success: f32,
    pub robustness: f32,
    pub reuse_gain: f32,
    pub verifier_confidence: f32,
    pub cost_penalty: f32,
    pub latency_penalty: f32,
    pub risk_penalty: f32,
    pub instability_penalty: f32,
    pub governance_violation_penalty: f32,
    pub total_score: f32,
    pub reasons: Vec<String>,
    pub scored_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PromotionDecision {
    #[serde(alias = "DISCARD")]
    Discard,
    #[serde(alias = "LOG_ONLY")]
    LogOnly,
    #[serde(alias = "LOCALIZE")]
    Localize,
    #[serde(alias = "PROMOTE_RUNTIME")]
    PromoteRuntimeUpdate,
    #[serde(alias = "PROMOTE_TEMPLATE")]
    PromoteTemplate,
    #[serde(alias = "PROMOTE_GOVERNANCE")]
    PromoteGovernanceContract,
    #[serde(alias = "CRYSTALLIZE_RULE")]
    CrystallizeMemoryRule,
    #[serde(alias = "ROLLBACK")]
    Rollback,
    #[serde(alias = "ESCALATE")]
    EscalateHumanReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustedPriorSnapshot {
    pub prior_id: String,
    #[serde(alias = "snapshot_ref")]
    pub based_on_reality_snapshot: String,
    #[serde(alias = "candidate_id")]
    pub promoted_candidate_id: Option<String>,
    pub decision: PromotionDecision,
    pub template_refs: Vec<String>,
    pub reusable_subgraph_refs: Vec<String>,
    pub governance_contract_refs: Vec<String>,
    pub routing_priors: Vec<String>,
    pub verifier_priors: Vec<String>,
    pub budget_priors: BTreeMap<String, u64>,
    pub trusted_boundary_version: String,
    pub created_by: String,
    pub created_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evolution_contract_v1_roundtrip() {
        let snapshot = RealitySnapshot {
            snapshot_id: "reality:s-1:1".into(),
            session_id: "s-1".into(),
            trace_id: "trace:s-1:1".into(),
            tenant_id: "tenant-a".into(),
            policy_version: "policy-v2".into(),
            runtime_mode: "normal".into(),
            available_tools: vec!["tool:a".into(), "tool:b".into()],
            memory_refs: vec!["memory:s-1:latest".into()],
            graph_refs: vec!["graph:s-1:latest".into()],
            budget_micros: 100_000,
            latency_budget_ms: 2_000,
            repo_refs: vec!["repo://autoloop".into()],
            policy_refs: vec!["policy:tenant-a:default".into()],
            tool_refs: vec!["tool:a".into(), "tool:b".into()],
            budget_profile: BTreeMap::from([
                ("token_budget".into(), 100_000_u64),
                ("latency_budget_ms".into(), 2_000_u64),
            ]),
            repo_digest: "repo:digest".into(),
            memory_digest: "memory:digest".into(),
            graph_digest: "graph:digest".into(),
            policy_digest: "policy:digest".into(),
            tool_digest: "tool:digest".into(),
            budget_digest: "budget:digest".into(),
            reality_fingerprint: "reality:fingerprint".into(),
            created_at_ms: 1_710_000_001_000,
        };
        let raw = serde_json::to_string(&snapshot).expect("serialize");
        let decoded: RealitySnapshot = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(decoded, snapshot);

        let prior = TrustedPriorSnapshot {
            prior_id: "prior:s-1:2".into(),
            based_on_reality_snapshot: snapshot.snapshot_id,
            promoted_candidate_id: Some("candidate:g2".into()),
            decision: PromotionDecision::PromoteRuntimeUpdate,
            template_refs: vec!["template:ops:v1".into()],
            reusable_subgraph_refs: vec!["subgraph:verify-route:v3".into()],
            governance_contract_refs: vec!["contract:governance:v2".into()],
            routing_priors: vec!["route:risk-first".into()],
            verifier_priors: vec!["verifier:strict".into()],
            budget_priors: BTreeMap::from([("default".to_string(), 120_000_u64)]),
            trusted_boundary_version: "trust-boundary-v1".into(),
            created_by: "evolution-board".into(),
            created_at_ms: 1_710_000_002_000,
        };
        let prior_raw = serde_json::to_string(&prior).expect("serialize prior");
        let prior_decoded: TrustedPriorSnapshot =
            serde_json::from_str(&prior_raw).expect("deserialize prior");
        assert_eq!(prior_decoded, prior);
    }

    #[test]
    fn trusted_prior_accepts_legacy_alias_fields_and_decision_names() {
        let raw = serde_json::json!({
            "prior_id": "prior:legacy:1",
            "snapshot_ref": "reality:legacy:1",
            "candidate_id": "candidate:legacy:2",
            "decision": "PROMOTE_RUNTIME",
            "template_refs": ["template:legacy"],
            "reusable_subgraph_refs": ["subgraph:legacy"],
            "governance_contract_refs": ["contract:legacy"],
            "routing_priors": ["route:legacy"],
            "verifier_priors": ["verifier:legacy"],
            "budget_priors": {"default": 42000},
            "trusted_boundary_version": "legacy-v1",
            "created_by": "legacy-board",
            "created_at_ms": 1710000003000u64,
            "legacy_unused_field": "ignored"
        });

        let decoded: TrustedPriorSnapshot =
            serde_json::from_value(raw).expect("legacy payload should deserialize");
        assert_eq!(decoded.based_on_reality_snapshot, "reality:legacy:1");
        assert_eq!(decoded.promoted_candidate_id.as_deref(), Some("candidate:legacy:2"));
        assert_eq!(decoded.decision, PromotionDecision::PromoteRuntimeUpdate);
    }
}
