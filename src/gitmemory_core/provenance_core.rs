use super::frozen_manifest;
use super::protocol::{CorePackageKind, CorePackageManifest};
use crate::observability::event_stream::digest_value;
use anyhow::Result;
use autoloop_state_adapter::StateStore;

pub struct ProvenanceCore;

impl ProvenanceCore {
    pub fn manifest_frozen() -> CorePackageManifest {
        frozen_manifest(
            "core.provenance",
            CorePackageKind::ProvenanceCore,
            "provenance-ledger",
        )
    }

    pub async fn append_lineage(
        db: &StateStore,
        session_id: &str,
        tenant_id: &str,
        trace_id: &str,
        segments: &[ProvenanceSegmentRef],
        replay_refs: &[String],
        audit_refs: &[String],
    ) -> Result<ProvenanceLineageRecord> {
        let generated_at_ms = current_time_ms();
        let normalized_segments = segments
            .iter()
            .map(|segment| ProvenanceSegmentRecord {
                stage: segment.stage.clone(),
                reference: segment.reference.clone(),
                digest: digest_value(&serde_json::json!({
                    "stage": segment.stage,
                    "reference": segment.reference,
                })),
            })
            .collect::<Vec<_>>();
        let lineage_digest = digest_value(&serde_json::json!({
            "session_id": session_id,
            "tenant_id": tenant_id,
            "trace_id": trace_id,
            "segments": normalized_segments,
            "replay_refs": replay_refs,
            "audit_refs": audit_refs,
            "generated_at_ms": generated_at_ms,
        }));
        let lineage_id = format!("lineage:{}:{}", trace_id, generated_at_ms);

        let record = ProvenanceLineageRecord {
            lineage_id: lineage_id.clone(),
            session_id: session_id.to_string(),
            tenant_id: tenant_id.to_string(),
            trace_id: trace_id.to_string(),
            segments: normalized_segments,
            replay_refs: replay_refs.to_vec(),
            audit_refs: audit_refs.to_vec(),
            lineage_digest,
            generated_at_ms,
        };

        db.upsert_json_knowledge(
            format!("memory:provenance:{session_id}:{trace_id}:{generated_at_ms}"),
            &record,
            "provenance-ledger",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:provenance:{generated_at_ms}"),
            &record,
            "provenance-ledger",
        )
        .await?;
        Ok(record)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProvenanceSegmentRef {
    pub stage: String,
    pub reference: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProvenanceSegmentRecord {
    pub stage: String,
    pub reference: String,
    pub digest: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProvenanceLineageRecord {
    pub lineage_id: String,
    pub session_id: String,
    pub tenant_id: String,
    pub trace_id: String,
    pub segments: Vec<ProvenanceSegmentRecord>,
    pub replay_refs: Vec<String>,
    pub audit_refs: Vec<String>,
    pub lineage_digest: String,
    pub generated_at_ms: u64,
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

