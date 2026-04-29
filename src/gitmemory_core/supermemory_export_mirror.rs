use anyhow::{Result, bail};
use autoloop_state_adapter::StateStore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MirrorExportRequest {
    pub session_id: String,
    pub trace_id: String,
    pub tenant_id: String,
    pub approved: bool,
    pub compiled: bool,
    pub traceable: bool,
    pub approval_ref: Option<String>,
    pub compile_refs: Vec<String>,
    pub trace_refs: Vec<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MirrorExportReceipt {
    pub export_ref: String,
    pub exported_at_ms: u64,
    pub direction: String,
    pub policy: String,
}

pub struct SupermemoryExportMirror;

impl SupermemoryExportMirror {
    pub async fn export(
        db: &StateStore,
        request: &MirrorExportRequest,
    ) -> Result<MirrorExportReceipt> {
        if !request.approved {
            bail!("mirror export rejected: approval is required");
        }
        if !request.compiled {
            bail!("mirror export rejected: compiled evidence is required");
        }
        if !request.traceable {
            bail!("mirror export rejected: traceable evidence is required");
        }
        if request.compile_refs.is_empty() {
            bail!("mirror export rejected: missing compile refs");
        }
        if request.trace_refs.is_empty() {
            bail!("mirror export rejected: missing trace refs");
        }

        let exported_at_ms = current_time_ms();
        let export_ref = format!(
            "memory:supermemory:mirror:export:{}:{}",
            request.session_id, exported_at_ms
        );
        db.upsert_json_knowledge(
            export_ref.clone(),
            &serde_json::json!({
                "session_id": request.session_id,
                "trace_id": request.trace_id,
                "tenant_id": request.tenant_id,
                "approved": request.approved,
                "compiled": request.compiled,
                "traceable": request.traceable,
                "approval_ref": request.approval_ref,
                "compile_refs": request.compile_refs,
                "trace_refs": request.trace_refs,
                "direction": "outbound_only",
                "policy": "approved+compiled+traceable-only",
                "payload": request.payload,
            }),
            "supermemory-export-mirror",
        )
        .await?;

        Ok(MirrorExportReceipt {
            export_ref,
            exported_at_ms,
            direction: "outbound_only".to_string(),
            policy: "approved+compiled+traceable-only".to_string(),
        })
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

