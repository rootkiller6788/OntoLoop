use crate::rag::model::{
    ChunkFactResult, ChunkRecord, CommunityBuildResult, CommunityRecord, DatabaseSnapshot,
    DocumentRecord, DocumentStatus, EntityRecord, ExtractedChunkGraph, ExtractedEntity,
    ExtractedRelationship, IngestDocumentResult, MentionRecord, RelationshipRecord,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const STOPWORDS: &[&str] = &[
    "A", "An", "And", "As", "At", "By", "For", "From", "In", "Into", "Of", "On", "Or", "The", "To",
    "With",
];

const RELATION_HINTS: &[(&str, &str)] = &[
    ("founded by", "FOUNDED_BY"),
    ("founded", "FOUNDED"),
    ("acquired", "ACQUIRED"),
    ("merged with", "MERGED_WITH"),
    ("works at", "WORKS_AT"),
    ("joined", "JOINED"),
    ("located in", "LOCATED_IN"),
    ("uses", "USES"),
    ("competes with", "COMPETES_WITH"),
    ("invested in", "INVESTED_IN"),
    ("partnered with", "PARTNERED_WITH"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReducerContext {
    pub caller: String,
    pub timestamp_ms: u64,
}

#[derive(Default)]
pub struct GraphModule {
    next_document_id: u64,
    next_chunk_id: u64,
    next_entity_id: u64,
    next_relationship_id: u64,
    next_mention_id: u64,
    next_community_id: u64,
    documents: Vec<DocumentRecord>,
    chunks: Vec<ChunkRecord>,
    entities: Vec<EntityRecord>,
    mentions: Vec<MentionRecord>,
    relationships: Vec<RelationshipRecord>,
    communities: Vec<CommunityRecord>,
}

impl GraphModule {
    pub fn documents(&self) -> &[DocumentRecord] {
        &self.documents
    }

    pub fn chunks(&self) -> &[ChunkRecord] {
        &self.chunks
    }

    pub fn entities(&self) -> &[EntityRecord] {
        &self.entities
    }

    pub fn mentions(&self) -> &[MentionRecord] {
        &self.mentions
    }

    pub fn relationships(&self) -> &[RelationshipRecord] {
        &self.relationships
    }

    pub fn communities(&self) -> &[CommunityRecord] {
        &self.communities
    }

    pub fn snapshot(&self) -> DatabaseSnapshot {
        DatabaseSnapshot {
            documents: self.documents.clone(),
            chunks: self.chunks.clone(),
            entities: self.entities.clone(),
            mentions: self.mentions.clone(),
            relationships: self.relationships.clone(),
            communities: self.communities.clone(),
        }
    }

    pub fn ingest_document_with_heuristics(
        &mut self,
        ctx: &ReducerContext,
        title: String,
        source_uri: String,
        raw_text: String,
        chunk_size: usize,
        overlap: usize,
    ) -> IngestDocumentResult {
        let document_id = self.insert_document(ctx, title, source_uri, raw_text.clone());
        let chunk_ids = self.insert_chunks(document_id, &raw_text, chunk_size, overlap);

        let mut entity_ids = BTreeSet::new();
        let mut relationship_ids = BTreeSet::new();

        for chunk_id in &chunk_ids {
            let chunk_text = self
                .chunks
                .iter()
                .find(|chunk| chunk.id == *chunk_id)
                .map(|chunk| chunk.text.clone())
                .unwrap_or_default();
            let extracted = heuristic_extract_chunk_graph(&chunk_text);
            let result = self.attach_chunk_graph(document_id, *chunk_id, extracted);
            entity_ids.extend(result.entity_ids);
            relationship_ids.extend(result.relationship_ids);
        }

        let community_result = self.rebuild_communities(document_id);
        self.update_document_stats(document_id);

        IngestDocumentResult {
            document_id,
            chunk_ids,
            entity_ids: entity_ids.into_iter().collect(),
            relationship_ids: relationship_ids.into_iter().collect(),
            community_ids: community_result.community_ids,
        }
    }

    pub fn attach_chunk_graph(
        &mut self,
        document_id: u64,
        chunk_id: u64,
        extracted: ExtractedChunkGraph,
    ) -> ChunkFactResult {
        let mut entity_ids = Vec::new();
        let mut entity_name_to_id = BTreeMap::new();

        for entity in extracted.entities {
            let entity_id = self.upsert_entity(
                document_id,
                &entity.name,
                &entity.entity_type,
                &entity.description,
            );
            self.insert_mention(document_id, chunk_id, entity_id, &entity.name);
            entity_name_to_id.insert(normalize_key(&entity.name), entity_id);
            entity_ids.push(entity_id);
        }

        let mut relationship_ids = Vec::new();
        for relationship in extracted.relationships {
            let source_id = match entity_name_to_id.get(&normalize_key(&relationship.source_name)) {
                Some(id) => *id,
                None => continue,
            };
            let target_id = match entity_name_to_id.get(&normalize_key(&relationship.target_name)) {
                Some(id) => *id,
                None => continue,
            };
            if source_id == target_id {
                continue;
            }
            let relationship_id = self.upsert_relationship(
                document_id,
                source_id,
                target_id,
                &relationship.relation_type,
                chunk_id,
                &relationship.description,
            );
            relationship_ids.push(relationship_id);
        }

        self.recompute_entity_weights(document_id);
        self.update_document_stats(document_id);

        ChunkFactResult {
            chunk_id,
            entity_ids,
            relationship_ids,
        }
    }

    pub fn rebuild_communities(&mut self, document_id: u64) -> CommunityBuildResult {
        self.communities
            .retain(|community| community.document_id != document_id);

        let entity_ids: Vec<u64> = self
            .mentions
            .iter()
            .filter(|mention| mention.document_id == document_id)
            .map(|mention| mention.entity_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        let relationships: Vec<(u64, u64, u64)> = self
            .relationships
            .iter()
            .filter(|relationship| relationship.document_id == document_id)
            .map(|relationship| {
                (
                    relationship.id,
                    relationship.source_entity_id,
                    relationship.target_entity_id,
                )
            })
            .collect();

        let mut community_ids = Vec::new();
        for (member_entity_ids, relationship_ids) in build_communities(&entity_ids, &relationships)
        {
            if member_entity_ids.is_empty() {
                continue;
            }

            let rank = member_entity_ids
                .iter()
                .filter_map(|id| self.entities.iter().find(|entity| entity.id == *id))
                .map(|entity| entity.weight)
                .sum::<u32>()
                + relationship_ids.len() as u32 * 4;

            let label = member_entity_ids
                .iter()
                .filter_map(|entity_id| self.entities.iter().find(|entity| entity.id == *entity_id))
                .max_by_key(|entity| entity.weight)
                .map(|entity| entity.canonical_name.clone())
                .unwrap_or_else(|| "Community".to_string());

            let summary = format!(
                "Community around {label}: entities={} relationships={} rank={rank}",
                member_entity_ids.len(),
                relationship_ids.len()
            );

            self.next_community_id += 1;
            self.communities.push(CommunityRecord {
                id: self.next_community_id,
                document_id,
                label,
                member_entity_ids,
                relationship_ids,
                rank,
                summary,
            });
            community_ids.push(self.next_community_id);
        }

        CommunityBuildResult {
            document_id,
            community_ids,
        }
    }

    fn insert_document(
        &mut self,
        ctx: &ReducerContext,
        title: String,
        source_uri: String,
        raw_text: String,
    ) -> u64 {
        self.next_document_id += 1;
        self.documents.push(DocumentRecord {
            id: self.next_document_id,
            title,
            source_uri,
            raw_text,
            status: DocumentStatus::Pending,
            created_at_ms: ctx.timestamp_ms,
            chunk_count: 0,
            entity_count: 0,
            relationship_count: 0,
        });
        self.next_document_id
    }

    fn insert_chunks(
        &mut self,
        document_id: u64,
        raw_text: &str,
        chunk_size: usize,
        overlap: usize,
    ) -> Vec<u64> {
        let mut draft_chunks = chunk_text(document_id, raw_text, chunk_size, overlap);
        let mut chunk_ids = Vec::new();

        for chunk in &mut draft_chunks {
            self.next_chunk_id += 1;
            chunk.id = self.next_chunk_id;
            chunk_ids.push(chunk.id);
        }

        for idx in 0..draft_chunks.len() {
            draft_chunks[idx].previous_chunk_id = idx.checked_sub(1).map(|i| draft_chunks[i].id);
            draft_chunks[idx].next_chunk_id = draft_chunks.get(idx + 1).map(|chunk| chunk.id);
        }

        self.chunks.extend(draft_chunks);

        if let Some(document) = self
            .documents
            .iter_mut()
            .find(|document| document.id == document_id)
        {
            document.status = DocumentStatus::Chunked;
            document.chunk_count = chunk_ids.len() as u32;
        }

        chunk_ids
    }

    fn upsert_entity(
        &mut self,
        document_id: u64,
        name: &str,
        entity_type: &str,
        description: &str,
    ) -> u64 {
        let normalized_name = normalize_key(name);
        if let Some(entity) = self
            .entities
            .iter_mut()
            .find(|entity| entity.normalized_name == normalized_name)
        {
            entity.salience += 1;
            entity.mention_count += 1;
            if entity.description.is_empty() {
                entity.description = description.to_string();
            }
            return entity.id;
        }

        self.next_entity_id += 1;
        self.entities.push(EntityRecord {
            id: self.next_entity_id,
            canonical_name: name.to_string(),
            normalized_name,
            entity_type: entity_type.to_string(),
            description: description.to_string(),
            salience: 1,
            mention_count: 1,
            degree: 0,
            weight: 1,
            first_document_id: document_id,
        });
        self.next_entity_id
    }

    fn insert_mention(&mut self, document_id: u64, chunk_id: u64, entity_id: u64, surface: &str) {
        let already_exists = self.mentions.iter().any(|mention| {
            mention.document_id == document_id
                && mention.chunk_id == chunk_id
                && mention.entity_id == entity_id
                && mention.surface == surface
        });
        if already_exists {
            return;
        }

        self.next_mention_id += 1;
        self.mentions.push(MentionRecord {
            id: self.next_mention_id,
            document_id,
            chunk_id,
            entity_id,
            surface: surface.to_string(),
        });
    }

    fn upsert_relationship(
        &mut self,
        document_id: u64,
        source_entity_id: u64,
        target_entity_id: u64,
        relation_type: &str,
        chunk_id: u64,
        description: &str,
    ) -> u64 {
        if let Some(relationship) = self.relationships.iter_mut().find(|relationship| {
            relationship.document_id == document_id
                && relationship.source_entity_id == source_entity_id
                && relationship.target_entity_id == target_entity_id
                && relationship.relation_type == relation_type
        }) {
            relationship.weight += 3;
            relationship.confidence = relationship.confidence.saturating_add(10).min(100);
            if !relationship.evidence_chunk_ids.contains(&chunk_id) {
                relationship.evidence_chunk_ids.push(chunk_id);
            }
            return relationship.id;
        }

        self.next_relationship_id += 1;
        self.relationships.push(RelationshipRecord {
            id: self.next_relationship_id,
            document_id,
            source_entity_id,
            target_entity_id,
            relation_type: relation_type.to_string(),
            weight: 3,
            confidence: 60,
            evidence_chunk_ids: vec![chunk_id],
            description: description.to_string(),
        });
        self.next_relationship_id
    }

    fn recompute_entity_weights(&mut self, document_id: u64) {
        let mut degree_map = BTreeMap::<u64, u32>::new();
        for relationship in self
            .relationships
            .iter()
            .filter(|r| r.document_id == document_id)
        {
            *degree_map.entry(relationship.source_entity_id).or_default() += 1;
            *degree_map.entry(relationship.target_entity_id).or_default() += 1;
        }

        for entity in &mut self.entities {
            entity.degree = *degree_map.get(&entity.id).unwrap_or(&0);
            entity.weight = entity.salience * 2 + entity.mention_count * 3 + entity.degree * 5;
        }

        let entity_weights = self
            .entities
            .iter()
            .map(|entity| (entity.id, entity.weight))
            .collect::<BTreeMap<_, _>>();
        for relationship in &mut self.relationships {
            if relationship.document_id != document_id {
                continue;
            }
            let left = *entity_weights
                .get(&relationship.source_entity_id)
                .unwrap_or(&0);
            let right = *entity_weights
                .get(&relationship.target_entity_id)
                .unwrap_or(&0);
            relationship.weight =
                relationship.evidence_chunk_ids.len() as u32 * 4 + (left + right) / 2;
        }
    }

    fn update_document_stats(&mut self, document_id: u64) {
        let entity_ids = self
            .mentions
            .iter()
            .filter(|mention| mention.document_id == document_id)
            .map(|mention| mention.entity_id)
            .collect::<BTreeSet<_>>();
        let relationship_count = self
            .relationships
            .iter()
            .filter(|relationship| relationship.document_id == document_id)
            .count() as u32;

        if let Some(document) = self
            .documents
            .iter_mut()
            .find(|document| document.id == document_id)
        {
            document.entity_count = entity_ids.len() as u32;
            document.relationship_count = relationship_count;
            if document.chunk_count > 0 {
                document.status = DocumentStatus::GraphReady;
            }
        }
    }
}

pub fn normalize_key(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

pub fn chunk_text(
    document_id: u64,
    text: &str,
    chunk_size: usize,
    overlap: usize,
) -> Vec<ChunkRecord> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let safe_chunk_size = chunk_size.max(48);
    let safe_overlap = overlap.min(safe_chunk_size / 3);
    let step = (safe_chunk_size - safe_overlap).max(1);

    let mut chunks = Vec::new();
    let mut start = 0usize;
    let mut ordinal = 0u32;
    let mut char_cursor = 0u32;

    while start < tokens.len() {
        let end = (start + safe_chunk_size).min(tokens.len());
        let text = tokens[start..end].join(" ");
        let token_count = (end - start) as u32;
        let chunk_len = text.len() as u32;
        chunks.push(ChunkRecord {
            id: 0,
            document_id,
            ordinal,
            text,
            token_count,
            offset_start: char_cursor,
            offset_end: char_cursor + chunk_len,
            previous_chunk_id: None,
            next_chunk_id: None,
        });
        char_cursor += chunk_len.saturating_add(1);
        ordinal += 1;
        start += step;
    }

    chunks
}

pub fn heuristic_extract_chunk_graph(text: &str) -> ExtractedChunkGraph {
    let mut entities = BTreeMap::<String, ExtractedEntity>::new();
    let mut relationships = Vec::new();

    for sentence in split_sentences(text) {
        let sentence_entities = extract_entities_from_sentence(sentence);
        for entity in &sentence_entities {
            entities
                .entry(normalize_key(&entity.name))
                .or_insert_with(|| entity.clone());
        }

        if sentence_entities.len() >= 2 {
            let relation = detect_relation_type(sentence);
            for pair in sentence_entities.windows(2) {
                if let [left, right] = pair {
                    let left_key = normalize_key(&left.name);
                    let right_key = normalize_key(&right.name);
                    if left_key != right_key {
                        relationships.push(ExtractedRelationship {
                            source_name: left.name.clone(),
                            target_name: right.name.clone(),
                            relation_type: relation.to_string(),
                            description: sentence.trim().to_string(),
                        });
                    }
                }
            }
        }
    }

    ExtractedChunkGraph {
        entities: entities.into_values().collect(),
        relationships,
    }
}

fn split_sentences(text: &str) -> Vec<&str> {
    text.split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

fn infer_entity_type(name: &str) -> String {
    if name.ends_with("Inc")
        || name.ends_with("Corp")
        || name.ends_with("Company")
        || name.ends_with("Amazon")
        || name.ends_with("Microsoft")
        || name.ends_with("Neo4j")
    {
        return "Organization".to_string();
    }

    if name.split_whitespace().count() >= 2 {
        return "Person".to_string();
    }

    "Concept".to_string()
}

fn extract_entities_from_sentence(sentence: &str) -> Vec<ExtractedEntity> {
    let cleaned = sentence.replace(|c: char| !c.is_ascii_alphanumeric() && c != ' ', " ");
    let mut current = Vec::<String>::new();
    let mut found = Vec::<ExtractedEntity>::new();
    let mut seen = HashSet::new();

    for token in cleaned.split_whitespace() {
        if is_entity_token(token) {
            current.push(token.to_string());
        } else if !current.is_empty() {
            push_entity_candidate(&mut found, &mut seen, &current);
            current.clear();
        }
    }

    if !current.is_empty() {
        push_entity_candidate(&mut found, &mut seen, &current);
    }

    found
}

fn push_entity_candidate(
    found: &mut Vec<ExtractedEntity>,
    seen: &mut HashSet<String>,
    parts: &[String],
) {
    let candidate = parts.join(" ");
    let key = normalize_key(&candidate);
    if key.len() < 3 || !seen.insert(key) {
        return;
    }

    found.push(ExtractedEntity {
        name: candidate.clone(),
        entity_type: infer_entity_type(&candidate),
        description: format!("Heuristically extracted from text: {candidate}"),
    });
}

fn is_entity_token(token: &str) -> bool {
    if STOPWORDS.contains(&token) {
        return false;
    }

    let first = token.chars().next().unwrap_or_default();
    first.is_ascii_uppercase() || token.chars().all(|c| c.is_ascii_uppercase())
}

fn detect_relation_type(sentence: &str) -> &'static str {
    let lowered = sentence.to_ascii_lowercase();
    RELATION_HINTS
        .iter()
        .find_map(|(needle, relation)| lowered.contains(needle).then_some(*relation))
        .unwrap_or("RELATED_TO")
}

fn build_communities(
    entity_ids: &[u64],
    relationships: &[(u64, u64, u64)],
) -> Vec<(Vec<u64>, Vec<u64>)> {
    let mut parent = BTreeMap::<u64, u64>::new();
    for entity_id in entity_ids {
        parent.insert(*entity_id, *entity_id);
    }

    for (_, source_id, target_id) in relationships {
        union(&mut parent, *source_id, *target_id);
    }

    let mut entity_groups = BTreeMap::<u64, Vec<u64>>::new();
    for entity_id in entity_ids {
        let root = find(&mut parent, *entity_id);
        entity_groups.entry(root).or_default().push(*entity_id);
    }

    let mut rel_groups = BTreeMap::<u64, Vec<u64>>::new();
    for (relationship_id, source_id, _) in relationships {
        let root = find(&mut parent, *source_id);
        rel_groups.entry(root).or_default().push(*relationship_id);
    }

    entity_groups
        .into_iter()
        .map(|(root, members)| (members, rel_groups.remove(&root).unwrap_or_default()))
        .collect()
}

fn find(parent: &mut BTreeMap<u64, u64>, node: u64) -> u64 {
    let current = *parent.get(&node).unwrap_or(&node);
    if current == node {
        node
    } else {
        let root = find(parent, current);
        parent.insert(node, root);
        root
    }
}

fn union(parent: &mut BTreeMap<u64, u64>, left: u64, right: u64) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root != right_root {
        parent.insert(right_root, left_root);
    }
}
