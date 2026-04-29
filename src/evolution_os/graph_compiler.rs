use std::collections::BTreeMap;

use crate::contracts::evolution_os::{CandidateGraph, RealitySnapshot};

use super::prior::CrystalPriorSelection;

#[derive(Debug, Clone)]
pub struct GraphCompileInput<'a> {
    pub reality: &'a RealitySnapshot,
    pub priors: &'a CrystalPriorSelection,
}

#[derive(Debug, Clone)]
struct StrategyProfile {
    id: String,
    retrieval_ratio_bps: u32,
    verification_ratio_bps: u32,
    execution_ratio_bps: u32,
    risk_bias_bps: i32,
    latency_bias_bps: u32,
}

#[derive(Debug, Clone)]
struct CandidateDraft {
    strategy: StrategyProfile,
    template: String,
    subgraph: String,
    budget_allocation: BTreeMap<String, u64>,
    expected_cost_micros: u64,
    expected_latency_ms: u64,
    expected_risk_score: f32,
    rank_score: f32,
    key: String,
}

#[derive(Debug, Clone)]
pub struct DynamicRuntimeGraphCompiler {
    max_candidates: usize,
    exploration_multiplier: usize,
}

impl Default for DynamicRuntimeGraphCompiler {
    fn default() -> Self {
        Self {
            max_candidates: 8,
            exploration_multiplier: 4,
        }
    }
}

impl DynamicRuntimeGraphCompiler {
    pub fn compile(&self, input: GraphCompileInput<'_>) -> Vec<CandidateGraph> {
        let strategies = strategy_profiles(input.priors);
        let templates = normalized_or_default(&input.priors.template_refs, "template:default");
        let subgraphs = normalized_or_default(&input.priors.reusable_subgraph_refs, "subgraph:default");

        let risk_threshold = risk_threshold(input.reality);
        let exploration_cap = self.max_candidates.max(1) * self.exploration_multiplier.max(1);

        let mut drafts = Vec::<CandidateDraft>::new();
        let mut combo_idx = 0usize;
        for strategy in &strategies {
            for template in &templates {
                for subgraph in &subgraphs {
                    if drafts.len() >= exploration_cap {
                        break;
                    }

                    let budget_allocation =
                        split_budget(input.reality.budget_micros, strategy, input.reality);
                    let expected_cost_micros = expected_cost(&budget_allocation, combo_idx);
                    let expected_latency_ms =
                        expected_latency(input.reality.latency_budget_ms, strategy, combo_idx);
                    let expected_risk_score = expected_risk(strategy, template, subgraph, combo_idx);
                    let key = format!("{}|{}|{}", strategy.id, template, subgraph);
                    let rank_score = rank_score(
                        expected_risk_score,
                        expected_cost_micros,
                        expected_latency_ms,
                        input.reality,
                    );

                    combo_idx = combo_idx.saturating_add(1);
                    drafts.push(CandidateDraft {
                        strategy: strategy.clone(),
                        template: template.clone(),
                        subgraph: subgraph.clone(),
                        budget_allocation,
                        expected_cost_micros,
                        expected_latency_ms,
                        expected_risk_score,
                        rank_score,
                        key,
                    });
                }
            }
        }

        // stable pre-sort: best rank first, then risk/cost/latency, finally lexical key
        drafts.sort_by(|a, b| {
            b.rank_score
                .total_cmp(&a.rank_score)
                .then(a.expected_risk_score.total_cmp(&b.expected_risk_score))
                .then(a.expected_cost_micros.cmp(&b.expected_cost_micros))
                .then(a.expected_latency_ms.cmp(&b.expected_latency_ms))
                .then(a.key.cmp(&b.key))
        });

        let mut selected = Vec::<CandidateDraft>::new();
        let mut per_template = BTreeMap::<String, usize>::new();
        let mut per_strategy = BTreeMap::<String, usize>::new();

        for draft in drafts
            .iter()
            .filter(|item| item.expected_risk_score <= risk_threshold)
        {
            if selected.len() >= self.max_candidates.max(1) {
                break;
            }

            let strategy_slot = per_strategy.entry(draft.strategy.id.clone()).or_insert(0);
            if *strategy_slot >= 2 {
                continue;
            }

            let template_slot = per_template.entry(draft.template.clone()).or_insert(0);
            if *template_slot >= 2 {
                continue;
            }

            if is_near_duplicate(&selected, draft) {
                continue;
            }

            *strategy_slot += 1;
            *template_slot += 1;
            selected.push(draft.clone());
        }

        // risk clipping fallback: if nothing passed threshold, keep the safest candidate so pipeline stays alive.
        if selected.is_empty() && !drafts.is_empty() {
            let safest = drafts
                .iter()
                .min_by(|a, b| {
                    a.expected_risk_score
                        .total_cmp(&b.expected_risk_score)
                        .then(a.expected_cost_micros.cmp(&b.expected_cost_micros))
                        .then(a.key.cmp(&b.key))
                })
                .expect("non-empty drafts");
            selected.push(safest.clone());
        }

        selected
            .iter()
            .enumerate()
            .map(|(idx, draft)| build_candidate_graph(idx, draft, input.reality))
            .collect()
    }
}

fn normalized_or_default(values: &[String], default: &str) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    if normalized.is_empty() {
        vec![default.to_string()]
    } else {
        normalized
    }
}

fn strategy_profiles(priors: &CrystalPriorSelection) -> Vec<StrategyProfile> {
    let mut profiles = Vec::<StrategyProfile>::new();
    let mut normalized = priors
        .strategy_priors
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();

    for raw in normalized {
        let lowered = raw.to_ascii_lowercase();
        if lowered.contains("risk-first") {
            profiles.push(StrategyProfile {
                id: raw,
                retrieval_ratio_bps: 3000,
                verification_ratio_bps: 3500,
                execution_ratio_bps: 3500,
                risk_bias_bps: -900,
                latency_bias_bps: 10500,
            });
            continue;
        }
        if lowered.contains("latency") {
            profiles.push(StrategyProfile {
                id: raw,
                retrieval_ratio_bps: 2600,
                verification_ratio_bps: 2400,
                execution_ratio_bps: 5000,
                risk_bias_bps: 300,
                latency_bias_bps: 8200,
            });
            continue;
        }
        if lowered.contains("cost") {
            profiles.push(StrategyProfile {
                id: raw,
                retrieval_ratio_bps: 2200,
                verification_ratio_bps: 2800,
                execution_ratio_bps: 5000,
                risk_bias_bps: 200,
                latency_bias_bps: 9200,
            });
            continue;
        }

        profiles.push(StrategyProfile {
            id: raw,
            retrieval_ratio_bps: 2500,
            verification_ratio_bps: 3000,
            execution_ratio_bps: 4500,
            risk_bias_bps: 0,
            latency_bias_bps: 10000,
        });
    }

    if profiles.is_empty() {
        profiles.push(StrategyProfile {
            id: "strategy:baseline".to_string(),
            retrieval_ratio_bps: 2500,
            verification_ratio_bps: 3000,
            execution_ratio_bps: 4500,
            risk_bias_bps: 0,
            latency_bias_bps: 10000,
        });
    }

    profiles
}

fn risk_threshold(reality: &RealitySnapshot) -> f32 {
    let strict = reality.runtime_mode.to_ascii_lowercase().contains("strict")
        || reality.runtime_mode.to_ascii_lowercase().contains("enforcing");
    let low_budget = reality.budget_micros <= 80_000 || reality.latency_budget_ms <= 1_500;
    if strict {
        if low_budget {
            0.40
        } else {
            0.45
        }
    } else if low_budget {
        0.60
    } else {
        0.72
    }
}

fn split_budget(
    total_budget: u64,
    strategy: &StrategyProfile,
    reality: &RealitySnapshot,
) -> BTreeMap<String, u64> {
    let total = total_budget.max(1);
    let ratio_sum = (strategy.retrieval_ratio_bps
        + strategy.verification_ratio_bps
        + strategy.execution_ratio_bps)
        .max(1) as u128;

    let mut retrieval =
        ((total as u128 * strategy.retrieval_ratio_bps as u128) / ratio_sum) as u64;
    let mut verification =
        ((total as u128 * strategy.verification_ratio_bps as u128) / ratio_sum) as u64;
    let mut execution =
        ((total as u128 * strategy.execution_ratio_bps as u128) / ratio_sum) as u64;

    // lightweight policy-aware tuning from budget profile.
    if let Some(value) = reality.budget_profile.get("retrieval_floor") {
        retrieval = retrieval.max(*value);
    }
    if let Some(value) = reality.budget_profile.get("verification_floor") {
        verification = verification.max(*value);
    }
    if let Some(value) = reality.budget_profile.get("execution_floor") {
        execution = execution.max(*value);
    }

    let mut budget = BTreeMap::from([
        ("retrieval".to_string(), retrieval),
        ("verification".to_string(), verification),
        ("execution".to_string(), execution),
    ]);

    let current_sum = budget.values().copied().sum::<u64>();
    if current_sum > total {
        let overflow = current_sum - total;
        let entry = budget.entry("execution".to_string()).or_insert(0);
        *entry = entry.saturating_sub(overflow);
    } else if current_sum < total {
        let delta = total - current_sum;
        let entry = budget.entry("execution".to_string()).or_insert(0);
        *entry = entry.saturating_add(delta);
    }

    budget
}

fn expected_cost(budget: &BTreeMap<String, u64>, idx: usize) -> u64 {
    let retrieval = budget.get("retrieval").copied().unwrap_or(0);
    let verification = budget.get("verification").copied().unwrap_or(0);
    let execution = budget.get("execution").copied().unwrap_or(0);
    retrieval / 3 + verification / 2 + execution / 4 + (idx as u64 * 37)
}

fn expected_latency(latency_budget_ms: u64, strategy: &StrategyProfile, idx: usize) -> u64 {
    let base = ((latency_budget_ms as u128 * strategy.latency_bias_bps as u128) / 10_000) as u64;
    base.saturating_add((idx as u64) * 15).max(1)
}

fn expected_risk(
    strategy: &StrategyProfile,
    template: &str,
    subgraph: &str,
    idx: usize,
) -> f32 {
    let mut base_bps = 2400 + strategy.risk_bias_bps + (idx as i32 * 120);
    let template_lower = template.to_ascii_lowercase();
    let subgraph_lower = subgraph.to_ascii_lowercase();

    if template_lower.contains("strict") {
        base_bps -= 250;
    }
    if template_lower.contains("balanced") {
        base_bps -= 120;
    }
    if subgraph_lower.contains("verify") {
        base_bps -= 200;
    }
    if subgraph_lower.contains("repair") || subgraph_lower.contains("escalate") {
        base_bps -= 140;
    }

    (base_bps as f32 / 10_000.0).clamp(0.05, 0.95)
}

fn rank_score(
    risk_score: f32,
    cost_micros: u64,
    latency_ms: u64,
    reality: &RealitySnapshot,
) -> f32 {
    let risk_component = (1.0 - risk_score).clamp(0.0, 1.0) * 0.55;
    let cost_component = (1.0 - (cost_micros as f32 / reality.budget_micros.max(1) as f32))
        .clamp(0.0, 1.0)
        * 0.25;
    let latency_component =
        (1.0 - (latency_ms as f32 / reality.latency_budget_ms.max(1) as f32)).clamp(0.0, 1.0)
            * 0.20;
    risk_component + cost_component + latency_component
}

fn is_near_duplicate(selected: &[CandidateDraft], incoming: &CandidateDraft) -> bool {
    selected.iter().any(|current| {
        current.template == incoming.template
            && current.subgraph == incoming.subgraph
            && (current.expected_risk_score - incoming.expected_risk_score).abs() < 0.02
            && current.expected_latency_ms.abs_diff(incoming.expected_latency_ms) < 30
    })
}

fn build_candidate_graph(
    idx: usize,
    draft: &CandidateDraft,
    reality: &RealitySnapshot,
) -> CandidateGraph {
    let template_slug = slugify(&draft.template);
    let strategy_slug = slugify(&draft.strategy.id);
    let subgraph_slug = slugify(&draft.subgraph);

    let mut node_ids = vec![
        "ingest".to_string(),
        format!("retrieve:{template_slug}"),
        format!("plan:{strategy_slug}"),
        "execute".to_string(),
        "verify".to_string(),
    ];
    if draft.subgraph.to_ascii_lowercase().contains("repair")
        || draft.subgraph.to_ascii_lowercase().contains("escalate")
    {
        node_ids.push("repair".to_string());
    }

    let mut edges = vec![
        ("ingest".to_string(), format!("retrieve:{template_slug}")),
        (
            format!("retrieve:{template_slug}"),
            format!("plan:{strategy_slug}"),
        ),
        (format!("plan:{strategy_slug}"), "execute".to_string()),
        ("execute".to_string(), "verify".to_string()),
    ];
    if node_ids.iter().any(|node| node == "repair") {
        edges.push(("verify".to_string(), "repair".to_string()));
    }

    CandidateGraph {
        candidate_id: format!("candidate:{}:{}:{}", reality.session_id, idx + 1, strategy_slug),
        reality_snapshot_id: reality.snapshot_id.clone(),
        graph_version: format!("graph:v2:{strategy_slug}:{template_slug}:{subgraph_slug}"),
        node_ids,
        edges,
        budget_allocation: draft.budget_allocation.clone(),
        expected_cost_micros: draft.expected_cost_micros,
        expected_latency_ms: draft.expected_latency_ms,
        expected_risk_score: draft.expected_risk_score,
        generated_at_ms: reality.created_at_ms,
    }
}

fn slugify(value: &str) -> String {
    let lowered = value.to_ascii_lowercase();
    lowered
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_reality() -> RealitySnapshot {
        RealitySnapshot {
            snapshot_id: "reality:s:1".into(),
            session_id: "session-s".into(),
            trace_id: "trace-s".into(),
            tenant_id: "tenant-a".into(),
            policy_version: "policy-v3".into(),
            runtime_mode: "strict".into(),
            available_tools: vec!["tool:a".into(), "tool:verifier".into()],
            memory_refs: vec!["memory:latest".into()],
            graph_refs: vec!["graph:latest".into()],
            budget_micros: 120_000,
            latency_budget_ms: 4_000,
            repo_refs: vec!["repo://autoloop-app".into()],
            policy_refs: vec!["policy:tenant-a:default".into()],
            tool_refs: vec!["tool:a".into(), "tool:verifier".into()],
            budget_profile: std::collections::BTreeMap::from([
                ("token_budget".into(), 120_000_u64),
                ("latency_budget_ms".into(), 4_000_u64),
            ]),
            repo_digest: "repo:digest".into(),
            memory_digest: "memory:digest".into(),
            graph_digest: "graph:digest".into(),
            policy_digest: "policy:digest".into(),
            tool_digest: "tool:digest".into(),
            budget_digest: "budget:digest".into(),
            reality_fingerprint: "fp:digest".into(),
            created_at_ms: 1_710_000_000_000,
        }
    }

    #[test]
    fn compile_candidate_count_and_order_stable() {
        let reality = sample_reality();
        let priors = CrystalPriorSelection {
            template_refs: vec![
                "template:governed-runtime:strict:v3".into(),
                "template:tenant-a:default:v3".into(),
            ],
            reusable_subgraph_refs: vec![
                "subgraph:retrieval-verify-route:v1".into(),
                "subgraph:repair-escalate-loop:v1".into(),
            ],
            governance_contract_refs: vec!["contract:tenant-a:policy-v3:v3".into()],
            strategy_priors: vec![
                "strategy:risk-first".into(),
                "strategy:latency-first".into(),
                "strategy:cost-aware".into(),
                "strategy:balanced".into(),
            ],
            promotion_policy_refs: vec!["promotion:board:strict:v2".into()],
        };

        let compiler = DynamicRuntimeGraphCompiler::default();
        let first = compiler.compile(GraphCompileInput {
            reality: &reality,
            priors: &priors,
        });
        let second = compiler.compile(GraphCompileInput {
            reality: &reality,
            priors: &priors,
        });

        assert!(!first.is_empty(), "expected non-empty candidate set");
        assert_eq!(first.len(), second.len(), "candidate count should be stable");
        assert_eq!(
            first
                .iter()
                .map(|g| g.graph_version.clone())
                .collect::<Vec<_>>(),
            second
                .iter()
                .map(|g| g.graph_version.clone())
                .collect::<Vec<_>>(),
            "candidate order should be stable for same input"
        );
    }

    #[test]
    fn compile_applies_risk_clipping_and_budget_conservation() {
        let mut reality = sample_reality();
        reality.runtime_mode = "strict-enforcing".into();
        reality.budget_micros = 80_000;
        reality.latency_budget_ms = 1_200;

        let priors = CrystalPriorSelection {
            template_refs: vec!["template:governed-runtime:strict:v3".into()],
            reusable_subgraph_refs: vec!["subgraph:retrieval-verify-route:v1".into()],
            governance_contract_refs: vec!["contract:tenant-a:policy-v3:v3".into()],
            strategy_priors: vec![
                "strategy:latency-first".into(),
                "strategy:cost-aware".into(),
                "strategy:risk-first".into(),
            ],
            promotion_policy_refs: vec!["promotion:board:strict:v2".into()],
        };

        let compiler = DynamicRuntimeGraphCompiler::default();
        let graphs = compiler.compile(GraphCompileInput {
            reality: &reality,
            priors: &priors,
        });

        assert!(!graphs.is_empty());
        assert!(graphs.len() <= 8, "should respect max candidate cap");
        let threshold = risk_threshold(&reality);
        assert!(
            graphs.iter().all(|g| g.expected_risk_score <= threshold),
            "strict mode should clip candidates above risk threshold"
        );
        assert!(
            graphs
                .iter()
                .all(|g| g.budget_allocation.values().copied().sum::<u64>() == reality.budget_micros)
        );
    }
}
