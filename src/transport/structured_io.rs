use crate::contracts::{
    ids::{SessionId, TraceId},
    transport::{TransportEnvelope, TransportKind, TransportMessageKind},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportIngressSource {
    Cli,
    Sse,
    WebSocket,
    Webhook,
}

impl TransportIngressSource {
    pub fn as_transport_kind(self) -> TransportKind {
        match self {
            Self::Cli => TransportKind::Cli,
            Self::Sse => TransportKind::Sse,
            Self::WebSocket => TransportKind::WebSocket,
            Self::Webhook => TransportKind::Webhook,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StructuredIoIngress;

impl StructuredIoIngress {
    pub fn cli_user_input(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        content: impl Into<String>,
    ) -> TransportEnvelope {
        Self::build(
            TransportIngressSource::Cli,
            TransportMessageKind::UserInput,
            session_id,
            trace_id,
            serde_json::json!({ "content": content.into() }),
        )
    }

    pub fn webhook_event(
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        topic: impl Into<String>,
        payload: serde_json::Value,
    ) -> TransportEnvelope {
        Self::build(
            TransportIngressSource::Webhook,
            TransportMessageKind::EventStream,
            session_id,
            trace_id,
            serde_json::json!({
                "topic": topic.into(),
                "payload": payload,
            }),
        )
    }

    pub fn build(
        source: TransportIngressSource,
        kind: TransportMessageKind,
        session_id: impl Into<String>,
        trace_id: impl Into<String>,
        payload: serde_json::Value,
    ) -> TransportEnvelope {
        let now = current_time_ms();
        let session = session_id.into();
        let trace = trace_id.into();
        let transport_id = format!("{}:{}", source_name(source), session.replace(':', "-"));
        let message_id = format!("msg:{}:{}", source_name(source), now);

        TransportEnvelope {
            session_id: SessionId::from(session),
            trace_id: TraceId::from(trace),
            transport_id,
            transport_kind: source.as_transport_kind(),
            message_id,
            kind,
            payload,
            sent_at_ms: now,
            received_at_ms: Some(now),
        }
    }
}

fn source_name(source: TransportIngressSource) -> &'static str {
    match source {
        TransportIngressSource::Cli => "cli",
        TransportIngressSource::Sse => "sse",
        TransportIngressSource::WebSocket => "ws",
        TransportIngressSource::Webhook => "webhook",
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
