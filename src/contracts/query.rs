use std::collections::BTreeMap;

use super::ids::{SessionId, TaskId, TraceId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryTurnPhase {
    Received,
    ContextCompiled,
    Streaming,
    ToolLoop,
    Completed,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationReason {
    ContextLimit,
    UserRequested,
    PolicyBoundary,
    TransportReconnect,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryFrame {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub turn_id: String,
    pub system_prompt: String,
    pub runtime_context: serde_json::Value,
    pub policy_scope: serde_json::Value,
    pub memory_context: serde_json::Value,
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
    pub compression_profile: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryTurn {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub turn_id: String,
    pub phase: QueryTurnPhase,
    pub user_content: serde_json::Value,
    pub retry_count: u8,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryToolIntent {
    pub task_id: TaskId,
    pub capability_id: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryTurnOutcome {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub turn_id: String,
    pub assistant_content: Option<String>,
    pub tool_intents: Vec<QueryToolIntent>,
    pub completion_reason: String,
    pub usage: serde_json::Value,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryContinuation {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub previous_turn_id: String,
    pub next_turn_id: String,
    pub reason: ContinuationReason,
    pub checkpoint_ref: String,
    pub resume_token: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionCheckpoint {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub checkpoint_id: String,
    pub turn_id: String,
    pub history_window: serde_json::Value,
    pub compacted_summary: Option<String>,
    pub created_at_ms: u64,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionResumeRequest {
    pub session_id: SessionId,
    pub checkpoint_id: String,
    pub requested_by: String,
    pub expected_trace_id: Option<TraceId>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionResumeOutcome {
    pub session_id: SessionId,
    pub restored: bool,
    pub resumed_trace_id: TraceId,
    pub active_turn_id: Option<String>,
    pub note: String,
}
