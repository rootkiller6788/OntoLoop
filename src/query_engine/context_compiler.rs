use anyhow::{Result, bail};

use crate::providers::ChatMessage;

use super::{
    compactor::CompactionBoundary,
    default_plugins::{
        DefaultContextCompilerPlugins, ObjectiveScore, ObjectiveWeights, ProofResult,
    },
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TokenBudgetFrame {
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub reserve_tokens: u32,
    pub compaction_trigger_tokens: u32,
}

impl Default for TokenBudgetFrame {
    fn default() -> Self {
        Self {
            max_input_tokens: 8_000,
            max_output_tokens: 2_000,
            reserve_tokens: 500,
            compaction_trigger_tokens: 6_000,
        }
    }
}

impl TokenBudgetFrame {
    pub fn bounded(max_input_tokens: u32, max_output_tokens: u32) -> Self {
        let reserve = (max_output_tokens / 4).clamp(64, 2_000);
        let trigger = max_input_tokens.saturating_sub(reserve).max(128);
        Self {
            max_input_tokens,
            max_output_tokens,
            reserve_tokens: reserve,
            compaction_trigger_tokens: trigger,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompiledContext {
    pub messages: Vec<ChatMessage>,
    pub estimated_tokens: u32,
    pub budget: TokenBudgetFrame,
    pub compaction_applied: bool,
    pub boundary: Option<CompactionBoundary>,
    pub proof: ProofResult,
    pub hardgate_pass_token: String,
    pub constraint_version: String,
    pub hard_constraint_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ContextCompiler {
    budget: TokenBudgetFrame,
    plugins: DefaultContextCompilerPlugins,
    objective_weights: ObjectiveWeights,
}

impl ContextCompiler {
    pub fn new(budget: TokenBudgetFrame, preserve_recent_messages: usize) -> Self {
        Self {
            budget,
            plugins: DefaultContextCompilerPlugins::new(preserve_recent_messages),
            objective_weights: load_objective_weights(),
        }
    }

    pub fn plugin_manifests(&self) -> Vec<crate::contracts::plugin::PluginManifestContract> {
        self.plugins.manifests()
    }

    pub fn compile(&self, messages: &[ChatMessage]) -> Result<CompiledContext> {
        let hard = self.plugins.constraint.evaluate(messages, &self.budget);
        if !hard.passed {
            bail!(
                "hardgate_reject: {} | constraint_ver={} | constraint_ids={}",
                hard.reason,
                hard.constraint_version,
                hard.constraint_ids.join(",")
            );
        }

        let estimation = self
            .plugins
            .estimator
            .estimate(&hard.pinned_messages, &hard.budget);
        let optimized = self
            .plugins
            .optimizer
            .optimize(&hard.pinned_messages, &hard.budget);
        let repaired = self.plugins.repair.repair(optimized, &hard.pinned_messages);
        let mut proof = self.plugins.proof.prove(
            &hard.pinned_messages,
            &repaired.messages,
            repaired.boundary.as_ref(),
        );
        let objective_score = compute_objective_score(
            &self.objective_weights,
            &estimation,
            repaired.estimated_tokens,
            hard.budget.max_input_tokens,
        );

        let hardgate_pass_token = build_hardgate_pass_token(
            &hard.pinned_messages,
            &hard.budget,
            repaired.boundary.as_ref(),
            &hard.constraint_version,
            &hard.constraint_ids,
        );
        let constraint_version = hard.constraint_version.clone();
        let hard_constraint_ids = hard.constraint_ids.clone();
        let replay_fp = crate::observability::event_stream::digest_value(&serde_json::json!({
            "messages": &repaired.messages,
            "boundary_id": repaired.boundary.as_ref().map(|b| b.boundary_id.clone()),
            "constraint_version": constraint_version,
            "constraint_ids": hard_constraint_ids,
        }));
        proof.objective = objective_score.clone();
        let prompt_pack = serde_json::json!({
            "message_count": repaired.messages.len(),
            "estimated_input_tokens": repaired.estimated_tokens,
            "budget": {
                "max_input_tokens": hard.budget.max_input_tokens,
                "max_output_tokens": hard.budget.max_output_tokens,
                "reserve_tokens": hard.budget.reserve_tokens,
                "compaction_trigger_tokens": hard.budget.compaction_trigger_tokens,
            },
            "boundary": repaired.boundary.as_ref().map(|b| serde_json::json!({
                "boundary_id": b.boundary_id,
                "summary": b.summary,
                "compressed_messages": b.compressed_messages,
                "preserved_messages": b.preserved_messages,
            })),
            "messages": repaired
                .messages
                .iter()
                .enumerate()
                .map(|(idx, msg)| serde_json::json!({
                    "index": idx,
                    "role": msg.role.clone(),
                    "content_preview": truncate_preview(&msg.content, 160),
                    "content_chars": msg.content.chars().count(),
                }))
                .collect::<Vec<_>>(),
        });
        let source_mapping = repaired
            .messages
            .iter()
            .enumerate()
            .map(|(idx, msg)| {
                serde_json::json!({
                    "compiled_index": idx,
                    "source_kind": "chat_message",
                    "role": msg.role.clone(),
                    "source_ref": format!("prompt_pack:{}:{}", msg.role, idx),
                })
            })
            .collect::<Vec<_>>();
        let risk_metadata = serde_json::json!({
            "risk_labels": proof.risk_flags.clone(),
            "semantic_distortion_risk": estimation.semantic_distortion_risk,
            "attention_mismatch_risk": estimation.attention_mismatch_risk,
            "malicious_intent_risk": estimation.malicious_intent_risk,
        });
        proof.annotation = serde_json::json!({
            "prompt_pack": prompt_pack,
            "dropped_mapping": [{
                "kind": "message_span",
                "dropped_count": proof.dropped_message_count,
                "boundary_id": repaired.boundary.as_ref().map(|b| b.boundary_id.clone()),
            }],
            "source_mapping": source_mapping,
            "compression_stats": {
                "estimated_input_tokens": repaired.estimated_tokens,
                "max_input_tokens": hard.budget.max_input_tokens,
                "compression_ratio": proof.compression_ratio,
            },
            "risk_labels": proof.risk_flags.clone(),
            "risk_metadata": risk_metadata,
            "policy_hints": [
                format!("constraint_ver={}", constraint_version),
                format!("objective_weights={}", serde_json::to_string(&self.objective_weights).unwrap_or_else(|_| "{}".into())),
            ],
            "replay_fingerprint": replay_fp,
            "decision_summary": {
                "hardgate_pass_token": hardgate_pass_token.clone(),
                "constraint_version": constraint_version,
                "constraint_ids": hard_constraint_ids,
                "objective": objective_score,
            }
        });

        Ok(CompiledContext {
            messages: repaired.messages,
            estimated_tokens: if repaired.boundary.is_some() {
                repaired.estimated_tokens
            } else {
                estimation.estimated_tokens
            },
            budget: hard.budget,
            compaction_applied: repaired.boundary.is_some(),
            boundary: repaired.boundary,
            proof,
            hardgate_pass_token,
            constraint_version: hard.constraint_version,
            hard_constraint_ids: hard.constraint_ids,
        })
    }
}

fn build_hardgate_pass_token(
    messages: &[ChatMessage],
    budget: &TokenBudgetFrame,
    boundary: Option<&CompactionBoundary>,
    constraint_version: &str,
    constraint_ids: &[String],
) -> String {
    let payload = serde_json::json!({
        "messages": messages,
        "budget": budget,
        "boundary_id": boundary.map(|b| b.boundary_id.clone()),
        "constraint_version": constraint_version,
        "constraint_ids": constraint_ids,
        "issued_at_ms": crate::orchestration::current_time_ms(),
    });
    let digest = crate::observability::event_stream::digest_value(&payload);
    format!("hgt:v2:{}", digest)
}

fn compute_objective_score(
    weights: &ObjectiveWeights,
    estimation: &super::default_plugins::EstimationResult,
    estimated_input_tokens: u32,
    max_input_tokens: u32,
) -> ObjectiveScore {
    let task_utility = estimation.relevance_score + (estimation.anchor_retention_benefit * 0.25);
    let distortion = estimation.semantic_distortion_risk;
    let attention_mismatch = estimation.attention_mismatch_risk;
    let token_cost = if max_input_tokens == 0 {
        1.0
    } else {
        (estimated_input_tokens as f32 / max_input_tokens as f32).clamp(0.0, 2.0)
    };
    let weighted_score = (weights.task_utility * task_utility)
        - (weights.distortion_penalty * distortion)
        - (weights.attention_mismatch_penalty * attention_mismatch)
        - (weights.token_cost_penalty * token_cost);
    ObjectiveScore {
        task_utility,
        distortion,
        attention_mismatch,
        token_cost,
        weighted_score,
    }
}

fn load_objective_weights() -> ObjectiveWeights {
    let raw = std::env::var("AUTOLOOP_CONTEXT_OBJECTIVE_WEIGHTS").unwrap_or_default();
    if raw.trim().is_empty() {
        return ObjectiveWeights::default();
    }
    serde_json::from_str::<ObjectiveWeights>(&raw).unwrap_or_else(|_| ObjectiveWeights::default())
}

fn truncate_preview(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let clipped = input.chars().take(max_chars).collect::<String>();
    format!("{clipped}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".into(),
                content: "policy anchor".into(),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: "build a governance-safe plan".into(),
            },
        ]
    }

    #[test]
    fn compiler_emits_constraint_protocol_and_annotation_schema() {
        let compiler = ContextCompiler::new(TokenBudgetFrame::bounded(120, 32), 2);
        let compiled = compiler.compile(&seed_messages()).expect("compile");
        assert!(!compiled.hard_constraint_ids.is_empty());
        assert!(!compiled.constraint_version.is_empty());
        assert!(compiled.hardgate_pass_token.starts_with("hgt:v2:"));

        let annotation = &compiled.proof.annotation;
        assert!(annotation.get("prompt_pack").is_some());
        assert!(annotation.get("dropped_mapping").is_some());
        assert!(annotation.get("source_mapping").is_some());
        assert!(annotation.get("compression_stats").is_some());
        assert!(annotation.get("risk_labels").is_some());
        assert!(annotation.get("risk_metadata").is_some());
        assert!(annotation.get("policy_hints").is_some());
        assert!(annotation.get("replay_fingerprint").is_some());
        assert!(annotation.get("decision_summary").is_some());
    }

    #[test]
    fn compiler_uses_objective_weight_override_when_valid_json() {
        unsafe {
            std::env::set_var(
                "AUTOLOOP_CONTEXT_OBJECTIVE_WEIGHTS",
                r#"{"task_utility":1.25,"distortion_penalty":1.1,"attention_mismatch_penalty":0.9,"token_cost_penalty":1.4}"#,
            );
        }
        let compiler = ContextCompiler::new(TokenBudgetFrame::bounded(120, 32), 2);
        let compiled = compiler.compile(&seed_messages()).expect("compile");
        let hints = compiled
            .proof
            .annotation
            .get("policy_hints")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let contains_weight_hint = hints.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|text| text.contains("objective_weights"))
        });
        assert!(contains_weight_hint);
        unsafe {
            std::env::remove_var("AUTOLOOP_CONTEXT_OBJECTIVE_WEIGHTS");
        }
    }
}
