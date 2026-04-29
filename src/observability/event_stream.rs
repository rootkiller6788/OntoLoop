use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::Result;
use autoloop_state_adapter::{KnowledgeRecord, StateStore};
use serde::{Deserialize, Serialize};

static EVENT_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event_id: String,
    pub kind: String,
    pub trace_id: String,
    pub session_id: String,
    pub task_id: Option<String>,
    pub capability_id: Option<String>,
    pub version: String,
    pub payload: serde_json::Value,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEventView {
    pub session_id: String,
    pub total_events: usize,
    pub by_kind: std::collections::BTreeMap<String, usize>,
    pub latest_trace_id: Option<String>,
    pub latest_event_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRejectView {
    pub session_id: String,
    pub total_rejects: usize,
    pub by_stage: std::collections::BTreeMap<String, usize>,
    pub by_reason: std::collections::BTreeMap<String, usize>,
    pub latest_trace_id: Option<String>,
    pub latest_reason: Option<String>,
    pub latest_reject_ms: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactDigest {
    pub name: String,
    pub algorithm: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeterminismBoundary {
    pub mode: String,
    pub locked_fields: Vec<String>,
    pub non_deterministic_steps: Vec<String>,
    pub external_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedRecord {
    pub seed_key: String,
    pub seed_value: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaySnapshot {
    pub snapshot_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub actor_id: String,
    pub preferred_model: Option<String>,
    pub route_model: Option<String>,
    pub input_digest: String,
    pub parameters_digest: String,
    pub output_digest: String,
    pub artifacts: Vec<ArtifactDigest>,
    pub boundary: DeterminismBoundary,
    pub seed: Option<SeedRecord>,
    pub replay_input: serde_json::Value,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayDeviation {
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub severity: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayAnalysisReport {
    pub snapshot_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub replay_output_digest: String,
    pub matched: bool,
    pub deterministic_boundary_respected: bool,
    pub deviations: Vec<ReplayDeviation>,
    pub notes: Vec<String>,
    pub created_at_ms: u64,
}

pub async fn append_event(
    db: &StateStore,
    kind: impl Into<String>,
    trace_id: impl Into<String>,
    session_id: impl Into<String>,
    task_id: Option<String>,
    capability_id: Option<String>,
    version: impl Into<String>,
    payload: serde_json::Value,
) -> Result<EventEnvelope> {
    let created_at_ms = current_time_ms();
    let seq = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
    let session_id = session_id.into();
    let event = EventEnvelope {
        event_id: format!("evt:{}:{}:{}", session_id, created_at_ms, seq),
        kind: kind.into(),
        trace_id: trace_id.into(),
        session_id: session_id.clone(),
        task_id,
        capability_id,
        version: version.into(),
        payload,
        created_at_ms,
    };
    let key = format!(
        "eventlog:{}:{}:{}",
        event.session_id, event.created_at_ms, seq
    );
    db.upsert_json_knowledge(key, &event, "event-stream")
        .await?;
    Ok(event)
}

pub async fn aggregate_session_view(
    db: &StateStore,
    session_id: &str,
) -> Result<SessionEventView> {
    let records = db
        .list_knowledge_by_prefix(&format!("eventlog:{session_id}:"))
        .await?;
    Ok(build_view(session_id, &records))
}

pub async fn list_session_events(db: &StateStore, session_id: &str) -> Result<Vec<EventEnvelope>> {
    let mut events = db
        .list_knowledge_by_prefix(&format!("eventlog:{session_id}:"))
        .await?
        .into_iter()
        .filter_map(|record| serde_json::from_str::<EventEnvelope>(&record.value).ok())
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.created_at_ms);
    Ok(events)
}

pub async fn replay_trace_events(
    db: &StateStore,
    session_id: &str,
    trace_id: &str,
) -> Result<Vec<EventEnvelope>> {
    let mut events = list_session_events(db, session_id)
        .await?
        .into_iter()
        .filter(|event| event.trace_id == trace_id)
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.created_at_ms);
    Ok(events)
}

pub async fn persist_replay_snapshot(
    db: &StateStore,
    mut snapshot: ReplaySnapshot,
) -> Result<ReplaySnapshot> {
    if snapshot.snapshot_id.trim().is_empty() {
        let seq = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
        snapshot.snapshot_id = format!(
            "replay:{}:{}:{}:{}",
            snapshot.session_id, snapshot.trace_id, snapshot.task_id, seq
        );
    }
    if snapshot.created_at_ms == 0 {
        snapshot.created_at_ms = current_time_ms();
    }
    db.upsert_json_knowledge(
        format!("replay:snapshot:{}", snapshot.snapshot_id),
        &snapshot,
        "replay",
    )
    .await?;
    Ok(snapshot)
}

pub async fn get_replay_snapshot(
    db: &StateStore,
    snapshot_id: &str,
) -> Result<Option<ReplaySnapshot>> {
    let record = db
        .get_knowledge(&format!("replay:snapshot:{snapshot_id}"))
        .await?;
    Ok(record.and_then(|item| serde_json::from_str::<ReplaySnapshot>(&item.value).ok()))
}

pub async fn list_replay_snapshots(
    db: &StateStore,
    session_id: &str,
) -> Result<Vec<ReplaySnapshot>> {
    let mut snapshots = db
        .list_knowledge_by_prefix("replay:snapshot:")
        .await?
        .into_iter()
        .filter_map(|record| serde_json::from_str::<ReplaySnapshot>(&record.value).ok())
        .filter(|snapshot| snapshot.session_id == session_id)
        .collect::<Vec<_>>();
    snapshots.sort_by_key(|snapshot| snapshot.created_at_ms);
    Ok(snapshots)
}

pub async fn persist_replay_analysis(
    db: &StateStore,
    report: &ReplayAnalysisReport,
) -> Result<()> {
    db.upsert_json_knowledge(
        format!(
            "replay:analysis:{}:{}",
            report.snapshot_id, report.created_at_ms
        ),
        report,
        "replay",
    )
    .await?;
    Ok(())
}

pub async fn aggregate_policy_reject_view(
    db: &StateStore,
    session_id: &str,
) -> Result<PolicyRejectView> {
    let events = list_session_events(db, session_id).await?;
    let mut by_stage = std::collections::BTreeMap::<String, usize>::new();
    let mut by_reason = std::collections::BTreeMap::<String, usize>::new();
    let mut latest_trace_id = None;
    let mut latest_reason = None;
    let mut latest_reject_ms = None;

    for event in events
        .into_iter()
        .filter(|event| event.kind == "policy_reject")
    {
        let stage = event
            .payload
            .get("stage")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let reason = event
            .payload
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        *by_stage.entry(stage).or_insert(0) += 1;
        *by_reason.entry(reason.clone()).or_insert(0) += 1;
        if latest_reject_ms
            .map(|ms| event.created_at_ms > ms)
            .unwrap_or(true)
        {
            latest_reject_ms = Some(event.created_at_ms);
            latest_trace_id = Some(event.trace_id.clone());
            latest_reason = Some(reason);
        }
    }

    let total_rejects = by_reason.values().sum();
    Ok(PolicyRejectView {
        session_id: session_id.to_string(),
        total_rejects,
        by_stage,
        by_reason,
        latest_trace_id,
        latest_reason,
        latest_reject_ms,
    })
}

pub fn digest_value(value: &serde_json::Value) -> String {
    let canonical = serde_json::to_string(value).unwrap_or_default();
    digest_text(&canonical)
}

pub fn digest_text(value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn build_view(session_id: &str, records: &[KnowledgeRecord]) -> SessionEventView {
    let mut by_kind = std::collections::BTreeMap::<String, usize>::new();
    let mut latest_trace_id = None;
    let mut latest_event_ms = None;

    for record in records {
        if let Ok(event) = serde_json::from_str::<EventEnvelope>(&record.value) {
            *by_kind.entry(event.kind.clone()).or_insert(0) += 1;
            if latest_event_ms
                .map(|ts| event.created_at_ms > ts)
                .unwrap_or(true)
            {
                latest_event_ms = Some(event.created_at_ms);
                latest_trace_id = Some(event.trace_id.clone());
            }
        }
    }

    SessionEventView {
        session_id: session_id.to_string(),
        total_events: records.len(),
        by_kind,
        latest_trace_id,
        latest_event_ms,
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[test]
    fn p10_digest_value_is_stable_for_same_payload() {
        let payload = serde_json::json!({
            "session": "s-1",
            "task": "t-1",
            "input": ["a", "b", "c"]
        });
        let left = digest_value(&payload);
        let right = digest_value(&payload);
        assert_eq!(left, right);
    }

    #[tokio::test]
    async fn p10_replay_snapshot_roundtrip() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let snapshot = ReplaySnapshot {
            snapshot_id: String::new(),
            session_id: "session-p10".into(),
            trace_id: "trace-p10".into(),
            task_id: "task-p10".into(),
            capability_id: "provider:openai-compatible".into(),
            actor_id: "runtime".into(),
            preferred_model: Some("qwen-plus".into()),
            route_model: Some("qwen-plus".into()),
            input_digest: "in".into(),
            parameters_digest: "params".into(),
            output_digest: "out".into(),
            artifacts: vec![ArtifactDigest {
                name: "provider_output".into(),
                algorithm: "siphash64".into(),
                digest: "deadbeef".into(),
            }],
            boundary: DeterminismBoundary {
                mode: "best_effort".into(),
                locked_fields: vec!["payload".into()],
                non_deterministic_steps: vec!["provider_api".into()],
                external_dependencies: vec!["openai-compatible".into()],
            },
            seed: Some(SeedRecord {
                seed_key: "routing_seed".into(),
                seed_value: "fixed:v1".into(),
                source: "runtime".into(),
            }),
            replay_input: serde_json::json!({"payload":"hello"}),
            created_at_ms: 0,
        };
        let persisted = persist_replay_snapshot(&db, snapshot)
            .await
            .expect("persist");
        let loaded = get_replay_snapshot(&db, &persisted.snapshot_id)
            .await
            .expect("load")
            .expect("exists");
        assert_eq!(loaded.snapshot_id, persisted.snapshot_id);
        assert_eq!(loaded.session_id, "session-p10");
    }
}

