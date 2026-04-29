use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::orchestration::current_time_ms;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SharedKnowledgeUpdate {
    pub session_id: String,
    pub source: String,
    pub summary: String,
    pub knowledge_refs: Vec<String>,
    pub policy_tags: Vec<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OrgKnowledgeSnapshot {
    pub tenant_id: String,
    pub updates: Vec<SharedKnowledgeUpdate>,
    pub updated_at_ms: u64,
}

pub struct OrgKnowledgePublisher;

impl OrgKnowledgePublisher {
    pub async fn publish(
        db: &StateStore,
        tenant_id: &str,
        update: &SharedKnowledgeUpdate,
    ) -> Result<String> {
        let key = format!(
            "org-knowledge:{tenant_id}:{}:{}",
            update.session_id, update.created_at_ms
        );
        db.upsert_json_knowledge(key.clone(), update, "org-knowledge")
            .await?;
        Ok(key)
    }

    pub async fn snapshot(db: &StateStore, tenant_id: &str) -> Result<OrgKnowledgeSnapshot> {
        let updates = db
            .list_knowledge_by_prefix(&format!("org-knowledge:{tenant_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<SharedKnowledgeUpdate>(&record.value).ok())
            .collect::<Vec<_>>();
        Ok(OrgKnowledgeSnapshot {
            tenant_id: tenant_id.to_string(),
            updates,
            updated_at_ms: current_time_ms(),
        })
    }
}

#[derive(Clone)]
pub struct SharedKnowledgePortAdapter {
    db: StateStore,
    tenant_id: String,
}

impl SharedKnowledgePortAdapter {
    pub fn new(db: StateStore, tenant_id: impl Into<String>) -> Self {
        Self {
            db,
            tenant_id: tenant_id.into(),
        }
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::SharedKnowledgePublisherPort for SharedKnowledgePortAdapter {
    async fn publish_shared_knowledge(
        &self,
        session_id: &crate::contracts::ids::SessionId,
        delta: &crate::contracts::types::LearningDelta,
    ) -> Result<(), crate::contracts::errors::ContractError> {
        let update = SharedKnowledgeUpdate {
            session_id: session_id.to_string(),
            source: "shared-knowledge-port".into(),
            summary: delta
                .notes
                .first()
                .cloned()
                .unwrap_or_else(|| "learning delta published".into()),
            knowledge_refs: delta
                .added_skills
                .iter()
                .map(|name| format!("skill:{}", name))
                .collect(),
            policy_tags: vec!["governed_learning".into()],
            created_at_ms: current_time_ms(),
        };
        OrgKnowledgePublisher::publish(&self.db, &self.tenant_id, &update)
            .await
            .map_err(|e| crate::contracts::errors::ContractError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::StrategyUpdaterPort for SharedKnowledgePortAdapter {
    async fn update_strategy(
        &self,
        session_id: &crate::contracts::ids::SessionId,
        verdict: &crate::contracts::types::VerificationVerdict,
        delta: &crate::contracts::types::LearningDelta,
    ) -> Result<(), crate::contracts::errors::ContractError> {
        let key = format!("strategy:{}:contract-port", session_id);
        self.db
            .upsert_json_knowledge(
                key,
                &serde_json::json!({
                    "session_id": session_id,
                    "tenant_id": self.tenant_id,
                    "verdict": format!("{:?}", verdict.verdict).to_ascii_lowercase(),
                    "score": verdict.score,
                    "added_skills": delta.added_skills,
                    "notes": delta.notes,
                    "updated_at_ms": current_time_ms(),
                }),
                "strategy-updater-port",
            )
            .await
            .map_err(|e| crate::contracts::errors::ContractError::Storage(e.to_string()))?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn org_knowledge_roundtrip() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let update = SharedKnowledgeUpdate {
            session_id: "session-1".into(),
            source: "learning".into(),
            summary: "promoted skill x".into(),
            knowledge_refs: vec!["memory:session-1:consolidation".into()],
            policy_tags: vec!["safe".into()],
            created_at_ms: current_time_ms(),
        };
        OrgKnowledgePublisher::publish(&db, "tenant-1", &update)
            .await
            .expect("publish");
        let snap = OrgKnowledgePublisher::snapshot(&db, "tenant-1")
            .await
            .expect("snapshot");
        assert_eq!(snap.updates.len(), 1);
        assert!(snap.updates[0].summary.contains("promoted"));
    }
}

