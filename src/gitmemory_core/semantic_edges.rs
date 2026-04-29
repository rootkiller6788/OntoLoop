#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct SemanticEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub confidence: f32,
    pub edge_type: String,
}

pub const EDGE_TYPE_EXTRACTED: &str = "extracted";
pub const EDGE_TYPE_INFERRED: &str = "inferred";
pub const EDGE_TYPE_AMBIGUOUS: &str = "ambiguous";

pub fn canonical_edge_type(edge_type: &str) -> &'static str {
    if edge_type.eq_ignore_ascii_case(EDGE_TYPE_EXTRACTED) {
        EDGE_TYPE_EXTRACTED
    } else if edge_type.eq_ignore_ascii_case(EDGE_TYPE_INFERRED) {
        EDGE_TYPE_INFERRED
    } else if edge_type.eq_ignore_ascii_case(EDGE_TYPE_AMBIGUOUS) {
        EDGE_TYPE_AMBIGUOUS
    } else {
        EDGE_TYPE_AMBIGUOUS
    }
}

pub fn edge_type_priority(edge_type: &str) -> u8 {
    match canonical_edge_type(edge_type) {
        EDGE_TYPE_EXTRACTED => 3,
        EDGE_TYPE_INFERRED => 2,
        EDGE_TYPE_AMBIGUOUS => 1,
        _ => 1,
    }
}

pub fn confidence_threshold(edge_type: &str) -> f32 {
    match canonical_edge_type(edge_type) {
        EDGE_TYPE_EXTRACTED => 0.55,
        EDGE_TYPE_INFERRED => 0.70,
        EDGE_TYPE_AMBIGUOUS => 0.85,
        _ => 0.85,
    }
}

pub fn normalize_semantic_edges(edges: Vec<SemanticEdge>) -> Vec<SemanticEdge> {
    use std::collections::BTreeMap;

    let mut best = BTreeMap::<(String, String, String), SemanticEdge>::new();
    for mut edge in edges {
        edge.edge_type = canonical_edge_type(&edge.edge_type).to_string();
        if edge.confidence < confidence_threshold(&edge.edge_type) {
            continue;
        }
        let key = (edge.from.clone(), edge.to.clone(), edge.relation.clone());
        match best.get(&key) {
            Some(existing)
                if edge_type_priority(&existing.edge_type) > edge_type_priority(&edge.edge_type)
                    || (edge_type_priority(&existing.edge_type)
                        == edge_type_priority(&edge.edge_type)
                        && existing.confidence >= edge.confidence) => {}
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
            .then_with(|| right.confidence.total_cmp(&left.confidence))
            .then_with(|| left.edge_type.cmp(&right.edge_type))
    });
    normalized
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct InferenceCacheEntry {
    pub source_file: String,
    pub source_digest: String,
    pub model: String,
    pub inferred_at_ms: u64,
    #[serde(default)]
    pub edges: Vec<SemanticEdge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct InferenceCheckpointRecord {
    pub checkpoint_id: String,
    pub source_file: String,
    pub source_digest: String,
    pub status: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub edges: Vec<SemanticEdge>,
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_edge_type, edge_type_priority, normalize_semantic_edges, InferenceCacheEntry,
        InferenceCheckpointRecord, SemanticEdge, EDGE_TYPE_AMBIGUOUS, EDGE_TYPE_EXTRACTED,
        EDGE_TYPE_INFERRED,
    };

    #[test]
    fn semantic_edge_contract_roundtrip() {
        let edge = SemanticEdge {
            from: "memory/A".to_string(),
            to: "memory/B".to_string(),
            relation: "supports".to_string(),
            confidence: 0.82,
            edge_type: "inferred".to_string(),
        };
        let raw = serde_json::to_string(&edge).expect("serialize edge");
        let parsed: SemanticEdge = serde_json::from_str(&raw).expect("deserialize edge");
        assert_eq!(parsed, edge);
    }

    #[test]
    fn cache_and_checkpoint_contracts_roundtrip() {
        let edge = SemanticEdge {
            from: "memory/A".to_string(),
            to: "memory/B".to_string(),
            relation: "related".to_string(),
            confidence: 0.7,
            edge_type: "ambiguous".to_string(),
        };
        let cache = InferenceCacheEntry {
            source_file: "memory/MEMORY.md".to_string(),
            source_digest: "sha256:abc".to_string(),
            model: "stub-model".to_string(),
            inferred_at_ms: 123,
            edges: vec![edge.clone()],
        };
        let checkpoint = InferenceCheckpointRecord {
            checkpoint_id: "ckpt-1".to_string(),
            source_file: cache.source_file.clone(),
            source_digest: cache.source_digest.clone(),
            status: "completed".to_string(),
            created_at_ms: 111,
            updated_at_ms: 123,
            error: None,
            edges: vec![edge],
        };

        let cache_raw = serde_json::to_string(&cache).expect("serialize cache");
        let checkpoint_raw = serde_json::to_string(&checkpoint).expect("serialize checkpoint");
        let parsed_cache: InferenceCacheEntry =
            serde_json::from_str(&cache_raw).expect("deserialize cache");
        let parsed_checkpoint: InferenceCheckpointRecord =
            serde_json::from_str(&checkpoint_raw).expect("deserialize checkpoint");

        assert_eq!(parsed_cache, cache);
        assert_eq!(parsed_checkpoint, checkpoint);
    }

    #[test]
    fn edge_type_priority_prefers_extracted_then_inferred_then_ambiguous() {
        assert_eq!(edge_type_priority(EDGE_TYPE_EXTRACTED), 3);
        assert_eq!(edge_type_priority(EDGE_TYPE_INFERRED), 2);
        assert_eq!(edge_type_priority(EDGE_TYPE_AMBIGUOUS), 1);
    }

    #[test]
    fn normalize_semantic_edges_applies_priority_and_threshold() {
        let edges = vec![
            SemanticEdge {
                from: "a".to_string(),
                to: "b".to_string(),
                relation: "references".to_string(),
                confidence: 0.88,
                edge_type: EDGE_TYPE_AMBIGUOUS.to_string(),
            },
            SemanticEdge {
                from: "a".to_string(),
                to: "b".to_string(),
                relation: "references".to_string(),
                confidence: 0.72,
                edge_type: EDGE_TYPE_INFERRED.to_string(),
            },
            SemanticEdge {
                from: "a".to_string(),
                to: "b".to_string(),
                relation: "references".to_string(),
                confidence: 0.61,
                edge_type: EDGE_TYPE_EXTRACTED.to_string(),
            },
            SemanticEdge {
                from: "x".to_string(),
                to: "y".to_string(),
                relation: "mentions".to_string(),
                confidence: 0.69,
                edge_type: EDGE_TYPE_INFERRED.to_string(),
            },
        ];

        let normalized = normalize_semantic_edges(edges);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].edge_type, EDGE_TYPE_EXTRACTED);
        assert_eq!(canonical_edge_type("INFERRED"), EDGE_TYPE_INFERRED);
    }
}
