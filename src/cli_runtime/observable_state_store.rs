use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{RwLock, broadcast};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservableStateTopic {
    CommandReceived,
    CommandCompleted,
    CommandFailed,
    SessionSnapshot,
    BridgeSnapshot,
}

impl ObservableStateTopic {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CommandReceived => "command.received",
            Self::CommandCompleted => "command.completed",
            Self::CommandFailed => "command.failed",
            Self::SessionSnapshot => "session.snapshot",
            Self::BridgeSnapshot => "bridge.snapshot",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservableStateEvent {
    pub topic: String,
    pub payload: Value,
    pub timestamp_ms: u64,
}

#[derive(Clone)]
pub struct ObservableStateStore {
    snapshots: Arc<RwLock<BTreeMap<String, Value>>>,
    event_tx: broadcast::Sender<ObservableStateEvent>,
}

impl Default for ObservableStateStore {
    fn default() -> Self {
        Self::new(256)
    }
}

impl ObservableStateStore {
    pub fn new(buffer: usize) -> Self {
        let (event_tx, _) = broadcast::channel(buffer.max(8));
        Self {
            snapshots: Arc::new(RwLock::new(BTreeMap::new())),
            event_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ObservableStateEvent> {
        self.event_tx.subscribe()
    }

    pub async fn publish_snapshot(
        &self,
        topic: ObservableStateTopic,
        key: impl Into<String>,
        payload: Value,
    ) {
        let key = key.into();
        self.snapshots
            .write()
            .await
            .insert(key, payload.clone());
        self.publish_event(topic, payload).await;
    }

    pub async fn publish_event(&self, topic: ObservableStateTopic, payload: Value) {
        let _ = self.event_tx.send(ObservableStateEvent {
            topic: topic.as_str().to_string(),
            payload,
            timestamp_ms: current_time_ms(),
        });
    }

    pub async fn snapshot(&self, key: &str) -> Option<Value> {
        self.snapshots.read().await.get(key).cloned()
    }

    pub async fn snapshot_keys(&self) -> Vec<String> {
        self.snapshots
            .read()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    }
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_snapshot_is_observable() {
        let store = ObservableStateStore::default();
        let mut receiver = store.subscribe();

        store
            .publish_snapshot(
                ObservableStateTopic::CommandReceived,
                "command/system/status",
                serde_json::json!({"command": "system.status"}),
            )
            .await;

        let event = receiver.recv().await.expect("event should be emitted");
        assert_eq!(event.topic, "command.received");
        assert_eq!(
            store.snapshot("command/system/status").await,
            Some(serde_json::json!({"command": "system.status"}))
        );
    }
}
