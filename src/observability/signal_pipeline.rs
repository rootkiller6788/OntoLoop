use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::{
    config::{SignalPipelineConfig, SignalPipelineMode},
    contracts::{
        signal::{SignalEvent, SignalReason},
        version::{CONTRACT_VERSION, SIGNAL_CONTRACT_VERSION},
    },
    observability::event_stream::append_event,
    orchestration::current_time_ms,
    runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestOutput {
    pub event: SignalEvent,
    pub ingested_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessOutput {
    pub event: SignalEvent,
    pub ingested_at_ms: u64,
    pub processed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalPipelineRecord {
    pub event: SignalEvent,
    pub mode: SignalPipelineMode,
    pub ingested_at_ms: u64,
    pub processed_at_ms: u64,
    pub persisted_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkOutput {
    pub sequence_no: u64,
    pub storage_key: String,
    pub latest_key: String,
    pub evidence_ref: String,
    pub query_explain_ref: Option<String>,
    pub accepted: bool,
    pub mode: SignalPipelineMode,
    pub attempts: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BatchFlushOutput {
    pub requested: usize,
    pub flushed: usize,
    pub outputs: Vec<SinkOutput>,
}

#[derive(Debug, Clone)]
pub struct SignalPipeline {
    cfg: SignalPipelineConfig,
    state: Arc<Mutex<SignalPipelineState>>,
}

static RATE_LIMIT_BUCKETS: LazyLock<Mutex<HashMap<String, u32>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_SIGNAL_SEQUENCE_NO: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default)]
struct SignalPipelineState {
    pending: Vec<SignalEvent>,
}

impl SignalPipeline {
    pub fn from_config(cfg: &SignalPipelineConfig) -> Self {
        Self {
            cfg: cfg.clone(),
            state: Arc::new(Mutex::new(SignalPipelineState::default())),
        }
    }

    #[allow(dead_code)]
    pub fn mode(&self) -> SignalPipelineMode {
        self.cfg.mode.clone()
    }

    pub fn pending_len(&self) -> usize {
        self.state
            .lock()
            .map(|guard| guard.pending.len())
            .unwrap_or_default()
    }

    pub fn ingest(&self, mut event: SignalEvent) -> IngestOutput {
        let ingested_at_ms = current_time_ms();
        let sequence_no = NEXT_SIGNAL_SEQUENCE_NO.fetch_add(1, Ordering::SeqCst);
        if event.signal_id.trim().is_empty() {
            event.signal_id = format!(
                "signal:{}:{}:{}",
                event.context.session_id, event.context.trace_id, ingested_at_ms
            );
        }
        event
            .attributes
            .entry("signal_sequence_no".into())
            .or_insert_with(|| sequence_no.to_string());
        IngestOutput {
            event,
            ingested_at_ms,
        }
    }

    pub fn process(&self, ingested: IngestOutput) -> ProcessOutput {
        let mut event = ingested.event;
        let processed_at_ms = current_time_ms();

        event.attributes.insert(
            "signal_pipeline_mode".into(),
            format!("{:?}", self.cfg.mode).to_ascii_lowercase(),
        );
        event.attributes.insert(
            "signal_contract_version".into(),
            SIGNAL_CONTRACT_VERSION.to_string(),
        );
        event.attributes.insert(
            "signal_processor_order".into(),
            "redact>filter>sample>rate_limit".to_string(),
        );

        let mut decision = event.decision.clone();
        if let Some(reason) = validate_context(&event) {
            decision.accepted = false;
            decision.reason = Some(reason);
        }

        if matches!(self.cfg.mode, SignalPipelineMode::Off) {
            decision.accepted = false;
            decision.reason = Some(SignalReason {
                code: "signal.pipeline_off".into(),
                message: "signal pipeline mode is off".into(),
                processor: Some("process".into()),
            });
        }

        let redacted = redact_event(&mut event);
        event.attributes.insert(
            "signal_redact_applied".into(),
            if redacted {
                "true".to_string()
            } else {
                "false".to_string()
            },
        );

        if decision.accepted {
            if let Some(reason) = filter_event(&event) {
                decision.accepted = false;
                decision.reason = Some(reason);
            }
        }

        event.attributes.insert(
            "signal_filter_applied".into(),
            "true".to_string(),
        );
        event.attributes.insert(
            "signal_filter_rejected".into(),
            if decision.accepted {
                "false".to_string()
            } else {
                "true".to_string()
            },
        );

        if decision.accepted {
            if let Some(reason) = sample_event(&event) {
                decision.accepted = false;
                decision.reason = Some(reason);
            }
        }
        event.attributes.insert("signal_sample_applied".into(), "true".to_string());
        event.attributes.insert(
            "signal_sample_rejected".into(),
            if decision
                .reason
                .as_ref()
                .map(|reason| reason.code.starts_with("signal.sampled_out"))
                .unwrap_or(false)
            {
                "true".to_string()
            } else {
                "false".to_string()
            },
        );

        if decision.accepted {
            if let Some(reason) = rate_limit_event(&event) {
                decision.accepted = false;
                decision.reason = Some(reason);
            }
        }
        event.attributes
            .insert("signal_rate_limit_applied".into(), "true".to_string());
        event.attributes.insert(
            "signal_rate_limit_rejected".into(),
            if decision
                .reason
                .as_ref()
                .map(|reason| reason.code.starts_with("signal.rate_limited"))
                .unwrap_or(false)
            {
                "true".to_string()
            } else {
                "false".to_string()
            },
        );

        event.decision = decision;
        ProcessOutput {
            event,
            ingested_at_ms: ingested.ingested_at_ms,
            processed_at_ms,
        }
    }

    async fn sink_once(&self, db: &StateStore, processed: ProcessOutput) -> Result<SinkOutput> {
        let persisted_at_ms = current_time_ms();
        let session_id = processed.event.context.session_id.clone();
        let signal_id = processed.event.signal_id.clone();
        let sequence_no = signal_sequence_no(&processed.event);

        let record = SignalPipelineRecord {
            event: processed.event.clone(),
            mode: self.cfg.mode.clone(),
            ingested_at_ms: processed.ingested_at_ms,
            processed_at_ms: processed.processed_at_ms,
            persisted_at_ms,
        };
        let storage_key = format!("signal:events:{session_id}:{persisted_at_ms}:{signal_id}");
        let latest_key = format!("signal:events:{session_id}:latest");
        db.upsert_json_knowledge(storage_key.clone(), &record, "signal-pipeline")
            .await?;
        db.upsert_json_knowledge(latest_key.clone(), &record, "signal-pipeline")
            .await?;

        let evidence_ref = EvidenceLedgerWriter::append_stage(
            db,
            &session_id,
            &processed.event.context.trace_id,
            EvidenceStage::Execution,
            serde_json::json!({
                "signal_id": processed.event.signal_id,
                "signal_name": processed.event.name,
                "signal_kind": processed.event.kind,
                "sequence_no": sequence_no,
                "accepted": processed.event.decision.accepted,
                "reason": processed.event.decision.reason,
                "pipeline_mode": format!("{:?}", self.cfg.mode).to_ascii_lowercase(),
                "signal_contract_version": SIGNAL_CONTRACT_VERSION,
            }),
            None,
        )
        .await?;

        let query_explain_ref = format!(
            "observability:signal-explain:{session_id}:{}:{sequence_no}",
            persisted_at_ms
        );
        let query_explain_payload = serde_json::json!({
            "session_id": session_id,
            "trace_id": processed.event.context.trace_id,
            "task_id": processed.event.context.task_id,
            "capability_id": processed.event.context.capability_id,
            "signal_id": processed.event.signal_id,
            "signal_name": processed.event.name,
            "signal_kind": processed.event.kind,
            "sequence_no": sequence_no,
            "accepted": processed.event.decision.accepted,
            "reason_code": processed.event.decision.reason.as_ref().map(|item| item.code.clone()),
            "reason_message": processed.event.decision.reason.as_ref().map(|item| item.message.clone()),
            "evidence_ref": evidence_ref.clone(),
            "created_at_ms": persisted_at_ms,
        });
        let query_explain_write = db
            .upsert_json_knowledge(
                query_explain_ref.clone(),
                &query_explain_payload,
                "signal-pipeline-query-explain",
            )
            .await;
        let query_explain_ref = if query_explain_write.is_ok() {
            let latest_explain_ref = format!("observability:signal-explain:{session_id}:latest");
            let _ = db
                .upsert_json_knowledge(
                    latest_explain_ref,
                    &serde_json::json!({
                        "ref": query_explain_ref,
                        "sequence_no": sequence_no,
                        "created_at_ms": persisted_at_ms,
                    }),
                    "signal-pipeline-query-explain",
                )
                .await;
            Some(query_explain_ref)
        } else {
            None
        };

        let _ = append_event(
            db,
            "signal.pipeline.sink",
            processed.event.context.trace_id.clone(),
            session_id,
            processed.event.context.task_id.clone(),
            processed.event.context.capability_id.clone(),
            CONTRACT_VERSION,
            serde_json::json!({
                "signal_id": processed.event.signal_id,
                "signal_kind": processed.event.kind,
                "accepted": processed.event.decision.accepted,
                "reason_code": processed.event.decision.reason.as_ref().map(|item| item.code.clone()),
                "signal_contract_version": SIGNAL_CONTRACT_VERSION,
                "pipeline_mode": format!("{:?}", self.cfg.mode).to_ascii_lowercase(),
            }),
        )
        .await;

        Ok(SinkOutput {
            sequence_no,
            storage_key,
            latest_key,
            evidence_ref,
            query_explain_ref,
            accepted: processed.event.decision.accepted,
            mode: self.cfg.mode.clone(),
            attempts: 1,
        })
    }

    pub async fn sink(&self, db: &StateStore, processed: ProcessOutput) -> Result<SinkOutput> {
        self.sink_with_retry(db, processed).await
    }

    async fn sink_with_retry(&self, db: &StateStore, processed: ProcessOutput) -> Result<SinkOutput> {
        let max_retries = self.cfg.max_retries;
        let mut attempt: u8 = 0;
        loop {
            attempt = attempt.saturating_add(1);
            match self.sink_once(db, processed.clone()).await {
                Ok(mut output) => {
                    output.attempts = attempt;
                    return Ok(output);
                }
                Err(error) => {
                    if attempt > max_retries {
                        return Err(error).with_context(|| {
                            format!(
                                "signal sink retries exhausted; sequence_no={} attempts={attempt}",
                                signal_sequence_no(&processed.event)
                            )
                        });
                    }
                    let backoff = retry_backoff_ms(self.cfg.retry_backoff_ms, attempt);
                    sleep(Duration::from_millis(backoff)).await;
                }
            }
        }
    }

    pub async fn execute(&self, db: &StateStore, event: SignalEvent) -> Result<SinkOutput> {
        let ingested = self.ingest(event);
        let processed = self.process(ingested);
        self.sink(db, processed).await
    }

    pub fn enqueue(&self, event: SignalEvent) -> usize {
        let ingested = self.ingest(event);
        match self.state.lock() {
            Ok(mut guard) => {
                guard.pending.push(ingested.event);
                guard.pending.len()
            }
            Err(_) => 0,
        }
    }

    pub async fn execute_batch(
        &self,
        db: &StateStore,
        events: Vec<SignalEvent>,
    ) -> Result<BatchFlushOutput> {
        self.flush_events(db, events).await
    }

    pub async fn flush_if_needed(&self, db: &StateStore) -> Result<Option<BatchFlushOutput>> {
        let batch_size = self.cfg.batch_size.max(1);
        let snapshot = {
            let guard = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("signal pipeline pending queue is unavailable"))?;
            if guard.pending.len() < batch_size {
                return Ok(None);
            }
            guard
                .pending
                .iter()
                .take(batch_size)
                .cloned()
                .collect::<Vec<_>>()
        };
        let output = self.flush_events(db, snapshot).await?;
        {
            let mut guard = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("signal pipeline pending queue is unavailable"))?;
            let to_remove = output.flushed.min(guard.pending.len());
            guard.pending.drain(0..to_remove);
        }
        Ok(Some(output))
    }

    pub async fn shutdown_flush(&self, db: &StateStore) -> Result<BatchFlushOutput> {
        let snapshot = {
            let guard = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("signal pipeline pending queue is unavailable"))?;
            guard.pending.clone()
        };
        let output = self.flush_events(db, snapshot).await?;
        {
            let mut guard = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("signal pipeline pending queue is unavailable"))?;
            guard.pending.clear();
        }
        Ok(output)
    }

    async fn flush_events(
        &self,
        db: &StateStore,
        events: Vec<SignalEvent>,
    ) -> Result<BatchFlushOutput> {
        let requested = events.len();
        if requested == 0 {
            return Ok(BatchFlushOutput::default());
        }

        let mut outputs = Vec::with_capacity(requested);
        for event in events {
            let sequence_no = signal_sequence_no(&event);
            let ingested = self.ingest(event);
            let processed = self.process(ingested);
            match self.sink_with_retry(db, processed).await {
                Ok(output) => outputs.push(output),
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("signal batch flush failed at sequence_no={sequence_no}")
                    });
                }
            }
        }

        Ok(BatchFlushOutput {
            requested,
            flushed: outputs.len(),
            outputs,
        })
    }
}

fn validate_context(event: &SignalEvent) -> Option<SignalReason> {
    if event.context.session_id.trim().is_empty() {
        return Some(SignalReason {
            code: "signal.missing_session_id".into(),
            message: "session_id is required".into(),
            processor: Some("process".into()),
        });
    }
    if event.context.trace_id.trim().is_empty() {
        return Some(SignalReason {
            code: "signal.missing_trace_id".into(),
            message: "trace_id is required".into(),
            processor: Some("process".into()),
        });
    }
    if event.name.trim().is_empty() {
        return Some(SignalReason {
            code: "signal.missing_name".into(),
            message: "signal name is required".into(),
            processor: Some("process".into()),
        });
    }
    None
}

fn redact_event(event: &mut SignalEvent) -> bool {
    let mut changed = false;
    let mut redacted_attrs = std::collections::BTreeMap::new();
    for (key, value) in &event.attributes {
        if is_sensitive_key(key) {
            redacted_attrs.insert(key.clone(), "[REDACTED]".to_string());
            changed = true;
        } else {
            let (next, replaced) = redact_text(value);
            redacted_attrs.insert(key.clone(), next);
            changed |= replaced;
        }
    }
    event.attributes = redacted_attrs;

    if let Some(body) = event.body.as_mut() {
        let (next, replaced) = redact_text(body);
        *body = next;
        changed |= replaced;
    }

    changed
}

fn redact_text(value: &str) -> (String, bool) {
    let mut output = value.to_string();
    let mut changed = false;
    for (key, replacement) in [
        ("api_key=", "[REDACTED]"),
        ("token=", "[REDACTED]"),
        ("secret_block=", "[REDACTED:BLOCK]"),
    ] {
        let (next, replaced) = redact_assignment(&output, key, replacement);
        if replaced {
            output = next;
            changed = true;
        }
    }
    let bearer_pattern = "authorization: bearer ";
    if output
        .to_ascii_lowercase()
        .contains(&bearer_pattern.to_ascii_lowercase())
    {
        output = replace_case_insensitive(
            &output,
            bearer_pattern,
            "authorization: bearer [REDACTED]",
        );
        changed = true;
    }
    (output, changed)
}

fn is_sensitive_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    ["api_key", "token", "authorization", "secret", "password"]
        .iter()
        .any(|candidate| lowered.contains(candidate))
}

fn replace_case_insensitive(source: &str, pattern: &str, replacement: &str) -> String {
    let source_lower = source.to_ascii_lowercase();
    let pattern_lower = pattern.to_ascii_lowercase();
    if let Some(index) = source_lower.find(&pattern_lower) {
        let mut output = String::new();
        output.push_str(&source[..index]);
        output.push_str(replacement);
        output.push_str(&source[index + pattern.len()..]);
        output
    } else {
        source.to_string()
    }
}

fn redact_assignment(source: &str, key: &str, replacement: &str) -> (String, bool) {
    let source_lower = source.to_ascii_lowercase();
    let key_lower = key.to_ascii_lowercase();
    let Some(start) = source_lower.find(&key_lower) else {
        return (source.to_string(), false);
    };
    let value_start = start + key.len();
    let tail = &source[value_start..];
    let value_end_offset = tail
        .find(|ch: char| matches!(ch, ' ' | '&' | ';' | ',' | '\n' | '\r' | '\t'))
        .unwrap_or(tail.len());
    let value_end = value_start + value_end_offset;

    let mut output = String::new();
    output.push_str(&source[..value_start]);
    output.push_str(replacement);
    output.push_str(&source[value_end..]);
    (output, true)
}

fn filter_event(event: &SignalEvent) -> Option<SignalReason> {
    if event.name.starts_with("signal.internal.drop.") {
        return Some(SignalReason {
            code: "signal.filtered.internal_prefix".into(),
            message: "signal name rejected by internal drop prefix rule".into(),
            processor: Some("filter".into()),
        });
    }

    let body = event.body.as_deref().unwrap_or_default();
    if body.contains("[REDACTED:BLOCK]") {
        return Some(SignalReason {
            code: "signal.filtered.redacted_block_marker".into(),
            message: "redacted block marker requires dropping this signal".into(),
            processor: Some("filter".into()),
        });
    }

    if event
        .attributes
        .values()
        .any(|value| value.contains("[REDACTED:BLOCK]"))
    {
        return Some(SignalReason {
            code: "signal.filtered.redacted_block_marker".into(),
            message: "redacted block marker requires dropping this signal".into(),
            processor: Some("filter".into()),
        });
    }

    None
}

fn sample_event(event: &SignalEvent) -> Option<SignalReason> {
    let ratio = event
        .attributes
        .get("sample_ratio")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);

    if ratio >= 1.0 {
        return None;
    }
    if ratio <= 0.0 {
        return Some(SignalReason {
            code: "signal.sampled_out.zero_ratio".into(),
            message: "signal dropped by zero sample ratio".into(),
            processor: Some("sample".into()),
        });
    }

    let hash = stable_hash(&format!(
        "{}:{}:{}",
        event.context.session_id, event.context.trace_id, event.signal_id
    ));
    let normalized = (hash % 10_000) as f64 / 10_000.0;
    if normalized > ratio {
        return Some(SignalReason {
            code: "signal.sampled_out.ratio".into(),
            message: format!("signal dropped by sample ratio {}", ratio),
            processor: Some("sample".into()),
        });
    }

    None
}

fn rate_limit_event(event: &SignalEvent) -> Option<SignalReason> {
    let limit = event
        .attributes
        .get("rate_limit_per_minute")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(120);
    if limit == 0 {
        return Some(SignalReason {
            code: "signal.rate_limited.zero_limit".into(),
            message: "signal dropped by zero per-minute limit".into(),
            processor: Some("rate_limit".into()),
        });
    }

    let minute_bucket = event.emitted_at_ms / 60_000;
    let key = format!(
        "{}:{}:{}",
        event.context.session_id, event.name, minute_bucket
    );

    let mut guard = match RATE_LIMIT_BUCKETS.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return Some(SignalReason {
                code: "signal.rate_limited.lock_poisoned".into(),
                message: "rate limit state unavailable".into(),
                processor: Some("rate_limit".into()),
            });
        }
    };
    let counter = guard.entry(key).or_insert(0);
    *counter = counter.saturating_add(1);
    if *counter > limit {
        return Some(SignalReason {
            code: "signal.rate_limited.per_minute".into(),
            message: format!("signal exceeded per-minute limit {}", limit),
            processor: Some("rate_limit".into()),
        });
    }
    None
}

fn stable_hash(value: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn signal_sequence_no(event: &SignalEvent) -> u64 {
    event
        .attributes
        .get("signal_sequence_no")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default()
}

fn retry_backoff_ms(base_ms: u64, attempt: u8) -> u64 {
    let attempt = attempt.max(1).saturating_sub(1) as u32;
    let multiplier = 2_u64.saturating_pow(attempt.min(10));
    base_ms.saturating_mul(multiplier.max(1))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use autoloop_state_adapter::{StateStore, StateStoreBackend, StateStoreConfig};

    use super::*;
    use crate::contracts::signal::{SignalContext, SignalDecision, SignalKind};

    fn in_memory_state_store() -> StateStore {
        StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        })
    }

    fn test_event() -> SignalEvent {
        SignalEvent {
            signal_id: String::new(),
            kind: SignalKind::Trace,
            name: "runtime.execute.start".into(),
            context: SignalContext {
                session_id: "session-signal-1".into(),
                trace_id: "trace:signal:1".into(),
                span_id: Some("span:signal:1".into()),
                task_id: Some("task:signal:1".into()),
                capability_id: Some("tool:write_file".into()),
                tenant_id: Some("tenant:test".into()),
                principal_id: Some("operator:test".into()),
            },
            attributes: BTreeMap::new(),
            numeric_value: None,
            body: Some("start".into()),
            decision: SignalDecision {
                accepted: true,
                reason: None,
                evidence_ref: Some("evidence:signal:1".into()),
            },
            emitted_at_ms: current_time_ms(),
        }
    }

    #[test]
    fn signal_pipeline_defaults_to_shadow_mode() {
        let cfg = SignalPipelineConfig::default();
        let pipeline = SignalPipeline::from_config(&cfg);
        assert!(matches!(pipeline.mode(), SignalPipelineMode::Shadow));
    }

    #[tokio::test]
    async fn signal_pipeline_ingest_process_sink_roundtrip() {
        let db = in_memory_state_store();
        let pipeline = SignalPipeline::from_config(&SignalPipelineConfig::default());
        let output = pipeline
            .execute(&db, test_event())
            .await
            .expect("signal pipeline execution");
        assert!(output.accepted);
        assert!(output.evidence_ref.starts_with("evidence:stage:"));
        assert!(output.query_explain_ref.is_some());

        let record = db
            .get_knowledge(&output.latest_key)
            .await
            .expect("read latest key")
            .expect("latest key exists");
        assert!(record.value.contains("runtime.execute.start"));
        assert!(record.value.contains("\"signal_pipeline_mode\":\"shadow\""));

        let explain_ref = output.query_explain_ref.expect("explain ref");
        let explain = db
            .get_knowledge(&explain_ref)
            .await
            .expect("read explain")
            .expect("explain exists");
        assert!(explain.value.contains("\"evidence_ref\""));
    }

    #[test]
    fn process_runs_redact_before_filter() {
        let pipeline = SignalPipeline::from_config(&SignalPipelineConfig::default());
        let mut event = test_event();
        event.body = Some("secret_block=raw-secret".into());
        let processed = pipeline.process(pipeline.ingest(event));

        assert!(!processed.event.decision.accepted);
        assert_eq!(
            processed
                .event
                .decision
                .reason
                .as_ref()
                .map(|item| item.code.as_str()),
            Some("signal.filtered.redacted_block_marker")
        );
        assert_eq!(
            processed.event.body.as_deref(),
            Some("secret_block=[REDACTED:BLOCK]")
        );
        assert_eq!(
            processed
                .event
                .attributes
                .get("signal_processor_order")
                .map(String::as_str),
            Some("redact>filter>sample>rate_limit")
        );
    }

    #[test]
    fn process_redacts_sensitive_content_without_filter_reject() {
        let pipeline = SignalPipeline::from_config(&SignalPipelineConfig::default());
        let mut event = test_event();
        event.body = Some("token=abc123".into());
        let processed = pipeline.process(pipeline.ingest(event));

        assert!(processed.event.decision.accepted);
        assert_eq!(
            processed.event.body.as_deref(),
            Some("token=[REDACTED]")
        );
        assert_eq!(
            processed
                .event
                .attributes
                .get("signal_redact_applied")
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            processed
                .event
                .attributes
                .get("signal_filter_rejected")
                .map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn process_rejects_when_sample_ratio_is_zero() {
        let pipeline = SignalPipeline::from_config(&SignalPipelineConfig::default());
        let mut event = test_event();
        event
            .attributes
            .insert("sample_ratio".into(), "0.0".into());

        let processed = pipeline.process(pipeline.ingest(event));
        assert!(!processed.event.decision.accepted);
        assert_eq!(
            processed
                .event
                .decision
                .reason
                .as_ref()
                .map(|item| item.code.as_str()),
            Some("signal.sampled_out.zero_ratio")
        );
    }

    #[test]
    fn process_rejects_when_rate_limit_exceeded() {
        let pipeline = SignalPipeline::from_config(&SignalPipelineConfig::default());
        let base_ms = current_time_ms();

        let mut event_a = test_event();
        event_a.signal_id = format!("signal-rate-limit-a-{base_ms}");
        event_a.name = format!("runtime.execute.rate-limit-{base_ms}");
        event_a.emitted_at_ms = base_ms;
        event_a
            .attributes
            .insert("rate_limit_per_minute".into(), "1".into());
        let first = pipeline.process(pipeline.ingest(event_a));
        assert!(first.event.decision.accepted);

        let mut event_b = test_event();
        event_b.signal_id = format!("signal-rate-limit-b-{base_ms}");
        event_b.name = format!("runtime.execute.rate-limit-{base_ms}");
        event_b.emitted_at_ms = base_ms + 1;
        event_b
            .attributes
            .insert("rate_limit_per_minute".into(), "1".into());
        let second = pipeline.process(pipeline.ingest(event_b));
        assert!(!second.event.decision.accepted);
        assert_eq!(
            second
                .event
                .decision
                .reason
                .as_ref()
                .map(|item| item.code.as_str()),
            Some("signal.rate_limited.per_minute")
        );
    }

    #[tokio::test]
    async fn flush_and_shutdown_keep_sequence_contiguous() {
        let db = in_memory_state_store();
        let mut cfg = SignalPipelineConfig::default();
        cfg.batch_size = 2;
        let pipeline = SignalPipeline::from_config(&cfg);

        let mut event_a = test_event();
        event_a.signal_id = "signal:batch:a".into();
        let mut event_b = test_event();
        event_b.signal_id = "signal:batch:b".into();
        let mut event_c = test_event();
        event_c.signal_id = "signal:batch:c".into();

        pipeline.enqueue(event_a);
        pipeline.enqueue(event_b);
        pipeline.enqueue(event_c);

        let first_flush = pipeline
            .flush_if_needed(&db)
            .await
            .expect("flush_if_needed")
            .expect("batch ready");
        assert_eq!(first_flush.flushed, 2);
        let seq1 = first_flush.outputs[0].sequence_no;
        let seq2 = first_flush.outputs[1].sequence_no;
        assert!(seq2 > seq1);

        let shutdown = pipeline.shutdown_flush(&db).await.expect("shutdown_flush");
        assert_eq!(shutdown.flushed, 1);
        let seq3 = shutdown.outputs[0].sequence_no;
        assert!(seq3 > seq2);
    }

    #[test]
    fn retry_backoff_is_exponential() {
        assert_eq!(retry_backoff_ms(25, 1), 25);
        assert_eq!(retry_backoff_ms(25, 2), 50);
        assert_eq!(retry_backoff_ms(25, 3), 100);
    }
}
