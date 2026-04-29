use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct VectorPoint {
    pub id: String,
    pub text: String,
    pub embedding: Vec<f32>,
}

pub trait VectorStore: Send + Sync {
    fn upsert(&mut self, id: String, text: String);
    fn query(&self, text: &str, top_k: usize) -> Vec<(String, f32)>;
}

#[derive(Debug, Default)]
pub struct InMemoryVectorStore {
    points: HashMap<String, VectorPoint>,
}

impl VectorStore for InMemoryVectorStore {
    fn upsert(&mut self, id: String, text: String) {
        let emb = embed(&text);
        self.points.insert(
            id.clone(),
            VectorPoint {
                id,
                text,
                embedding: emb,
            },
        );
    }

    fn query(&self, text: &str, top_k: usize) -> Vec<(String, f32)> {
        let q = embed(text);
        let mut scored: Vec<(String, f32)> = self
            .points
            .values()
            .map(|p| (p.id.clone(), cosine(&q, &p.embedding)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(top_k).collect()
    }
}

fn embed(text: &str) -> Vec<f32> {
    let mut v = vec![0.0_f32; 26];
    for c in text.chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_lowercase() {
            let idx = (lc as u8 - b'a') as usize;
            v[idx] += 1.0;
        }
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut s = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        s += x * y;
    }
    s
}
