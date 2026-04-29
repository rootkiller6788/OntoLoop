use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use super::{
    graph_health::{GraphEdge, GraphHealthInput, GraphHealthReport, GraphHealthThresholds, lint_graph_health},
    semantic_edges::{InferenceCacheEntry, EDGE_TYPE_EXTRACTED},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
pub struct GraphExportOptions {
    #[serde(default)]
    pub clean: bool,
    #[serde(default)]
    pub no_infer: bool,
    #[serde(default)]
    pub report: bool,
    #[serde(default)]
    pub save: Option<PathBuf>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct GraphExportEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub edge_type: String,
    pub confidence: f32,
    pub source_file: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct GraphExportReport {
    pub node_count: usize,
    pub edge_count: usize,
    pub removed_by_clean: usize,
    pub removed_by_no_infer: usize,
    pub edge_types: BTreeMap<String, usize>,
    #[serde(default)]
    pub health: Option<GraphHealthReport>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct GraphExportArtifact {
    pub repo_root: String,
    pub generated_at_ms: u64,
    pub options: GraphExportOptions,
    pub nodes: Vec<String>,
    pub edges: Vec<GraphExportEdge>,
    #[serde(default)]
    pub report: Option<GraphExportReport>,
    #[serde(default)]
    pub saved_to: Option<String>,
}

pub struct GraphExportService;

impl GraphExportService {
    pub fn export_offline(repo_root: &Path, options: GraphExportOptions) -> Result<GraphExportArtifact> {
        let mut edges = load_edges(repo_root)?;
        let original_count = edges.len();

        if options.no_infer {
            edges.retain(|edge| edge.edge_type.eq_ignore_ascii_case(EDGE_TYPE_EXTRACTED));
        }
        let removed_by_no_infer = original_count.saturating_sub(edges.len());

        let before_clean = edges.len();
        if options.clean {
            edges = clean_edges(edges);
        } else {
            edges.sort_by(|left, right| {
                left.from
                    .cmp(&right.from)
                    .then_with(|| left.to.cmp(&right.to))
                    .then_with(|| left.relation.cmp(&right.relation))
                    .then_with(|| left.edge_type.cmp(&right.edge_type))
                    .then_with(|| right.confidence.total_cmp(&left.confidence))
            });
        }
        let removed_by_clean = before_clean.saturating_sub(edges.len());

        let mut nodes = BTreeSet::new();
        let mut edge_types = BTreeMap::<String, usize>::new();
        for edge in &edges {
            nodes.insert(edge.from.clone());
            nodes.insert(edge.to.clone());
            *edge_types.entry(edge.edge_type.clone()).or_insert(0) += 1;
        }
        let nodes = nodes.into_iter().collect::<Vec<_>>();

        let report = if options.report {
            let health_edges = edges
                .iter()
                .map(|edge| GraphEdge {
                    from: edge.from.clone(),
                    to: edge.to.clone(),
                })
                .collect::<Vec<_>>();
            Some(GraphExportReport {
                node_count: nodes.len(),
                edge_count: edges.len(),
                removed_by_clean,
                removed_by_no_infer,
                edge_types,
                health: Some(lint_graph_health(
                    &GraphHealthInput {
                        nodes: nodes.clone(),
                        edges: health_edges,
                    },
                    &GraphHealthThresholds::default(),
                )),
            })
        } else {
            None
        };

        let generated_at_ms = current_time_ms();
        let mut artifact = GraphExportArtifact {
            repo_root: repo_root.display().to_string(),
            generated_at_ms,
            options: options.clone(),
            nodes,
            edges,
            report,
            saved_to: None,
        };

        if let Some(target) = options.save {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, serde_json::to_string_pretty(&artifact)?)?;
            artifact.saved_to = Some(target.display().to_string());
        }

        Ok(artifact)
    }
}

fn load_edges(repo_root: &Path) -> Result<Vec<GraphExportEdge>> {
    let cache_path = repo_root.join(".gitmemory").join("semantic").join("edge_cache.json");
    if cache_path.exists() {
        let raw = fs::read_to_string(&cache_path)?;
        let entries = serde_json::from_str::<Vec<InferenceCacheEntry>>(&raw).unwrap_or_default();
        let mut edges = Vec::new();
        for entry in entries {
            for edge in entry.edges {
                edges.push(GraphExportEdge {
                    from: edge.from,
                    to: edge.to,
                    relation: edge.relation,
                    edge_type: edge.edge_type,
                    confidence: edge.confidence,
                    source_file: entry.source_file.clone(),
                });
            }
        }
        return Ok(edges);
    }

    let dep_graph_path = repo_root.join(".gitmemory").join("dependency_graph.json");
    if dep_graph_path.exists() {
        #[derive(Debug, serde::Deserialize, Default)]
        struct DependencyGraph {
            #[serde(default)]
            edges: BTreeMap<String, Vec<String>>,
        }
        let raw = fs::read_to_string(dep_graph_path)?;
        let graph = serde_json::from_str::<DependencyGraph>(&raw).unwrap_or_default();
        let mut edges = Vec::new();
        for (from, deps) in graph.edges {
            for to in deps {
                edges.push(GraphExportEdge {
                    from: from.clone(),
                    to,
                    relation: "references".to_string(),
                    edge_type: EDGE_TYPE_EXTRACTED.to_string(),
                    confidence: 1.0,
                    source_file: from.clone(),
                });
            }
        }
        return Ok(edges);
    }

    Ok(Vec::new())
}

fn clean_edges(edges: Vec<GraphExportEdge>) -> Vec<GraphExportEdge> {
    let mut best = BTreeMap::<(String, String, String, String), GraphExportEdge>::new();
    for edge in edges {
        if edge.from.trim().is_empty() || edge.to.trim().is_empty() || edge.from == edge.to {
            continue;
        }
        let key = (
            edge.from.clone(),
            edge.to.clone(),
            edge.relation.clone(),
            edge.edge_type.clone(),
        );
        match best.get(&key) {
            Some(existing) if existing.confidence >= edge.confidence => {}
            _ => {
                best.insert(key, edge);
            }
        }
    }
    let mut normalized = best.into_values().collect::<Vec<_>>();
    normalized.sort_by(|left, right| {
        left.from
            .cmp(&right.from)
            .then_with(|| left.to.cmp(&right.to))
            .then_with(|| left.relation.cmp(&right.relation))
            .then_with(|| left.edge_type.cmp(&right.edge_type))
            .then_with(|| right.confidence.total_cmp(&left.confidence))
    });
    normalized
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::gitmemory_core::semantic_edges::SemanticEdge;

    #[test]
    fn graph_export_honors_clean_no_infer_report_and_save() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-graph-export-{}",
            current_time_ms()
        ));
        let semantic_dir = temp.join(".gitmemory").join("semantic");
        fs::create_dir_all(&semantic_dir).expect("mkdir");

        let entries = vec![InferenceCacheEntry {
            source_file: "docs/a.md".to_string(),
            source_digest: "digest:a".to_string(),
            model: "semantic-v1".to_string(),
            inferred_at_ms: current_time_ms(),
            edges: vec![
                SemanticEdge {
                    from: "docs/a.md".to_string(),
                    to: "docs/b.md".to_string(),
                    relation: "references".to_string(),
                    confidence: 0.9,
                    edge_type: "inferred".to_string(),
                },
                SemanticEdge {
                    from: "docs/a.md".to_string(),
                    to: "docs/b.md".to_string(),
                    relation: "references".to_string(),
                    confidence: 0.8,
                    edge_type: "inferred".to_string(),
                },
                SemanticEdge {
                    from: "docs/a.md".to_string(),
                    to: "docs/a.md".to_string(),
                    relation: "references".to_string(),
                    confidence: 1.0,
                    edge_type: "extracted".to_string(),
                },
                SemanticEdge {
                    from: "docs/a.md".to_string(),
                    to: "docs/c.md".to_string(),
                    relation: "references".to_string(),
                    confidence: 1.0,
                    edge_type: "extracted".to_string(),
                },
            ],
        }];
        fs::write(
            semantic_dir.join("edge_cache.json"),
            serde_json::to_string_pretty(&entries).expect("serialize"),
        )
        .expect("write cache");

        let saved = temp.join(".gitmemory").join("graph_export").join("artifact.json");
        let artifact = GraphExportService::export_offline(
            &temp,
            GraphExportOptions {
                clean: true,
                no_infer: true,
                report: true,
                save: Some(saved.clone()),
            },
        )
        .expect("export");

        assert_eq!(artifact.edges.len(), 1);
        assert_eq!(artifact.edges[0].edge_type, EDGE_TYPE_EXTRACTED);
        assert!(artifact.report.is_some());
        assert!(saved.exists());

        let _ = fs::remove_dir_all(&temp);
    }
}
