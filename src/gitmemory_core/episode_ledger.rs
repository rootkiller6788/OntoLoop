use anyhow::Result;
use autoloop_state_adapter::StateStore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeStage {
    Gateway,
    Recall,
    Patch,
    Compiler,
    HotIndex,
    View,
    Governance,
    MirrorExport,
    Provenance,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EpisodeRecord {
    pub session_id: String,
    pub trace_id: String,
    pub stage: EpisodeStage,
    pub payload: serde_json::Value,
    pub created_at_ms: u64,
}

pub struct EpisodeLedger;

impl EpisodeLedger {
    pub async fn append(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        stage: EpisodeStage,
        payload: serde_json::Value,
    ) -> Result<String> {
        let created_at_ms = current_time_ms();
        let record = EpisodeRecord {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            stage: stage.clone(),
            payload,
            created_at_ms,
        };
        let key = format!(
            "memory:episode:{}:{}:{}:{:?}",
            session_id, trace_id, created_at_ms, stage
        )
        .to_ascii_lowercase();
        db.upsert_json_knowledge(key.clone(), &record, "episode-ledger")
            .await?;
        Ok(key)
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

