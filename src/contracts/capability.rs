#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilityIntent {
    pub session_id: String,
    pub objective: String,
    pub required_tags: Vec<String>,
    pub preferred_servers: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilityCandidate {
    pub capability_id: String,
    pub server: Option<String>,
    pub tool: String,
    pub score: f32,
    pub active: bool,
    pub verified: bool,
    pub trusted: bool,
    pub approval_required: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CapabilityAdmissionDecision {
    pub allowed: bool,
    pub reason: String,
    pub candidate: Option<CapabilityCandidate>,
    pub quota_remaining_micros: Option<u64>,
    pub evidence_ref: Option<String>,
}
