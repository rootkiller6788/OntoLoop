use super::ids::{SessionId, TraceId};
use crate::contracts::errors::{ContractError, RuntimeError};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Cli,
    WebSocket,
    Sse,
    Webhook,
    Sdk,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransportMessageKind {
    UserInput,
    ControlRequest,
    ControlResponse,
    ToolPermissionPrompt,
    ToolPermissionDecision,
    EventStream,
    Heartbeat,
    SessionState,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventType {
    Ready,
    StateSnapshot,
    AssistantDelta,
    ToolStarted,
    ToolCompleted,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransportEnvelope {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub transport_id: String,
    pub transport_kind: TransportKind,
    pub message_id: String,
    pub kind: TransportMessageKind,
    pub payload: serde_json::Value,
    pub sent_at_ms: u64,
    pub received_at_ms: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BridgeSessionDescriptor {
    pub session_id: SessionId,
    pub bridge_id: String,
    pub transport_kind: TransportKind,
    pub auth_subject: String,
    pub tenant_id: String,
    pub connected_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BridgeControlDecision {
    pub session_id: SessionId,
    pub request_id: String,
    pub approved: bool,
    pub reason: String,
    pub mode_hint: Option<String>,
    pub decided_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransportReplayPointer {
    pub session_id: SessionId,
    pub transport_id: String,
    pub last_message_id: String,
    pub offset: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionEventV2 {
    pub schema_version: String,
    pub event_type: SessionEventType,
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub transport_id: String,
    pub sequence: u64,
    pub emitted_at_ms: u64,
    pub payload: serde_json::Value,
}

impl SessionEventV2 {
    pub const SCHEMA_VERSION: &'static str = "transport-session-event/v2";

    pub fn ready(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        transport_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        state: serde_json::Value,
    ) -> Self {
        Self::build(
            SessionEventType::Ready,
            session_id,
            trace_id,
            transport_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({ "state": state }),
        )
    }

    pub fn state_snapshot(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        transport_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        state: serde_json::Value,
    ) -> Self {
        Self::build(
            SessionEventType::StateSnapshot,
            session_id,
            trace_id,
            transport_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({ "state": state }),
        )
    }

    pub fn assistant_delta(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        transport_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        turn_id: impl Into<String>,
        delta: impl Into<String>,
    ) -> Self {
        Self::build(
            SessionEventType::AssistantDelta,
            session_id,
            trace_id,
            transport_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({
                "turn_id": turn_id.into(),
                "delta": delta.into(),
            }),
        )
    }

    pub fn tool_started(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        transport_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        tool_name: impl Into<String>,
        call_id: impl Into<String>,
        input: serde_json::Value,
    ) -> Self {
        Self::build(
            SessionEventType::ToolStarted,
            session_id,
            trace_id,
            transport_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({
                "tool_name": tool_name.into(),
                "call_id": call_id.into(),
                "input": input,
            }),
        )
    }

    pub fn tool_completed(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        transport_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        tool_name: impl Into<String>,
        call_id: impl Into<String>,
        output: serde_json::Value,
        is_error: bool,
    ) -> Self {
        Self::build(
            SessionEventType::ToolCompleted,
            session_id,
            trace_id,
            transport_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({
                "tool_name": tool_name.into(),
                "call_id": call_id.into(),
                "output": output,
                "is_error": is_error,
            }),
        )
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(schema_error("schema_version mismatch"));
        }
        if self.session_id.as_ref().trim().is_empty() {
            return Err(schema_error("session_id must be non-empty"));
        }
        if self.trace_id.as_ref().trim().is_empty() {
            return Err(schema_error("trace_id must be non-empty"));
        }
        if self.transport_id.trim().is_empty() {
            return Err(schema_error("transport_id must be non-empty"));
        }
        if self.sequence == 0 {
            return Err(schema_error("sequence must be > 0"));
        }
        if self.emitted_at_ms == 0 {
            return Err(schema_error("emitted_at_ms must be > 0"));
        }
        if !self.payload.is_object() {
            return Err(schema_error("payload must be an object"));
        }
        match self.event_type {
            SessionEventType::Ready | SessionEventType::StateSnapshot => {
                if self.payload.get("state").is_none() {
                    return Err(schema_error("state payload is required"));
                }
            }
            SessionEventType::AssistantDelta => {
                required_non_empty_str(&self.payload, "turn_id")?;
                required_non_empty_str(&self.payload, "delta")?;
            }
            SessionEventType::ToolStarted => {
                required_non_empty_str(&self.payload, "tool_name")?;
                required_non_empty_str(&self.payload, "call_id")?;
                if self.payload.get("input").is_none() {
                    return Err(schema_error("input payload is required"));
                }
            }
            SessionEventType::ToolCompleted => {
                required_non_empty_str(&self.payload, "tool_name")?;
                required_non_empty_str(&self.payload, "call_id")?;
                if self.payload.get("output").is_none() {
                    return Err(schema_error("output payload is required"));
                }
                if self.payload.get("is_error").and_then(|v| v.as_bool()).is_none() {
                    return Err(schema_error("is_error bool field is required"));
                }
            }
        }
        Ok(())
    }

    fn build(
        event_type: SessionEventType,
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        transport_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            event_type,
            session_id: SessionId::from(session_id.into()),
            trace_id: TraceId::from(trace_id.into()),
            transport_id: transport_id.into(),
            sequence,
            emitted_at_ms,
            payload,
        }
    }
}

fn required_non_empty_str(payload: &serde_json::Value, key: &str) -> Result<(), ContractError> {
    if payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(());
    }
    Err(schema_error(format!("{key} must be a non-empty string")))
}

fn schema_error(message: impl Into<String>) -> ContractError {
    ContractError::Runtime(RuntimeError {
        code: "transport_event_schema_invalid".into(),
        message: message.into(),
    })
}
