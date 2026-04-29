#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvidenceStepRecord {
    pub trace_id: String,
    pub step_name: String,
    pub prev_hash: String,
    pub record_hash: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetLedgerOp {
    Reserve,
    Consume,
    Refund,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BudgetLedgerRecord {
    pub trace_id: String,
    pub session_id: String,
    pub task_id: String,
    pub op: BudgetLedgerOp,
    pub amount_micros: u64,
    pub reason: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApprovalRecord {
    pub session_id: String,
    pub task_id: String,
    pub approved: bool,
    pub approver: String,
    pub reason: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplayFingerprint {
    pub trace_id: String,
    pub boundary: String,
    pub input_hash: String,
    pub output_hash: String,
    pub matched: bool,
    pub mismatch_explanation: Option<String>,
}
