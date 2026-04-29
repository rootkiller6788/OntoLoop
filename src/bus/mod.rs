use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, Clone)]
pub struct InboundMessage {
    pub session_id: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub session_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageTopic {
    TaskDispatch,
    TaskResult,
    TaskError,
}

impl AgentMessageTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentMessageTopic::TaskDispatch => "task.dispatch",
            AgentMessageTopic::TaskResult => "task.result",
            AgentMessageTopic::TaskError => "task.error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub message_id: String,
    pub topic: AgentMessageTopic,
    pub session_id: String,
    pub content: String,
    pub retry_count: u8,
    pub max_retries: u8,
}

#[derive(Clone)]
pub struct MessageBus {
    inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Arc<Mutex<mpsc::Receiver<InboundMessage>>>,
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: Arc<Mutex<mpsc::Receiver<OutboundMessage>>>,
    agent_tx: mpsc::Sender<AgentMessage>,
    agent_rx: Arc<Mutex<mpsc::Receiver<AgentMessage>>>,
    in_flight_agent: Arc<Mutex<HashMap<String, AgentMessage>>>,
    next_message_seq: Arc<AtomicU64>,
}

impl Default for MessageBus {
    fn default() -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(256);
        let (outbound_tx, outbound_rx) = mpsc::channel(256);
        let (agent_tx, agent_rx) = mpsc::channel(256);

        Self {
            inbound_tx,
            inbound_rx: Arc::new(Mutex::new(inbound_rx)),
            outbound_tx,
            outbound_rx: Arc::new(Mutex::new(outbound_rx)),
            agent_tx,
            agent_rx: Arc::new(Mutex::new(agent_rx)),
            in_flight_agent: Arc::new(Mutex::new(HashMap::new())),
            next_message_seq: Arc::new(AtomicU64::new(1)),
        }
    }
}

impl MessageBus {
    pub async fn publish_inbound(
        &self,
        message: InboundMessage,
    ) -> Result<(), mpsc::error::SendError<InboundMessage>> {
        self.inbound_tx.send(message).await
    }

    pub async fn consume_inbound(&self) -> Option<InboundMessage> {
        self.inbound_rx.lock().await.recv().await
    }

    pub async fn publish_outbound(
        &self,
        message: OutboundMessage,
    ) -> Result<(), mpsc::error::SendError<OutboundMessage>> {
        self.outbound_tx.send(message).await
    }

    pub async fn consume_outbound(&self) -> Option<OutboundMessage> {
        self.outbound_rx.lock().await.recv().await
    }

    pub async fn publish_task_dispatch(
        &self,
        session_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<String, mpsc::error::SendError<AgentMessage>> {
        self.publish_agent_message(AgentMessageTopic::TaskDispatch, session_id, content, 3)
            .await
    }

    pub async fn publish_task_result(
        &self,
        session_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<String, mpsc::error::SendError<AgentMessage>> {
        self.publish_agent_message(AgentMessageTopic::TaskResult, session_id, content, 3)
            .await
    }

    pub async fn publish_task_error(
        &self,
        session_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<String, mpsc::error::SendError<AgentMessage>> {
        self.publish_agent_message(AgentMessageTopic::TaskError, session_id, content, 3)
            .await
    }

    pub async fn publish_agent_message(
        &self,
        topic: AgentMessageTopic,
        session_id: impl Into<String>,
        content: impl Into<String>,
        max_retries: u8,
    ) -> Result<String, mpsc::error::SendError<AgentMessage>> {
        let message_id = self.next_message_id();
        let message = AgentMessage {
            message_id: message_id.clone(),
            topic,
            session_id: session_id.into(),
            content: content.into(),
            retry_count: 0,
            max_retries,
        };
        self.agent_tx.send(message).await?;
        Ok(message_id)
    }

    pub async fn consume_agent_message(&self) -> Option<AgentMessage> {
        let message = self.agent_rx.lock().await.recv().await?;
        self.in_flight_agent
            .lock()
            .await
            .insert(message.message_id.clone(), message.clone());
        Some(message)
    }

    pub async fn ack_agent_message(&self, message_id: &str) -> bool {
        self.in_flight_agent
            .lock()
            .await
            .remove(message_id)
            .is_some()
    }

    pub async fn retry_agent_message(
        &self,
        message_id: &str,
    ) -> Result<bool, mpsc::error::SendError<AgentMessage>> {
        let mut in_flight = self.in_flight_agent.lock().await;
        let Some(message) = in_flight.get_mut(message_id) else {
            return Ok(false);
        };

        if message.retry_count >= message.max_retries {
            in_flight.remove(message_id);
            return Ok(false);
        }

        message.retry_count = message.retry_count.saturating_add(1);
        let retry_payload = message.clone();
        drop(in_flight);
        self.agent_tx.send(retry_payload).await?;
        Ok(true)
    }

    fn next_message_id(&self) -> String {
        let sequence = self.next_message_seq.fetch_add(1, Ordering::Relaxed);
        format!("msg-{sequence:016x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_topics_roundtrip_and_ack() {
        let bus = MessageBus::default();
        let message_id = bus
            .publish_task_dispatch("session-a", "dispatch")
            .await
            .expect("publish");

        let msg = bus.consume_agent_message().await.expect("consume");
        assert_eq!(msg.message_id, message_id);
        assert_eq!(msg.topic, AgentMessageTopic::TaskDispatch);
        assert!(bus.ack_agent_message(&msg.message_id).await);
        assert!(!bus.ack_agent_message(&msg.message_id).await);
    }

    #[tokio::test]
    async fn retry_increments_count_and_stops_at_limit() {
        let bus = MessageBus::default();
        let message_id = bus
            .publish_agent_message(
                AgentMessageTopic::TaskError,
                "session-b",
                "boom",
                1,
            )
            .await
            .expect("publish");

        let first = bus.consume_agent_message().await.expect("consume-first");
        assert_eq!(first.retry_count, 0);

        let retried = bus
            .retry_agent_message(&message_id)
            .await
            .expect("retry-call");
        assert!(retried);

        let second = bus.consume_agent_message().await.expect("consume-second");
        assert_eq!(second.message_id, message_id);
        assert_eq!(second.retry_count, 1);

        let no_more = bus
            .retry_agent_message(&message_id)
            .await
            .expect("retry-limit-call");
        assert!(!no_more);
    }
}
