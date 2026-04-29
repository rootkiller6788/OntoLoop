use std::collections::BTreeMap;

use crate::contracts::transport::{SessionEventType, SessionEventV2};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontendOutputFormat {
    Pretty,
    Json,
}

impl FrontendOutputFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pretty" => Some(Self::Pretty),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FrontendStatusView {
    pub session_id: String,
    pub transport_event_count: usize,
    pub latest_event_type: Option<String>,
    pub latest_emitted_at_ms: Option<u64>,
    pub event_type_counts: BTreeMap<String, usize>,
    pub bridge_status: serde_json::Value,
    pub observable_snapshot_keys: Vec<String>,
}

pub fn summarize_event_type_counts(events: &[SessionEventV2]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for event in events {
        let key = event_type_name(event.event_type);
        let entry = counts.entry(key.to_string()).or_insert(0);
        *entry += 1;
    }
    counts
}

pub fn render_session_event_pretty(event: &SessionEventV2) -> String {
    match event.event_type {
        SessionEventType::Ready => {
            format!(
                "[ready] seq={} ts={} state={}",
                event.sequence,
                event.emitted_at_ms,
                event
                    .payload
                    .get("state")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
            )
        }
        SessionEventType::StateSnapshot => {
            format!(
                "[state_snapshot] seq={} ts={} state={}",
                event.sequence,
                event.emitted_at_ms,
                event
                    .payload
                    .get("state")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
            )
        }
        SessionEventType::AssistantDelta => {
            let turn_id = event
                .payload
                .get("turn_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("turn:unknown");
            let delta = event
                .payload
                .get("delta")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            format!(
                "[assistant_delta] seq={} ts={} turn={} delta={}",
                event.sequence, event.emitted_at_ms, turn_id, delta
            )
        }
        SessionEventType::ToolStarted => {
            let tool_name = event
                .payload
                .get("tool_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool:unknown");
            let call_id = event
                .payload
                .get("call_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("call:unknown");
            format!(
                "[tool_started] seq={} ts={} tool={} call_id={} input={}",
                event.sequence,
                event.emitted_at_ms,
                tool_name,
                call_id,
                event
                    .payload
                    .get("input")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
            )
        }
        SessionEventType::ToolCompleted => {
            let tool_name = event
                .payload
                .get("tool_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool:unknown");
            let call_id = event
                .payload
                .get("call_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("call:unknown");
            let is_error = event
                .payload
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            format!(
                "[tool_completed] seq={} ts={} tool={} call_id={} is_error={} output={}",
                event.sequence,
                event.emitted_at_ms,
                tool_name,
                call_id,
                is_error,
                event
                    .payload
                    .get("output")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}))
            )
        }
    }
}

fn event_type_name(event_type: SessionEventType) -> &'static str {
    match event_type {
        SessionEventType::Ready => "ready",
        SessionEventType::StateSnapshot => "state_snapshot",
        SessionEventType::AssistantDelta => "assistant_delta",
        SessionEventType::ToolStarted => "tool_started",
        SessionEventType::ToolCompleted => "tool_completed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::transport::SessionEventV2;

    fn sample_event(event_type: SessionEventType) -> SessionEventV2 {
        match event_type {
            SessionEventType::Ready => SessionEventV2::ready(
                "s1",
                "t1",
                "cli",
                1,
                1000,
                serde_json::json!({"mode":"shadow"}),
            ),
            SessionEventType::StateSnapshot => SessionEventV2::state_snapshot(
                "s1",
                "t1",
                "cli",
                2,
                1001,
                serde_json::json!({"phase":"loop"}),
            ),
            SessionEventType::AssistantDelta => {
                SessionEventV2::assistant_delta("s1", "t1", "cli", 3, 1002, "turn:1", "hello")
            }
            SessionEventType::ToolStarted => SessionEventV2::tool_started(
                "s1",
                "t1",
                "cli",
                4,
                1003,
                "read",
                "call:1",
                serde_json::json!({"path":"README.md"}),
            ),
            SessionEventType::ToolCompleted => SessionEventV2::tool_completed(
                "s1",
                "t1",
                "cli",
                5,
                1004,
                "read",
                "call:1",
                serde_json::json!({"bytes": 42}),
                false,
            ),
        }
    }

    #[test]
    fn format_parser_accepts_json_and_pretty() {
        assert_eq!(FrontendOutputFormat::parse("json"), Some(FrontendOutputFormat::Json));
        assert_eq!(
            FrontendOutputFormat::parse("pretty"),
            Some(FrontendOutputFormat::Pretty)
        );
        assert_eq!(FrontendOutputFormat::parse("yaml"), None);
    }

    #[test]
    fn summary_counts_all_event_types() {
        let events = vec![
            sample_event(SessionEventType::Ready),
            sample_event(SessionEventType::AssistantDelta),
            sample_event(SessionEventType::AssistantDelta),
        ];
        let counts = summarize_event_type_counts(&events);
        assert_eq!(counts.get("ready"), Some(&1));
        assert_eq!(counts.get("assistant_delta"), Some(&2));
    }

    #[test]
    fn pretty_renderer_contains_type_marker() {
        let rendered = render_session_event_pretty(&sample_event(SessionEventType::ToolCompleted));
        assert!(rendered.contains("[tool_completed]"));
        assert!(rendered.contains("call:1"));
    }
}
