use serde::{Deserialize, Serialize};

use crate::contracts::evolution_os::{CandidateGraph, RealitySnapshot, WorldlineScore};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposalKind {
    PromptPatch,
    GraphPatch,
    VerifierPlacementPatch,
    RoutingBudgetPatch,
    ReusableSubgraphPatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExternalProposalSignals {
    pub foundry_promotion_hints: Vec<serde_json::Value>,
    pub patch_reviews: Vec<serde_json::Value>,
    pub plugin_lifecycle_updates: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionProposal {
    pub proposal_id: String,
    pub candidate_id: String,
    pub kind: ProposalKind,
    pub source_bus: String,
    pub proposal_only: bool,
    pub summary: String,
    pub expected_lift: f32,
}

#[derive(Debug, Clone)]
pub struct ProposalSelectionInput<'a> {
    pub reality: &'a RealitySnapshot,
    pub candidates: &'a [CandidateGraph],
    pub scores: &'a [WorldlineScore],
    pub external_signals: Option<&'a ExternalProposalSignals>,
}

#[derive(Debug, Clone, Default)]
pub struct ControlledProposalHub;

impl ControlledProposalHub {
    pub fn select(&self, input: ProposalSelectionInput<'_>) -> Vec<EvolutionProposal> {
        let mut proposals = Vec::<EvolutionProposal>::new();

        for (candidate, score) in input.candidates.iter().zip(input.scores.iter()) {
            if score.total_score < 0.0 {
                continue;
            }

            proposals.push(self.task_agent_proposal(input.reality, candidate, score));
            proposals.push(self.meta_agent_proposal(input.reality, candidate, score));
        }

        if let Some(signals) = input.external_signals {
            self.merge_external_signals(&mut proposals, input.reality, signals);
        }

        // Hard invariant: proposal bus may emit proposals only, never direct-write intents.
        proposals.iter_mut().for_each(|item| {
            item.proposal_only = true;
        });

        proposals.sort_by(|a, b| {
            b.expected_lift
                .partial_cmp(&a.expected_lift)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        retain_priority_buses(proposals, 8)
    }

    fn task_agent_proposal(
        &self,
        reality: &RealitySnapshot,
        candidate: &CandidateGraph,
        score: &WorldlineScore,
    ) -> EvolutionProposal {
        let kind = if score.cost_penalty > 0.5 {
            ProposalKind::RoutingBudgetPatch
        } else {
            ProposalKind::GraphPatch
        };
        EvolutionProposal {
            proposal_id: format!(
                "proposal:{}:task:{}",
                reality.session_id, candidate.candidate_id
            ),
            candidate_id: candidate.candidate_id.clone(),
            kind,
            source_bus: "task.agent".to_string(),
            proposal_only: true,
            summary: format!(
                "proposal-only task bus selected {} with score {:.3}",
                candidate.candidate_id, score.total_score
            ),
            expected_lift: (score.task_success + score.robustness + score.total_score.max(0.0))
                / 3.0,
        }
    }

    fn meta_agent_proposal(
        &self,
        reality: &RealitySnapshot,
        candidate: &CandidateGraph,
        score: &WorldlineScore,
    ) -> EvolutionProposal {
        let kind = if score.verifier_confidence < 0.6 {
            ProposalKind::VerifierPlacementPatch
        } else {
            ProposalKind::ReusableSubgraphPatch
        };
        EvolutionProposal {
            proposal_id: format!(
                "proposal:{}:meta:{}",
                reality.session_id, candidate.candidate_id
            ),
            candidate_id: candidate.candidate_id.clone(),
            kind,
            source_bus: "meta.agent".to_string(),
            proposal_only: true,
            summary: format!(
                "proposal-only meta bus evaluated {} (verifier={:.2}, reuse={:.2})",
                candidate.candidate_id, score.verifier_confidence, score.reuse_gain
            ),
            expected_lift: ((1.0 - score.governance_violation_penalty).max(0.0)
                + score.reuse_gain.max(0.0))
                / 2.0,
        }
    }

    fn merge_external_signals(
        &self,
        proposals: &mut Vec<EvolutionProposal>,
        reality: &RealitySnapshot,
        signals: &ExternalProposalSignals,
    ) {
        for (idx, hint) in signals.foundry_promotion_hints.iter().enumerate() {
            let hint_id = hint
                .get("hint_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("foundry-hint");
            let reason = hint
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("foundry promotion hint");
            let direct_write = is_direct_write_signal(hint);
            let expected_lift = if hint
                .get("requires_human_approval")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true)
            {
                0.32
            } else {
                0.46
            };
            proposals.push(EvolutionProposal {
                proposal_id: format!("proposal:{}:foundry:{}:{idx}", reality.session_id, hint_id),
                candidate_id: format!("foundry:{hint_id}"),
                kind: ProposalKind::PromptPatch,
                source_bus: if direct_write {
                    "foundry.promotion.guard".to_string()
                } else {
                    "foundry.promotion".to_string()
                },
                proposal_only: true,
                summary: if direct_write {
                    format!(
                        "proposal-only guard converted blocked foundry direct-write request: {reason}"
                    )
                } else {
                    format!("proposal-only foundry hint queued: {reason}")
                },
                expected_lift,
            });
        }

        for (idx, review) in signals.patch_reviews.iter().enumerate() {
            let review_id = review
                .get("review_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("patch-review");
            let status = review
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("queued");
            let risk_score = review
                .get("decision")
                .and_then(|decision| decision.get("risk_score"))
                .and_then(serde_json::Value::as_f64)
                .map(|value| value as f32)
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            let direct_write = is_direct_write_signal(review);
            let kind = if status.eq_ignore_ascii_case("queued") {
                ProposalKind::GraphPatch
            } else {
                ProposalKind::ReusableSubgraphPatch
            };
            proposals.push(EvolutionProposal {
                proposal_id: format!("proposal:{}:patch:{}:{idx}", reality.session_id, review_id),
                candidate_id: format!("patch-review:{review_id}"),
                kind,
                source_bus: if direct_write {
                    "patch.review.guard".to_string()
                } else {
                    "patch.review".to_string()
                },
                proposal_only: true,
                summary: if direct_write {
                    format!(
                        "proposal-only guard converted blocked patch direct-write request: status={status}, risk={risk_score:.2}"
                    )
                } else {
                    format!(
                        "proposal-only patch review surfaced: status={status}, risk={risk_score:.2}"
                    )
                },
                expected_lift: (1.0 - risk_score) * 0.42 + 0.18,
            });
        }

        for (idx, plugin) in signals.plugin_lifecycle_updates.iter().enumerate() {
            let plugin_id = plugin
                .get("plugin_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("plugin:unknown");
            let state = plugin
                .get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("installed");
            let verified = plugin
                .get("verified")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let direct_write = is_direct_write_signal(plugin);
            let kind = if verified {
                ProposalKind::RoutingBudgetPatch
            } else {
                ProposalKind::VerifierPlacementPatch
            };
            let expected_lift = if verified { 0.38 } else { 0.16 };
            proposals.push(EvolutionProposal {
                proposal_id: format!("proposal:{}:plugin:{}:{idx}", reality.session_id, plugin_id),
                candidate_id: plugin_id.to_string(),
                kind,
                source_bus: if direct_write {
                    "plugin.lifecycle.guard".to_string()
                } else {
                    "plugin.lifecycle".to_string()
                },
                proposal_only: true,
                summary: if direct_write {
                    format!(
                        "proposal-only guard converted blocked plugin direct-write request: plugin={plugin_id}, state={state}, verified={verified}"
                    )
                } else {
                    format!(
                        "proposal-only plugin lifecycle suggestion: plugin={plugin_id}, state={state}, verified={verified}"
                    )
                },
                expected_lift,
            });
        }
    }
}

fn is_direct_write_signal(signal: &serde_json::Value) -> bool {
    let bool_flags = [
        "direct_write",
        "apply_immediately",
        "production_write",
        "write_production",
        "mutate_production",
    ];
    if bool_flags.iter().any(|flag| {
        signal
            .get(flag)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }) {
        return true;
    }

    let target_like = ["target", "target_path", "write_target", "destination"];
    for key in target_like {
        let Some(text) = signal.get(key).and_then(serde_json::Value::as_str) else {
            continue;
        };
        let lowered = text.to_ascii_lowercase();
        if lowered.contains("prod")
            || lowered.contains("production")
            || lowered.contains("canonical")
            || lowered.contains("runtime/live")
        {
            return true;
        }
    }

    false
}

fn retain_priority_buses(
    proposals: Vec<EvolutionProposal>,
    limit: usize,
) -> Vec<EvolutionProposal> {
    if proposals.len() <= limit {
        return proposals;
    }

    let priority_buses = [
        "task.agent",
        "meta.agent",
        "foundry.promotion",
        "patch.review",
        "plugin.lifecycle",
    ];
    let mut selected = Vec::<EvolutionProposal>::new();
    let mut used_ids = std::collections::BTreeSet::<String>::new();

    for bus in priority_buses {
        if let Some(item) = proposals.iter().find(|item| item.source_bus == bus) {
            selected.push(item.clone());
            used_ids.insert(item.proposal_id.clone());
        }
    }

    for item in proposals {
        if selected.len() >= limit {
            break;
        }
        if used_ids.contains(&item.proposal_id) {
            continue;
        }
        used_ids.insert(item.proposal_id.clone());
        selected.push(item);
    }

    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_reality() -> RealitySnapshot {
        RealitySnapshot {
            snapshot_id: "reality:test:1".to_string(),
            session_id: "session:test".to_string(),
            trace_id: "trace:test".to_string(),
            tenant_id: "tenant:test".to_string(),
            policy_version: "policy:v1".to_string(),
            runtime_mode: "shadow".to_string(),
            available_tools: vec![],
            memory_refs: vec![],
            graph_refs: vec![],
            budget_micros: 10_000,
            latency_budget_ms: 2_000,
            repo_refs: vec!["repo://test".to_string()],
            policy_refs: vec!["policy:test".to_string()],
            tool_refs: vec![],
            budget_profile: std::collections::BTreeMap::new(),
            repo_digest: "repo:test".to_string(),
            memory_digest: "memory:test".to_string(),
            graph_digest: "graph:test".to_string(),
            policy_digest: "policy:test".to_string(),
            tool_digest: "tool:test".to_string(),
            budget_digest: "budget:test".to_string(),
            reality_fingerprint: "fp:test".to_string(),
            created_at_ms: 1,
        }
    }

    fn sample_candidates_and_scores(reality: &RealitySnapshot) -> (Vec<CandidateGraph>, Vec<WorldlineScore>) {
        (
            vec![CandidateGraph {
                candidate_id: "g1".to_string(),
                reality_snapshot_id: reality.snapshot_id.clone(),
                graph_version: "graph:v1".to_string(),
                node_ids: vec!["n1".to_string()],
                edges: vec![],
                budget_allocation: std::collections::BTreeMap::new(),
                expected_cost_micros: 1_000,
                expected_latency_ms: 50,
                expected_risk_score: 0.1,
                generated_at_ms: 1,
            }],
            vec![WorldlineScore {
                candidate_id: "g1".to_string(),
                task_success: 0.9,
                robustness: 0.9,
                reuse_gain: 0.7,
                verifier_confidence: 0.9,
                cost_penalty: 0.1,
                latency_penalty: 0.1,
                risk_penalty: 0.1,
                instability_penalty: 0.1,
                governance_violation_penalty: 0.0,
                total_score: 1.8,
                reasons: vec![],
                scored_at_ms: 1,
            }],
        )
    }

    #[test]
    fn proposal_hub_merges_task_meta_and_external_buses_as_proposal_only() {
        let hub = ControlledProposalHub;
        let reality = sample_reality();
        let (candidates, scores) = sample_candidates_and_scores(&reality);

        let signals = ExternalProposalSignals {
            foundry_promotion_hints: vec![serde_json::json!({
                "hint_id": "hint-1",
                "reason": "upgrade S1 to S2",
                "requires_human_approval": true
            })],
            patch_reviews: vec![serde_json::json!({
                "review_id": "review-1",
                "status": "queued",
                "decision": {"risk_score": 0.8}
            })],
            plugin_lifecycle_updates: vec![serde_json::json!({
                "plugin_id": "plugin:graph-projection",
                "state": "enabled",
                "verified": true
            })],
        };

        let proposals = hub.select(ProposalSelectionInput {
            reality: &reality,
            candidates: &candidates,
            scores: &scores,
            external_signals: Some(&signals),
        });

        assert!(proposals.iter().any(|item| item.source_bus == "task.agent"));
        assert!(proposals.iter().any(|item| item.source_bus == "meta.agent"));
        assert!(proposals.iter().any(|item| item.source_bus == "foundry.promotion"));
        assert!(proposals.iter().any(|item| item.source_bus == "patch.review"));
        assert!(proposals.iter().any(|item| item.source_bus == "plugin.lifecycle"));
        assert!(proposals.iter().all(|item| item.proposal_only));
    }

    #[test]
    fn proposal_hub_blocks_external_direct_write_requests() {
        let hub = ControlledProposalHub;
        let reality = sample_reality();
        let (candidates, scores) = sample_candidates_and_scores(&reality);

        let signals = ExternalProposalSignals {
            foundry_promotion_hints: vec![serde_json::json!({
                "hint_id": "hint-blocked",
                "reason": "try direct write",
                "direct_write": true,
                "target_path": "production/runtime/live"
            })],
            patch_reviews: vec![serde_json::json!({
                "review_id": "review-blocked",
                "status": "approved",
                "apply_immediately": true,
                "target": "prod/patch"
            })],
            plugin_lifecycle_updates: vec![serde_json::json!({
                "plugin_id": "plugin:unsafe",
                "state": "enabled",
                "verified": false,
                "production_write": true
            })],
        };

        let proposals = hub.select(ProposalSelectionInput {
            reality: &reality,
            candidates: &candidates,
            scores: &scores,
            external_signals: Some(&signals),
        });

        assert!(proposals.iter().all(|item| item.proposal_only));
        assert!(
            proposals
                .iter()
                .any(|item| item.source_bus.ends_with(".guard") && item.summary.contains("blocked"))
        );
    }
}
