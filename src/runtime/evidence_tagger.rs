use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde_json::{Map, Value, json};

use crate::runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTagStage {
    Admission,
    Guard,
    Execute,
    Verify,
    Learn,
    Replay,
}

impl EvidenceTagStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::Guard => "guard",
            Self::Execute => "execute",
            Self::Verify => "verify",
            Self::Learn => "learn",
            Self::Replay => "replay",
        }
    }

    pub fn to_ledger_stage(&self) -> EvidenceStage {
        match self {
            Self::Admission => EvidenceStage::Admission,
            Self::Guard => EvidenceStage::Execution,
            Self::Execute => EvidenceStage::Execution,
            Self::Verify => EvidenceStage::Verify,
            Self::Learn => EvidenceStage::Learn,
            Self::Replay => EvidenceStage::Replay,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvidenceTag {
    pub session_id: String,
    pub trace_id: String,
    pub task_id: Option<String>,
    pub capability_id: Option<String>,
    pub stage: EvidenceTagStage,
    pub label: String,
    pub tags: Vec<String>,
    pub payload: serde_json::Value,
    pub created_at_ms: u64,
}

pub struct EvidenceTagger;

impl EvidenceTagger {
    pub fn infer_stage(step_id: &str, event_type: &str) -> EvidenceTagStage {
        let step = step_id.to_ascii_lowercase();
        let event = event_type.to_ascii_lowercase();
        if step.starts_with("admission") || event.contains("admission") {
            EvidenceTagStage::Admission
        } else if step.starts_with("guard") || event.contains("guard") {
            EvidenceTagStage::Guard
        } else if step.starts_with("execute") || event.contains("execution") {
            EvidenceTagStage::Execute
        } else if step.starts_with("verify") || event.contains("verifier") {
            EvidenceTagStage::Verify
        } else if step.starts_with("learn")
            || step.starts_with("promotion")
            || event.contains("learning")
            || event.contains("promotion")
        {
            EvidenceTagStage::Learn
        } else if step.starts_with("replay") || event.contains("replay") {
            EvidenceTagStage::Replay
        } else {
            EvidenceTagStage::Execute
        }
    }

    pub async fn write(
        db: &StateStore,
        tag: &EvidenceTag,
        prev_hash: Option<&str>,
    ) -> Result<String> {
        let normalized_payload = normalize_payload_with_tracking(tag);
        let tracking_context = normalized_payload
            .get("tracking_context")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));

        let stage_payload = serde_json::json!({
            "label": tag.label,
            "stage": tag.stage.as_str(),
            "tags": tag.tags,
            "task_id": tag.task_id,
            "capability_id": tag.capability_id,
            "tracking_context": tracking_context,
            "payload": normalized_payload,
            "created_at_ms": tag.created_at_ms,
        });
        let _ = EvidenceLedgerWriter::append_stage(
            db,
            &tag.session_id,
            &tag.trace_id,
            tag.stage.to_ledger_stage(),
            stage_payload,
            prev_hash,
        )
        .await?;

        let key = format!(
            "evidence:tag:{}:{}:{}:{}:{}",
            tag.session_id,
            tag.trace_id,
            tag.created_at_ms,
            tag.stage.as_str(),
            sanitize_key_part(&tag.label)
        );

        let persisted_tag = json!({
            "session_id": tag.session_id,
            "trace_id": tag.trace_id,
            "task_id": tag.task_id,
            "capability_id": tag.capability_id,
            "stage": tag.stage,
            "label": tag.label,
            "tags": tag.tags,
            "payload": normalized_payload,
            "created_at_ms": tag.created_at_ms,
        });
        db.upsert_json_knowledge(key.clone(), &persisted_tag, "evidence-tagger")
            .await?;
        Ok(key)
    }
}

fn sanitize_key_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_ascii_lowercase()
}

fn normalize_payload_with_tracking(tag: &EvidenceTag) -> Value {
    let mut payload = match tag.payload.clone() {
        Value::Object(map) => map,
        other => {
            let mut wrapped = Map::new();
            wrapped.insert("raw".into(), other);
            wrapped
        }
    };

    let tracking = build_tracking_context(tag, &payload);
    payload.insert("tracking_context".into(), Value::Object(tracking));
    Value::Object(payload)
}

fn build_tracking_context(tag: &EvidenceTag, payload: &Map<String, Value>) -> Map<String, Value> {
    let mut context = extract_tracking_object(payload).unwrap_or_default();

    set_if_missing(
        &mut context,
        "run_id",
        text_or_default(
            payload,
            &["run_id", "execution_id"],
            format!("run:{}:{}", tag.session_id, tag.trace_id),
        ),
    );
    set_if_missing(
        &mut context,
        "plan_id",
        text_or_default(payload, &["plan_id"], format!("plan:{}", tag.trace_id)),
    );
    set_if_missing(
        &mut context,
        "focus_id",
        text_or_default(
            payload,
            &["focus_id"],
            format!("focus-board:{}:latest", tag.session_id),
        ),
    );
    set_if_missing(
        &mut context,
        "trigger_id",
        text_or_default(
            payload,
            &["trigger_id"],
            format!("trigger-runtime:{}:latest", tag.session_id),
        ),
    );

    let capability = payload
        .get("capability_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| tag.capability_id.clone())
        .unwrap_or_else(|| "capability:unknown".to_string());
    set_if_missing(&mut context, "capability_id", capability);

    let verifier_id = payload
        .get("verifier_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| infer_verifier_id(tag));
    set_if_missing(&mut context, "verifier_id", verifier_id);

    let tenant_id = payload
        .get("tenant_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("identity")
                .and_then(Value::as_object)
                .and_then(|identity| identity.get("tenant_id"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "tenant:unknown".to_string());
    set_if_missing(&mut context, "tenant_id", tenant_id);

    let replay_fp = payload
        .get("replay_fp")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            payload
                .get("snapshot_id")
                .and_then(Value::as_str)
                .map(|snapshot_id| format!("replay-fp:{snapshot_id}"))
        })
        .or_else(|| {
            if matches!(tag.stage, EvidenceTagStage::Replay) {
                Some(format!("replay-fp:{}", tag.trace_id))
            } else {
                None
            }
        })
        .unwrap_or_else(|| "replay-fp:pending".to_string());
    set_if_missing(&mut context, "replay_fp", replay_fp);

    set_if_missing(
        &mut context,
        "org_ctx_id",
        text_or_default(
            payload,
            &["org_ctx_id"],
            format!("org-context:{}:latest", tag.session_id),
        ),
    );

    context
}

fn extract_tracking_object(payload: &Map<String, Value>) -> Option<Map<String, Value>> {
    payload
        .get("tracking_context")
        .and_then(Value::as_object)
        .cloned()
}

fn text_or_default(payload: &Map<String, Value>, keys: &[&str], default: String) -> String {
    for key in keys {
        if let Some(value) = payload.get(*key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return value.to_string();
            }
        }
    }
    default
}

fn set_if_missing(context: &mut Map<String, Value>, key: &str, value: String) {
    let needs_set = match context.get(key) {
        Some(Value::String(existing)) => existing.trim().is_empty(),
        Some(Value::Null) | None => true,
        _ => false,
    };
    if needs_set {
        context.insert(key.to_string(), Value::String(value));
    }
}

fn infer_verifier_id(tag: &EvidenceTag) -> String {
    let lowered_label = tag.label.to_ascii_lowercase();
    if matches!(tag.stage, EvidenceTagStage::Verify) || lowered_label.contains("verifier") {
        "verifier-agent".to_string()
    } else if matches!(tag.stage, EvidenceTagStage::Learn) {
        "promotion-verifier".to_string()
    } else {
        "verifier:n/a".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[test]
    fn infer_stage_maps_core_paths() {
        assert_eq!(
            EvidenceTagger::infer_stage("admission", "ADMISSION_ACCEPTED"),
            EvidenceTagStage::Admission
        );
        assert_eq!(
            EvidenceTagger::infer_stage("guard", "GUARD_DECISION"),
            EvidenceTagStage::Guard
        );
        assert_eq!(
            EvidenceTagger::infer_stage("execute.tool", "EXECUTION_COMPLETED"),
            EvidenceTagStage::Execute
        );
        assert_eq!(
            EvidenceTagger::infer_stage("verify.execution", "VERIFIER_PASSED"),
            EvidenceTagStage::Verify
        );
        assert_eq!(
            EvidenceTagger::infer_stage("learn.consolidation", "LEARNING_UPDATED"),
            EvidenceTagStage::Learn
        );
        assert_eq!(
            EvidenceTagger::infer_stage("replay.snapshot", "REPLAY_FINGERPRINT_CAPTURED"),
            EvidenceTagStage::Replay
        );
    }

    #[tokio::test]
    async fn write_persists_tag_record() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let tag = EvidenceTag {
            session_id: "session-tag".into(),
            trace_id: "trace-tag".into(),
            task_id: Some("task-tag".into()),
            capability_id: Some("capability-tag".into()),
            stage: EvidenceTagStage::Verify,
            label: "verifier.execution_decision".into(),
            tags: vec!["verify".into(), "contract-v1".into()],
            payload: serde_json::json!({"score":0.92,"verdict":"pass"}),
            created_at_ms: 1,
        };

        let key = EvidenceTagger::write(&db, &tag, None)
            .await
            .expect("write tag");
        let stored = db.get_knowledge(&key).await.expect("get").expect("exists");
        assert!(stored.value.contains("verifier.execution_decision"));

        let stages = db
            .list_knowledge_by_prefix("evidence:stage:session-tag:trace-tag:")
            .await
            .expect("list stages");
        assert!(!stages.is_empty());
    }

    #[tokio::test]
    async fn write_enforces_uniform_tracking_context_schema() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let tag = EvidenceTag {
            session_id: "session-schema".into(),
            trace_id: "trace-schema".into(),
            task_id: Some("task-schema".into()),
            capability_id: Some("capability-schema".into()),
            stage: EvidenceTagStage::Execute,
            label: "execution.completed".into(),
            tags: vec!["execute".into()],
            payload: serde_json::json!({"decision":"allow"}),
            created_at_ms: 42,
        };

        let key = EvidenceTagger::write(&db, &tag, None)
            .await
            .expect("write tag");
        let stored = db.get_knowledge(&key).await.expect("get").expect("exists");
        let value: Value = serde_json::from_str(&stored.value).expect("json");
        let tracking = value
            .get("payload")
            .and_then(|payload| payload.get("tracking_context"))
            .and_then(Value::as_object)
            .expect("tracking context object");

        for key in [
            "run_id",
            "plan_id",
            "focus_id",
            "trigger_id",
            "capability_id",
            "verifier_id",
            "tenant_id",
            "replay_fp",
            "org_ctx_id",
        ] {
            let item = tracking.get(key).and_then(Value::as_str).unwrap_or("");
            assert!(!item.is_empty(), "tracking key {} should be non-empty", key);
        }
    }
}

