use anyhow::Result;
use autoloop_state_adapter::StateStore;

use super::patch_review_queue::{PatchReviewItem, PatchReviewQueue, PatchReviewStatus};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConflictRecord {
    pub conflict_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub severity: String,
    pub category: String,
    pub summary: String,
    pub resolution: String,
    pub created_at_ms: u64,
}

pub struct ConflictManager;

impl ConflictManager {
    pub async fn analyze(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<Vec<ConflictRecord>> {
        let reviews = PatchReviewQueue::list(db, session_id).await?;
        let records = derive_conflicts(session_id, trace_id, &reviews);
        for record in &records {
            db.upsert_json_knowledge(
                format!(
                    "memory:conflict:{}:{}:{}",
                    session_id, trace_id, record.created_at_ms
                ),
                record,
                "conflict-manager",
            )
            .await?;
        }
        if let Some(latest) = records.last() {
            db.upsert_json_knowledge(
                format!("memory:conflict:{}:latest", session_id),
                latest,
                "conflict-manager",
            )
            .await?;
        }
        Ok(records)
    }
}

fn derive_conflicts(
    session_id: &str,
    trace_id: &str,
    reviews: &[PatchReviewItem],
) -> Vec<ConflictRecord> {
    let mut conflicts = Vec::<ConflictRecord>::new();
    let now = current_time_ms();

    let queued_high = reviews
        .iter()
        .filter(|item| item.status == PatchReviewStatus::Queued && item.decision.risk_score >= 0.7)
        .count();
    if queued_high > 0 {
        conflicts.push(ConflictRecord {
            conflict_id: format!("conflict:{session_id}:{trace_id}:queued-high-risk"),
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            severity: "high".to_string(),
            category: "approval_pending".to_string(),
            summary: format!("{queued_high} high-risk patch items are still queued"),
            resolution: "manual review required before promote/export".to_string(),
            created_at_ms: now,
        });
    }

    let rejected = reviews
        .iter()
        .filter(|item| item.status == PatchReviewStatus::Rejected)
        .count();
    if rejected > 0 {
        conflicts.push(ConflictRecord {
            conflict_id: format!("conflict:{session_id}:{trace_id}:rejected-patch"),
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            severity: "medium".to_string(),
            category: "policy_reject".to_string(),
            summary: format!("{rejected} patch items were rejected"),
            resolution: "replan or supersede patch before continuing".to_string(),
            created_at_ms: now.saturating_add(1),
        });
    }

    conflicts
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

