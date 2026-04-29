use std::fs;

use autoloop::plugins::gitmemory_core::{
    graph_export::{GraphExportOptions, GraphExportService},
    semantic_edges::{InferenceCacheEntry, SemanticEdge},
};

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[test]
fn pwiki11_graph_export_no_infer_filters_inferred_edges() {
    let temp = std::env::temp_dir().join(format!("pwiki11_graph_noinfer_{}", now_ms()));
    let semantic_dir = temp.join(".gitmemory").join("semantic");
    fs::create_dir_all(&semantic_dir).expect("mkdir semantic");

    let entries = vec![InferenceCacheEntry {
        source_file: "docs/a.md".to_string(),
        source_digest: "digest:a".to_string(),
        model: "semantic-v1".to_string(),
        inferred_at_ms: now_ms(),
        edges: vec![
            SemanticEdge {
                from: "docs/a.md".to_string(),
                to: "docs/b.md".to_string(),
                relation: "references".to_string(),
                confidence: 0.92,
                edge_type: "extracted".to_string(),
            },
            SemanticEdge {
                from: "docs/a.md".to_string(),
                to: "docs/c.md".to_string(),
                relation: "supports".to_string(),
                confidence: 0.78,
                edge_type: "inferred".to_string(),
            },
        ],
    }];

    fs::write(
        semantic_dir.join("edge_cache.json"),
        serde_json::to_string_pretty(&entries).expect("serialize cache"),
    )
    .expect("write cache");

    let artifact = GraphExportService::export_offline(
        &temp,
        GraphExportOptions {
            clean: true,
            no_infer: true,
            report: true,
            save: None,
        },
    )
    .expect("export");

    assert_eq!(artifact.edges.len(), 1);
    assert!(artifact.edges[0].edge_type.eq_ignore_ascii_case("extracted"));
    assert!(artifact.report.as_ref().is_some_and(|r| r.removed_by_no_infer >= 1));

    let _ = fs::remove_dir_all(&temp);
}



