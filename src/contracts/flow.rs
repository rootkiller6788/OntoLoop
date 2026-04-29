use std::collections::BTreeMap;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    Safe,
    Normal,
    Degraded,
    Replay,
    Shadow,
    Mirror,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RuntimeModeDecision {
    pub mode: RuntimeMode,
    pub enforce_gate: bool,
    pub stage: String,
    pub reason: String,
    pub dispatched_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlowNodeState {
    Pending,
    Ready,
    Running,
    Succeeded,
    Failed,
    Blocked,
    Skipped,
    Degraded,
    Replayed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct FlowStateFlags {
    pub side_effect_state: String,
    pub budget_state: String,
    pub trigger_state: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FlowNodeRecord {
    pub node_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub state: FlowNodeState,
    pub depends_on: Vec<String>,
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    pub flags: FlowStateFlags,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct FlowMapSnapshot {
    pub flow_id: String,
    pub session_id: String,
    pub plan_id: String,
    pub run_id: String,
    pub nodes: Vec<FlowNodeRecord>,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FlowStatePatch {
    pub flow_id: String,
    pub node_id: String,
    pub from: Option<FlowNodeState>,
    pub to: FlowNodeState,
    pub reason: String,
    pub trace_id: String,
    pub updated_at_ms: u64,
    pub metadata: BTreeMap<String, String>,
}
