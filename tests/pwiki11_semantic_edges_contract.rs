use autoloop::plugins::gitmemory_core::semantic_edges::{
    EDGE_TYPE_EXTRACTED, SemanticEdge, edge_type_priority, normalize_semantic_edges,
};

#[test]
fn pwiki11_semantic_edges_priority_and_threshold_contract() {
    let edges = vec![
        SemanticEdge {
            from: "a".to_string(),
            to: "b".to_string(),
            relation: "references".to_string(),
            confidence: 0.86,
            edge_type: "ambiguous".to_string(),
        },
        SemanticEdge {
            from: "a".to_string(),
            to: "b".to_string(),
            relation: "references".to_string(),
            confidence: 0.73,
            edge_type: "inferred".to_string(),
        },
        SemanticEdge {
            from: "a".to_string(),
            to: "b".to_string(),
            relation: "references".to_string(),
            confidence: 0.60,
            edge_type: "extracted".to_string(),
        },
        SemanticEdge {
            from: "x".to_string(),
            to: "y".to_string(),
            relation: "mentions".to_string(),
            confidence: 0.69,
            edge_type: "inferred".to_string(),
        },
    ];

    let normalized = normalize_semantic_edges(edges);
    assert_eq!(normalized.len(), 1);
    assert_eq!(normalized[0].edge_type, EDGE_TYPE_EXTRACTED);
    assert_eq!(edge_type_priority(&normalized[0].edge_type), 3);
}



