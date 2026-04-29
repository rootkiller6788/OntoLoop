use crate::rag::{
    graph_core::{GraphModule, normalize_key},
    model::{ChunkRecord, QueryContext, QueryMode, RankedRelationship, SearchHit},
};
use std::collections::{BTreeSet, HashMap, HashSet};

impl GraphModule {
    pub fn joint_query_context(
        &self,
        document_id: u64,
        query: &str,
        max_chunks: usize,
        max_relationships: usize,
        max_communities: usize,
    ) -> QueryContext {
        let local = self.local_query_context(document_id, query, max_chunks, max_relationships);
        let global = self.global_query_context(document_id, query, max_communities);
        let alias_terms = alias_terms(query);

        let mut matched_chunk_ids = local
            .matched_chunk_ids
            .iter()
            .chain(global.matched_chunk_ids.iter())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let matched_entity_ids = local
            .matched_entity_ids
            .iter()
            .chain(global.matched_entity_ids.iter())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let matched_relationship_ids = local
            .matched_relationship_ids
            .iter()
            .chain(global.matched_relationship_ids.iter())
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(max_relationships.max(global.matched_relationship_ids.len()))
            .collect::<Vec<_>>();
        matched_chunk_ids.sort_by(|left, right| {
            let left_score = self
                .chunks()
                .iter()
                .find(|chunk| chunk.id == *left)
                .map(|chunk| alias_overlap_score(&alias_terms, &chunk.text))
                .unwrap_or(0);
            let right_score = self
                .chunks()
                .iter()
                .find(|chunk| chunk.id == *right)
                .map(|chunk| alias_overlap_score(&alias_terms, &chunk.text))
                .unwrap_or(0);
            right_score.cmp(&left_score).then_with(|| left.cmp(right))
        });
        matched_chunk_ids.truncate(max_chunks.max(global.matched_chunk_ids.len()));

        QueryContext {
            document_id,
            mode: QueryMode::Local,
            query: query.to_string(),
            matched_chunk_ids: matched_chunk_ids.clone(),
            matched_entity_ids: matched_entity_ids.clone(),
            matched_relationship_ids: matched_relationship_ids.clone(),
            summary: format!(
                "Joint GraphRAG ranked {} chunks, {} entities and {} relationships using alias-aware local/global evidence fusion",
                matched_chunk_ids.len(),
                matched_entity_ids.len(),
                matched_relationship_ids.len()
            ),
        }
    }

    pub fn local_query_context(
        &self,
        document_id: u64,
        query: &str,
        max_chunks: usize,
        max_relationships: usize,
    ) -> QueryContext {
        let chunks = self
            .chunks()
            .iter()
            .filter(|chunk| chunk.document_id == document_id)
            .cloned()
            .collect::<Vec<_>>();
        let recency_bonus = self
            .documents()
            .iter()
            .find(|document| document.id == document_id)
            .map(|document| self.document_recency_bonus(document.created_at_ms))
            .unwrap_or(0);
        let chunk_entities = self.chunk_entity_map(document_id);
        let entity_names = self
            .entities()
            .iter()
            .map(|entity| (entity.id, entity.canonical_name.clone()))
            .collect::<HashMap<_, _>>();
        let hits = compute_search_hits(
            query,
            &alias_terms(query),
            &chunks,
            &chunk_entities,
            &entity_names,
            recency_bonus,
        );
        let selected_hits = hits.into_iter().take(max_chunks).collect::<Vec<_>>();

        let matched_chunk_ids = selected_hits
            .iter()
            .map(|hit| hit.chunk_id)
            .collect::<Vec<_>>();
        let matched_entity_ids = selected_hits
            .iter()
            .flat_map(|hit| hit.entity_ids.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        let ranked_relationships = self.rank_relationships(document_id, &matched_entity_ids);
        let matched_relationship_ids = ranked_relationships
            .into_iter()
            .take(max_relationships)
            .map(|row| row.relationship_id)
            .collect::<Vec<_>>();

        QueryContext {
            document_id,
            mode: QueryMode::Local,
            query: query.to_string(),
            matched_chunk_ids: matched_chunk_ids.clone(),
            matched_entity_ids: matched_entity_ids.clone(),
            matched_relationship_ids: matched_relationship_ids.clone(),
            summary: format!(
                "Local GraphRAG matched {} chunks, {} entities and {} relationships",
                matched_chunk_ids.len(),
                matched_entity_ids.len(),
                matched_relationship_ids.len()
            ),
        }
    }

    pub fn global_query_context(
        &self,
        document_id: u64,
        query: &str,
        max_communities: usize,
    ) -> QueryContext {
        let terms = tokenize_query(query);
        let mut ranked = self
            .communities()
            .iter()
            .filter(|community| community.document_id == document_id)
            .map(|community| {
                (
                    community.id,
                    overlap_score(&terms, &community.summary)
                        + community.rank
                        + self.document_recency_bonus_for_community(community.document_id),
                )
            })
            .filter(|(_, score)| *score > 0)
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let matched_community_ids = ranked
            .into_iter()
            .take(max_communities)
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        let matched_entity_ids = self
            .communities()
            .iter()
            .filter(|community| matched_community_ids.contains(&community.id))
            .flat_map(|community| community.member_entity_ids.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let matched_relationship_ids = self
            .communities()
            .iter()
            .filter(|community| matched_community_ids.contains(&community.id))
            .flat_map(|community| community.relationship_ids.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let matched_chunk_ids = self
            .mentions()
            .iter()
            .filter(|mention| matched_entity_ids.contains(&mention.entity_id))
            .map(|mention| mention.chunk_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        QueryContext {
            document_id,
            mode: QueryMode::Global,
            query: query.to_string(),
            matched_chunk_ids: matched_chunk_ids.clone(),
            matched_entity_ids: matched_entity_ids.clone(),
            matched_relationship_ids: matched_relationship_ids.clone(),
            summary: format!(
                "Global GraphRAG matched {} communities, {} entities and {} relationships",
                matched_community_ids.len(),
                matched_entity_ids.len(),
                matched_relationship_ids.len()
            ),
        }
    }

    pub fn rank_relationships(
        &self,
        document_id: u64,
        entity_ids: &[u64],
    ) -> Vec<RankedRelationship> {
        let entity_set = entity_ids.iter().copied().collect::<HashSet<_>>();
        let mut rows = self
            .relationships()
            .iter()
            .filter(|relationship| {
                relationship.document_id == document_id
                    && entity_set.contains(&relationship.source_entity_id)
                    && entity_set.contains(&relationship.target_entity_id)
            })
            .map(|relationship| RankedRelationship {
                relationship_id: relationship.id,
                weight: relationship.weight + relationship.confidence,
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            b.weight
                .cmp(&a.weight)
                .then_with(|| a.relationship_id.cmp(&b.relationship_id))
        });
        rows
    }

    fn chunk_entity_map(&self, document_id: u64) -> HashMap<u64, BTreeSet<u64>> {
        let mut map = HashMap::<u64, BTreeSet<u64>>::new();
        for mention in self
            .mentions()
            .iter()
            .filter(|mention| mention.document_id == document_id)
        {
            map.entry(mention.chunk_id)
                .or_default()
                .insert(mention.entity_id);
        }
        map
    }

    fn document_recency_bonus(&self, created_at_ms: u64) -> u32 {
        let newest = self
            .documents()
            .iter()
            .map(|document| document.created_at_ms)
            .max()
            .unwrap_or(created_at_ms);
        if newest <= created_at_ms {
            6
        } else {
            let delta = newest.saturating_sub(created_at_ms);
            if delta < 86_400_000 {
                5
            } else if delta < 7 * 86_400_000 {
                3
            } else {
                1
            }
        }
    }

    fn document_recency_bonus_for_community(&self, document_id: u64) -> u32 {
        self.documents()
            .iter()
            .find(|document| document.id == document_id)
            .map(|document| self.document_recency_bonus(document.created_at_ms))
            .unwrap_or(0)
    }
}

pub fn compute_search_hits(
    query: &str,
    alias_terms: &BTreeSet<String>,
    chunks: &[ChunkRecord],
    chunk_entities: &HashMap<u64, BTreeSet<u64>>,
    entity_names: &HashMap<u64, String>,
    recency_bonus: u32,
) -> Vec<SearchHit> {
    let terms = tokenize_query(query);
    let mut hits = Vec::new();

    for chunk in chunks {
        let mut score = overlap_score(&terms, &chunk.text) * 3
            + alias_overlap_score(alias_terms, &chunk.text) * 2
            + chunk.token_count / 24
            + recency_bonus;
        let mut entity_ids = BTreeSet::new();

        if let Some(ids) = chunk_entities.get(&chunk.id) {
            for entity_id in ids {
                if let Some(name) = entity_names.get(entity_id) {
                    let entity_score = overlap_score(&terms, name);
                    let alias_score = alias_overlap_score(alias_terms, name);
                    if entity_score > 0 || alias_score > 0 {
                        score += entity_score * 7 + alias_score * 2;
                        entity_ids.insert(*entity_id);
                    }
                }
            }
        }

        if score > 0 {
            hits.push(SearchHit {
                chunk_id: chunk.id,
                entity_ids,
                score,
            });
        }
    }

    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
    hits
}

fn tokenize_query(text: &str) -> BTreeSet<String> {
    text.split_whitespace()
        .map(normalize_key)
        .filter(|token| token.len() > 1)
        .collect()
}

fn alias_terms(text: &str) -> BTreeSet<String> {
    let mut terms = tokenize_query(text);
    let normalized = normalize_key(text);
    for segment in normalized.split('_') {
        if segment.len() > 2 {
            terms.insert(segment.to_string());
        }
    }
    terms
}

fn overlap_score(terms: &BTreeSet<String>, haystack: &str) -> u32 {
    let hay_tokens: HashSet<String> = haystack
        .split_whitespace()
        .map(normalize_key)
        .filter(|token| token.len() > 1)
        .collect();
    terms
        .iter()
        .filter(|term| hay_tokens.contains(*term))
        .count() as u32
}

fn alias_overlap_score(terms: &BTreeSet<String>, haystack: &str) -> u32 {
    let normalized = normalize_key(haystack);
    terms
        .iter()
        .filter(|term| normalized.contains(term.as_str()))
        .count() as u32
}
