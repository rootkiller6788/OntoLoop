use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::contracts::identity::AgentWorkspaceSnapshot;

#[derive(Clone)]
pub struct WorkspaceLoader {
    state_store: StateStore,
}

impl WorkspaceLoader {
    pub fn new(state_store: StateStore) -> Self {
        Self { state_store }
    }

    pub async fn load(&self, session_id: &str) -> Result<AgentWorkspaceSnapshot> {
        let soul_profile = self
            .state_store
            .get_knowledge(&format!("memory:{session_id}:soul"))
            .await?
            .map(|record| record.value)
            .unwrap_or_else(|| "adaptive-agent".to_string());

        let long_term_memory_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let workspace_artifacts = self
            .state_store
            .list_knowledge_by_prefix(&format!("workspace:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let peers = self
            .state_store
            .list_knowledge_by_prefix(&format!("team:{session_id}:peer:"))
            .await?
            .into_iter()
            .map(|record| {
                record
                    .key
                    .trim_start_matches(&format!("team:{session_id}:peer:"))
                    .to_string()
            })
            .collect::<Vec<_>>();

        Ok(AgentWorkspaceSnapshot {
            session_id: session_id.to_string(),
            agent_id: format!("agent:{session_id}"),
            role: "execution-agent".to_string(),
            soul_profile,
            long_term_memory_refs,
            private_workspace_root: format!("workspace/{session_id}"),
            peers,
            workspace_artifacts,
        })
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::AgentWorkspaceLoader for WorkspaceLoader {
    async fn load_workspace(
        &self,
        session_id: &crate::contracts::ids::SessionId,
    ) -> Result<AgentWorkspaceSnapshot, crate::contracts::errors::ContractError> {
        self.load(session_id.as_ref())
            .await
            .map_err(|e| crate::contracts::errors::ContractError::Storage(e.to_string()))
    }
}

