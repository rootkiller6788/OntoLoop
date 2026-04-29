use anyhow::Result;
use autoloop_state_adapter::{KnowledgeRecord, StateStore};

use super::graph_export::{GraphExportArtifact, GraphExportOptions, GraphExportService};
use super::graph_health::{
    GraphEdge, GraphHealthInput, GraphHealthReport, GraphHealthThresholds, lint_graph_health,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MindMapNode {
    pub id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MindMapEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MindMapView {
    pub root: String,
    pub nodes: Vec<MindMapNode>,
    pub edges: Vec<MindMapEdge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExplainerView {
    pub headline: String,
    pub sections: Vec<String>,
    pub supporting_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ViewPlaneSnapshot {
    pub session_id: String,
    pub trace_id: String,
    pub generated_at_ms: u64,
    pub mindmap: MindMapView,
    pub explainer: ExplainerView,
    pub sources: Vec<String>,
    #[serde(default)]
    pub graph_health: Option<GraphHealthReport>,
}

pub struct ViewPlane;

impl ViewPlane {
    pub async fn build(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<ViewPlaneSnapshot> {
        let recall = latest(
            db.list_knowledge_by_prefix(&format!("memory:recall:route:{session_id}:"))
                .await?,
        );
        let patch_review = latest(
            db.list_knowledge_by_prefix(&format!("memory:patch:review:{session_id}:"))
                .await?,
        );
        let compile = latest(
            db.list_knowledge_by_prefix(&format!("memory:compiler:run:{session_id}:"))
                .await?,
        );
        let provenance = latest(
            db.list_knowledge_by_prefix(&format!("memory:provenance:{session_id}:{trace_id}:"))
                .await?,
        );
        let graph_snapshot = db
            .get_knowledge(&format!("graph:{session_id}:snapshot"))
            .await?
            .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok());
        let graph_health = graph_snapshot
            .as_ref()
            .map(build_graph_health_from_snapshot);

        let mut nodes = vec![MindMapNode {
            id: "runtime".to_string(),
            label: format!("runtime:{session_id}"),
            kind: "runtime".to_string(),
        }];
        let mut edges = Vec::<MindMapEdge>::new();
        let mut refs = Vec::<String>::new();
        let mut sections = Vec::<String>::new();

        if let Some(record) = recall {
            refs.push(record.key.clone());
            nodes.push(MindMapNode {
                id: "recall".to_string(),
                label: "recall-route".to_string(),
                kind: "recall".to_string(),
            });
            edges.push(MindMapEdge {
                from: "runtime".to_string(),
                to: "recall".to_string(),
                relation: "reads".to_string(),
            });
            sections.push(format!("Recall route selected from `{}`.", record.key));
        }
        if let Some(record) = patch_review {
            refs.push(record.key.clone());
            nodes.push(MindMapNode {
                id: "patch-review".to_string(),
                label: "patch-review-queue".to_string(),
                kind: "governance".to_string(),
            });
            edges.push(MindMapEdge {
                from: "recall".to_string(),
                to: "patch-review".to_string(),
                relation: "proposes".to_string(),
            });
            sections.push(format!("Patch review state anchored at `{}`.", record.key));
        }
        if let Some(record) = compile {
            refs.push(record.key.clone());
            nodes.push(MindMapNode {
                id: "compile".to_string(),
                label: "incremental-compile".to_string(),
                kind: "source-plane".to_string(),
            });
            edges.push(MindMapEdge {
                from: "patch-review".to_string(),
                to: "compile".to_string(),
                relation: "materializes".to_string(),
            });
            sections.push(format!("Source rebuild recorded by `{}`.", record.key));
        }
        if let Some(record) = provenance {
            refs.push(record.key.clone());
            nodes.push(MindMapNode {
                id: "provenance".to_string(),
                label: "provenance-lineage".to_string(),
                kind: "audit".to_string(),
            });
            edges.push(MindMapEdge {
                from: "compile".to_string(),
                to: "provenance".to_string(),
                relation: "proves".to_string(),
            });
            sections.push(format!(
                "Replay/audit lineage captured in `{}`.",
                record.key
            ));
        }
        if let Some(health) = &graph_health {
            sections.push(format!(
                "Graph health: hub_stub={}, fragile_bridge={}, isolated_community={}, orphan={}.",
                health.hub_stub.len(),
                health.fragile_bridge.len(),
                health.isolated_community.len(),
                health.orphan.len()
            ));
        }

        if sections.is_empty() {
            sections.push(
                "No view-plane sources found yet; run source-plane stages first.".to_string(),
            );
        }

        let snapshot = ViewPlaneSnapshot {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            generated_at_ms: current_time_ms(),
            mindmap: MindMapView {
                root: "runtime".to_string(),
                nodes,
                edges,
            },
            explainer: ExplainerView {
                headline: "Gitmemory Source/View Plane Summary".to_string(),
                sections,
                supporting_refs: refs.clone(),
            },
            sources: refs,
            graph_health,
        };
        Ok(snapshot)
    }

    pub async fn persist(
        db: &StateStore,
        session_id: &str,
        snapshot: &ViewPlaneSnapshot,
    ) -> Result<ViewPlaneReceipt> {
        let ts = snapshot.generated_at_ms;
        let mindmap_ref = format!("memory:view:mindmap:{session_id}:{ts}");
        let explainer_ref = format!("memory:view:explainer:{session_id}:{ts}");
        let plane_ref = format!("memory:view:plane:{session_id}:{ts}");
        let graph_health_ref = format!("memory:graph:health:{session_id}:{ts}");

        db.upsert_json_knowledge(mindmap_ref.clone(), &snapshot.mindmap, "view-plane")
            .await?;
        db.upsert_json_knowledge(explainer_ref.clone(), &snapshot.explainer, "view-plane")
            .await?;
        db.upsert_json_knowledge(plane_ref.clone(), snapshot, "view-plane")
            .await?;
        let graph_health_ref_written = if let Some(graph_health) = &snapshot.graph_health {
            db.upsert_json_knowledge(graph_health_ref.clone(), graph_health, "view-plane")
                .await?;
            db.upsert_json_knowledge(
                format!("memory:graph:health:{session_id}:latest"),
                &serde_json::json!({"ref": graph_health_ref, "generated_at_ms": ts}),
                "view-plane",
            )
            .await?;
            Some(graph_health_ref.clone())
        } else {
            None
        };

        db.upsert_json_knowledge(
            format!("memory:view:mindmap:{session_id}:latest"),
            &serde_json::json!({"ref": mindmap_ref, "generated_at_ms": ts}),
            "view-plane",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("memory:view:explainer:{session_id}:latest"),
            &serde_json::json!({"ref": explainer_ref, "generated_at_ms": ts}),
            "view-plane",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("memory:view:plane:{session_id}:latest"),
            &serde_json::json!({"ref": plane_ref, "generated_at_ms": ts}),
            "view-plane",
        )
        .await?;

        Ok(ViewPlaneReceipt {
            session_id: session_id.to_string(),
            generated_at_ms: ts,
            mindmap_ref,
            explainer_ref,
            plane_ref,
            graph_health_ref: graph_health_ref_written,
        })
    }

    pub fn export_offline_graph(
        repo_root: &std::path::Path,
        options: GraphExportOptions,
    ) -> Result<GraphExportArtifact> {
        GraphExportService::export_offline(repo_root, options)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ViewPlaneReceipt {
    pub session_id: String,
    pub generated_at_ms: u64,
    pub mindmap_ref: String,
    pub explainer_ref: String,
    pub plane_ref: String,
    pub graph_health_ref: Option<String>,
}

fn latest(mut records: Vec<KnowledgeRecord>) -> Option<KnowledgeRecord> {
    records.sort_by(|left, right| left.key.cmp(&right.key));
    records.pop()
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn build_graph_health_from_snapshot(snapshot: &serde_json::Value) -> GraphHealthReport {
    let nodes = snapshot
        .get("entities")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| extract_entity_id(item).map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let edges = snapshot
        .get("relationships")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(extract_relationship_edge)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    lint_graph_health(
        &GraphHealthInput { nodes, edges },
        &GraphHealthThresholds::default(),
    )
}

fn extract_entity_id(entity: &serde_json::Value) -> Option<&str> {
    entity
        .get("id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| entity.get("canonical_name").and_then(serde_json::Value::as_str))
        .or_else(|| entity.get("name").and_then(serde_json::Value::as_str))
}

fn extract_relationship_edge(relationship: &serde_json::Value) -> Option<GraphEdge> {
    let from = relationship
        .get("from")
        .and_then(serde_json::Value::as_str)
        .or_else(|| relationship.get("source").and_then(serde_json::Value::as_str))
        .or_else(|| relationship.get("head").and_then(serde_json::Value::as_str))
        .or_else(|| relationship.get("entity_a").and_then(serde_json::Value::as_str))?;
    let to = relationship
        .get("to")
        .and_then(serde_json::Value::as_str)
        .or_else(|| relationship.get("target").and_then(serde_json::Value::as_str))
        .or_else(|| relationship.get("tail").and_then(serde_json::Value::as_str))
        .or_else(|| relationship.get("entity_b").and_then(serde_json::Value::as_str))?;
    Some(GraphEdge {
        from: from.to_string(),
        to: to.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn view_plane_persists_graph_health_record_and_latest_ref() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-view-health";
        let trace_id = "trace-view-health";

        db.upsert_json_knowledge(
            format!("graph:{session_id}:snapshot"),
            &serde_json::json!({
                "entities": [
                    {"canonical_name":"A"},
                    {"canonical_name":"B"},
                    {"canonical_name":"C"}
                ],
                "relationships": [
                    {"source":"A","target":"B"},
                    {"source":"B","target":"C"}
                ]
            }),
            "graph-rag",
        )
        .await
        .expect("seed graph");

        let snapshot = ViewPlane::build(&db, session_id, trace_id)
            .await
            .expect("build snapshot");
        let receipt = ViewPlane::persist(&db, session_id, &snapshot)
            .await
            .expect("persist snapshot");

        assert!(receipt.graph_health_ref.is_some());
        let latest = db
            .get_knowledge(&format!("memory:graph:health:{session_id}:latest"))
            .await
            .expect("query latest")
            .expect("latest exists");
        assert!(latest.value.contains("memory:graph:health"));
    }
}

