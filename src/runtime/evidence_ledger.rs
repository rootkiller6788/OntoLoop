use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use autoloop_state_adapter::StateStore;

use crate::contracts::evidence::{
    ApprovalRecord, BudgetLedgerRecord, EvidenceStepRecord, ReplayFingerprint,
};
use crate::contracts::wiki_compat::OpLogEvent;
use crate::orchestration::current_time_ms;
use crate::security::policy_host::sanitize_decision_payload;
use crate::evolution_os::replay;

const EVIDENCE_EVENT_REPLAY_SCHEMA_VERSION: &str = "evidence-event-replay/v1";
const EVIDENCE_EVENT_REPLAY_SEED_VERSION: &str = "evidence-event-seed/v1";
const EVIDENCE_EVENT_REPLAY_VERSION: &str = "evidence-event-replay-contract/v1";

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStage {
    Admission,
    Execution,
    Budget,
    Verify,
    Learn,
    Promotion,
    Replay,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StageEvidenceRecord {
    pub session_id: String,
    pub trace_id: String,
    pub stage: EvidenceStage,
    pub prev_hash: String,
    pub record_hash: String,
    #[serde(default)]
    pub replay_fingerprint: String,
    #[serde(default)]
    pub replay_schema_version: String,
    #[serde(default)]
    pub replay_seed_version: String,
    #[serde(default)]
    pub replay_version: String,
    pub payload: serde_json::Value,
    pub created_at_ms: u64,
}


#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FoundryFeedbackEvidenceRecord {
    pub event_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub operation: String,
    pub kind: String,
    pub detail: String,
    pub created_at_ms: u64,
}
pub struct EvidenceLedgerWriter;
static EVIDENCE_SEQ: AtomicU64 = AtomicU64::new(1);

impl EvidenceLedgerWriter {
    pub async fn append_stage(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        stage: EvidenceStage,
        payload: serde_json::Value,
        prev_hash: Option<&str>,
    ) -> Result<String> {
        let sanitized = sanitize_decision_payload(payload);
        let payload = sanitized.payload;
        let created_at_ms = current_time_ms();
        let latest = Self::latest_stage_record(db, session_id, trace_id).await?;
        let expected_prev_hash = latest
            .as_ref()
            .map(|record| record.record_hash.clone())
            .unwrap_or_else(|| "genesis".to_string());
        if let Some(provided_prev_hash) = prev_hash {
            if provided_prev_hash != expected_prev_hash {
                let audit_ref = Self::audit_worm_violation(
                    db,
                    session_id,
                    trace_id,
                    "prev_hash_mismatch",
                    serde_json::json!({
                        "expected_prev_hash": expected_prev_hash,
                        "provided_prev_hash": provided_prev_hash,
                        "stage": stage,
                    }),
                )
                .await?;
                anyhow::bail!(
                    "evidence worm violation: prev_hash mismatch (audit_ref={audit_ref})"
                );
            }
        }
        let prev_hash = expected_prev_hash;
        let replay_payload = serde_json::json!({
            "stage": stage,
            "payload": payload,
        });
        let replay_fingerprint = replay::build_fingerprint(
            "evidenceeventfp",
            EVIDENCE_EVENT_REPLAY_SCHEMA_VERSION,
            EVIDENCE_EVENT_REPLAY_SEED_VERSION,
            EVIDENCE_EVENT_REPLAY_VERSION,
            &replay_payload,
        );
        let record_hash = hash_value(&serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "stage": stage,
            "prev_hash": prev_hash,
            "payload": payload,
            "created_at_ms": created_at_ms,
        }));
        let record = StageEvidenceRecord {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            stage,
            prev_hash,
            record_hash,
            replay_fingerprint,
            replay_schema_version: EVIDENCE_EVENT_REPLAY_SCHEMA_VERSION.to_string(),
            replay_seed_version: EVIDENCE_EVENT_REPLAY_SEED_VERSION.to_string(),
            replay_version: EVIDENCE_EVENT_REPLAY_VERSION.to_string(),
            payload,
            created_at_ms,
        };
        let seq = EVIDENCE_SEQ.fetch_add(1, Ordering::Relaxed);
        let key = format!(
            "evidence:stage:{session_id}:{trace_id}:{created_at_ms}:{seq}:{:?}",
            record.stage
        )
        .to_ascii_lowercase();
        db.upsert_json_knowledge(key.clone(), &record, "evidence-ledger")
            .await?;
        Ok(key)
    }

    pub async fn append_step(
        db: &StateStore,
        session_id: &str,
        record: &EvidenceStepRecord,
    ) -> Result<String> {
        let key = format!(
            "evidence:step:{session_id}:{}:{}",
            record.created_at_ms, record.step_name
        );
        db.upsert_json_knowledge(key.clone(), record, "evidence-ledger")
            .await?;
        Ok(key)
    }

    pub async fn append_budget(
        db: &StateStore,
        session_id: &str,
        record: &BudgetLedgerRecord,
    ) -> Result<String> {
        let key = format!(
            "evidence:budget:{session_id}:{}:{}",
            record.created_at_ms, record.task_id
        );
        db.upsert_json_knowledge(key.clone(), record, "evidence-ledger")
            .await?;
        Ok(key)
    }

    pub async fn append_approval(
        db: &StateStore,
        session_id: &str,
        record: &ApprovalRecord,
    ) -> Result<String> {
        let key = format!(
            "evidence:approval:{session_id}:{}:{}",
            record.created_at_ms, record.task_id
        );
        db.upsert_json_knowledge(key.clone(), record, "evidence-ledger")
            .await?;
        Ok(key)
    }

    pub async fn append_replay(
        db: &StateStore,
        session_id: &str,
        fingerprint: &ReplayFingerprint,
    ) -> Result<String> {
        let key = format!(
            "evidence:replay:{session_id}:{}:{}",
            current_time_ms(),
            fingerprint.trace_id
        );
        db.upsert_json_knowledge(key.clone(), fingerprint, "evidence-ledger")
            .await?;
        Ok(key)
    }

    pub async fn append_foundry_feedback(
        db: &StateStore,
        session_id: &str,
        record: &FoundryFeedbackEvidenceRecord,
    ) -> Result<String> {
        let key = format!(
            "evidence:foundry:{session_id}:{}:{}",
            record.created_at_ms, record.event_id
        );
        db.upsert_json_knowledge(key.clone(), record, "evidence-ledger")
            .await?;
        Ok(key)
    }

    pub async fn append_op_log(
        db: &StateStore,
        session_id: &str,
        record: &OpLogEvent,
    ) -> Result<String> {
        let key = format!(
            "evidence:op-log:{session_id}:{}:{}",
            record.emitted_at_ms, record.event_id
        );
        db.upsert_json_knowledge(key.clone(), record, "evidence-ledger")
            .await?;
        Ok(key)
    }

    pub async fn verify_stage_chain(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<()> {
        let records = Self::list_stage_records(db, session_id, trace_id).await?;
        let mut prev_hash = "genesis".to_string();
        for (index, record) in records.iter().enumerate() {
            if record.prev_hash != prev_hash {
                anyhow::bail!(
                    "evidence worm chain broken at index={index}: expected prev_hash={}, got={}",
                    prev_hash,
                    record.prev_hash
                );
            }
            let expected_hash = hash_value(&serde_json::json!({
                "session_id": record.session_id,
                "trace_id": record.trace_id,
                "stage": record.stage,
                "prev_hash": record.prev_hash,
                "payload": record.payload,
                "created_at_ms": record.created_at_ms,
            }));
            if expected_hash != record.record_hash {
                anyhow::bail!(
                    "evidence worm chain broken at index={index}: record_hash mismatch (expected={}, got={})",
                    expected_hash,
                    record.record_hash
                );
            }
            prev_hash = record.record_hash.clone();
        }
        Ok(())
    }

    async fn list_stage_records(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<Vec<StageEvidenceRecord>> {
        let mut records = db
            .list_knowledge_by_prefix(&format!("evidence:stage:{session_id}:{trace_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<StageEvidenceRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        records.sort_by_key(|item| item.created_at_ms);
        Ok(records)
    }

    async fn latest_stage_record(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<Option<StageEvidenceRecord>> {
        let mut records = Self::list_stage_records(db, session_id, trace_id).await?;
        Ok(records.pop())
    }

    async fn audit_worm_violation(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        reason: &str,
        detail: serde_json::Value,
    ) -> Result<String> {
        let ts = current_time_ms();
        let key = format!("audit:evidence:worm:{session_id}:{trace_id}:{ts}:{reason}");
        db.upsert_json_knowledge(
            key.clone(),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "reason": reason,
                "detail": detail,
                "created_at_ms": ts,
            }),
            "evidence-worm-audit",
        )
        .await?;
        Ok(key)
    }
}

fn hash_value(value: &serde_json::Value) -> String {
    let mut hasher = DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[derive(Clone)]
pub struct EvidenceLedgerPortAdapter {
    db: StateStore,
    session_id: String,
}

impl EvidenceLedgerPortAdapter {
    pub fn new(db: StateStore, session_id: impl Into<String>) -> Self {
        Self {
            db,
            session_id: session_id.into(),
        }
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::EvidenceLedgerPort for EvidenceLedgerPortAdapter {
    async fn append_evidence_step(
        &self,
        event: &crate::contracts::events::DomainEvent,
    ) -> Result<(), crate::contracts::errors::ContractError> {
        let payload = serde_json::to_value(event)
            .map_err(|e| crate::contracts::errors::ContractError::Internal(e.to_string()))?;
        EvidenceLedgerWriter::append_stage(
            &self.db,
            &self.session_id,
            event.trace_id.as_ref(),
            EvidenceStage::Execution,
            payload,
            None,
        )
        .await
        .map_err(|e| crate::contracts::errors::ContractError::Storage(e.to_string()))?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn evidence_writer_persists_all_segments() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let session = "evidence-session";
        let trace = "trace-a";

        let stage_key = EvidenceLedgerWriter::append_stage(
            &db,
            session,
            trace,
            EvidenceStage::Admission,
            serde_json::json!({"status":"admitted"}),
            None,
        )
        .await
        .expect("stage");
        let _step_key = EvidenceLedgerWriter::append_step(
            &db,
            session,
            &EvidenceStepRecord {
                trace_id: trace.into(),
                step_name: "execution".into(),
                prev_hash: "a".into(),
                record_hash: "b".into(),
                created_at_ms: current_time_ms(),
            },
        )
        .await
        .expect("step");
        let _budget_key = EvidenceLedgerWriter::append_budget(
            &db,
            session,
            &BudgetLedgerRecord {
                trace_id: trace.into(),
                session_id: session.into(),
                task_id: "task-1".into(),
                op: crate::contracts::evidence::BudgetLedgerOp::Reserve,
                amount_micros: 10,
                reason: "reserve".into(),
                created_at_ms: current_time_ms(),
            },
        )
        .await
        .expect("budget");
        let _approval_key = EvidenceLedgerWriter::append_approval(
            &db,
            session,
            &ApprovalRecord {
                session_id: session.into(),
                task_id: "task-1".into(),
                approved: true,
                approver: "operator".into(),
                reason: "ok".into(),
                created_at_ms: current_time_ms(),
            },
        )
        .await
        .expect("approval");
        let _replay_key = EvidenceLedgerWriter::append_replay(
            &db,
            session,
            &ReplayFingerprint {
                trace_id: trace.into(),
                boundary: "strict".into(),
                input_hash: "i".into(),
                output_hash: "o".into(),
                matched: true,
                mismatch_explanation: None,
            },
        )
        .await
        .expect("replay");

        let stage = db
            .get_knowledge(&stage_key)
            .await
            .expect("get")
            .expect("exists");
        assert!(stage.value.contains("admission"));
        assert!(stage.value.contains("\"replay_schema_version\":\"evidence-event-replay/v1\""));
        assert!(stage.value.contains("\"replay_seed_version\":\"evidence-event-seed/v1\""));
        assert!(stage.value.contains("\"replay_version\":\"evidence-event-replay-contract/v1\""));
    }

    #[tokio::test]
    async fn evidence_event_replay_fingerprint_stable_for_same_input_version() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });

        let first = EvidenceLedgerWriter::append_stage(
            &db,
            "evidence-session-a",
            "trace-fp",
            EvidenceStage::Verify,
            serde_json::json!({"decision":"allow","risk":"low"}),
            None,
        )
        .await
        .expect("append first");
        let second = EvidenceLedgerWriter::append_stage(
            &db,
            "evidence-session-b",
            "trace-fp",
            EvidenceStage::Verify,
            serde_json::json!({"risk":"low","decision":"allow"}),
            None,
        )
        .await
        .expect("append second");

        let first_value = db
            .get_knowledge(&first)
            .await
            .expect("first get")
            .expect("first exists");
        let second_value = db
            .get_knowledge(&second)
            .await
            .expect("second get")
            .expect("second exists");
        let first_json: serde_json::Value =
            serde_json::from_str(&first_value.value).expect("first json");
        let second_json: serde_json::Value =
            serde_json::from_str(&second_value.value).expect("second json");

        assert_eq!(
            first_json["replay_fingerprint"],
            second_json["replay_fingerprint"],
            "same stage payload with same schema/seed/version must keep stable replay fingerprint"
        );
    }
    #[tokio::test]
    async fn evidence_writer_applies_decision_log_mask_and_drop() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session = "evidence-mask";
        let trace = "trace-mask";

        let stage_key = EvidenceLedgerWriter::append_stage(
            &db,
            session,
            trace,
            EvidenceStage::Admission,
            serde_json::json!({
                "tenant_id": "tenant-a",
                "secret": "api-key-123456",
                "drop_me": "remove",
                "decision_log_policy": {
                    "policy_version": {"id":"policy-mask-v1","revision":1},
                    "mask_rules": [{"id":"mask-secret","selector":"secret","strategy":"last4"}],
                    "drop_rules": [{"id":"drop-token","selector":"drop_me"}]
                }
            }),
            None,
        )
        .await
        .expect("append stage with mask/drop");

        let stage = db
            .get_knowledge(&stage_key)
            .await
            .expect("get")
            .expect("exists");
        assert!(stage.value.contains("***3456"));
        assert!(!stage.value.contains("\"drop_me\":\"remove\""));
        assert!(stage.value.contains("decision_log_artifact"));
    }
    #[tokio::test]
    async fn evidence_writer_persists_unified_op_log_events() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session = "evidence-oplog";

        let key = EvidenceLedgerWriter::append_op_log(
            &db,
            session,
            &OpLogEvent {
                event_id: "oplog-1".into(),
                session_id: session.into(),
                trace_id: "trace-oplog".into(),
                op_type: "ingest".into(),
                status: "ok".into(),
                emitted_at_ms: current_time_ms(),
                metadata: std::collections::BTreeMap::from([(
                    "source".to_string(),
                    "wiki-ingest".to_string(),
                )]),
                evidence_refs: vec!["evidence:stage:evidence-oplog:trace-oplog".into()],
            },
        )
        .await
        .expect("append op log");

        let stored = db
            .get_knowledge(&key)
            .await
            .expect("get")
            .expect("exists");
        assert!(stored.value.contains("\"op_type\":\"ingest\""));
        assert!(stored.value.contains("\"status\":\"ok\""));
    }
}



