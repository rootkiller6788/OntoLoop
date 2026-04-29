use std::collections::BTreeMap;

use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::{
    contracts::context::{ContextItem, ContextItemKind, KnowledgeContext, ProjectMemoryPolicy},
    session::ContextCacheOrchestrator,
};

#[derive(Clone)]
pub struct KnowledgeContextResolver {
    state_store: StateStore,
}

impl KnowledgeContextResolver {
    pub fn new(state_store: StateStore) -> Self {
        Self { state_store }
    }

    pub async fn resolve(&self, session_id: &str) -> Result<KnowledgeContext> {
        let tenant_id = self
            .state_store
            .get_session_lease(session_id)
            .await?
            .map(|lease| lease.tenant_id)
            .unwrap_or_else(|| "tenant-default".to_string());

        let project_policy_ref = format!("project:{tenant_id}:memory-policy");
        let (project_policy, has_project_policy) = self
            .resolve_project_memory_policy(&project_policy_ref)
            .await?;

        let kb_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("kb:{tenant_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let plaza_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("plaza:{tenant_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let replay_scope_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("eventlog:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let playbook_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("playbook:{tenant_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let org_memory_slice_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("org-knowledge:{tenant_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let private_memory_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:supermemory:atomic:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let supermemory_scope_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:supermemory:scope-index:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let supermemory_decision_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:supermemory:decisions:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();
        let supermemory_history_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:supermemory:history:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();

        let mut source_evidence_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:supermemory:chunks:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();
        source_evidence_refs.extend(
            self.state_store
                .list_knowledge_by_prefix(&format!("memory:supermemory:documents:{session_id}:"))
                .await?
                .into_iter()
                .map(|record| record.key)
                .take(24),
        );

        let mut context_bundle_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:supermemory:context:{session_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(24)
            .collect::<Vec<_>>();
        context_bundle_refs.extend(
            self.state_store
                .list_knowledge_by_prefix(&format!("memory:supermemory:profile:{session_id}:"))
                .await?
                .into_iter()
                .map(|record| record.key)
                .take(24),
        );

        source_evidence_refs.sort();
        source_evidence_refs.dedup();
        if has_project_policy {
            context_bundle_refs.push(project_policy_ref.clone());
        }
        context_bundle_refs.sort();
        context_bundle_refs.dedup();

        let tenant_knowledge_scope = format!("tenant:{tenant_id}:knowledge:read");
        let tenant_supermemory_scope = format!("tenant:{tenant_id}:supermemory:read");
        let mut context_items = Vec::new();
        context_items.extend(build_context_items_from_refs(
            session_id,
            &kb_refs,
            ContextItemKind::Knowledge,
            &tenant_knowledge_scope,
            0.82,
            6_000,
            "kb",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &plaza_refs,
            ContextItemKind::Knowledge,
            &tenant_knowledge_scope,
            0.78,
            5_500,
            "plaza",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &playbook_refs,
            ContextItemKind::Knowledge,
            &tenant_knowledge_scope,
            0.76,
            5_500,
            "playbook",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &org_memory_slice_refs,
            ContextItemKind::Knowledge,
            &tenant_knowledge_scope,
            0.74,
            5_000,
            "org_memory",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &private_memory_refs,
            ContextItemKind::Supermemory,
            &tenant_supermemory_scope,
            0.88,
            7_000,
            "supermemory_atomic",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &source_evidence_refs,
            ContextItemKind::Supermemory,
            &tenant_supermemory_scope,
            0.86,
            7_500,
            "supermemory_source",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &context_bundle_refs,
            ContextItemKind::Supermemory,
            &tenant_supermemory_scope,
            0.84,
            8_000,
            "supermemory_bundle",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &supermemory_scope_refs,
            ContextItemKind::Supermemory,
            &tenant_supermemory_scope,
            0.83,
            6_500,
            "supermemory_scope",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &supermemory_decision_refs,
            ContextItemKind::Supermemory,
            &tenant_supermemory_scope,
            0.82,
            6_500,
            "supermemory_decision",
        ));
        context_items.extend(build_context_items_from_refs(
            session_id,
            &supermemory_history_refs,
            ContextItemKind::Supermemory,
            &tenant_supermemory_scope,
            0.8,
            6_000,
            "supermemory_history",
        ));

        let context_cache_bundle = ContextCacheOrchestrator::default()
            .load(session_id)
            .ok()
            .flatten();
        if let Some(bundle) = context_cache_bundle.as_ref() {
            context_items.extend(
                bundle
                    .context_items
                    .iter()
                    .filter(|item| item.kind == "session" || item.kind == "tool_state")
                    .cloned(),
            );
        }

        context_items.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.source_ref.cmp(&right.source_ref))
                .then_with(|| left.item_id.cmp(&right.item_id))
        });
        context_items.dedup_by(|left, right| {
            left.kind == right.kind
                && left.source_ref == right.source_ref
                && left.permission_scope == right.permission_scope
        });

        let context_item_knowledge_count = context_items
            .iter()
            .filter(|item| item.kind == "knowledge")
            .count();
        let context_item_supermemory_count = context_items
            .iter()
            .filter(|item| item.kind == "supermemory")
            .count();
        let context_item_session_count = context_items
            .iter()
            .filter(|item| item.kind == "session")
            .count();
        let context_item_tool_state_count = context_items
            .iter()
            .filter(|item| item.kind == "tool_state")
            .count();
        let context_item_budget_micros_total = context_items
            .iter()
            .map(|item| item.budget_micros)
            .sum::<u64>();

        let mut metadata = BTreeMap::new();
        metadata.insert(
            "context_source".to_string(),
            "knowledge-context-resolver".to_string(),
        );
        metadata.insert("tenant_scope".to_string(), tenant_id);
        metadata.insert("replay_binding".to_string(), session_id.to_string());
        metadata.insert(
            "memory_policy_retrieval_criteria".to_string(),
            project_policy.retrieval_criteria.join(","),
        );
        metadata.insert(
            "memory_policy_multilingual".to_string(),
            project_policy.multilingual.to_string(),
        );
        metadata.insert(
            "memory_policy_enable_graph".to_string(),
            project_policy.enable_graph.to_string(),
        );
        metadata.insert("memory_policy_ref".to_string(), project_policy_ref.clone());
        metadata.insert(
            "supermemory_private_ref_count".to_string(),
            private_memory_refs.len().to_string(),
        );
        metadata.insert(
            "supermemory_source_ref_count".to_string(),
            source_evidence_refs.len().to_string(),
        );
        metadata.insert(
            "supermemory_context_ref_count".to_string(),
            context_bundle_refs.len().to_string(),
        );
        metadata.insert(
            "supermemory_scope_ref_count".to_string(),
            supermemory_scope_refs.len().to_string(),
        );
        metadata.insert(
            "supermemory_decision_ref_count".to_string(),
            supermemory_decision_refs.len().to_string(),
        );
        metadata.insert(
            "supermemory_history_ref_count".to_string(),
            supermemory_history_refs.len().to_string(),
        );
        metadata.insert(
            "context_item_total_count".to_string(),
            context_items.len().to_string(),
        );
        metadata.insert(
            "context_item_knowledge_count".to_string(),
            context_item_knowledge_count.to_string(),
        );
        metadata.insert(
            "context_item_supermemory_count".to_string(),
            context_item_supermemory_count.to_string(),
        );
        metadata.insert(
            "context_item_session_count".to_string(),
            context_item_session_count.to_string(),
        );
        metadata.insert(
            "context_item_tool_state_count".to_string(),
            context_item_tool_state_count.to_string(),
        );
        metadata.insert(
            "context_item_budget_micros_total".to_string(),
            context_item_budget_micros_total.to_string(),
        );
        if let Some(bundle) = context_cache_bundle.as_ref() {
            metadata.insert(
                "context_cache_summary_ref".to_string(),
                bundle.summary_cache.summary_id.clone(),
            );
            metadata.insert(
                "context_cache_retrieval_index_ref".to_string(),
                bundle.retrieval_index.index_id.clone(),
            );
            metadata.insert(
                "context_cache_state_snapshot_ref".to_string(),
                bundle.state_snapshot.snapshot_id.clone(),
            );
            metadata.insert(
                "context_cache_compaction_digest".to_string(),
                bundle.state_snapshot.compaction_digest.clone(),
            );
            metadata.insert(
                "context_cache_message_count".to_string(),
                bundle.state_snapshot.message_count.to_string(),
            );
            metadata.insert(
                "context_cache_context_item_count".to_string(),
                bundle.state_snapshot.context_item_count.to_string(),
            );
        }
        let supermemory_context_latest_ref =
            format!("memory:supermemory:context:{session_id}:latest");
        let supermemory_profile_latest_ref =
            format!("memory:supermemory:profile:{session_id}:latest");
        let supermemory_metrics_ref = format!("observability:{session_id}:supermemory-metrics");
        let supermemory_trace_ref = format!("conversation:{session_id}:supermemory-context");

        metadata.insert(
            "supermemory_context_latest_ref".to_string(),
            supermemory_context_latest_ref.clone(),
        );
        metadata.insert(
            "supermemory_profile_latest_ref".to_string(),
            supermemory_profile_latest_ref.clone(),
        );
        metadata.insert(
            "supermemory_metrics_ref".to_string(),
            supermemory_metrics_ref.clone(),
        );
        metadata.insert("supermemory_trace_ref".to_string(), supermemory_trace_ref);
        metadata.insert(
            "supermemory_scope_prefix".to_string(),
            format!("memory:supermemory:scope-index:{session_id}:"),
        );
        metadata.insert(
            "supermemory_decision_prefix".to_string(),
            format!("memory:supermemory:decisions:{session_id}:"),
        );
        metadata.insert(
            "supermemory_history_prefix".to_string(),
            format!("memory:supermemory:history:{session_id}:"),
        );

        if let Some(context_record) = self
            .state_store
            .get_knowledge(&supermemory_context_latest_ref)
            .await?
        {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&context_record.value) {
                let retrieval_hits = value
                    .get("hits")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or(0);
                metadata.insert(
                    "supermemory_context_retrieval_hits".to_string(),
                    retrieval_hits.to_string(),
                );
                if let Some(assembled_at_ms) = value
                    .get("assembled_at_ms")
                    .and_then(serde_json::Value::as_u64)
                {
                    metadata.insert(
                        "supermemory_context_assembled_at_ms".to_string(),
                        assembled_at_ms.to_string(),
                    );
                }
            }
        }

        if let Some(metrics_record) = self
            .state_store
            .get_knowledge(&supermemory_metrics_ref)
            .await?
        {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&metrics_record.value) {
                if let Some(retrieval_hits) = value
                    .get("retrieval_hits")
                    .and_then(serde_json::Value::as_u64)
                {
                    metadata.insert(
                        "supermemory_metrics_retrieval_hits".to_string(),
                        retrieval_hits.to_string(),
                    );
                }
            }
        }

        Ok(KnowledgeContext {
            session_id: session_id.to_string(),
            kb_refs,
            plaza_refs,
            replay_scope_refs,
            playbook_refs,
            org_memory_slice_refs,
            private_memory_refs,
            source_evidence_refs,
            context_bundle_refs,
            context_items,
            project_policy,
            metadata,
        })
    }

    async fn resolve_project_memory_policy(
        &self,
        policy_ref: &str,
    ) -> Result<(ProjectMemoryPolicy, bool)> {
        let Some(record) = self.state_store.get_knowledge(policy_ref).await? else {
            return Ok((ProjectMemoryPolicy::default(), false));
        };

        if let Ok(policy) = serde_json::from_str::<ProjectMemoryPolicy>(&record.value) {
            return Ok((normalize_project_policy(policy), true));
        }

        let value = serde_json::from_str::<serde_json::Value>(&record.value)?;
        let retrieval_criteria = value
            .get("retrieval_criteria")
            .and_then(|raw| raw.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let multilingual = value
            .get("multilingual")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        let enable_graph = value
            .get("enable_graph")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        Ok((
            normalize_project_policy(ProjectMemoryPolicy {
                retrieval_criteria,
                multilingual,
                enable_graph,
            }),
            true,
        ))
    }
}

fn build_context_items_from_refs(
    session_id: &str,
    refs: &[String],
    kind: ContextItemKind,
    permission_scope: &str,
    base_priority: f32,
    base_budget_micros: u64,
    source_bucket: &str,
) -> Vec<ContextItem> {
    refs.iter()
        .take(48)
        .enumerate()
        .map(|(index, source_ref)| {
            let mut metadata = BTreeMap::new();
            metadata.insert("source_bucket".to_string(), source_bucket.to_string());
            metadata.insert("source_index".to_string(), index.to_string());
            metadata.insert("source_ref".to_string(), source_ref.clone());
            metadata.insert(
                "source_ref_hash".to_string(),
                format!("{:016x}", stable_hash(source_ref)),
            );
            metadata.insert(
                "context_protocol".to_string(),
                "context-item-v1".to_string(),
            );

            ContextItem::new(
                session_id.to_string(),
                format!(
                    "ctx:{}:{}:{}",
                    source_bucket,
                    index,
                    stable_hash(source_ref)
                ),
                kind.clone(),
                source_ref.clone(),
                permission_scope.to_string(),
                (base_priority - ((index as f32) * 0.003)).max(0.35),
                base_budget_micros.saturating_add((source_ref.len() as u64).saturating_mul(8)),
                metadata,
            )
        })
        .collect::<Vec<_>>()
}

fn stable_hash(value: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn normalize_project_policy(policy: ProjectMemoryPolicy) -> ProjectMemoryPolicy {
    let mut seen = std::collections::BTreeSet::new();
    let mut retrieval_criteria = policy
        .retrieval_criteria
        .into_iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .filter(|item| seen.insert(item.clone()))
        .collect::<Vec<_>>();
    if retrieval_criteria.is_empty() {
        retrieval_criteria = ProjectMemoryPolicy::default().retrieval_criteria;
    }

    ProjectMemoryPolicy {
        retrieval_criteria,
        multilingual: policy.multilingual,
        enable_graph: policy.enable_graph,
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::KnowledgeContextInjector for KnowledgeContextResolver {
    async fn inject_knowledge_context(
        &self,
        session_id: &crate::contracts::ids::SessionId,
    ) -> Result<KnowledgeContext, crate::contracts::errors::ContractError> {
        self.resolve(session_id.as_ref())
            .await
            .map_err(|error| crate::contracts::errors::ContractError::Storage(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn knowledge_context_injects_project_memory_policy_surface() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        db.upsert_json_knowledge(
            "project:tenant-default:memory-policy".to_string(),
            &ProjectMemoryPolicy {
                retrieval_criteria: vec!["hybrid".into(), "recency".into(), "hybrid".into()],
                multilingual: false,
                enable_graph: false,
            },
            "test",
        )
        .await
        .expect("seed project policy");

        let resolver = KnowledgeContextResolver::new(db);
        let context = resolver.resolve("session-policy").await.expect("resolve");

        assert_eq!(
            context.project_policy.retrieval_criteria,
            vec!["hybrid".to_string(), "recency".to_string()]
        );
        assert!(!context.project_policy.multilingual);
        assert!(!context.project_policy.enable_graph);
        assert_eq!(
            context
                .metadata
                .get("memory_policy_retrieval_criteria")
                .map(String::as_str),
            Some("hybrid,recency")
        );
        assert!(
            context
                .context_bundle_refs
                .iter()
                .any(|item| item == "project:tenant-default:memory-policy")
        );
    }
}

