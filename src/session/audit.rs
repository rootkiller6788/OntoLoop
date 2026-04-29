use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

use super::{signal::WorkflowSignal, state::WorkflowState};
use crate::observability::event_stream::append_event;
use autoloop_state_adapter::{LearningEventKind, StateStore, WitnessLogRecord};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransitionRecord {
    pub session_id: String,
    pub from: WorkflowState,
    pub signal: WorkflowSignal,
    pub to: WorkflowState,
    pub timestamp_ms: u64,
    pub reason: Option<String>,
}

#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn record_transition(&self, record: TransitionRecord) -> anyhow::Result<()>;
}

#[derive(Default)]
pub struct NoopAuditSink;

#[async_trait]
impl AuditSink for NoopAuditSink {
    async fn record_transition(&self, _record: TransitionRecord) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub struct InMemoryAuditSink {
    records: RwLock<Vec<TransitionRecord>>,
}

impl InMemoryAuditSink {
    pub async fn records(&self) -> Vec<TransitionRecord> {
        self.records.read().await.clone()
    }
}

#[async_trait]
impl AuditSink for InMemoryAuditSink {
    async fn record_transition(&self, record: TransitionRecord) -> anyhow::Result<()> {
        self.records.write().await.push(record);
        Ok(())
    }
}

#[derive(Clone)]
pub struct StateAuditSink {
    db: StateStore,
    source: String,
}

impl StateAuditSink {
    pub fn new(db: StateStore) -> Self {
        Self {
            db,
            source: "workflow-machine".into(),
        }
    }

    pub fn with_source(db: StateStore, source: impl Into<String>) -> Self {
        Self {
            db,
            source: source.into(),
        }
    }
}

#[async_trait]
impl AuditSink for StateAuditSink {
    async fn record_transition(&self, record: TransitionRecord) -> anyhow::Result<()> {
        static AUDIT_SEQ: AtomicU64 = AtomicU64::new(1);
        let seq = AUDIT_SEQ.fetch_add(1, Ordering::Relaxed);
        let event = WitnessLogRecord {
            id: format!(
                "transition:{}:{}:{:?}:{:?}:{}",
                record.session_id, record.timestamp_ms, record.from, record.to, seq
            ),
            session_id: record.session_id.clone(),
            event_type: LearningEventKind::Audit,
            source: self.source.clone(),
            detail: format!(
                "workflow transition {:?} --({:?})-> {:?}",
                record.from, record.signal, record.to
            ),
            score: 1.0,
            created_at_ms: record.timestamp_ms,
            metadata_json: serde_json::to_string(&record)?,
        };
        self.db.append_witness_log_record(event).await?;
        let _ = append_event(
            &self.db,
            "state_transitions",
            format!("trace:{}", record.timestamp_ms),
            record.session_id.clone(),
            None,
            None,
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::to_value(&record)?,
        )
        .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn state_module_audit_sink_persists_transition_records() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let sink = StateAuditSink::new(db.clone());
        let record = TransitionRecord {
            session_id: "session-audit-db".into(),
            from: WorkflowState::Intake,
            signal: WorkflowSignal::IntentReceived,
            to: WorkflowState::PolicyReview,
            timestamp_ms: 1_700_000_000_000,
            reason: Some("ingested".into()),
        };
        sink.record_transition(record).await.expect("persist");

        let records = db
            .list_witness_log_records("session-audit-db")
            .await
            .expect("list records");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event_type, LearningEventKind::Audit);
        assert!(records[0].detail.contains("workflow transition"));
    }
}

