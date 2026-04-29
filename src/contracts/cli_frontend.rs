use super::ids::{SessionId, TraceId};
use super::transport::{SessionEventType, SessionEventV2};
use crate::contracts::errors::{ContractError, RuntimeError};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliFrontendEventType {
    Ready,
    StateSnapshot,
    AssistantDelta,
    ToolStarted,
    ToolCompleted,
    PermissionAsked,
    SessionIdle,
    Error,
}

impl CliFrontendEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::StateSnapshot => "state_snapshot",
            Self::AssistantDelta => "assistant_delta",
            Self::ToolStarted => "tool_started",
            Self::ToolCompleted => "tool_completed",
            Self::PermissionAsked => "permission_asked",
            Self::SessionIdle => "session_idle",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CliFrontendEventV1 {
    pub schema_version: String,
    pub event_type: CliFrontendEventType,
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub sequence: u64,
    pub emitted_at_ms: u64,
    pub payload: serde_json::Value,
}

impl CliFrontendEventV1 {
    pub const SCHEMA_VERSION: &'static str = "cli-frontend-event/v1";

    pub fn from_session_event(event: &SessionEventV2) -> Result<Self, ContractError> {
        event.validate()?;
        let event_type = match event.event_type {
            SessionEventType::Ready => CliFrontendEventType::Ready,
            SessionEventType::StateSnapshot => CliFrontendEventType::StateSnapshot,
            SessionEventType::AssistantDelta => CliFrontendEventType::AssistantDelta,
            SessionEventType::ToolStarted => CliFrontendEventType::ToolStarted,
            SessionEventType::ToolCompleted => CliFrontendEventType::ToolCompleted,
        };
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            event_type,
            session_id: event.session_id.clone(),
            trace_id: event.trace_id.clone(),
            sequence: event.sequence,
            emitted_at_ms: event.emitted_at_ms,
            payload: event.payload.clone(),
        })
    }

    pub fn permission_asked(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        request_id: impl Into<String>,
        permission: impl Into<String>,
        patterns: Vec<String>,
    ) -> Self {
        Self::build(
            CliFrontendEventType::PermissionAsked,
            session_id,
            trace_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({
                "request_id": request_id.into(),
                "permission": permission.into(),
                "patterns": patterns,
            }),
        )
    }

    pub fn session_idle(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
    ) -> Self {
        Self::build(
            CliFrontendEventType::SessionIdle,
            session_id,
            trace_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({}),
        )
    }

    pub fn error(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::build(
            CliFrontendEventType::Error,
            session_id,
            trace_id,
            sequence,
            emitted_at_ms,
            serde_json::json!({
                "code": code.into(),
                "message": message.into(),
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
            CliFrontendEventType::Ready | CliFrontendEventType::StateSnapshot => {
                if self.payload.get("state").is_none() {
                    return Err(schema_error("state payload is required"));
                }
            }
            CliFrontendEventType::AssistantDelta => {
                required_non_empty_str(&self.payload, "turn_id")?;
                required_non_empty_str(&self.payload, "delta")?;
            }
            CliFrontendEventType::ToolStarted => {
                required_non_empty_str(&self.payload, "tool_name")?;
                required_non_empty_str(&self.payload, "call_id")?;
                if self.payload.get("input").is_none() {
                    return Err(schema_error("input payload is required"));
                }
            }
            CliFrontendEventType::ToolCompleted => {
                required_non_empty_str(&self.payload, "tool_name")?;
                required_non_empty_str(&self.payload, "call_id")?;
                if self.payload.get("output").is_none() {
                    return Err(schema_error("output payload is required"));
                }
                if self.payload.get("is_error").and_then(|v| v.as_bool()).is_none() {
                    return Err(schema_error("is_error bool field is required"));
                }
            }
            CliFrontendEventType::PermissionAsked => {
                required_non_empty_str(&self.payload, "request_id")?;
                required_non_empty_str(&self.payload, "permission")?;
                if !self
                    .payload
                    .get("patterns")
                    .and_then(serde_json::Value::as_array)
                    .is_some()
                {
                    return Err(schema_error("patterns array is required"));
                }
            }
            CliFrontendEventType::SessionIdle => {}
            CliFrontendEventType::Error => {
                required_non_empty_str(&self.payload, "code")?;
                required_non_empty_str(&self.payload, "message")?;
            }
        }
        Ok(())
    }

    fn build(
        event_type: CliFrontendEventType,
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        sequence: u64,
        emitted_at_ms: u64,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION.to_string(),
            event_type,
            session_id: SessionId::from(session_id.into()),
            trace_id: TraceId::from(trace_id.into()),
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
        code: "cli_frontend_event_schema_invalid".into(),
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_permission_asked_shape() {
        let event = CliFrontendEventV1::permission_asked(
            "session:test",
            "trace:test",
            1,
            1000,
            "req:1",
            "tool.execute",
            vec!["tool:*".to_string()],
        );
        assert!(event.validate().is_ok());
    }

    #[test]
    fn validates_error_shape() {
        let event = CliFrontendEventV1::error(
            "session:test",
            "trace:test",
            2,
            1001,
            "runtime_failed",
            "runtime failed",
        );
        assert!(event.validate().is_ok());
    }

    #[test]
    fn from_session_event_maps_tool_completed() {
        let source = SessionEventV2::tool_completed(
            "session:test",
            "trace:test",
            "cli",
            3,
            1002,
            "bash",
            "call:1",
            serde_json::json!({"ok": true}),
            false,
        );
        let mapped = CliFrontendEventV1::from_session_event(&source).expect("map");
        assert_eq!(mapped.event_type, CliFrontendEventType::ToolCompleted);
        assert!(mapped.validate().is_ok());
    }
}
