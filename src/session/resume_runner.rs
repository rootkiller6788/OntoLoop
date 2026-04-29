use anyhow::Result;

use crate::providers::ChatMessage;

use super::{checkpoint::SessionCheckpoint, runtime::SessionRuntime};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionResumeSnapshot {
    pub session_id: String,
    pub recovered_messages: usize,
    pub compacted_history: Vec<ChatMessage>,
    pub checkpoint_digest: String,
    pub updated_at_ms: u64,
    pub evidence_ref: Option<String>,
    pub evidence_bound: bool,
    pub redacted_fields: u32,
}

#[derive(Debug, Clone)]
pub struct SessionResumeRunner {
    runtime: SessionRuntime,
}

impl SessionResumeRunner {
    pub fn new(runtime: SessionRuntime) -> Self {
        Self { runtime }
    }

    pub fn resume(&self, session_id: &str) -> Result<Option<SessionResumeSnapshot>> {
        let Some(checkpoint) = self.runtime.checkpoint_store().load(session_id)? else {
            return Ok(None);
        };
        Ok(Some(snapshot_from_checkpoint(checkpoint)))
    }
}

fn snapshot_from_checkpoint(checkpoint: SessionCheckpoint) -> SessionResumeSnapshot {
    SessionResumeSnapshot {
        session_id: checkpoint.session_id,
        recovered_messages: checkpoint.history.len(),
        compacted_history: if checkpoint.redacted_compacted_history.is_empty() {
            checkpoint.compaction.compacted_history
        } else {
            checkpoint.redacted_compacted_history
        },
        checkpoint_digest: checkpoint.compaction.digest,
        updated_at_ms: checkpoint.updated_at_ms,
        evidence_ref: checkpoint.evidence_ref.clone(),
        evidence_bound: checkpoint.evidence_ref.is_some(),
        redacted_fields: checkpoint.redaction_summary.redacted_fields,
    }
}
