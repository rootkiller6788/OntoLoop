use anyhow::Result;
use autoloop_state_adapter::{AtomicRelationWriteInput, StateStore};

use crate::contracts::relation::{RelationContract, RelationEdge, RelationEvent};

#[derive(Clone)]
pub struct RelationFacade {
    state_store: StateStore,
}

impl RelationFacade {
    pub fn new(state_store: StateStore) -> Self {
        Self { state_store }
    }

    pub async fn upsert_contract(
        &self,
        session_id: &str,
        trace_id: &str,
        contract: &RelationContract,
        source: &str,
    ) -> Result<serde_json::Value> {
        let now_ms = current_time_ms();
        let state_key = format!("relation:state:{session_id}:{trace_id}:{now_ms}");
        let relation_event_key = format!("relation:event:{session_id}:{trace_id}:{now_ms}");
        let evidence_key = format!("relation:evidence:{session_id}:{trace_id}:{now_ms}");
        let write_proof_key = format!("relation:write_proof:{session_id}:{trace_id}:{now_ms}");
        let replay_fp = replay_fingerprint(session_id, trace_id, now_ms, &state_key, &relation_event_key);

        let input = AtomicRelationWriteInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                state_key: state_key.clone(),
                state_payload: serde_json::json!({
                    "kind": "relation_contract",
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "contract": contract,
                    "updated_at_ms": now_ms,
                }),
                relation_event_key: relation_event_key.clone(),
                relation_event_payload: serde_json::json!({
                    "event_type": "relation.contract_upserted",
                    "edge_count": contract.edges.len(),
                    "event_count": contract.events.len(),
                    "updated_at_ms": now_ms,
                }),
                evidence_key: evidence_key.clone(),
                evidence_payload: serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "replay_fp": replay_fp,
                    "state_ref": state_key,
                    "relation_event_ref": relation_event_key,
                    "updated_at_ms": now_ms,
                }),
                write_proof_key: write_proof_key.clone(),
                write_proof_payload: serde_json::json!({
                    "op": "upsert_contract",
                    "path_count": 4,
                    "state_hash": stable_hash(&serde_json::to_string(contract).unwrap_or_default()),
                    "readable": true,
                    "updated_at_ms": now_ms,
                }),
                source: source.to_string(),
                edge_current: None,
                event_append: Some(autoloop_state_adapter::RelationEventAppendWrite {
                    event_id: format!("contract-upsert:{session_id}:{trace_id}:{now_ms}"),
                    event_type: "relation.contract_upserted".to_string(),
                    payload: serde_json::json!({
                        "edge_count": contract.edges.len(),
                        "event_count": contract.events.len(),
                    }),
                }),
                hot_index_entries: vec![autoloop_state_adapter::RelationHotIndexWrite {
                    hot_key: format!("contract:{trace_id}:{now_ms}"),
                    relation_kind: "contract".to_string(),
                    relation_ref: relation_event_key.clone(),
                    score: (contract.edges.len() + contract.events.len()) as f64,
                payload: serde_json::json!({
                        "session_id": session_id,
                        "trace_id": trace_id,
                        "edge_count": contract.edges.len(),
                        "event_count": contract.events.len(),
                    }),
                }],
            };
        let evidence_ref =
            atomic_write_relation_bundle_with_fallback(&self.state_store, &input).await?;

        Ok(serde_json::json!({
            "status": "ok",
            "operation": "upsert_contract",
            "evidence_ref": evidence_ref,
            "replay_fp": replay_fp,
            "state_ref": state_key,
            "relation_event_ref": relation_event_key,
            "write_proof_ref": write_proof_key,
            "edge_count": contract.edges.len(),
            "event_count": contract.events.len(),
        }))
    }

    pub async fn upsert_edge(
        &self,
        session_id: &str,
        trace_id: &str,
        edge: &RelationEdge,
        source: &str,
    ) -> Result<serde_json::Value> {
        let now_ms = current_time_ms();
        let state_key = format!("relation:state:{session_id}:{trace_id}:{now_ms}");
        let relation_event_key = format!("relation:event:{session_id}:{trace_id}:{now_ms}");
        let evidence_key = format!("relation:evidence:{session_id}:{trace_id}:{now_ms}");
        let write_proof_key = format!("relation:write_proof:{session_id}:{trace_id}:{now_ms}");
        let replay_fp = replay_fingerprint(session_id, trace_id, now_ms, &state_key, &relation_event_key);
        let input = AtomicRelationWriteInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                state_key: state_key.clone(),
                state_payload: serde_json::json!({
                    "kind": "relation_edge",
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "edge": edge,
                    "updated_at_ms": now_ms,
                }),
                relation_event_key: relation_event_key.clone(),
                relation_event_payload: serde_json::json!({
                    "event_type": "relation.edge_upserted",
                    "edge_id": edge.edge_id,
                    "edge_type": format!("{:?}", edge.edge_type).to_ascii_lowercase(),
                    "updated_at_ms": now_ms,
                }),
                evidence_key: evidence_key.clone(),
                evidence_payload: serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "replay_fp": replay_fp,
                    "state_ref": state_key,
                    "relation_event_ref": relation_event_key,
                    "updated_at_ms": now_ms,
                }),
                write_proof_key: write_proof_key.clone(),
                write_proof_payload: serde_json::json!({
                    "op": "upsert_edge",
                    "edge_id": edge.edge_id,
                    "state_hash": stable_hash(&serde_json::to_string(edge).unwrap_or_default()),
                    "readable": true,
                    "updated_at_ms": now_ms,
                }),
                source: source.to_string(),
                edge_current: Some(autoloop_state_adapter::RelationEdgeCurrentWrite {
                    edge_id: edge.edge_id.clone(),
                    from_node: edge.from_node_id.clone(),
                    to_node: edge.to_node_id.clone(),
                    edge_type: format!("{:?}", edge.edge_type).to_ascii_lowercase(),
                    payload: serde_json::to_value(edge).unwrap_or_else(|_| serde_json::json!({})),
                }),
                event_append: Some(autoloop_state_adapter::RelationEventAppendWrite {
                    event_id: format!("edge-upsert:{}:{now_ms}", edge.edge_id),
                    event_type: "relation.edge_upserted".to_string(),
                    payload: serde_json::json!({
                        "edge_id": edge.edge_id,
                        "from_node": edge.from_node_id,
                        "to_node": edge.to_node_id,
                    }),
                }),
                hot_index_entries: vec![autoloop_state_adapter::RelationHotIndexWrite {
                    hot_key: format!("edge:{}:{}", edge.edge_id, now_ms),
                    relation_kind: "edge".to_string(),
                    relation_ref: relation_event_key.clone(),
                    score: 1.0,
                    payload: serde_json::json!({
                        "edge_id": edge.edge_id,
                        "edge_type": format!("{:?}", edge.edge_type).to_ascii_lowercase(),
                        "from_node": edge.from_node_id,
                        "to_node": edge.to_node_id,
                    }),
                }],
            };
        let evidence_ref =
            atomic_write_relation_bundle_with_fallback(&self.state_store, &input).await?;
        Ok(serde_json::json!({
            "status": "ok",
            "operation": "upsert_edge",
            "evidence_ref": evidence_ref,
            "replay_fp": replay_fp,
            "state_ref": state_key,
            "relation_event_ref": relation_event_key,
            "write_proof_ref": write_proof_key,
            "edge_id": edge.edge_id,
        }))
    }

    pub async fn append_event(
        &self,
        session_id: &str,
        trace_id: &str,
        event: &RelationEvent,
        source: &str,
    ) -> Result<serde_json::Value> {
        let now_ms = current_time_ms();
        let state_key = format!("relation:state:{session_id}:{trace_id}:{now_ms}");
        let relation_event_key = format!("relation:event:{session_id}:{trace_id}:{now_ms}");
        let evidence_key = format!("relation:evidence:{session_id}:{trace_id}:{now_ms}");
        let write_proof_key = format!("relation:write_proof:{session_id}:{trace_id}:{now_ms}");
        let replay_fp = replay_fingerprint(session_id, trace_id, now_ms, &state_key, &relation_event_key);
        let input = AtomicRelationWriteInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                state_key: state_key.clone(),
                state_payload: serde_json::json!({
                    "kind": "relation_event",
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "event": event,
                    "updated_at_ms": now_ms,
                }),
                relation_event_key: relation_event_key.clone(),
                relation_event_payload: serde_json::json!({
                    "event_type": "relation.event_appended",
                    "event_id": event.event_id,
                    "updated_at_ms": now_ms,
                }),
                evidence_key: evidence_key.clone(),
                evidence_payload: serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "replay_fp": replay_fp,
                    "state_ref": state_key,
                    "relation_event_ref": relation_event_key,
                    "updated_at_ms": now_ms,
                }),
                write_proof_key: write_proof_key.clone(),
                write_proof_payload: serde_json::json!({
                    "op": "append_event",
                    "event_id": event.event_id,
                    "state_hash": stable_hash(&serde_json::to_string(event).unwrap_or_default()),
                    "readable": true,
                    "updated_at_ms": now_ms,
                }),
                source: source.to_string(),
                edge_current: None,
                event_append: Some(autoloop_state_adapter::RelationEventAppendWrite {
                    event_id: event.event_id.clone(),
                    event_type: format!("{:?}", event.event_type).to_ascii_lowercase(),
                    payload: serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})),
                }),
                hot_index_entries: vec![autoloop_state_adapter::RelationHotIndexWrite {
                    hot_key: format!("event:{}:{now_ms}", event.event_id),
                    relation_kind: "event".to_string(),
                    relation_ref: relation_event_key.clone(),
                    score: 1.0,
                    payload: serde_json::json!({
                        "event_id": event.event_id,
                        "event_type": format!("{:?}", event.event_type).to_ascii_lowercase(),
                    }),
                }],
            };
        let evidence_ref =
            atomic_write_relation_bundle_with_fallback(&self.state_store, &input).await?;
        Ok(serde_json::json!({
            "status": "ok",
            "operation": "append_event",
            "evidence_ref": evidence_ref,
            "replay_fp": replay_fp,
            "state_ref": state_key,
            "relation_event_ref": relation_event_key,
            "write_proof_ref": write_proof_key,
            "event_id": event.event_id,
        }))
    }

    pub async fn list_edges(&self, session_id: &str, limit: usize) -> Result<Vec<serde_json::Value>> {
        let records = self
            .state_store
            .list_relation_edges_current(session_id, limit)
            .await?;
        Ok(records
            .into_iter()
            .map(|item| serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({})))
            .collect())
    }

    pub async fn list_events(&self, session_id: &str, limit: usize) -> Result<Vec<serde_json::Value>> {
        let records = self.state_store.list_relation_events(session_id, limit).await?;
        Ok(records
            .into_iter()
            .map(|item| serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({})))
            .collect())
    }

    pub async fn list_hot_index(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let records = self
            .state_store
            .list_relation_hot_index(session_id, limit)
            .await?;
        Ok(records
            .into_iter()
            .map(|item| serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({})))
            .collect())
    }

}

async fn atomic_write_relation_bundle_with_fallback(
    state_store: &StateStore,
    input: &AtomicRelationWriteInput,
) -> Result<String> {
    match state_store.atomic_write_relation_bundle(input.clone()).await {
        Ok(evidence_ref) => Ok(evidence_ref),
        Err(error) if is_missing_postgres_mirror_error(&error) => {
            state_store
                .upsert_json_knowledge(input.state_key.clone(), &input.state_payload, &input.source)
                .await?;
            state_store
                .upsert_json_knowledge(
                    input.relation_event_key.clone(),
                    &input.relation_event_payload,
                    &input.source,
                )
                .await?;
            state_store
                .upsert_json_knowledge(
                    input.evidence_key.clone(),
                    &input.evidence_payload,
                    &input.source,
                )
                .await?;
            state_store
                .upsert_json_knowledge(
                    input.write_proof_key.clone(),
                    &input.write_proof_payload,
                    &input.source,
                )
                .await?;
            Ok(input.evidence_key.clone())
        }
        Err(error) => Err(error),
    }
}

fn is_missing_postgres_mirror_error(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("requires postgres mirror")
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn stable_hash(input: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn replay_fingerprint(
    session_id: &str,
    trace_id: &str,
    now_ms: u64,
    state_key: &str,
    event_key: &str,
) -> String {
    stable_hash(&format!("{session_id}|{trace_id}|{now_ms}|{state_key}|{event_key}"))
}

#[cfg(test)]
mod tests {
    fn has_direct_relation_upsert(content: &str) -> bool {
        let mut cursor = 0usize;
        let relation_markers = [
            "format!(\"relation:state:",
            "format!(\"relation:event:",
            "format!(\"relation:evidence:",
            "format!(\"relation:write_proof:",
            "format!(\"relation:hot-index:",
            "\"relation:state:",
            "\"relation:event:",
            "\"relation:evidence:",
            "\"relation:write_proof:",
            "\"relation:hot-index:",
        ];

        while let Some(offset) = content[cursor..].find("upsert_json_knowledge") {
            let start = cursor + offset;
            let end = (start + 320).min(content.len());
            let window = &content[start..end];
            if relation_markers.iter().any(|marker| window.contains(marker)) {
                return true;
            }
            cursor = start + "upsert_json_knowledge".len();
        }
        false
    }

    #[test]
    fn relation_writes_are_restricted_to_relation_facade() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut violations = Vec::new();
        let mut stack = vec![root.clone()];

        while let Some(path) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.flatten() {
                let file_path = entry.path();
                if file_path.is_dir() {
                    stack.push(file_path);
                    continue;
                }
                if file_path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                let normalized = file_path
                    .strip_prefix(&root)
                    .ok()
                    .map(|item| item.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|| file_path.to_string_lossy().replace('\\', "/"));
                let allowed = matches!(
                    normalized.as_str(),
                    "services/relation_facade.rs" | "main.rs" | "observability/query_plane.rs"
                );
                let content = std::fs::read_to_string(&file_path).unwrap_or_default();
                if !allowed && has_direct_relation_upsert(&content) {
                    violations.push(normalized);
                }
            }
        }

        assert!(
            violations.is_empty(),
            "relation write bypass detected outside RelationFacade: {}",
            violations.join(", ")
        );
    }
}
