use std::collections::BTreeMap;

use super::ids::{CapabilityId, SessionId, TaskId, TraceId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Intent {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub message: String,
    pub anchor: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConstraintSet {
    pub max_cpu_percent: u8,
    pub max_memory_mb: u32,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub max_tokens: u32,
    pub io_allow_paths: Vec<String>,
    pub io_deny_paths: Vec<String>,
    pub sandbox_profile: String,
    pub requires_human_approval: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PolicyDecision {
    Approved { reason: String },
    Rejected { reason: String },
    NeedsRevision { reason: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionStep {
    pub task_id: TaskId,
    pub capability_id: CapabilityId,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionPlan {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub objective: String,
    pub steps: Vec<ExecutionStep>,
    pub constraints: ConstraintSet,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskEnvelope {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub task_id: TaskId,
    pub capability_id: CapabilityId,
    pub identity: ExecutionIdentity,
    pub payload: serde_json::Value,
    pub constraints: ConstraintSet,
    pub trust_plan: Option<TrustExecutionPlan>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExecutionIdentity {
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub lease_token: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustExecutionPlan {
    pub trust_level: String,
    pub verify_identity: bool,
    pub verify_environment: bool,
    pub rollout_gate: String,
    pub attestation_backend: String,
    pub attestation_required: bool,
    #[serde(default)]
    pub attestation_policy_version: Option<String>,
    pub policy_refs: Vec<String>,
    pub budget_scope: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttestationPlatform {
    Sgx,
    SevSnp,
    Tpm,
    Generic,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuoteBundle {
    pub platform: AttestationPlatform,
    pub quote: String,
    pub cert_chain: Option<String>,
    pub endorsements: Vec<String>,
    pub tcb_version: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub tenant_binding: Option<String>,
    pub nonce: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AttestationEvidence {
    pub evidence_id: String,
    pub backend: String,
    pub quote_bundle: Option<QuoteBundle>,
    pub remote_report: Option<serde_json::Value>,
    pub digest: Option<String>,
    pub source_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerifierVerdict {
    pub verified: bool,
    pub reason: String,
    pub policy_version: String,
    pub evidence_id: Option<String>,
    pub verifier_name: String,
    pub min_tcb_passed: bool,
    pub freshness_passed: bool,
    pub tenant_binding_passed: bool,
    pub nonce_present: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AttestationPolicy {
    pub version: String,
    pub strict: bool,
    pub min_tcb_version: String,
    pub evidence_ttl_ms: u64,
    pub require_tenant_binding: bool,
    pub require_nonce: bool,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunReceipt {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub task_id: TaskId,
    pub success: bool,
    pub latency_ms: u64,
    pub output: serde_json::Value,
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Verdict {
    Pass,
    Iterate,
    Reject,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VerificationVerdict {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub verdict: Verdict,
    pub score: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LearningDelta {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub added_skills: Vec<String>,
    pub updated_edges: usize,
    pub episode_count: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReportArtifact {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub report_type: String,
    pub summary: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTier {
    Basic,
    Standard,
    Premium,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkOrderStatus {
    Accepted,
    Delivered,
    Rejected,
    SlaBreached,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkOrder {
    pub work_order_id: String,
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub task_id: TaskId,
    pub capability_id: CapabilityId,
    pub service_tier: ServiceTier,
    pub status: WorkOrderStatus,
    pub accepted_at_ms: u64,
    pub delivered_at_ms: Option<u64>,
    pub acceptance_note: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RevenueEvent {
    pub revenue_event_id: String,
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub task_id: TaskId,
    pub work_order_id: String,
    pub capability_id: CapabilityId,
    pub service_tier: ServiceTier,
    pub revenue_micros: u64,
    pub cost_micros: u64,
    pub profit_micros: i64,
    pub recognized_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarginReport {
    pub session_id: SessionId,
    pub recognized_revenue_micros: u64,
    pub allocated_cost_micros: u64,
    pub gross_profit_micros: i64,
    pub gross_margin_ratio: f32,
    pub negative_margin_tasks: Vec<TaskId>,
    pub summary: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SLAReport {
    pub session_id: SessionId,
    pub delivered_orders: usize,
    pub breached_orders: usize,
    pub sla_success_ratio: f32,
    pub breach_tasks: Vec<TaskId>,
    pub summary: String,
}
