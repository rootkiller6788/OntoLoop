use autoloop::plugins::gitmemory_core::semantic_edges::{
    InferenceCacheEntry, InferenceCheckpointRecord, SemanticEdge,
};

#[test]
fn pwiki11_inference_cache_checkpoint_roundtrip() {
    let edge = SemanticEdge {
        from: "memory/A".to_string(),
        to: "memory/B".to_string(),
        relation: "supports".to_string(),
        confidence: 0.81,
        edge_type: "inferred".to_string(),
    };

    let cache = InferenceCacheEntry {
        source_file: "memory/MEMORY.md".to_string(),
        source_digest: "sha256:abc".to_string(),
        model: "semantic-v1".to_string(),
        inferred_at_ms: 10,
        edges: vec![edge.clone()],
    };
    let checkpoint = InferenceCheckpointRecord {
        checkpoint_id: "semantic-ckpt:1".to_string(),
        source_file: cache.source_file.clone(),
        source_digest: cache.source_digest.clone(),
        status: "completed".to_string(),
        created_at_ms: 9,
        updated_at_ms: 10,
        error: None,
        edges: vec![edge],
    };

    let cache_raw = serde_json::to_string(&cache).expect("serialize cache");
    let checkpoint_raw = serde_json::to_string(&checkpoint).expect("serialize checkpoint");

    let cache_parsed: InferenceCacheEntry =
        serde_json::from_str(&cache_raw).expect("deserialize cache");
    let checkpoint_parsed: InferenceCheckpointRecord =
        serde_json::from_str(&checkpoint_raw).expect("deserialize checkpoint");

    assert_eq!(cache_parsed, cache);
    assert_eq!(checkpoint_parsed, checkpoint);
}



