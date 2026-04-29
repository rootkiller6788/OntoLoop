use super::ids::{SessionId, TaskId, TraceId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum DomainEventKind {
    IntentReceived,
    ConstraintFrozen,
    PlanSelected,
    RouteSelected,
    PolicyRejected,
    ExecutionStarted,
    ExecutionFailed,
    ExecutionRetried,
    VerifierPassed,
    VerifierRejected,
    LearningUpdated,
    GraphUpdated,
    WorkOrderAccepted,
    WorkOrderDelivered,
    RevenueRecognized,
    MarginReported,
    SlaReported,
    SnapshotEmitted,
    AlertRaised,
}

impl DomainEventKind {
    pub fn as_key(&self) -> &'static str {
        match self {
            Self::IntentReceived => "intent.received",
            Self::ConstraintFrozen => "constraint.frozen",
            Self::PlanSelected => "plan.selected",
            Self::RouteSelected => "route.selected",
            Self::PolicyRejected => "policy.rejected",
            Self::ExecutionStarted => "execution.started",
            Self::ExecutionFailed => "execution.failed",
            Self::ExecutionRetried => "execution.retried",
            Self::VerifierPassed => "verifier.passed",
            Self::VerifierRejected => "verifier.rejected",
            Self::LearningUpdated => "learning.updated",
            Self::GraphUpdated => "graph.updated",
            Self::WorkOrderAccepted => "workorder.accepted",
            Self::WorkOrderDelivered => "workorder.delivered",
            Self::RevenueRecognized => "revenue.recognized",
            Self::MarginReported => "margin.reported",
            Self::SlaReported => "sla.reported",
            Self::SnapshotEmitted => "snapshot.emitted",
            Self::AlertRaised => "alert.raised",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DomainEvent {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub task_id: Option<TaskId>,
    pub kind: DomainEventKind,
    pub timestamp_ms: u64,
    pub payload: serde_json::Value,
}
