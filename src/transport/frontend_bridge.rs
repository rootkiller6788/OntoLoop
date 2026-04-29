use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::{
    contracts::transport::{SessionEventV2, TransportKind},
    transport::{StructuredIoIngress, TransportBridgeRuntime, parse_transport_kind},
};

#[derive(Clone)]
pub struct FrontendBridgeRuntime {
    transport: TransportBridgeRuntime,
    state_store: StateStore,
}

impl FrontendBridgeRuntime {
    pub fn new(transport: TransportBridgeRuntime, state_store: StateStore) -> Self {
        Self {
            transport,
            state_store,
        }
    }

    pub async fn ensure_session(
        &self,
        session_id: &str,
        transport_kind: Option<&str>,
        subject: Option<&str>,
        tenant_id: &str,
        ttl_ms: u64,
    ) -> Result<()> {
        let status = self.transport.status(session_id).await?;
        if status.running {
            return Ok(());
        }

        let parsed = transport_kind.map(parse_transport_kind).unwrap_or(TransportKind::Cli);
        let auth_subject = subject.unwrap_or("frontend:operator");
        self.transport
            .start(session_id, parsed, auth_subject, tenant_id, ttl_ms)
            .await?;
        Ok(())
    }

    pub async fn ingest_user_input(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Result<()> {
        let envelope = StructuredIoIngress::cli_user_input(session_id, trace_id, content);
        self.transport.ingest_envelope(&envelope).await?;
        Ok(())
    }

    pub async fn emit_assistant_delta(
        &self,
        session_id: &str,
        trace_id: &str,
        turn_id: &str,
        delta: &str,
    ) -> Result<SessionEventV2> {
        let sequence = self.next_sequence(session_id).await?;
        let event = SessionEventV2::assistant_delta(
            session_id,
            trace_id,
            "frontend:cli",
            sequence,
            current_time_ms(),
            turn_id,
            delta,
        );
        self.transport.emit_session_event_v2(&event).await?;
        Ok(event)
    }

    pub async fn emit_session_idle(&self, session_id: &str, trace_id: &str) -> Result<SessionEventV2> {
        let sequence = self.next_sequence(session_id).await?;
        let event = SessionEventV2::state_snapshot(
            session_id,
            trace_id,
            "frontend:cli",
            sequence,
            current_time_ms(),
            serde_json::json!({
                "status": "idle",
            }),
        );
        self.transport.emit_session_event_v2(&event).await?;
        Ok(event)
    }

    pub async fn emit_error(
        &self,
        session_id: &str,
        trace_id: &str,
        code: &str,
        message: &str,
    ) -> Result<SessionEventV2> {
        let sequence = self.next_sequence(session_id).await?;
        let event = SessionEventV2::tool_completed(
            session_id,
            trace_id,
            "frontend:cli",
            sequence,
            current_time_ms(),
            "frontend.error",
            format!("error:{code}:{sequence}"),
            serde_json::json!({
                "code": code,
                "message": message,
            }),
            true,
        );
        self.transport.emit_session_event_v2(&event).await?;
        Ok(event)
    }

    pub async fn emit_tool_started(
        &self,
        session_id: &str,
        trace_id: &str,
        tool_name: &str,
        call_id: &str,
        input: serde_json::Value,
    ) -> Result<SessionEventV2> {
        let sequence = self.next_sequence(session_id).await?;
        let event = SessionEventV2::tool_started(
            session_id,
            trace_id,
            "frontend:cli",
            sequence,
            current_time_ms(),
            tool_name,
            call_id,
            input,
        );
        self.transport.emit_session_event_v2(&event).await?;
        Ok(event)
    }

    pub async fn emit_tool_completed(
        &self,
        session_id: &str,
        trace_id: &str,
        tool_name: &str,
        call_id: &str,
        output: serde_json::Value,
        is_error: bool,
    ) -> Result<SessionEventV2> {
        let sequence = self.next_sequence(session_id).await?;
        let event = SessionEventV2::tool_completed(
            session_id,
            trace_id,
            "frontend:cli",
            sequence,
            current_time_ms(),
            tool_name,
            call_id,
            output,
            is_error,
        );
        self.transport.emit_session_event_v2(&event).await?;
        Ok(event)
    }

    async fn next_sequence(&self, session_id: &str) -> Result<u64> {
        let key = format!("transport:frontend-seq:{session_id}:latest");
        let next = self
            .state_store
            .get_knowledge(&key)
            .await?
            .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
            .and_then(|value| value.get("sequence").and_then(serde_json::Value::as_u64))
            .unwrap_or(0)
            .saturating_add(1);

        self.state_store
            .upsert_json_knowledge(
                key,
                &serde_json::json!({
                    "session_id": session_id,
                    "sequence": next,
                    "updated_at_ms": current_time_ms(),
                }),
                "frontend-bridge",
            )
            .await?;
        Ok(next)
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
