use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::contracts::ids::{SessionId, TraceId};
use crate::providers::ChatMessage;

use super::compactor::CompactionBoundary;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ContinuationCheckpoint {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub turn_id: String,
    pub compacted_summary: Option<String>,
    pub message_count: usize,
    pub checkpoint_token: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ContinuationRequest {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub previous_turn_id: String,
    pub resume_token: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ContinuationProtocol {
    pub checkpoint: ContinuationCheckpoint,
    pub boundary_id: Option<String>,
    pub replay_fingerprint: String,
}

impl ContinuationCheckpoint {
    pub fn from_turn(
        session_id: SessionId,
        trace_id: TraceId,
        turn_id: impl Into<String>,
        message_count: usize,
        compacted_summary: Option<String>,
    ) -> Self {
        let turn_id = turn_id.into();
        Self {
            session_id,
            trace_id,
            checkpoint_token: build_checkpoint_token(&turn_id, message_count),
            turn_id,
            compacted_summary,
            message_count,
            created_at_ms: current_time_ms(),
        }
    }
}

pub fn build_continuation_protocol(
    checkpoint: ContinuationCheckpoint,
    boundary: Option<&CompactionBoundary>,
    messages: &[ChatMessage],
    final_text: &str,
) -> ContinuationProtocol {
    ContinuationProtocol {
        boundary_id: boundary.map(|item| item.boundary_id.clone()),
        replay_fingerprint: build_replay_fingerprint(messages, final_text),
        checkpoint,
    }
}

pub fn build_checkpoint_token(turn_id: &str, message_count: usize) -> String {
    format!("resume:{}:{}:{}", turn_id, message_count, current_time_ms())
}

pub fn build_replay_fingerprint(messages: &[ChatMessage], final_text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    for message in messages {
        message.role.hash(&mut hasher);
        message.content.hash(&mut hasher);
    }
    final_text.hash(&mut hasher);
    format!("replay:{:016x}", hasher.finish())
}

impl ContinuationProtocol {
    pub fn is_replay_consistent(&self, messages: &[ChatMessage], final_text: &str) -> bool {
        self.replay_fingerprint == build_replay_fingerprint(messages, final_text)
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
