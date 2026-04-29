use anyhow::Result;
use autoloop_state_adapter::StateStore;

use super::patch_core::{PatchOpKind, PatchPlan};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatchReviewStatus {
    Queued,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PatchPolicyDecision {
    pub risk_score: f32,
    pub approval_required: bool,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PatchReviewItem {
    pub review_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub status: PatchReviewStatus,
    pub decision: PatchPolicyDecision,
    pub patch: PatchPlan,
    pub reviewer: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default = "default_review_kind")]
    pub review_kind: String,
    #[serde(default)]
    pub proposal_ref: Option<String>,
}

pub struct PatchPolicyEngine;

impl PatchPolicyEngine {
    pub fn evaluate(patch: &PatchPlan) -> PatchPolicyDecision {
        let mut score = 0.1_f32;
        for op in &patch.ops {
            score += match op.kind {
                PatchOpKind::Add => 0.15,
                PatchOpKind::Update => 0.25,
                PatchOpKind::Delete => 0.45,
                PatchOpKind::None => 0.0,
            };
        }
        let approval_required = score >= 0.7
            || patch
                .ops
                .iter()
                .any(|op| matches!(op.kind, PatchOpKind::Delete));
        PatchPolicyDecision {
            risk_score: score.min(1.0),
            approval_required,
            reason: if approval_required {
                "high-risk patch requires review".to_string()
            } else {
                "low-risk patch auto-approved".to_string()
            },
        }
    }
}

pub struct PatchReviewQueue;

impl PatchReviewQueue {
    pub async fn enqueue(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        patch: &PatchPlan,
    ) -> Result<PatchReviewItem> {
        Self::enqueue_internal(db, session_id, trace_id, patch, "patch", None).await
    }

    pub async fn enqueue_heal_proposal(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        patch: &PatchPlan,
        proposal_ref: &str,
    ) -> Result<PatchReviewItem> {
        Self::enqueue_internal(
            db,
            session_id,
            trace_id,
            patch,
            "heal_proposal",
            Some(proposal_ref.to_string()),
        )
        .await
    }

    pub async fn enqueue_relation_repair_proposal(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        patch: &PatchPlan,
        proposal_ref: &str,
    ) -> Result<PatchReviewItem> {
        Self::enqueue_internal(
            db,
            session_id,
            trace_id,
            patch,
            "relation_repair",
            Some(proposal_ref.to_string()),
        )
        .await
    }

    async fn enqueue_internal(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        patch: &PatchPlan,
        review_kind: &str,
        proposal_ref: Option<String>,
    ) -> Result<PatchReviewItem> {
        let mut decision = PatchPolicyEngine::evaluate(patch);
        let must_manual_review =
            review_kind.eq_ignore_ascii_case("heal_proposal")
                || review_kind.eq_ignore_ascii_case("relation_repair");
        if must_manual_review {
            decision.approval_required = true;
            decision.reason = format!("{review_kind} requires manual approval");
        }
        let now = current_time_ms();
        let review_id = format!("patch-review:{}:{}:{}", session_id, trace_id, now);
        let item = PatchReviewItem {
            review_id: review_id.clone(),
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            status: if must_manual_review || decision.approval_required {
                PatchReviewStatus::Queued
            } else {
                PatchReviewStatus::Approved
            },
            decision,
            patch: patch.clone(),
            reviewer: if must_manual_review || PatchPolicyEngine::evaluate(patch).approval_required {
                None
            } else {
                Some("system:auto-approve".to_string())
            },
            created_at_ms: now,
            updated_at_ms: now,
            review_kind: review_kind.to_string(),
            proposal_ref,
        };
        db.upsert_json_knowledge(
            format!("memory:patch:review:{}:{}", session_id, now),
            &item,
            "patch-review-queue",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("memory:patch:review:{}:latest", session_id),
            &item,
            "patch-review-queue",
        )
        .await?;
        Ok(item)
    }

    pub async fn list(db: &StateStore, session_id: &str) -> Result<Vec<PatchReviewItem>> {
        let records = db
            .list_knowledge_by_prefix(&format!("memory:patch:review:{session_id}:"))
            .await?;
        let mut by_review_id = std::collections::BTreeMap::<String, PatchReviewItem>::new();
        for record in records {
            if let Ok(item) = serde_json::from_str::<PatchReviewItem>(&record.value) {
                match by_review_id.get(&item.review_id) {
                    Some(existing) if existing.updated_at_ms >= item.updated_at_ms => {}
                    _ => {
                        by_review_id.insert(item.review_id.clone(), item);
                    }
                }
            }
        }
        Ok(by_review_id.into_values().collect())
    }

    pub async fn approve(
        db: &StateStore,
        session_id: &str,
        review_id: &str,
        reviewer: &str,
        reason: &str,
    ) -> Result<PatchReviewItem> {
        Self::decide(
            db,
            session_id,
            review_id,
            PatchReviewStatus::Approved,
            reviewer,
            reason,
        )
        .await
    }

    pub async fn reject(
        db: &StateStore,
        session_id: &str,
        review_id: &str,
        reviewer: &str,
        reason: &str,
    ) -> Result<PatchReviewItem> {
        Self::decide(
            db,
            session_id,
            review_id,
            PatchReviewStatus::Rejected,
            reviewer,
            reason,
        )
        .await
    }

    async fn decide(
        db: &StateStore,
        session_id: &str,
        review_id: &str,
        status: PatchReviewStatus,
        reviewer: &str,
        reason: &str,
    ) -> Result<PatchReviewItem> {
        let mut item = Self::list(db, session_id)
            .await?
            .into_iter()
            .find(|row| row.review_id == review_id)
            .ok_or_else(|| anyhow::anyhow!("review item not found: {}", review_id))?;
        let now = current_time_ms();
        item.status = status;
        item.reviewer = Some(reviewer.to_string());
        item.updated_at_ms = now;
        item.decision.reason = reason.to_string();

        db.upsert_json_knowledge(
            format!("memory:patch:review:{}:{}", session_id, now),
            &item,
            "patch-review-queue",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("memory:patch:review:{}:latest", session_id),
            &item,
            "patch-review-queue",
        )
        .await?;
        Ok(item)
    }
}

fn default_review_kind() -> String {
    "patch".to_string()
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
    use crate::plugins::gitmemory_core::patch_core::{PatchOp, PatchOpKind, PatchPlan};
    use autoloop_state_adapter::{StateStoreBackend, StateStore, StateStoreConfig};

    fn db() -> StateStore {
        StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        })
    }

    #[tokio::test]
    async fn list_and_decide_roundtrip() {
        let db = db();
        let patch = PatchPlan {
            namespace: "tenant:test".to_string(),
            ops: vec![PatchOp {
                kind: PatchOpKind::Delete,
                target: "canonical/a.md".to_string(),
                reason: "removed".to_string(),
            }],
        };

        let queued = PatchReviewQueue::enqueue(&db, "session:test", "trace:test", &patch)
            .await
            .expect("enqueue");
        assert_eq!(queued.status, PatchReviewStatus::Queued);

        let listed = PatchReviewQueue::list(&db, "session:test")
            .await
            .expect("list");
        assert!(!listed.is_empty());

        let approved = PatchReviewQueue::approve(
            &db,
            "session:test",
            &queued.review_id,
            "operator:test",
            "approved in test",
        )
        .await
        .expect("approve");
        assert_eq!(approved.status, PatchReviewStatus::Approved);
        assert_eq!(approved.reviewer.as_deref(), Some("operator:test"));
    }

    #[tokio::test]
    async fn heal_proposal_requires_manual_approval_even_if_low_risk() {
        let db = db();
        let patch = PatchPlan {
            namespace: "tenant:test".to_string(),
            ops: vec![PatchOp {
                kind: PatchOpKind::Add,
                target: "canonical/a.md".to_string(),
                reason: "low risk add".to_string(),
            }],
        };

        let queued = PatchReviewQueue::enqueue_heal_proposal(
            &db,
            "session:test-heal",
            "trace:test-heal",
            &patch,
            "heal-proposal:test",
        )
        .await
        .expect("enqueue heal");

        assert_eq!(queued.status, PatchReviewStatus::Queued);
        assert!(queued.decision.approval_required);
        assert_eq!(queued.reviewer, None);
        assert_eq!(queued.review_kind, "heal_proposal");
    }
}

