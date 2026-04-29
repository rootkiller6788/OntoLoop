use serde::{Deserialize, Serialize};

use crate::contracts::evolution_os::RealitySnapshot;
use crate::evolution_os::replay;

const PRIOR_CATALOG_VERSION: &str = "prior-catalog:v1";
const PRIOR_REPLAY_SCHEMA_VERSION: &str = "prior-replay/v1";
const PRIOR_REPLAY_SEED_VERSION: &str = "prior-seed/v1";
const PRIOR_REPLAY_VERSION: &str = "prior-replay-contract/v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrystalPriorSelection {
    pub template_refs: Vec<String>,
    pub reusable_subgraph_refs: Vec<String>,
    pub governance_contract_refs: Vec<String>,
    pub strategy_priors: Vec<String>,
    pub promotion_policy_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorHitReason {
    pub domain: String,
    pub selected_ref: String,
    pub selected_version: String,
    pub rule_id: String,
    pub reason: String,
    pub matched_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrystalPriorDecision {
    pub catalog_version: String,
    pub replay_fingerprint: String,
    #[serde(default)]
    pub replay_schema_version: String,
    #[serde(default)]
    pub replay_seed_version: String,
    #[serde(default)]
    pub replay_version: String,
    pub selection: CrystalPriorSelection,
    pub hit_reasons: Vec<PriorHitReason>,
}

#[derive(Debug, Clone, Default)]
pub struct CrystalPriorLayer;

impl CrystalPriorLayer {
    pub fn select(&self, reality: &RealitySnapshot) -> CrystalPriorSelection {
        self.select_with_trace(reality).selection
    }

    pub fn select_with_trace(&self, reality: &RealitySnapshot) -> CrystalPriorDecision {
        let policy_major = extract_policy_major(&reality.policy_version);
        let strict_runtime = is_strict_runtime_mode(&reality.runtime_mode);
        let low_budget = reality.budget_micros <= 75_000 || reality.latency_budget_ms <= 1_500;
        let has_verifier_tool = reality
            .tool_refs
            .iter()
            .chain(reality.available_tools.iter())
            .any(|tool| tool.to_ascii_lowercase().contains("verifier"));

        let template_version = if policy_major >= 3 { "v3" } else { "v2" };
        let template_runtime_ref = if strict_runtime {
            format!("template:governed-runtime:strict:{template_version}")
        } else {
            format!("template:governed-runtime:balanced:{template_version}")
        };

        let mut template_refs = vec![format!(
            "template:{}:default:{}",
            reality.tenant_id, template_version
        )];
        template_refs.push(template_runtime_ref.clone());

        let mut reusable_subgraph_refs = vec!["subgraph:retrieval-verify-route:v1".to_string()];
        if strict_runtime || has_verifier_tool {
            reusable_subgraph_refs.push("subgraph:attestation-verify-reject:v1".to_string());
        }
        if strict_runtime || !low_budget {
            reusable_subgraph_refs.push("subgraph:repair-escalate-loop:v1".to_string());
        }

        let governance_contract_ref =
            format!("contract:{}:{}:v{}", reality.tenant_id, reality.policy_version, policy_major);

        let promotion_policy_ref = if strict_runtime {
            "promotion:board:strict:v2".to_string()
        } else if low_budget {
            "promotion:board:cost-aware:v1".to_string()
        } else {
            "promotion:board:balanced:v1".to_string()
        };

        let strategy_priors = if low_budget {
            vec![
                "strategy:cost-aware".to_string(),
                "strategy:latency-first".to_string(),
            ]
        } else if strict_runtime {
            vec![
                "strategy:risk-first".to_string(),
                "strategy:cost-aware".to_string(),
            ]
        } else {
            vec![
                "strategy:balanced".to_string(),
                "strategy:cost-aware".to_string(),
            ]
        };

        let selection = CrystalPriorSelection {
            template_refs,
            reusable_subgraph_refs,
            governance_contract_refs: vec![governance_contract_ref.clone()],
            strategy_priors,
            promotion_policy_refs: vec![promotion_policy_ref.clone()],
        };

        let hit_reasons = vec![
            PriorHitReason {
                domain: "template".to_string(),
                selected_ref: template_runtime_ref,
                selected_version: template_version.to_string(),
                rule_id: "template.policy-runtime.v1".to_string(),
                reason: "Template version follows policy major and runtime strictness".to_string(),
                matched_signals: vec![
                    format!("policy_version={}", reality.policy_version),
                    format!("runtime_mode={}", reality.runtime_mode),
                ],
            },
            PriorHitReason {
                domain: "subgraph".to_string(),
                selected_ref: selection.reusable_subgraph_refs.join(","),
                selected_version: "v1".to_string(),
                rule_id: "subgraph.verify-repair.v1".to_string(),
                reason:
                    "Subgraph fan-in depends on verifier availability, strict mode and budget profile"
                        .to_string(),
                matched_signals: vec![
                    format!("strict_runtime={strict_runtime}"),
                    format!("low_budget={low_budget}"),
                    format!("has_verifier_tool={has_verifier_tool}"),
                ],
            },
            PriorHitReason {
                domain: "governance_contract".to_string(),
                selected_ref: governance_contract_ref,
                selected_version: format!("v{policy_major}"),
                rule_id: "governance.tenant-policy.v1".to_string(),
                reason: "Governance contract binds tenant and policy version".to_string(),
                matched_signals: vec![
                    format!("tenant_id={}", reality.tenant_id),
                    format!("policy_version={}", reality.policy_version),
                ],
            },
            PriorHitReason {
                domain: "promotion_policy".to_string(),
                selected_ref: promotion_policy_ref,
                selected_version: if strict_runtime { "v2" } else { "v1" }.to_string(),
                rule_id: "promotion.runtime-budget.v1".to_string(),
                reason: "Promotion policy prioritizes strict safety first, then cost envelope"
                    .to_string(),
                matched_signals: vec![
                    format!("runtime_mode={}", reality.runtime_mode),
                    format!("budget_micros={}", reality.budget_micros),
                    format!("latency_budget_ms={}", reality.latency_budget_ms),
                ],
            },
        ];

        let replay_payload = serde_json::json!({
            "catalog_version": PRIOR_CATALOG_VERSION,
            "reality": {
                "tenant_id": reality.tenant_id,
                "policy_version": reality.policy_version,
                "runtime_mode": reality.runtime_mode,
                "budget_micros": reality.budget_micros,
                "latency_budget_ms": reality.latency_budget_ms,
                "tool_digest": reality.tool_digest,
                "policy_digest": reality.policy_digest,
                "budget_digest": reality.budget_digest,
                "reality_fingerprint": reality.reality_fingerprint,
            },
            "selection": {
                "template_refs": &selection.template_refs,
                "reusable_subgraph_refs": &selection.reusable_subgraph_refs,
                "governance_contract_refs": &selection.governance_contract_refs,
                "strategy_priors": &selection.strategy_priors,
                "promotion_policy_refs": &selection.promotion_policy_refs,
            },
            "hit_reasons": hit_reasons.iter().map(|item| serde_json::json!({
                "domain": item.domain,
                "selected_ref": item.selected_ref,
                "selected_version": item.selected_version,
                "rule_id": item.rule_id,
                "reason": item.reason,
                "matched_signals": item.matched_signals,
            })).collect::<Vec<_>>(),
        });
        let replay_fingerprint = replay::build_fingerprint(
            "priorfp",
            PRIOR_REPLAY_SCHEMA_VERSION,
            PRIOR_REPLAY_SEED_VERSION,
            PRIOR_REPLAY_VERSION,
            &replay_payload,
        );

        CrystalPriorDecision {
            catalog_version: PRIOR_CATALOG_VERSION.to_string(),
            replay_fingerprint,
            replay_schema_version: PRIOR_REPLAY_SCHEMA_VERSION.to_string(),
            replay_seed_version: PRIOR_REPLAY_SEED_VERSION.to_string(),
            replay_version: PRIOR_REPLAY_VERSION.to_string(),
            selection,
            hit_reasons,
        }
    }
}

fn is_strict_runtime_mode(mode: &str) -> bool {
    let lowered = mode.to_ascii_lowercase();
    lowered.contains("strict") || lowered.contains("enforcing") || lowered.contains("hard")
}

fn extract_policy_major(policy_version: &str) -> u32 {
    let digits = policy_version
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse::<u32>().ok().filter(|v| *v > 0).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn sample_reality() -> RealitySnapshot {
        RealitySnapshot {
            snapshot_id: "reality:s:1".into(),
            session_id: "session-s".into(),
            trace_id: "trace-s".into(),
            tenant_id: "tenant-a".into(),
            policy_version: "policy-v3".into(),
            runtime_mode: "strict".into(),
            available_tools: vec!["tool:planner".into(), "tool:verifier".into()],
            memory_refs: vec!["memory:latest".into()],
            graph_refs: vec!["graph:latest".into()],
            budget_micros: 100_000,
            latency_budget_ms: 2_500,
            repo_refs: vec!["repo://autoloop-app".into()],
            policy_refs: vec!["policy:tenant-a:default".into()],
            tool_refs: vec!["tool:planner".into(), "tool:verifier".into()],
            budget_profile: BTreeMap::from([("token_budget".into(), 100_000_u64)]),
            repo_digest: "repo:digest".into(),
            memory_digest: "memory:digest".into(),
            graph_digest: "graph:digest".into(),
            policy_digest: "policy:digest".into(),
            tool_digest: "tool:digest".into(),
            budget_digest: "budget:digest".into(),
            reality_fingerprint: "realityfp:digest".into(),
            created_at_ms: 1_710_000_000_000,
        }
    }

    #[test]
    fn prior_selection_is_explainable_and_has_domain_coverage() {
        let layer = CrystalPriorLayer;
        let decision = layer.select_with_trace(&sample_reality());

        assert_eq!(decision.catalog_version, "prior-catalog:v1");
        assert!(!decision.replay_fingerprint.is_empty());
        assert_eq!(decision.replay_schema_version, "prior-replay/v1");
        assert_eq!(decision.replay_seed_version, "prior-seed/v1");
        assert_eq!(decision.replay_version, "prior-replay-contract/v1");
        assert!(!decision.selection.template_refs.is_empty());
        assert!(!decision.selection.reusable_subgraph_refs.is_empty());
        assert!(!decision.selection.governance_contract_refs.is_empty());
        assert!(!decision.selection.promotion_policy_refs.is_empty());
        assert!(
            decision
                .hit_reasons
                .iter()
                .any(|item| item.domain == "template")
        );
        assert!(
            decision
                .hit_reasons
                .iter()
                .any(|item| item.domain == "governance_contract")
        );
    }

    #[test]
    fn prior_selection_is_replayable_for_same_input() {
        let layer = CrystalPriorLayer;
        let reality = sample_reality();
        let first = layer.select_with_trace(&reality);
        let second = layer.select_with_trace(&reality);

        assert_eq!(first.replay_fingerprint, second.replay_fingerprint);
        assert_eq!(first.selection.template_refs, second.selection.template_refs);
        assert_eq!(
            first.selection.reusable_subgraph_refs,
            second.selection.reusable_subgraph_refs
        );
        assert_eq!(
            first.selection.governance_contract_refs,
            second.selection.governance_contract_refs
        );
        assert_eq!(
            first.selection.promotion_policy_refs,
            second.selection.promotion_policy_refs
        );
    }
}
