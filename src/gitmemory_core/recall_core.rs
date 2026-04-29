use super::frozen_manifest;
use super::gateway_core::GatewayDecision;
use super::protocol::{CorePackageKind, CorePackageManifest};
use crate::contracts::wiki_compat::QueryRouteReason;

pub struct RecallCore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct NeighborExpansionHit {
    pub source_ref: String,
    pub neighbor_ref: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CjkLexicalHitEvidence {
    pub bigram: String,
    pub candidate_ref: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct QueryRouteFallbackEvidence {
    pub reason: QueryRouteReason,
    pub lexical_hit_count: usize,
    pub fallback_selected_refs: Vec<String>,
    pub applied: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecallPlan {
    pub scope: String,
    pub strategy: String,
    pub query: String,
    pub sources: Vec<String>,
    #[serde(default)]
    pub seed_hits: Vec<String>,
    #[serde(default)]
    pub neighbor_expansion: Vec<NeighborExpansionHit>,
    #[serde(default = "default_neighbor_threshold")]
    pub threshold: f32,
    #[serde(default = "default_max_neighbors")]
    pub max_neighbors: usize,
    #[serde(default)]
    pub graph_enabled: bool,
    #[serde(default)]
    pub cjk_lexical_hits: Vec<CjkLexicalHitEvidence>,
    #[serde(default)]
    pub query_route_fallback: Option<QueryRouteFallbackEvidence>,
}

impl RecallCore {
    pub fn manifest_frozen() -> CorePackageManifest {
        frozen_manifest("core.recall", CorePackageKind::RecallCore, "recall-planner")
    }

    pub fn plan(decision: &GatewayDecision) -> RecallPlan {
        Self::plan_with_sources(
            decision,
            "memory-first+source-chunk-inject",
            vec![
                "memory:atomic".to_string(),
                "memory:chunks".to_string(),
                "memory:relations".to_string(),
            ],
        )
    }

    pub fn plan_with_sources(
        decision: &GatewayDecision,
        strategy: &str,
        sources: Vec<String>,
    ) -> RecallPlan {
        Self::plan_with_graph_expansion(
            decision,
            strategy,
            sources,
            Vec::new(),
            false,
            default_neighbor_threshold(),
            default_max_neighbors(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn plan_with_graph_expansion(
        decision: &GatewayDecision,
        strategy: &str,
        sources: Vec<String>,
        seed_hits: Vec<String>,
        graph_enabled: bool,
        threshold: f32,
        max_neighbors: usize,
    ) -> RecallPlan {
        let mut normalized_seed_hits = normalize_seed_hits(seed_hits);
        let cjk_lexical_hits =
            cjk_bigram_fallback_hits(&decision.normalized_intent, &sources, &normalized_seed_hits);
        if normalized_seed_hits.is_empty() && !cjk_lexical_hits.is_empty() {
            normalized_seed_hits = normalize_seed_hits(
                cjk_lexical_hits
                    .iter()
                    .map(|hit| hit.candidate_ref.clone())
                    .collect(),
            );
        }
        let query_route_fallback = quick_page_selector_fallback(
            &decision.normalized_intent,
            &sources,
            &mut normalized_seed_hits,
        );
        let neighbor_expansion = if graph_enabled {
            expand_neighbors(&normalized_seed_hits, threshold, max_neighbors)
        } else {
            Vec::new()
        };
        RecallPlan {
            scope: decision.scope.clone(),
            strategy: strategy.to_string(),
            query: decision.normalized_intent.clone(),
            sources,
            seed_hits: normalized_seed_hits,
            neighbor_expansion,
            threshold,
            max_neighbors,
            graph_enabled,
            cjk_lexical_hits,
            query_route_fallback,
        }
    }
}

fn default_neighbor_threshold() -> f32 {
    0.6
}

fn default_max_neighbors() -> usize {
    5
}

fn normalize_seed_hits(seed_hits: Vec<String>) -> Vec<String> {
    let mut normalized = seed_hits
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn expand_neighbors(
    seed_hits: &[String],
    threshold: f32,
    max_neighbors: usize,
) -> Vec<NeighborExpansionHit> {
    let mut candidates = Vec::<NeighborExpansionHit>::new();
    for (idx, seed) in seed_hits.iter().enumerate() {
        let stem = seed
            .rsplit(':')
            .next()
            .map(|value| value.replace('/', "_"))
            .unwrap_or_else(|| seed.replace('/', "_"));
        let base_confidence = 0.9 - ((idx as f32) * 0.05);
        let direct = NeighborExpansionHit {
            source_ref: seed.clone(),
            neighbor_ref: format!("graph:neighbor:{stem}"),
            confidence: clamp_confidence(base_confidence),
        };
        let contextual = NeighborExpansionHit {
            source_ref: seed.clone(),
            neighbor_ref: format!("graph:neighbor:{stem}:context"),
            confidence: clamp_confidence(base_confidence - 0.12),
        };
        candidates.push(direct);
        candidates.push(contextual);
    }
    candidates.retain(|item| item.confidence >= threshold);
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.source_ref.cmp(&right.source_ref))
            .then_with(|| left.neighbor_ref.cmp(&right.neighbor_ref))
    });
    candidates.dedup_by(|left, right| left.neighbor_ref == right.neighbor_ref);
    candidates.into_iter().take(max_neighbors).collect()
}

fn clamp_confidence(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn quick_page_selector_fallback(
    query: &str,
    sources: &[String],
    normalized_seed_hits: &mut Vec<String>,
) -> Option<QueryRouteFallbackEvidence> {
    const MIN_LEXICAL_HITS_BEFORE_FALLBACK: usize = 2;
    let lexical_hit_count = normalized_seed_hits.len();
    if lexical_hit_count >= MIN_LEXICAL_HITS_BEFORE_FALLBACK {
        return None;
    }

    let selected = select_quick_pages(query, sources, normalized_seed_hits);
    if selected.is_empty() {
        return Some(QueryRouteFallbackEvidence {
            reason: QueryRouteReason::FastSelectorFallback,
            lexical_hit_count,
            fallback_selected_refs: Vec::new(),
            applied: false,
        });
    }

    normalized_seed_hits.extend(selected.clone());
    *normalized_seed_hits = normalize_seed_hits(normalized_seed_hits.clone());

    Some(QueryRouteFallbackEvidence {
        reason: QueryRouteReason::FastSelectorFallback,
        lexical_hit_count,
        fallback_selected_refs: selected,
        applied: true,
    })
}

fn select_quick_pages(
    query: &str,
    sources: &[String],
    existing_hits: &[String],
) -> Vec<String> {
    let terms = extract_query_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let existing = existing_hits
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut scored = Vec::<(String, usize)>::new();
    for source in sources {
        if existing.contains(source) {
            continue;
        }
        let source_lower = source.to_ascii_lowercase();
        let score = terms
            .iter()
            .filter(|term| source.contains(term.as_str()) || source_lower.contains(term.as_str()))
            .count();
        if score > 0 {
            scored.push((source.clone(), score));
        }
    }
    scored.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.len().cmp(&right.0.len()))
            .then_with(|| left.0.cmp(&right.0))
    });
    scored
        .into_iter()
        .take(3)
        .map(|(source, _)| source)
        .collect()
}

fn extract_query_terms(query: &str) -> Vec<String> {
    let mut terms = extract_cjk_bigrams(query);
    terms.extend(
        query
            .split(|ch: char| !ch.is_alphanumeric() && !is_cjk(ch))
            .filter_map(|token| {
                let token = token.trim();
                if token.chars().count() >= 3 {
                    Some(token.to_ascii_lowercase())
                } else {
                    None
                }
            }),
    );
    let mut normalized = terms
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn cjk_bigram_fallback_hits(
    query: &str,
    sources: &[String],
    normalized_seed_hits: &[String],
) -> Vec<CjkLexicalHitEvidence> {
    if !normalized_seed_hits.is_empty() {
        return Vec::new();
    }

    let bigrams = extract_cjk_bigrams(query);
    if bigrams.is_empty() {
        return Vec::new();
    }

    let mut hits = Vec::<CjkLexicalHitEvidence>::new();
    for source in sources {
        for bigram in &bigrams {
            if source.contains(bigram) {
                hits.push(CjkLexicalHitEvidence {
                    bigram: bigram.clone(),
                    candidate_ref: source.clone(),
                    reason: "cjk_bigram_fallback".to_string(),
                });
            }
        }
    }
    hits.sort_by(|left, right| {
        left.bigram
            .cmp(&right.bigram)
            .then_with(|| left.candidate_ref.cmp(&right.candidate_ref))
    });
    hits.dedup_by(|left, right| {
        left.bigram == right.bigram && left.candidate_ref == right.candidate_ref
    });
    hits
}

fn extract_cjk_bigrams(input: &str) -> Vec<String> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut bigrams = std::collections::BTreeSet::<String>::new();
    for window in chars.windows(2) {
        let first = window[0];
        let second = window[1];
        if is_cjk(first) && is_cjk(second) {
            bigrams.insert([first, second].iter().collect());
        }
    }
    bigrams.into_iter().collect()
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
    )
}

#[cfg(test)]
mod tests {
    use super::RecallCore;
    use crate::plugins::gitmemory_core::gateway_core::GatewayDecision;

    #[test]
    fn graph_disabled_keeps_neighbor_expansion_empty() {
        let decision = GatewayDecision {
            accepted: true,
            reason: "ok".to_string(),
            normalized_intent: "test".to_string(),
            scope: "session".to_string(),
        };
        let plan = RecallCore::plan_with_graph_expansion(
            &decision,
            "test",
            vec!["memory:relations".to_string()],
            vec!["memory:supermemory:atomic:s1:1".to_string()],
            false,
            0.6,
            5,
        );
        assert!(plan.neighbor_expansion.is_empty());
    }

    #[test]
    fn graph_enabled_expands_neighbors_with_confidence() {
        let decision = GatewayDecision {
            accepted: true,
            reason: "ok".to_string(),
            normalized_intent: "test".to_string(),
            scope: "session".to_string(),
        };
        let plan = RecallCore::plan_with_graph_expansion(
            &decision,
            "test",
            vec!["memory:relations".to_string()],
            vec![
                "memory:supermemory:atomic:s1:1".to_string(),
                "memory:supermemory:atomic:s1:2".to_string(),
            ],
            true,
            0.7,
            3,
        );
        assert_eq!(plan.threshold, 0.7);
        assert_eq!(plan.max_neighbors, 3);
        assert!(!plan.neighbor_expansion.is_empty());
        assert!(plan.neighbor_expansion.iter().all(|item| item.confidence >= 0.7));
        assert!(plan.neighbor_expansion.windows(2).all(|pair| {
            pair[0].confidence >= pair[1].confidence
        }));
    }

    #[test]
    fn cjk_bigram_fallback_adds_seed_hits_and_evidence_when_seed_empty() {
        let decision = GatewayDecision {
            accepted: true,
            reason: "ok".to_string(),
            normalized_intent: "请总结图谱健康".to_string(),
            scope: "session".to_string(),
        };
        let plan = RecallCore::plan_with_graph_expansion(
            &decision,
            "test",
            vec![
                "memory:graph:健康".to_string(),
                "memory:notes:其他".to_string(),
            ],
            Vec::new(),
            false,
            0.6,
            5,
        );
        assert!(plan.seed_hits.iter().any(|item| item == "memory:graph:健康"));
        assert!(plan.cjk_lexical_hits.iter().any(|item| {
            item.bigram == "健康"
                && item.candidate_ref == "memory:graph:健康"
                && item.reason == "cjk_bigram_fallback"
        }));
    }

    #[test]
    fn cjk_bigram_fallback_skips_when_seed_hits_already_exist() {
        let decision = GatewayDecision {
            accepted: true,
            reason: "ok".to_string(),
            normalized_intent: "请总结图谱健康".to_string(),
            scope: "session".to_string(),
        };
        let plan = RecallCore::plan_with_graph_expansion(
            &decision,
            "test",
            vec!["memory:graph:健康".to_string()],
            vec!["memory:supermemory:atomic:s1:1".to_string()],
            false,
            0.6,
            5,
        );
        assert!(plan
            .seed_hits
            .contains(&"memory:supermemory:atomic:s1:1".to_string()));
        assert!(plan.cjk_lexical_hits.is_empty());
    }

    #[test]
    fn quick_selector_fallback_applies_when_lexical_hits_low() {
        let decision = GatewayDecision {
            accepted: true,
            reason: "ok".to_string(),
            normalized_intent: "review memory graph health".to_string(),
            scope: "session".to_string(),
        };
        let plan = RecallCore::plan_with_graph_expansion(
            &decision,
            "test",
            vec![
                "memory:graph:health".to_string(),
                "memory:graph:health:summary".to_string(),
                "memory:other:unrelated".to_string(),
            ],
            vec!["memory:lexical:single-hit".to_string()],
            true,
            0.6,
            5,
        );
        let fallback = plan
            .query_route_fallback
            .as_ref()
            .expect("fallback evidence present");
        assert!(fallback.applied);
        assert_eq!(fallback.lexical_hit_count, 1);
        assert!(!fallback.fallback_selected_refs.is_empty());
        assert!(plan.seed_hits.len() >= 2);
        assert!(!plan.neighbor_expansion.is_empty());
    }

    #[test]
    fn quick_selector_fallback_skips_when_lexical_hits_enough() {
        let decision = GatewayDecision {
            accepted: true,
            reason: "ok".to_string(),
            normalized_intent: "review memory graph health".to_string(),
            scope: "session".to_string(),
        };
        let plan = RecallCore::plan_with_graph_expansion(
            &decision,
            "test",
            vec![
                "memory:graph:health".to_string(),
                "memory:graph:health:summary".to_string(),
            ],
            vec![
                "memory:lexical:hit1".to_string(),
                "memory:lexical:hit2".to_string(),
            ],
            false,
            0.6,
            5,
        );
        assert!(plan.query_route_fallback.is_none());
        assert_eq!(
            plan.seed_hits,
            vec![
                "memory:lexical:hit1".to_string(),
                "memory:lexical:hit2".to_string()
            ]
        );
    }
}
