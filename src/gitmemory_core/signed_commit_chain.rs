use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::observability::event_stream::digest_value;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignedCommitRecord {
    pub session_id: String,
    pub trace_id: String,
    pub commit_id: String,
    pub prev_commit_id: Option<String>,
    pub tree_digest: String,
    pub payload_digest: String,
    pub signature: String,
    pub signer: String,
    pub created_at_ms: u64,
}

pub struct SignedCommitChain;

impl SignedCommitChain {
    pub async fn append(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        tree_digest: &str,
        payload: &serde_json::Value,
        signer: &str,
    ) -> Result<SignedCommitRecord> {
        let prev = latest_commit(db, session_id).await?;
        let payload_digest = digest_value(payload);
        let created_at_ms = current_time_ms();
        let prev_commit_id = prev.as_ref().map(|record| record.commit_id.clone());
        let commit_id = digest_value(&serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "prev_commit_id": prev_commit_id,
            "tree_digest": tree_digest,
            "payload_digest": payload_digest,
            "signer": signer,
            "created_at_ms": created_at_ms,
        }));
        let signature = digest_value(&serde_json::json!({
            "commit_id": commit_id,
            "signer": signer,
            "proof": "canonical-memory-signature-v1",
        }));

        let record = SignedCommitRecord {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            commit_id,
            prev_commit_id,
            tree_digest: tree_digest.to_string(),
            payload_digest,
            signature,
            signer: signer.to_string(),
            created_at_ms,
        };

        db.upsert_json_knowledge(
            format!(
                "memory:commit-chain:{}:{}",
                session_id, record.created_at_ms
            ),
            &record,
            "episode-ledger",
        )
        .await?;
        Ok(record)
    }
}

async fn latest_commit(db: &StateStore, session_id: &str) -> Result<Option<SignedCommitRecord>> {
    let prefix = format!("memory:commit-chain:{}:", session_id);
    let mut records = db.list_knowledge_by_prefix(&prefix).await?;
    records.sort_by_key(|row| row.key.clone());
    let latest = records
        .last()
        .and_then(|row| serde_json::from_str::<SignedCommitRecord>(&row.value).ok());
    Ok(latest)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

