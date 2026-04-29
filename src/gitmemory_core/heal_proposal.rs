use anyhow::Result;
use autoloop_state_adapter::StateStore;

use super::patch_core::{PatchOp, PatchOpKind, PatchPlan};
use super::patch_review_queue::PatchReviewQueue;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealProposalRequest {
    pub namespace: String,
    pub target: String,
    pub reason: String,
    #[serde(default = "default_heal_op_kind")]
    pub op_kind: PatchOpKind,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealProposalRecord {
    pub proposal_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub request: HealProposalRequest,
    pub patch: PatchPlan,
    pub review_id: String,
    pub review_status: String,
    pub created_at_ms: u64,
}

pub struct HealProposalService;

impl HealProposalService {
    pub async fn propose(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        request: HealProposalRequest,
    ) -> Result<HealProposalRecord> {
        let now = current_time_ms();
        let proposal_id = format!("heal-proposal:{session_id}:{trace_id}:{now}");
        let patch = PatchPlan {
            namespace: request.namespace.clone(),
            ops: vec![PatchOp {
                kind: request.op_kind.clone(),
                target: request.target.clone(),
                reason: request.reason.clone(),
            }],
        };
        let review =
            PatchReviewQueue::enqueue_heal_proposal(db, session_id, trace_id, &patch, &proposal_id)
                .await?;
        let record = HealProposalRecord {
            proposal_id: proposal_id.clone(),
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            request,
            patch,
            review_id: review.review_id,
            review_status: format!("{:?}", review.status).to_ascii_lowercase(),
            created_at_ms: now,
        };
        db.upsert_json_knowledge(
            format!("memory:heal:proposal:{session_id}:{now}"),
            &record,
            "heal-proposal",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("memory:heal:proposal:{session_id}:latest"),
            &record,
            "heal-proposal",
        )
        .await?;
        Ok(record)
    }
}

fn default_heal_op_kind() -> PatchOpKind {
    PatchOpKind::Update
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

