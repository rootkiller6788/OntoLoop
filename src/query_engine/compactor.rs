use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::providers::ChatMessage;

use super::context_compiler::TokenBudgetFrame;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    Micro,
    Full,
}

impl CompactionStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Micro => "micro",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CompactionBoundary {
    pub boundary_id: String,
    pub compressed_messages: usize,
    pub preserved_messages: usize,
    pub summary: String,
    pub summary_digest: String,
    #[serde(default)]
    pub strategy: String,
    #[serde(default)]
    pub throttle_applied: bool,
    #[serde(default)]
    pub failed_attempts: u32,
}

#[derive(Debug, Clone)]
pub struct CompactionResult {
    pub messages: Vec<ChatMessage>,
    pub boundary: Option<CompactionBoundary>,
    pub estimated_tokens: u32,
    pub strategy: Option<CompactionStrategy>,
    pub throttle_applied: bool,
    pub failed_attempts: u32,
}

#[derive(Debug, Clone)]
pub struct ContextCompactor {
    preserve_recent_messages: usize,
}

impl ContextCompactor {
    pub fn new(preserve_recent_messages: usize) -> Self {
        Self {
            preserve_recent_messages,
        }
    }

    pub fn compact(&self, messages: &[ChatMessage], budget: &TokenBudgetFrame) -> CompactionResult {
        let estimated_tokens = estimate_message_tokens(messages);
        if estimated_tokens <= budget.compaction_trigger_tokens
            || messages.len() <= self.preserve_recent_messages.saturating_add(1)
        {
            return CompactionResult {
                messages: messages.to_vec(),
                boundary: None,
                estimated_tokens,
                strategy: None,
                throttle_applied: false,
                failed_attempts: 0,
            };
        }

        let micro_preserve = self
            .preserve_recent_messages
            .saturating_mul(2)
            .max(self.preserve_recent_messages.saturating_add(2));
        let micro = self.compact_with_stage(
            messages,
            micro_preserve,
            CompactionStrategy::Micro,
            0,
            false,
            8,
            16,
        );
        if micro.estimated_tokens <= budget.max_input_tokens {
            return micro;
        }

        let full = self.compact_with_stage(
            messages,
            self.preserve_recent_messages,
            CompactionStrategy::Full,
            1,
            false,
            20,
            28,
        );
        if full.estimated_tokens <= budget.max_input_tokens {
            return full;
        }

        self.apply_failure_throttle(full, budget)
    }

    fn compact_with_stage(
        &self,
        messages: &[ChatMessage],
        preserve_recent_messages: usize,
        strategy: CompactionStrategy,
        failed_attempts: u32,
        throttle_applied: bool,
        summary_lines: usize,
        summary_terms: usize,
    ) -> CompactionResult {
        let preserve = preserve_recent_messages
            .max(1)
            .min(messages.len().saturating_sub(1).max(1));
        let summary_cutoff = messages.len().saturating_sub(preserve);
        let compacted_slice = &messages[..summary_cutoff];
        let preserved_slice = &messages[summary_cutoff..];
        let summary = summarize_messages(compacted_slice, summary_lines, summary_terms);
        let summary_digest = digest_text(&summary);
        let boundary = CompactionBoundary {
            boundary_id: format!(
                "boundary:{}:{}:{}",
                strategy.as_str(),
                current_time_ms(),
                summary_digest
            ),
            compressed_messages: compacted_slice.len(),
            preserved_messages: preserved_slice.len(),
            summary_digest,
            summary: summary.clone(),
            strategy: strategy.as_str().to_string(),
            throttle_applied,
            failed_attempts,
        };

        let mut compacted_messages = Vec::new();
        compacted_messages.push(ChatMessage {
            role: "system".into(),
            content: format!(
                "[CompactionBoundary:{}|{}]\n{}\n[Continuation] Resume from preserved recent messages.",
                boundary.boundary_id,
                strategy.as_str(),
                boundary.summary
            ),
        });
        compacted_messages.extend_from_slice(preserved_slice);

        let new_tokens = estimate_message_tokens(&compacted_messages);
        CompactionResult {
            messages: compacted_messages,
            boundary: Some(boundary),
            estimated_tokens: new_tokens,
            strategy: Some(strategy),
            throttle_applied,
            failed_attempts,
        }
    }

    fn apply_failure_throttle(
        &self,
        mut result: CompactionResult,
        budget: &TokenBudgetFrame,
    ) -> CompactionResult {
        let mut failed_attempts = result.failed_attempts.max(2);

        while estimate_message_tokens(&result.messages) > budget.max_input_tokens && result.messages.len() > 2 {
            result.messages.remove(1);
            failed_attempts = failed_attempts.saturating_add(1);
        }

        if estimate_message_tokens(&result.messages) > budget.max_input_tokens {
            if let Some(system) = result.messages.first_mut() {
                let max_chars = (budget.max_input_tokens as usize)
                    .saturating_mul(4)
                    .saturating_sub(16)
                    .max(64);
                let clipped = system.content.chars().take(max_chars).collect::<String>();
                system.content = format!(
                    "{}\n[Throttle] full compaction fallback applied due to repeated budget overflow.",
                    clipped
                );
                failed_attempts = failed_attempts.saturating_add(1);
            }
        }

        result.estimated_tokens = estimate_message_tokens(&result.messages);
        result.throttle_applied = true;
        result.failed_attempts = failed_attempts;
        result.strategy = Some(CompactionStrategy::Full);

        if let Some(boundary) = result.boundary.as_mut() {
            boundary.strategy = CompactionStrategy::Full.as_str().to_string();
            boundary.throttle_applied = true;
            boundary.failed_attempts = failed_attempts;
            boundary.summary = format!(
                "{}\n[Throttle] full compaction fallback applied after repeated budget overflow.",
                boundary.summary
            );
            boundary.summary_digest = digest_text(&boundary.summary);
        }

        result
    }
}

pub fn estimate_message_tokens(messages: &[ChatMessage]) -> u32 {
    messages
        .iter()
        .map(|msg| estimate_text_tokens(&msg.content).saturating_add(4))
        .sum()
}

pub fn estimate_text_tokens(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    (chars / 4).saturating_add(1)
}

fn summarize_messages(messages: &[ChatMessage], max_lines: usize, max_terms: usize) -> String {
    messages
        .iter()
        .take(max_lines)
        .map(|message| {
            let snippet = message
                .content
                .split_whitespace()
                .take(max_terms)
                .collect::<Vec<_>>()
                .join(" ");
            format!("- {}: {}", message.role, snippet)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn digest_text(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(count: usize, words: usize) -> Vec<ChatMessage> {
        (0..count)
            .map(|idx| ChatMessage {
                role: if idx % 2 == 0 { "user".into() } else { "assistant".into() },
                content: (0..words)
                    .map(|w| format!("m{idx}_w{w}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            })
            .collect()
    }

    #[test]
    fn micro_compact_selected_when_it_fits_budget() {
        let compactor = ContextCompactor::new(2);
        let budget = TokenBudgetFrame {
            max_input_tokens: 140,
            max_output_tokens: 64,
            reserve_tokens: 16,
            compaction_trigger_tokens: 20,
        };
        let result = compactor.compact(&messages(6, 8), &budget);
        assert!(result.boundary.is_some());
        assert_eq!(result.strategy, Some(CompactionStrategy::Micro));
        assert!(!result.throttle_applied);
    }

    #[test]
    fn full_compact_selected_when_micro_still_too_large() {
        let compactor = ContextCompactor::new(1);
        let budget = TokenBudgetFrame {
            max_input_tokens: 110,
            max_output_tokens: 64,
            reserve_tokens: 16,
            compaction_trigger_tokens: 60,
        };
        let result = compactor.compact(&messages(12, 20), &budget);
        assert!(result.boundary.is_some());
        assert_eq!(result.strategy, Some(CompactionStrategy::Full));
    }

    #[test]
    fn throttle_applies_when_full_compact_keeps_overflowing() {
        let compactor = ContextCompactor::new(2);
        let budget = TokenBudgetFrame {
            max_input_tokens: 40,
            max_output_tokens: 16,
            reserve_tokens: 8,
            compaction_trigger_tokens: 20,
        };
        let result = compactor.compact(&messages(24, 30), &budget);
        assert!(result.boundary.is_some());
        assert!(result.throttle_applied);
        assert!(result.failed_attempts >= 2);
        assert_eq!(result.strategy, Some(CompactionStrategy::Full));
    }
}
