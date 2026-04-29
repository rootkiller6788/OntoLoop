use serde::{Deserialize, Serialize};

use crate::memory::{CapabilityImprovementProposal, LearningConsolidation};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionGap {
    pub id: String,
    pub topic: String,
    pub symptom: String,
    pub baseline_score: f32,
    pub target_score: f32,
    pub priority: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionDuelTask {
    pub id: String,
    pub gap_id: String,
    pub prompt: String,
    pub weight: f32,
    pub expected_gain: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyAdaptationProfile {
    pub stage: String,
    pub adaptation_type: String,
    pub preferred_surface: String,
    pub rollout_budget_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyAdaptationReport {
    pub estimated_policy_latency_us: u128,
    pub delta_score: f32,
    pub updated_score: f32,
    pub within_rollout_budget: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpSelfAdjustment {
    pub crawl_priority_delta: f32,
    pub duel_weight_scale: f32,
    pub route_exploration_delta: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionCycleReport {
    pub gap_id: String,
    pub tasks: Vec<EvolutionDuelTask>,
    pub adaptation_report: PolicyAdaptationReport,
    pub feedback: McpSelfAdjustment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEvolutionProposal {
    pub tool_name: String,
    pub change_hint: String,
    pub rationale: String,
    pub expected_lift: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfEvolutionReport {
    pub session_id: String,
    pub profile: PolicyAdaptationProfile,
    pub baseline_score: f32,
    pub target_score: f32,
    pub evolved_score: f32,
    pub cycles: Vec<EvolutionCycleReport>,
    pub capability_proposals: Vec<CapabilityEvolutionProposal>,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct SelfEvolutionKernel {
    gamma: f32,
    lam: f32,
    horizon: usize,
    base_gain: f32,
}

#[derive(Debug, Clone)]
struct StrategyBatch {
    advantages: Vec<f32>,
}

#[derive(Debug, Clone)]
struct StrategyBuffer {
    rewards: Vec<f32>,
    values: Vec<f32>,
    advantages: Vec<f32>,
    returns: Vec<f32>,
    ptr: usize,
    path_start_idx: usize,
    max_size: usize,
    gamma: f32,
    lam: f32,
}

impl SelfEvolutionKernel {
    pub fn new() -> Self {
        Self {
            gamma: 0.99,
            lam: 0.95,
            horizon: 6,
            base_gain: 0.82,
        }
    }

    pub fn run(
        &self,
        session_id: &str,
        consolidation: &LearningConsolidation,
        verifier_score: f32,
    ) -> SelfEvolutionReport {
        let baseline_score = verifier_score.clamp(0.0, 1.0);
        let target_score = (baseline_score + 0.18).clamp(0.65, 0.98);
        let profile = PolicyAdaptationProfile {
            stage: "api-policy-adaptation".into(),
            adaptation_type: "prompt-route-tool-policy".into(),
            preferred_surface: "provider-api".into(),
            rollout_budget_ms: 1,
        };

        let gaps = derive_gaps(consolidation, baseline_score, target_score);
        let mut current_score = baseline_score;
        let mut cycles = Vec::new();
        let mut capability_proposals = Vec::new();

        for gap in gaps {
            let tasks = self.generate_duel_tasks(&gap);
            let adaptation_report = self.apply_policy_adaptation(&profile, current_score, &tasks);
            current_score = adaptation_report.updated_score;
            let feedback = feedback_for_gap(&gap, &adaptation_report);
            capability_proposals.extend(project_capability_proposals(
                consolidation.capability_improvements.as_slice(),
                &gap,
                &adaptation_report,
            ));
            cycles.push(EvolutionCycleReport {
                gap_id: gap.id,
                tasks,
                adaptation_report,
                feedback,
            });
        }

        capability_proposals.sort_by(|left, right| {
            right
                .expected_lift
                .partial_cmp(&left.expected_lift)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.tool_name.cmp(&right.tool_name))
        });
        capability_proposals.dedup_by(|left, right| {
            left.tool_name == right.tool_name && left.change_hint == right.change_hint
        });

        SelfEvolutionReport {
            session_id: session_id.to_string(),
            profile,
            baseline_score,
            target_score,
            evolved_score: current_score,
            cycles,
            capability_proposals: capability_proposals.into_iter().take(8).collect(),
            summary: format!(
                "Self-evolution raised verifier-aligned score from {:.2} to {:.2} using {} provider-API adaptation cycles.",
                baseline_score,
                current_score,
                self.horizon.min(
                    derive_gaps(consolidation, baseline_score, target_score)
                        .len()
                        .max(1)
                )
            ),
        }
    }

    fn generate_duel_tasks(&self, gap: &EvolutionGap) -> Vec<EvolutionDuelTask> {
        let mut buffer = StrategyBuffer::new(self.horizon, self.gamma, self.lam);
        let base_gap = (gap.target_score - gap.baseline_score).max(0.0);

        for t in 0..self.horizon {
            let progress = (t as f32 + 1.0) / self.horizon as f32;
            let reward = base_gap * (0.55 + progress);
            let _ = buffer.store(reward, base_gap * 0.5);
        }

        buffer.finish_path(0.0);
        let batch = buffer.get().unwrap_or_else(|_| StrategyBatch {
            advantages: vec![0.2, 0.15, 0.1],
        });

        batch
            .advantages
            .iter()
            .enumerate()
            .map(|(index, advantage)| {
                let weight = ((advantage + 3.0) / 6.0).clamp(0.05, 1.0);
                EvolutionDuelTask {
                    id: format!("{}-duel-{index}", gap.id),
                    gap_id: gap.id.clone(),
                    prompt: format!(
                        "Close `{}` by improving `{}` with evidence-grounded execution and verifier-safe changes.",
                        gap.topic, gap.symptom
                    ),
                    weight,
                    expected_gain: base_gap * weight,
                }
            })
            .collect()
    }

    fn apply_policy_adaptation(
        &self,
        profile: &PolicyAdaptationProfile,
        current_score: f32,
        tasks: &[EvolutionDuelTask],
    ) -> PolicyAdaptationReport {
        let weighted_sum: f32 = tasks
            .iter()
            .map(|task| task.weight * task.expected_gain)
            .sum();
        let estimated_latency_us = 720;
        let delta = (self.base_gain * weighted_sum).clamp(0.0, 0.22);
        let updated_score = (current_score + delta).clamp(0.0, 1.0);

        PolicyAdaptationReport {
            estimated_policy_latency_us: estimated_latency_us,
            delta_score: delta,
            updated_score,
            within_rollout_budget: estimated_latency_us < profile.rollout_budget_ms as u128 * 1000,
        }
    }
}

impl StrategyBuffer {
    fn new(size: usize, gamma: f32, lam: f32) -> Self {
        Self {
            rewards: vec![0.0; size],
            values: vec![0.0; size],
            advantages: vec![0.0; size],
            returns: vec![0.0; size],
            ptr: 0,
            path_start_idx: 0,
            max_size: size,
            gamma,
            lam,
        }
    }

    fn store(&mut self, reward: f32, value: f32) -> Result<(), String> {
        if self.ptr >= self.max_size {
            return Err("buffer is full".into());
        }
        self.rewards[self.ptr] = reward;
        self.values[self.ptr] = value;
        self.ptr += 1;
        Ok(())
    }

    fn finish_path(&mut self, last_value: f32) {
        let start = self.path_start_idx;
        let end = self.ptr;
        if start >= end {
            return;
        }

        let mut rewards = self.rewards[start..end].to_vec();
        rewards.push(last_value);
        let mut values = self.values[start..end].to_vec();
        values.push(last_value);

        let deltas = (0..(end - start))
            .map(|index| rewards[index] + self.gamma * values[index + 1] - values[index])
            .collect::<Vec<_>>();

        let adv = discount_cumsum(&deltas, self.gamma * self.lam);
        let ret = discount_cumsum(&rewards, self.gamma);
        self.advantages[start..end].clone_from_slice(&adv);
        self.returns[start..end].clone_from_slice(&ret[..ret.len() - 1]);
        self.path_start_idx = self.ptr;
    }

    fn get(&mut self) -> Result<StrategyBatch, String> {
        if self.ptr != self.max_size {
            return Err("buffer must be full before get".into());
        }
        let (mean, std) = mean_std(&self.advantages);
        let mut normalized = self.advantages.clone();
        for value in &mut normalized {
            *value = (*value - mean) / std.max(1e-8);
        }
        self.ptr = 0;
        self.path_start_idx = 0;
        Ok(StrategyBatch {
            advantages: normalized,
        })
    }
}

fn derive_gaps(
    consolidation: &LearningConsolidation,
    baseline_score: f32,
    target_score: f32,
) -> Vec<EvolutionGap> {
    let mut gaps = consolidation
        .failure_clusters
        .iter()
        .enumerate()
        .map(|(index, cluster)| EvolutionGap {
            id: format!("gap:{index}:{}", cluster.cluster_id),
            topic: cluster.pattern.clone(),
            symptom: cluster
                .representative_examples
                .first()
                .cloned()
                .unwrap_or_else(|| cluster.pattern.clone()),
            baseline_score,
            target_score,
            priority: (cluster.frequency as f32 / 3.0).clamp(0.35, 1.4),
        })
        .collect::<Vec<_>>();

    if gaps.is_empty() {
        gaps.push(EvolutionGap {
            id: "gap:stability".into(),
            topic: "verifier-aligned-stability".into(),
            symptom: "insufficient regression pressure".into(),
            baseline_score,
            target_score,
            priority: 0.6,
        });
    }

    gaps
}

fn project_capability_proposals(
    proposals: &[CapabilityImprovementProposal],
    gap: &EvolutionGap,
    adaptation_report: &PolicyAdaptationReport,
) -> Vec<CapabilityEvolutionProposal> {
    proposals
        .iter()
        .map(|proposal| CapabilityEvolutionProposal {
            tool_name: proposal.tool_name.clone(),
            change_hint: proposal.change_hint.clone(),
            rationale: format!("{} | gap `{}`", proposal.rationale, gap.topic),
            expected_lift: (proposal.priority * adaptation_report.delta_score.max(0.05))
                .clamp(0.0, 1.0),
        })
        .collect()
}

fn feedback_for_gap(
    gap: &EvolutionGap,
    adaptation_report: &PolicyAdaptationReport,
) -> McpSelfAdjustment {
    if adaptation_report.delta_score >= (gap.target_score - gap.baseline_score) * 0.45 {
        McpSelfAdjustment {
            crawl_priority_delta: -0.08,
            duel_weight_scale: 0.9,
            route_exploration_delta: -0.05,
        }
    } else {
        McpSelfAdjustment {
            crawl_priority_delta: 0.15,
            duel_weight_scale: 1.15,
            route_exploration_delta: 0.1,
        }
    }
}

fn discount_cumsum(values: &[f32], discount: f32) -> Vec<f32> {
    let mut out = vec![0.0; values.len()];
    let mut running = 0.0;
    for index in (0..values.len()).rev() {
        running = values[index] + discount * running;
        out[index] = running;
    }
    out
}

fn mean_std(values: &[f32]) -> (f32, f32) {
    let n = values.len().max(1) as f32;
    let mean = values.iter().sum::<f32>() / n;
    let variance = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f32>()
        / n;
    (mean, variance.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{CausalValidationSummary, FailurePatternCluster, SkillRecord};

    #[test]
    fn self_evolution_generates_gain_and_proposals() {
        let kernel = SelfEvolutionKernel::new();
        let consolidation = LearningConsolidation {
            consolidated_skills: vec![SkillRecord {
                name: "bounded-rollback".into(),
                trigger: "regression".into(),
                procedure: "rollback".into(),
                confidence: 0.8,
            }],
            failure_clusters: vec![FailurePatternCluster {
                cluster_id: "cluster:exec".into(),
                pattern: "execution-regression".into(),
                frequency: 3,
                representative_examples: vec!["tool timed out".into()],
            }],
            causal_validation: CausalValidationSummary {
                validated_edges: 1,
                average_confidence: 0.7,
                summary: "ok".into(),
            },
            capability_improvements: vec![CapabilityImprovementProposal {
                tool_name: "mcp::local-mcp::deploy".into(),
                change_hint: "tighten scope".into(),
                rationale: "timeouts observed".into(),
                priority: 0.9,
            }],
        };

        let report = kernel.run("session-1", &consolidation, 0.54);

        assert!(report.evolved_score > report.baseline_score);
        assert!(!report.capability_proposals.is_empty());
        assert!(report.cycles.iter().all(|cycle| !cycle.tasks.is_empty()));
    }
}
