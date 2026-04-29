use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::contracts::context::ProjectMemoryPolicy;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LexicalFallbackPlan {
    pub enabled: bool,
    pub strategy: String,
    pub hits: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecallPluginRoute {
    pub pluginized: bool,
    pub strategy: String,
    pub selected_sources: Vec<String>,
    pub plugin_ids: Vec<String>,
    #[serde(default)]
    pub graph_enabled: bool,
    #[serde(default = "default_neighbor_threshold")]
    pub neighbor_threshold: f32,
    #[serde(default = "default_max_neighbors")]
    pub max_neighbors: usize,
    #[serde(default)]
    pub lexical_fallback: Option<LexicalFallbackPlan>,
}

pub struct RecallPluginRouter;

impl RecallPluginRouter {
    pub async fn route(
        db: &StateStore,
        session_id: &str,
        tenant_id: &str,
        query: &str,
    ) -> Result<RecallPluginRoute> {
        let policy = load_project_memory_policy(db, tenant_id).await?;
        let graph_enabled = policy.enable_graph;
        let lifecycle = db.get_knowledge("plugin:lifecycle:index").await?;
        let mut plugin_ids = Vec::<String>::new();
        if let Some(record) = lifecycle {
            if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(&record.value) {
                for item in items {
                    let plugin_id = item
                        .get("plugin_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let state = item
                        .get("state")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if state.eq_ignore_ascii_case("enabled")
                        && (plugin_id.contains("search")
                            || plugin_id.contains("graph")
                            || plugin_id.contains("vector")
                            || plugin_id.contains("supermemory")
                            || plugin_id.contains("source"))
                    {
                        plugin_ids.push(plugin_id.to_string());
                    }
                }
            }
        }
        plugin_ids.sort();
        plugin_ids.dedup();

        let pluginized = !plugin_ids.is_empty();
        let mut selected_sources = if pluginized {
            plugin_ids
                .iter()
                .map(|id| format!("plugin:{id}"))
                .collect::<Vec<_>>()
        } else {
            vec![
                "memory:atomic".to_string(),
                "memory:chunks".to_string(),
                "memory:relations".to_string(),
                "federated:supermemory".to_string(),
            ]
        };
        if !graph_enabled {
            selected_sources.retain(|source| !source.contains("graph"));
        }

        // D8: lexical fallback is the final safety net at route tail.
        let lexical_fallback_source = "memory:lexical-fallback".to_string();
        if !selected_sources.contains(&lexical_fallback_source) {
            selected_sources.push(lexical_fallback_source.clone());
        }

        let lexical_hits = collect_lexical_fallback_hits(db, session_id, query).await?;
        let lexical_fallback = LexicalFallbackPlan {
            enabled: true,
            strategy: "memory-lexical-tail-fallback-v1".to_string(),
            hits: lexical_hits,
        };

        let route = RecallPluginRoute {
            pluginized,
            strategy: if pluginized {
                "plugin-recall-router-v1".to_string()
            } else {
                "memory-first+source-chunk-inject".to_string()
            },
            selected_sources,
            plugin_ids,
            graph_enabled,
            neighbor_threshold: default_neighbor_threshold(),
            max_neighbors: default_max_neighbors(),
            lexical_fallback: Some(lexical_fallback),
        };

        db.upsert_json_knowledge(
            format!("memory:recall:route:{session_id}:latest"),
            &serde_json::json!({
                "session_id": session_id,
                "tenant_id": tenant_id,
                "route": route,
            }),
            "recall-plugin-router",
        )
        .await?;
        Ok(route)
    }
}

async fn collect_lexical_fallback_hits(
    db: &StateStore,
    session_id: &str,
    query: &str,
) -> Result<Vec<String>> {
    let tokens = tokenize_query(query);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut scored = Vec::<(String, usize)>::new();
    let prefixes = [
        format!("memory:supermemory:atomic:{session_id}:"),
        format!("memory:supermemory:chunks:{session_id}:"),
        format!("memory:supermemory:documents:{session_id}:"),
    ];

    for prefix in prefixes {
        let records = db.list_knowledge_by_prefix(&prefix).await?;
        for record in records {
            let blob = format!("{} {}", record.key, record.value).to_ascii_lowercase();
            let score = tokens.iter().filter(|token| blob.contains(token.as_str())).count();
            if score > 0 {
                scored.push((record.key, score));
            }
        }
    }

    scored.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    scored.dedup_by(|left, right| left.0 == right.0);

    Ok(scored
        .into_iter()
        .take(5)
        .map(|(key, _)| key)
        .collect::<Vec<_>>())
}

async fn load_project_memory_policy(db: &StateStore, tenant_id: &str) -> Result<ProjectMemoryPolicy> {
    let key = format!("project:{tenant_id}:memory-policy");
    let policy = db
        .get_knowledge(&key)
        .await?
        .and_then(|record| serde_json::from_str::<ProjectMemoryPolicy>(&record.value).ok())
        .unwrap_or_default();
    Ok(policy)
}

fn default_neighbor_threshold() -> f32 {
    0.6
}

fn default_max_neighbors() -> usize {
    5
}

fn tokenize_query(query: &str) -> Vec<String> {
    let mut tokens = query
        .to_ascii_lowercase()
        .split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '-')
                .collect::<String>()
        })
        .filter(|token| token.len() >= 2)
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

#[cfg(test)]
mod tests {
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};
    use super::*;

    #[tokio::test]
    async fn route_appends_lexical_fallback_as_tail_source() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });

        let route = RecallPluginRouter::route(&db, "session:d8", "tenant:d8", "fallback")
            .await
            .expect("route");
        assert_eq!(
            route.selected_sources.last().map(String::as_str),
            Some("memory:lexical-fallback")
        );
        assert!(route.lexical_fallback.is_some());
    }

    #[tokio::test]
    async fn lexical_fallback_collects_session_supermemory_hits() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });

        db.upsert_json_knowledge(
            "memory:supermemory:atomic:session:d8:1".to_string(),
            &serde_json::json!({"memory":"incident rollback strategy"}),
            "test",
        )
        .await
        .expect("seed atomic");

        let route = RecallPluginRouter::route(
            &db,
            "session:d8",
            "tenant:d8",
            "incident rollback",
        )
        .await
        .expect("route");

        let hits = route
            .lexical_fallback
            .as_ref()
            .map(|item| item.hits.clone())
            .unwrap_or_default();
        assert!(
            hits.iter()
                .any(|item| item.contains("memory:supermemory:atomic:session:d8"))
        );
    }

    #[tokio::test]
    async fn route_disables_graph_sources_when_project_policy_disables_graph() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });

        db.upsert_json_knowledge(
            "project:tenant:d8:memory-policy".to_string(),
            &serde_json::json!({
                "retrieval_criteria":["relevance","recency","evidence"],
                "multilingual": true,
                "enable_graph": false
            }),
            "project-policy",
        )
        .await
        .expect("seed policy");

        let route = RecallPluginRouter::route(&db, "session:d8", "tenant:d8", "rollback strategy")
            .await
            .expect("route");
        assert!(!route.graph_enabled);
        assert!(route
            .selected_sources
            .iter()
            .all(|source| !source.contains("graph")));
    }
}

