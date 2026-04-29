use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use hnsw_rs::{
    api::AnnT,
    prelude::{Distance, Hnsw},
};
use parking_lot::RwLock;

use crate::config::{LearningConfig, LearningIndexKind, QuantizationMode};

use super::learning::{
    LearningDocument, LearningFilter, RetrievalEvidence, SharedEmbeddingProvider,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StorageTier {
    Hot,
    Cold,
}

#[derive(Debug, Clone)]
struct SidecarEntry {
    document: LearningDocument,
    vector: Vec<f32>,
}

#[derive(Debug, Clone)]
enum ColdVector {
    Full(Vec<f32>),
    Scalar { data: Vec<u8>, min: f32, scale: f32 },
    Binary { bits: Vec<u8>, dimensions: usize },
    Product { centroids: Vec<f32> },
}

#[derive(Debug, Clone)]
struct ColdEntry {
    document: LearningDocument,
    vector: ColdVector,
}

#[derive(Debug, Clone)]
pub struct SidecarQuery {
    pub query: String,
    pub filter: LearningFilter,
    pub top_k: usize,
}

#[derive(Clone)]
pub struct SidecarMemoryIndex {
    provider: SharedEmbeddingProvider,
    config: LearningConfig,
    inner: Arc<RwLock<SidecarBackend>>,
}

impl std::fmt::Debug for SidecarMemoryIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidecarMemoryIndex").finish_non_exhaustive()
    }
}

enum SidecarBackend {
    Flat(FlatIndex),
    Hnsw(HnswIndex),
}

#[derive(Debug, Default)]
struct FlatIndex {
    hot_entries: HashMap<String, SidecarEntry>,
    cold_entries: HashMap<String, ColdEntry>,
}

struct HnswIndex {
    dimensions: usize,
    hot_entries: HashMap<String, SidecarEntry>,
    cold_entries: HashMap<String, ColdEntry>,
    id_to_idx: HashMap<String, usize>,
    idx_to_id: HashMap<usize, String>,
    next_idx: usize,
    graph: Hnsw<'static, f32, CosineDistance>,
}

#[derive(Debug, Clone, Copy)]
struct CosineDistance;

impl Distance<f32> for CosineDistance {
    fn eval(&self, a: &[f32], b: &[f32]) -> f32 {
        cosine_distance(a, b)
    }
}

impl SidecarMemoryIndex {
    pub fn new(config: LearningConfig, provider: SharedEmbeddingProvider) -> Self {
        let backend = match config.index_kind {
            LearningIndexKind::Flat => SidecarBackend::Flat(FlatIndex::default()),
            LearningIndexKind::Hnsw => {
                SidecarBackend::Hnsw(HnswIndex::new(config.embedding_dimensions))
            }
        };

        Self {
            provider,
            config,
            inner: Arc::new(RwLock::new(backend)),
        }
    }

    pub async fn upsert(&self, document: LearningDocument) -> Result<()> {
        let text = vec![document.text.clone()];
        let embedding = self
            .provider
            .embed_texts(&text)
            .await?
            .into_iter()
            .next()
            .unwrap_or_else(|| vec![0.0; self.provider.dimensions()]);
        let tier = self.storage_tier(&document);
        let hot_entry = SidecarEntry {
            document: document.clone(),
            vector: embedding.clone(),
        };
        let cold_entry = ColdEntry {
            document,
            vector: quantize(&embedding, self.config.cold_quantization.clone()),
        };

        let mut guard = self.inner.write();
        match &mut *guard {
            SidecarBackend::Flat(index) => index.upsert(hot_entry, cold_entry, tier),
            SidecarBackend::Hnsw(index) => index.upsert(hot_entry, cold_entry, tier)?,
        }

        Ok(())
    }

    pub fn rebuild(&self) -> Result<()> {
        let mut guard = self.inner.write();
        if let SidecarBackend::Hnsw(index) = &mut *guard {
            index.rebuild()?;
        }
        Ok(())
    }

    pub async fn search(&self, query: SidecarQuery) -> Result<Vec<RetrievalEvidence>> {
        let embedding = self
            .provider
            .embed_texts(&[query.query])
            .await?
            .into_iter()
            .next()
            .unwrap_or_else(|| vec![0.0; self.provider.dimensions()]);

        let guard = self.inner.read();
        let mut results = match &*guard {
            SidecarBackend::Flat(index) => index.search(&embedding, &query.filter, query.top_k),
            SidecarBackend::Hnsw(index) => index.search(&embedding, &query.filter, query.top_k),
        };

        results.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(query.top_k);

        Ok(results)
    }

    fn storage_tier(&self, document: &LearningDocument) -> StorageTier {
        let now_ms = current_time_ms();
        let hot_window_ms = self.config.hot_window_days as u64 * 24 * 60 * 60 * 1000;
        if now_ms.saturating_sub(document.created_at_ms) <= hot_window_ms {
            StorageTier::Hot
        } else {
            StorageTier::Cold
        }
    }
}

impl FlatIndex {
    fn upsert(&mut self, hot_entry: SidecarEntry, cold_entry: ColdEntry, tier: StorageTier) {
        self.hot_entries.remove(&hot_entry.document.id);
        self.cold_entries.remove(&cold_entry.document.id);
        match tier {
            StorageTier::Hot => {
                self.hot_entries
                    .insert(hot_entry.document.id.clone(), hot_entry);
            }
            StorageTier::Cold => {
                self.cold_entries
                    .insert(cold_entry.document.id.clone(), cold_entry);
            }
        }
    }

    fn search(
        &self,
        query: &[f32],
        filter: &LearningFilter,
        top_k: usize,
    ) -> Vec<RetrievalEvidence> {
        let mut results = Vec::new();
        for entry in self.hot_entries.values() {
            if matches_filter(&entry.document, filter) {
                results.push(RetrievalEvidence {
                    document: entry.document.clone(),
                    similarity: 1.0 - cosine_distance(query, &entry.vector),
                });
            }
        }
        for entry in self.cold_entries.values() {
            if matches_filter(&entry.document, filter) {
                results.push(RetrievalEvidence {
                    document: entry.document.clone(),
                    similarity: cold_similarity(&entry.vector, query),
                });
            }
        }
        results.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }
}

impl HnswIndex {
    fn new(dimensions: usize) -> Self {
        Self {
            dimensions,
            hot_entries: HashMap::new(),
            cold_entries: HashMap::new(),
            id_to_idx: HashMap::new(),
            idx_to_id: HashMap::new(),
            next_idx: 0,
            graph: Hnsw::new(16, 100_000, dimensions, 128, CosineDistance),
        }
    }

    fn upsert(
        &mut self,
        hot_entry: SidecarEntry,
        cold_entry: ColdEntry,
        tier: StorageTier,
    ) -> Result<()> {
        self.hot_entries.remove(&hot_entry.document.id);
        self.cold_entries.remove(&cold_entry.document.id);
        match tier {
            StorageTier::Hot => {
                self.hot_entries
                    .insert(hot_entry.document.id.clone(), hot_entry);
            }
            StorageTier::Cold => {
                self.cold_entries
                    .insert(cold_entry.document.id.clone(), cold_entry);
            }
        }
        self.rebuild()
    }

    fn rebuild(&mut self) -> Result<()> {
        let graph = Hnsw::new(16, 100_000, self.dimensions, 128, CosineDistance);
        let entries = self.hot_entries.values().cloned().collect::<Vec<_>>();
        self.graph = graph;
        self.id_to_idx.clear();
        self.idx_to_id.clear();
        self.next_idx = 0;

        for entry in entries {
            if entry.vector.len() != self.dimensions {
                return Err(anyhow::anyhow!(
                    "sidecar vector dimensions mismatch: expected {}, got {}",
                    self.dimensions,
                    entry.vector.len()
                ));
            }
            let idx = self.next_idx;
            self.next_idx += 1;
            self.graph.insert_data(&entry.vector, idx);
            self.id_to_idx.insert(entry.document.id.clone(), idx);
            self.idx_to_id.insert(idx, entry.document.id.clone());
        }
        Ok(())
    }

    fn search(
        &self,
        query: &[f32],
        filter: &LearningFilter,
        top_k: usize,
    ) -> Vec<RetrievalEvidence> {
        let mut results = self
            .graph
            .search(query, top_k.saturating_mul(3).max(4), 64)
            .into_iter()
            .filter_map(|neighbor| {
                let id = self.idx_to_id.get(&neighbor.d_id)?;
                let entry = self.hot_entries.get(id)?;
                if !matches_filter(&entry.document, filter) {
                    return None;
                }
                Some(RetrievalEvidence {
                    document: entry.document.clone(),
                    similarity: 1.0 - neighbor.distance,
                })
            })
            .collect::<Vec<_>>();

        for entry in self.cold_entries.values() {
            if matches_filter(&entry.document, filter) {
                results.push(RetrievalEvidence {
                    document: entry.document.clone(),
                    similarity: cold_similarity(&entry.vector, query),
                });
            }
        }

        results.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }
}

fn quantize(vector: &[f32], mode: QuantizationMode) -> ColdVector {
    match mode {
        QuantizationMode::None => ColdVector::Full(vector.to_vec()),
        QuantizationMode::Scalar => quantize_scalar(vector),
        QuantizationMode::Binary => quantize_binary(vector),
        QuantizationMode::Product => quantize_product(vector),
    }
}

fn quantize_scalar(vector: &[f32]) -> ColdVector {
    let min = vector.iter().copied().fold(f32::INFINITY, f32::min);
    let max = vector.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let scale = if (max - min).abs() < f32::EPSILON {
        1.0
    } else {
        (max - min) / 255.0
    };
    let data = vector
        .iter()
        .map(|value| ((value - min) / scale).round().clamp(0.0, 255.0) as u8)
        .collect::<Vec<_>>();
    ColdVector::Scalar { data, min, scale }
}

fn quantize_binary(vector: &[f32]) -> ColdVector {
    let dimensions = vector.len();
    let mut bits = vec![0u8; dimensions.div_ceil(8)];
    for (index, value) in vector.iter().enumerate() {
        if *value > 0.0 {
            bits[index / 8] |= 1 << (index % 8);
        }
    }
    ColdVector::Binary { bits, dimensions }
}

fn quantize_product(vector: &[f32]) -> ColdVector {
    let chunk = 8usize.max(vector.len() / 8).min(vector.len().max(1));
    let centroids = vector
        .chunks(chunk)
        .map(|slice| slice.iter().sum::<f32>() / slice.len() as f32)
        .collect::<Vec<_>>();
    ColdVector::Product { centroids }
}

fn cold_similarity(vector: &ColdVector, query: &[f32]) -> f32 {
    match vector {
        ColdVector::Full(values) => 1.0 - cosine_distance(values, query),
        ColdVector::Scalar { data, min, scale } => {
            let restored = data
                .iter()
                .map(|value| *min + *value as f32 * *scale)
                .collect::<Vec<_>>();
            1.0 - cosine_distance(&restored, query)
        }
        ColdVector::Binary { bits, dimensions } => {
            let query_bits = quantize_binary(query);
            if let ColdVector::Binary {
                bits: query_bits,
                dimensions: query_dimensions,
            } = query_bits
            {
                let compared = (*dimensions).min(query_dimensions);
                if compared == 0 {
                    return 0.0;
                }
                let mut different = 0u32;
                for (left, right) in bits.iter().zip(query_bits.iter()) {
                    different += (left ^ right).count_ones();
                }
                1.0 - (different as f32 / compared as f32)
            } else {
                0.0
            }
        }
        ColdVector::Product { centroids } => {
            let chunk = (query.len() / centroids.len().max(1)).max(1);
            let query_centroids = query
                .chunks(chunk)
                .map(|slice| slice.iter().sum::<f32>() / slice.len() as f32)
                .collect::<Vec<_>>();
            1.0 - cosine_distance(centroids, &query_centroids)
        }
    }
}

fn matches_filter(document: &LearningDocument, filter: &LearningFilter) -> bool {
    if let Some(session_id) = &filter.session_id {
        if &document.session_id != session_id && document.session_id != "global" {
            return false;
        }
    }

    if !filter.asset_kinds.is_empty() && !filter.asset_kinds.contains(&document.asset_kind) {
        return false;
    }

    filter
        .metadata
        .iter()
        .all(|(key, value)| document.metadata.get(key) == Some(value))
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 1.0;
    }

    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;

    for (lhs, rhs) in left.iter().zip(right) {
        dot += lhs * rhs;
        left_norm += lhs * lhs;
        right_norm += rhs * rhs;
    }

    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        return 1.0;
    }

    1.0 - (dot / (left_norm.sqrt() * right_norm.sqrt()))
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::{
        config::{LearningConfig, LearningIndexKind, QuantizationMode},
        memory::learning::{HashEmbeddingProvider, LearningAssetKind},
    };

    fn config(mode: QuantizationMode) -> LearningConfig {
        LearningConfig {
            enabled: true,
            sidecar_enabled: true,
            index_kind: LearningIndexKind::Hnsw,
            embedding_dimensions: 32,
            top_k: 4,
            gray_routing_ratio: 0.2,
            routing_takeover_threshold: 2.0,
            hot_window_days: 1,
            cold_quantization: mode,
        }
    }

    #[tokio::test]
    async fn sidecar_supports_rebuild_and_filtered_search() {
        let sidecar = SidecarMemoryIndex::new(
            config(QuantizationMode::Scalar),
            Arc::new(HashEmbeddingProvider::new(32)),
        );
        sidecar
            .upsert(LearningDocument {
                id: "skill-1".into(),
                session_id: "s1".into(),
                asset_kind: LearningAssetKind::Skill,
                text: "Use MCP scheduler after repeated validation failures".into(),
                score: 0.8,
                created_at_ms: current_time_ms(),
                metadata: HashMap::from([(String::from("source"), String::from("ops"))]),
            })
            .await
            .expect("insert");

        sidecar.rebuild().expect("rebuild");

        let results = sidecar
            .search(SidecarQuery {
                query: "validation scheduler".into(),
                filter: LearningFilter {
                    session_id: Some("s1".into()),
                    asset_kinds: vec![LearningAssetKind::Skill],
                    metadata: HashMap::from([(String::from("source"), String::from("ops"))]),
                },
                top_k: 3,
            })
            .await
            .expect("search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document.id, "skill-1");
    }

    #[tokio::test]
    async fn sidecar_uses_cold_quantized_tier() {
        let sidecar = SidecarMemoryIndex::new(
            config(QuantizationMode::Binary),
            Arc::new(HashEmbeddingProvider::new(32)),
        );
        sidecar
            .upsert(LearningDocument {
                id: "old-witness".into(),
                session_id: "s1".into(),
                asset_kind: LearningAssetKind::WitnessLog,
                text: "binary quantized cold memory for old routing decisions".into(),
                score: 0.2,
                created_at_ms: current_time_ms().saturating_sub(3 * 24 * 60 * 60 * 1000),
                metadata: HashMap::new(),
            })
            .await
            .expect("insert");

        let results = sidecar
            .search(SidecarQuery {
                query: "old routing decisions".into(),
                filter: LearningFilter {
                    session_id: Some("s1".into()),
                    asset_kinds: vec![LearningAssetKind::WitnessLog],
                    metadata: HashMap::new(),
                },
                top_k: 2,
            })
            .await
            .expect("search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document.id, "old-witness");
    }
}
