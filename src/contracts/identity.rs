#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct AgentWorkspaceSnapshot {
    pub session_id: String,
    pub agent_id: String,
    pub role: String,
    pub soul_profile: String,
    pub long_term_memory_refs: Vec<String>,
    pub private_workspace_root: String,
    pub peers: Vec<String>,
    pub workspace_artifacts: Vec<String>,
}
