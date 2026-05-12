use anyhow::Result;

use crate::contracts::context::ContextItem;

use super::checkpoint::{SessionCheckpoint, SessionCheckpointStore};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextSummaryCache {
    pub summary_id: String,
    pub session_id: String,
    pub digest: String,
    pub summary: String,
    pub message_count: usize,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextRetrievalEntry {
    pub item_id: String,
    pub kind: String,
    pub source_ref: String,
    pub permission_scope: String,
    pub priority: f32,
    pub budget_micros: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextRetrievalIndex {
    pub index_id: String,
    pub session_id: String,
    pub entries: Vec<ContextRetrievalEntry>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextStateSnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub message_count: usize,
    pub context_item_count: usize,
    pub continuation_turn_id: Option<String>,
    pub continuation_checkpoint_token: Option<String>,
    pub compaction_digest: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContextCacheBundle {
    pub summary_cache: ContextSummaryCache,
    pub retrieval_index: ContextRetrievalIndex,
    pub state_snapshot: ContextStateSnapshot,
    #[serde(default)]
    pub context_items: Vec<ContextItem>,
}

#[derive(Debug, Clone)]
pub struct ContextCacheOrchestrator {
    checkpoints: SessionCheckpointStore,
}

impl ContextCacheOrchestrator {
    pub fn new(checkpoints: SessionCheckpointStore) -> Self {
        Self { checkpoints }
    }

    pub fn default() -> Self {
        Self::new(SessionCheckpointStore::default())
    }

    pub fn load(&self, session_id: &str) -> Result<Option<ContextCacheBundle>> {
        let Some(checkpoint) = self.checkpoints.load(session_id)? else {
            return Ok(None);
        };
        Ok(Some(Self::from_checkpoint(&checkpoint)))
    }

    pub fn from_checkpoint(checkpoint: &SessionCheckpoint) -> ContextCacheBundle {
        let summary = build_summary(checkpoint);
        let summary_cache = ContextSummaryCache {
            summary_id: format!("context-cache:summary:{}:latest", checkpoint.session_id),
            session_id: checkpoint.session_id.clone(),
            digest: checkpoint.compaction.digest.clone(),
            summary,
            message_count: checkpoint.history.len(),
            updated_at_ms: checkpoint.updated_at_ms,
        };

        let retrieval_index = ContextRetrievalIndex {
            index_id: format!(
                "context-cache:retrieval-index:{}:latest",
                checkpoint.session_id
            ),
            session_id: checkpoint.session_id.clone(),
            entries: checkpoint
                .context_items
                .iter()
                .map(|item| ContextRetrievalEntry {
                    item_id: item.item_id.clone(),
                    kind: item.kind.clone(),
                    source_ref: item.source_ref.clone(),
                    permission_scope: item.permission_scope.clone(),
                    priority: item.priority,
                    budget_micros: item.budget_micros,
                })
                .collect::<Vec<_>>(),
            updated_at_ms: checkpoint.updated_at_ms,
        };

        let state_snapshot = ContextStateSnapshot {
            snapshot_id: format!(
                "context-cache:state-snapshot:{}:latest",
                checkpoint.session_id
            ),
            session_id: checkpoint.session_id.clone(),
            message_count: checkpoint.history.len(),
            context_item_count: checkpoint.context_items.len(),
            continuation_turn_id: checkpoint.continuation_turn_id.clone(),
            continuation_checkpoint_token: checkpoint.continuation_checkpoint_token.clone(),
            compaction_digest: checkpoint.compaction.digest.clone(),
            updated_at_ms: checkpoint.updated_at_ms,
        };

        ContextCacheBundle {
            summary_cache,
            retrieval_index,
            state_snapshot,
            context_items: checkpoint.context_items.clone(),
        }
    }
}

fn build_summary(checkpoint: &SessionCheckpoint) -> String {
    let compacted = &checkpoint.compaction.compacted_history;
    let user_hint = compacted
        .iter()
        .rev()
        .find(|message| message.role.eq_ignore_ascii_case("user"))
        .map(|message| message.content.clone())
        .unwrap_or_else(|| "n/a".to_string());
    let assistant_hint = compacted
        .iter()
        .rev()
        .find(|message| message.role.eq_ignore_ascii_case("assistant"))
        .map(|message| message.content.clone())
        .unwrap_or_else(|| "n/a".to_string());

    format!(
        "window={} digest={} user={} assistant={}",
        checkpoint.compaction.window_size,
        checkpoint.compaction.digest,
        trim_snippet(&user_hint),
        trim_snippet(&assistant_hint)
    )
}

fn trim_snippet(value: &str) -> String {
    let mut snippet = value.trim().replace('\n', " ");
    if snippet.chars().count() > 120 {
        snippet = snippet.chars().take(120).collect::<String>();
        snippet.push_str("...");
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        contracts::context::{ContextItem, ContextItemKind},
        providers::ChatMessage,
        session::checkpoint::{SessionHistoryCompaction, SessionRedactionSummary},
    };
    use std::collections::BTreeMap;

    #[test]
    fn context_cache_bundle_contains_summary_index_and_state_snapshot() {
        let checkpoint = SessionCheckpoint {
            session_id: "cache-test".to_string(),
            history: vec![
                ChatMessage { tool_call_id: None, tool_calls: None,
                    role: "user".to_string(),
                    content: "Need summary".to_string(),
                },
                ChatMessage { tool_call_id: None, tool_calls: None,
                    role: "assistant".to_string(),
                    content: "Here is summary".to_string(),
                },
            ],
            compaction: SessionHistoryCompaction {
                window_size: 2,
                compacted_history: vec![],
                digest: "abc123".to_string(),
            },
            updated_at_ms: 42,
            continuation_turn_id: Some("turn-1".to_string()),
            continuation_checkpoint_token: Some("cp-1".to_string()),
            evidence_ref: Some("session-evidence:cache-test:42:abc123".to_string()),
            redacted_compacted_history: vec![],
            redaction_summary: SessionRedactionSummary::default(),
            context_items: vec![ContextItem::new(
                "cache-test".to_string(),
                "item-1".to_string(),
                ContextItemKind::Session,
                "session:cache-test:1".to_string(),
                "tenant:test".to_string(),
                0.9,
                1000,
                BTreeMap::new(),
            )],
        };

        let bundle = ContextCacheOrchestrator::from_checkpoint(&checkpoint);
        assert_eq!(bundle.retrieval_index.entries.len(), 1);
        assert_eq!(bundle.state_snapshot.context_item_count, 1);
        assert!(bundle.summary_cache.summary.contains("digest=abc123"));
    }
}
