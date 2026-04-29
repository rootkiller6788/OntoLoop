use std::{collections::BTreeMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;

use autoloop_state_adapter::StateStore;

use crate::{
    contracts::{
        errors::ContractError,
        ports::TransportBridgePort,
        transport::{
            BridgeControlDecision, BridgeSessionDescriptor, SessionEventV2, TransportEnvelope, TransportKind,
            TransportMessageKind,
        },
    },
    session::SessionStore,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BridgeRuntimeStatus {
    pub session_id: String,
    pub running: bool,
    pub descriptor: Option<BridgeSessionDescriptor>,
    pub buffered_messages: usize,
    pub started_at_ms: Option<u64>,
    pub stopped_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
struct BridgeRuntimeEntry {
    descriptor: BridgeSessionDescriptor,
    running: bool,
    buffered_messages: usize,
    started_at_ms: u64,
    stopped_at_ms: Option<u64>,
}

#[derive(Clone)]
pub struct TransportBridgeRuntime {
    sessions: SessionStore,
    state_store: StateStore,
    entries: Arc<RwLock<BTreeMap<String, BridgeRuntimeEntry>>>,
}

impl TransportBridgeRuntime {
    pub fn new(sessions: SessionStore, state_store: StateStore) -> Self {
        Self {
            sessions,
            state_store,
            entries: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub async fn start(
        &self,
        session_id: &str,
        transport_kind: TransportKind,
        auth_subject: &str,
        tenant_id: &str,
        ttl_ms: u64,
    ) -> Result<BridgeRuntimeStatus> {
        let now = current_time_ms();
        let descriptor = BridgeSessionDescriptor {
            session_id: session_id.into(),
            bridge_id: format!("bridge:{}:{}", sanitize_session_id(session_id), now),
            transport_kind,
            auth_subject: auth_subject.to_string(),
            tenant_id: tenant_id.to_string(),
            connected_at_ms: now,
            expires_at_ms: now.saturating_add(ttl_ms),
        };
        let entry = BridgeRuntimeEntry {
            descriptor: descriptor.clone(),
            running: true,
            buffered_messages: 0,
            started_at_ms: now,
            stopped_at_ms: None,
        };
        self.entries
            .write()
            .await
            .insert(session_id.to_string(), entry);
        self.persist_status(session_id).await?;
        self.status(session_id).await
    }

    pub async fn stop(&self, session_id: &str) -> Result<BridgeRuntimeStatus> {
        let now = current_time_ms();
        if let Some(entry) = self.entries.write().await.get_mut(session_id) {
            entry.running = false;
            entry.stopped_at_ms = Some(now);
        }
        self.persist_status(session_id).await?;
        self.status(session_id).await
    }

    pub async fn status(&self, session_id: &str) -> Result<BridgeRuntimeStatus> {
        let entry = self.entries.read().await.get(session_id).cloned();
        let status = if let Some(entry) = entry {
            BridgeRuntimeStatus {
                session_id: session_id.to_string(),
                running: entry.running,
                descriptor: Some(entry.descriptor),
                buffered_messages: entry.buffered_messages,
                started_at_ms: Some(entry.started_at_ms),
                stopped_at_ms: entry.stopped_at_ms,
            }
        } else {
            BridgeRuntimeStatus {
                session_id: session_id.to_string(),
                running: false,
                descriptor: None,
                buffered_messages: 0,
                started_at_ms: None,
                stopped_at_ms: None,
            }
        };
        Ok(status)
    }

    pub async fn ingest_envelope(&self, envelope: &TransportEnvelope) -> Result<()> {
        let status = self.status(envelope.session_id.as_ref()).await?;
        if !status.running {
            return Err(anyhow::anyhow!(
                "bridge session '{}' is not running",
                envelope.session_id
            ));
        }

        match envelope.kind {
            TransportMessageKind::UserInput => {
                if let Some(content) = envelope
                    .payload
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                {
                    self.sessions
                        .append_user_message(envelope.session_id.as_ref(), content)
                        .await;
                }
            }
            TransportMessageKind::EventStream => {
                self.sessions
                    .append_tool_message(
                        envelope.session_id.as_ref(),
                        "transport:event",
                        &envelope.payload.to_string(),
                    )
                    .await;
            }
            _ => {}
        }

        if let Some(entry) = self
            .entries
            .write()
            .await
            .get_mut(envelope.session_id.as_ref())
        {
            entry.buffered_messages = entry.buffered_messages.saturating_add(1);
        }

        self.state_store
            .upsert_json_knowledge(
                format!(
                    "transport:envelope:{}:{}",
                    envelope.session_id, envelope.message_id
                ),
                envelope,
                "transport-bridge",
            )
            .await?;
        self.persist_status(envelope.session_id.as_ref()).await?;
        Ok(())
    }

    pub async fn dispatch_envelope(&self, envelope: &TransportEnvelope) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!(
                    "transport:dispatch:{}:{}",
                    envelope.session_id, envelope.message_id
                ),
                envelope,
                "transport-bridge",
            )
            .await?;
        Ok(())
    }

    pub async fn decide_control(&self, decision: &BridgeControlDecision) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!(
                    "transport:decision:{}:{}",
                    decision.session_id, decision.request_id
                ),
                decision,
                "transport-bridge",
            )
            .await?;
        Ok(())
    }

    pub async fn emit_session_event_v2(&self, event: &SessionEventV2) -> Result<()> {
        event
            .validate()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;

        self.state_store
            .upsert_json_knowledge(
                format!(
                    "transport:event-v2:{}:{}:{}",
                    event.session_id, event.emitted_at_ms, event.sequence
                ),
                event,
                "transport-bridge",
            )
            .await?;
        Ok(())
    }

    pub async fn replay_session_events_v2(&self, session_id: &str) -> Result<Vec<SessionEventV2>> {
        let mut events = self
            .state_store
            .list_knowledge_by_prefix(&format!("transport:event-v2:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<SessionEventV2>(&record.value).ok())
            .collect::<Vec<_>>();
        events.sort_by_key(|event| (event.emitted_at_ms, event.sequence));
        Ok(events)
    }

    pub async fn record_remote_audit(
        &self,
        session_id: &str,
        action: &str,
        payload: &serde_json::Value,
    ) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!("transport:remote-audit:{session_id}:{}", current_time_ms()),
                &serde_json::json!({
                    "session_id": session_id,
                    "action": action,
                    "payload": payload,
                    "recorded_at_ms": current_time_ms(),
                }),
                "transport-bridge",
            )
            .await?;
        Ok(())
    }

    async fn persist_status(&self, session_id: &str) -> Result<()> {
        let status = self.status(session_id).await?;
        self.state_store
            .upsert_json_knowledge(
                format!("transport:bridge:{session_id}:status"),
                &status,
                "transport-bridge",
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl TransportBridgePort for TransportBridgeRuntime {
    async fn ingest(&self, envelope: &TransportEnvelope) -> Result<(), ContractError> {
        self.ingest_envelope(envelope).await.map_err(|error| {
            ContractError::Runtime(crate::contracts::errors::RuntimeError {
                code: "transport_ingest_failed".into(),
                message: error.to_string(),
            })
        })
    }

    async fn dispatch(&self, envelope: &TransportEnvelope) -> Result<(), ContractError> {
        self.dispatch_envelope(envelope).await.map_err(|error| {
            ContractError::Runtime(crate::contracts::errors::RuntimeError {
                code: "transport_dispatch_failed".into(),
                message: error.to_string(),
            })
        })
    }

    async fn decide(&self, decision: &BridgeControlDecision) -> Result<(), ContractError> {
        self.decide_control(decision).await.map_err(|error| {
            ContractError::Runtime(crate::contracts::errors::RuntimeError {
                code: "transport_decision_failed".into(),
                message: error.to_string(),
            })
        })
    }
}

pub fn parse_transport_kind(value: &str) -> TransportKind {
    match value.to_ascii_lowercase().as_str() {
        "cli" => TransportKind::Cli,
        "sse" => TransportKind::Sse,
        "ws" | "websocket" => TransportKind::WebSocket,
        "webhook" => TransportKind::Webhook,
        "sdk" => TransportKind::Sdk,
        _ => TransportKind::Cli,
    }
}

fn sanitize_session_id(session_id: &str) -> String {
    session_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}




