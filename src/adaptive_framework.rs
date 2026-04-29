use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplateProfile {
    pub stage: String,
    pub adaptation_type: String,
    pub preferred_surface: String,
    pub rollout_budget_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplateAsset {
    pub template_id: String,
    pub kind: String,
    pub title: String,
    pub instructions: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptTemplateBundle {
    pub profile: Option<PromptTemplateProfile>,
    pub system_templates: Vec<PromptTemplateAsset>,
    pub routing_templates: Vec<PromptTemplateAsset>,
    pub tool_templates: Vec<PromptTemplateAsset>,
    pub forge_templates: Vec<PromptTemplateAsset>,
    pub gaps: Vec<GapSignal>,
    pub duel_batches: Vec<DuelBatch>,
    pub feedback: Vec<PolicyFeedback>,
}

impl PromptTemplateBundle {
    pub fn all_directives(&self) -> Vec<String> {
        self.system_templates
            .iter()
            .chain(self.routing_templates.iter())
            .chain(self.tool_templates.iter())
            .chain(self.forge_templates.iter())
            .flat_map(|asset| asset.instructions.clone())
            .collect()
    }

    pub fn all_rationales(&self) -> Vec<String> {
        self.system_templates
            .iter()
            .chain(self.routing_templates.iter())
            .chain(self.tool_templates.iter())
            .chain(self.forge_templates.iter())
            .map(|asset| asset.rationale.clone())
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapSignal {
    pub id: String,
    pub topic: String,
    pub symptom: String,
    pub baseline_confidence: f32,
    pub target_confidence: f32,
    pub priority: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuelTask {
    pub id: String,
    pub gap_id: String,
    pub prompt: String,
    pub weight: f32,
    pub expected_gain: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuelBatch {
    pub gap: GapSignal,
    pub tasks: Vec<DuelTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateEffect {
    pub before: f32,
    pub after: f32,
    pub absolute_gain: f32,
    pub relative_gain_pct: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFeedback {
    pub gap_id: String,
    pub route_exploration_delta: f32,
    pub tool_specificity_delta: f32,
    pub forge_threshold_delta: f32,
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
    ptr: usize,
    path_start_idx: usize,
    max_size: usize,
    gamma: f32,
    lam: f32,
}

#[derive(Debug, Clone)]
struct GapDrivenTemplateGenerator {
    gamma: f32,
    lam: f32,
    horizon: usize,
}

impl GapDrivenTemplateGenerator {
    fn new(gamma: f32, lam: f32, horizon: usize) -> Self {
        Self {
            gamma,
            lam,
            horizon,
        }
    }

    fn generate_duel_batch(&self, gap: &GapSignal) -> DuelBatch {
        let mut buffer = StrategyBuffer::new(self.horizon, self.gamma, self.lam);
        let base_gap = (gap.target_confidence - gap.baseline_confidence).max(0.0);

        for t in 0..self.horizon {
            let progress = (t as f32 + 1.0) / self.horizon as f32;
            let reward = base_gap * (0.5 + progress) * gap.priority.max(0.25);
            let _ = buffer.store(reward, base_gap * 0.45);
        }

        buffer.finish_path(0.0);
        let batch = buffer.get().unwrap_or_else(|_| StrategyBatch {
            advantages: vec![0.3, 0.2, 0.1],
        });

        let tasks = batch
            .advantages
            .iter()
            .enumerate()
            .map(|(index, advantage)| {
                let weight = ((advantage + 3.0) / 6.0).clamp(0.05, 1.0);
                DuelTask {
                    id: format!("{}-duel-{index}", gap.id),
                    gap_id: gap.id.clone(),
                    prompt: format!(
                        "Close `{}` by addressing `{}` with bounded, evidence-grounded API prompting and verifier-safe execution.",
                        gap.topic, gap.symptom
                    ),
                    weight,
                    expected_gain: base_gap * weight,
                }
            })
            .collect();

        DuelBatch {
            gap: gap.clone(),
            tasks,
        }
    }
}

impl StrategyBuffer {
    fn new(size: usize, gamma: f32, lam: f32) -> Self {
        Self {
            rewards: vec![0.0; size],
            values: vec![0.0; size],
            advantages: vec![0.0; size],
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
        self.advantages[start..end].clone_from_slice(&adv);
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

pub fn build_prompt_template_bundle(
    profile: Option<PromptTemplateProfile>,
    evolution_summary: Option<&str>,
    research_summary: Option<&str>,
    capability_hints: &[String],
) -> PromptTemplateBundle {
    let evolution = evolution_summary.unwrap_or("Prefer bounded API-side adaptations.");
    let research = research_summary.unwrap_or("Use recent authoritative evidence when available.");
    let lowered_evolution = evolution.to_ascii_lowercase();
    let lowered_research = research.to_ascii_lowercase();

    let gaps = derive_gap_signals(evolution, research, capability_hints);
    let generator = GapDrivenTemplateGenerator::new(0.99, 0.95, 4);
    let duel_batches = gaps
        .iter()
        .map(|gap| generator.generate_duel_batch(gap))
        .collect::<Vec<_>>();
    let feedback = duel_batches
        .iter()
        .map(|batch| feedback_for_batch(batch))
        .collect::<Vec<_>>();
    let capability_text = if capability_hints.is_empty() {
        "No verified capability hints were recovered yet.".to_string()
    } else {
        format!(
            "Verified capability hints available: {}",
            capability_hints.join(", ")
        )
    };
    let top_gap = gaps.first().cloned().unwrap_or_else(default_gap_signal);
    let top_batch = duel_batches
        .first()
        .cloned()
        .unwrap_or_else(|| generator.generate_duel_batch(&top_gap));
    let effect = evaluate_batch_effect(&top_batch);
    let top_duels = top_batch
        .tasks
        .iter()
        .take(3)
        .map(|task| format!("{} (weight {:.2})", task.prompt, task.weight))
        .collect::<Vec<_>>();

    let mut system_instructions = vec![
        format!("Self-evolution guidance: {evolution}"),
        format!("Research grounding: {research}"),
        format!(
            "Primary adaptation gap: {} / {}",
            top_gap.topic, top_gap.symptom
        ),
        "Prefer prompt, routing, tool, and capability-policy changes over local weight updates."
            .into(),
    ];
    if lowered_evolution.contains("regression") || lowered_evolution.contains("drift") {
        system_instructions.push(
            "Bias toward narrower prompts, explicit constraints, and verifier checkpoints before broader synthesis."
                .into(),
        );
    }
    if lowered_research.contains("official") || lowered_research.contains("fresh") {
        system_instructions.push(
            "Prefer recent official or first-party evidence over older or derivative summaries."
                .into(),
        );
    }

    let mut routing_instructions = vec![
        capability_text.clone(),
        format!(
            "Route against the highest-priority gap `{}` with expected confidence lift {:.2}.",
            top_gap.topic, effect.absolute_gain
        ),
        "Prefer verified capabilities and recent successful execution channels.".into(),
        "If confidence is low, fall back to bounded forge-or-research loops before broad execution."
            .into(),
    ];
    routing_instructions.extend(
        top_duels
            .iter()
            .map(|duel| format!("Routing duel candidate: {duel}")),
    );

    let mut tool_instructions = vec![
        "Prefer the smallest tool action that can validate the next hypothesis.".into(),
        "Explain tool choices with verifier-safe, evidence-grounded language.".into(),
        format!(
            "Use batch-weighted execution order; current top gap confidence target is {:.2}.",
            top_gap.target_confidence
        ),
    ];
    tool_instructions.extend(
        top_batch
            .tasks
            .iter()
            .take(2)
            .map(|task| format!("Tool duel objective: {}", task.prompt)),
    );

    let mut forge_instructions = vec![
        capability_text,
        "Forge only when the active catalog lacks a verified capability for the objective.".into(),
        "Encode adaptive guidance, safety scope, and expected verifier checks into the forged wrapper."
            .into(),
        format!(
            "Current forge pressure delta: {:.2}",
            feedback
                .first()
                .map(|item| item.forge_threshold_delta)
                .unwrap_or(0.0)
        ),
    ];
    if capability_hints.is_empty() {
        forge_instructions.push(
            "No reusable capability was found; keep the forged wrapper minimal and objective-specific."
                .into(),
        );
    }

    PromptTemplateBundle {
        profile,
        system_templates: vec![PromptTemplateAsset {
            template_id: "system:adaptive-grounding".into(),
            kind: "system".into(),
            title: "Adaptive grounding".into(),
            instructions: system_instructions,
            rationale: format!(
                "System prompt reflects verifier-aligned strategy, live research evidence, and top gap `{}`.",
                top_gap.topic
            ),
        }],
        routing_templates: vec![PromptTemplateAsset {
            template_id: "routing:capability-first".into(),
            kind: "routing".into(),
            title: "Capability-first routing".into(),
            instructions: routing_instructions,
            rationale: format!(
                "Routing is optimized around duel-weighted gap closure with {:.1}% relative gain potential.",
                effect.relative_gain_pct
            ),
        }],
        tool_templates: vec![PromptTemplateAsset {
            template_id: "tool:bounded-execution".into(),
            kind: "tool".into(),
            title: "Bounded tool execution".into(),
            instructions: tool_instructions,
            rationale:
                "Tool prompts reduce scope expansion and keep execution inspectable under verifier pressure."
                    .into(),
        }],
        forge_templates: vec![PromptTemplateAsset {
            template_id: "forge:governed-capability".into(),
            kind: "forge".into(),
            title: "Governed capability forging".into(),
            instructions: forge_instructions,
            rationale:
                "Capability forging behaves like governed prompt-template specialization rather than local fine-tuning."
                    .into(),
        }],
        gaps,
        duel_batches,
        feedback,
    }
}

fn derive_gap_signals(
    evolution_summary: &str,
    research_summary: &str,
    capability_hints: &[String],
) -> Vec<GapSignal> {
    let mut gaps = Vec::new();
    let lowered_evolution = evolution_summary.to_ascii_lowercase();
    let lowered_research = research_summary.to_ascii_lowercase();

    if lowered_evolution.contains("route") || lowered_evolution.contains("drift") {
        gaps.push(GapSignal {
            id: "gap:routing".into(),
            topic: "routing-stability".into(),
            symptom: "route drift under verifier pressure".into(),
            baseline_confidence: 0.52,
            target_confidence: 0.76,
            priority: 0.95,
        });
    }

    if lowered_research.contains("official")
        || lowered_research.contains("fresh")
        || lowered_research.contains("report")
    {
        gaps.push(GapSignal {
            id: "gap:research".into(),
            topic: "research-grounding".into(),
            symptom: "fresh evidence underused".into(),
            baseline_confidence: 0.56,
            target_confidence: 0.8,
            priority: 0.88,
        });
    }

    if capability_hints.is_empty() {
        gaps.push(GapSignal {
            id: "gap:capability".into(),
            topic: "capability-coverage".into(),
            symptom: "verified catalog coverage missing".into(),
            baseline_confidence: 0.48,
            target_confidence: 0.74,
            priority: 1.0,
        });
    } else {
        gaps.push(GapSignal {
            id: "gap:reuse".into(),
            topic: "capability-reuse".into(),
            symptom: "available capabilities not fully exploited".into(),
            baseline_confidence: 0.6,
            target_confidence: 0.82,
            priority: 0.72,
        });
    }

    if gaps.is_empty() {
        gaps.push(default_gap_signal());
    }

    gaps
}

fn default_gap_signal() -> GapSignal {
    GapSignal {
        id: "gap:stability".into(),
        topic: "verifier-aligned-stability".into(),
        symptom: "insufficient bounded adaptation pressure".into(),
        baseline_confidence: 0.55,
        target_confidence: 0.75,
        priority: 0.7,
    }
}

fn evaluate_batch_effect(batch: &DuelBatch) -> TemplateEffect {
    let before = batch.gap.baseline_confidence;
    let weighted_sum: f32 = batch
        .tasks
        .iter()
        .map(|task| task.weight * task.expected_gain)
        .sum();
    let after = (before + weighted_sum).clamp(0.0, 1.0);
    let gain = (after - before).max(0.0);
    let relative = if before > 1e-8 {
        gain / before * 100.0
    } else {
        0.0
    };

    TemplateEffect {
        before,
        after,
        absolute_gain: gain,
        relative_gain_pct: relative,
    }
}

fn feedback_for_batch(batch: &DuelBatch) -> PolicyFeedback {
    let effect = evaluate_batch_effect(batch);
    if effect.relative_gain_pct >= 20.0 {
        PolicyFeedback {
            gap_id: batch.gap.id.clone(),
            route_exploration_delta: -0.08,
            tool_specificity_delta: 0.12,
            forge_threshold_delta: -0.1,
        }
    } else {
        PolicyFeedback {
            gap_id: batch.gap.id.clone(),
            route_exploration_delta: 0.1,
            tool_specificity_delta: 0.05,
            forge_threshold_delta: 0.08,
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

    #[test]
    fn prompt_template_bundle_contains_all_surfaces_and_first_principles() {
        let bundle = build_prompt_template_bundle(
            Some(PromptTemplateProfile {
                stage: "api-policy-adaptation".into(),
                adaptation_type: "prompt-route-tool-policy".into(),
                preferred_surface: "provider-api".into(),
                rollout_budget_ms: 1,
            }),
            Some("route drift observed, verifier regression rising"),
            Some("official sources were refreshed into the research report"),
            &["mcp::local-mcp::deploy".into()],
        );

        assert_eq!(bundle.system_templates.len(), 1);
        assert_eq!(bundle.routing_templates.len(), 1);
        assert_eq!(bundle.tool_templates.len(), 1);
        assert_eq!(bundle.forge_templates.len(), 1);
        assert!(!bundle.gaps.is_empty());
        assert!(!bundle.duel_batches.is_empty());
        assert!(!bundle.feedback.is_empty());
        assert!(
            bundle
                .all_directives()
                .iter()
                .any(|line| line.contains("Verified capability hints"))
        );
    }

    #[test]
    fn duel_batch_produces_weighted_tasks() {
        let generator = GapDrivenTemplateGenerator::new(0.99, 0.95, 4);
        let gap = GapSignal {
            id: "gap:test".into(),
            topic: "routing".into(),
            symptom: "drift".into(),
            baseline_confidence: 0.5,
            target_confidence: 0.75,
            priority: 1.0,
        };

        let batch = generator.generate_duel_batch(&gap);
        assert_eq!(batch.tasks.len(), 4);
        assert!(batch.tasks.iter().all(|task| task.weight > 0.0));
    }
}
