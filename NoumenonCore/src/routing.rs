use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Result, anyhow};

pub trait LlmBackend: Send + Sync {
    fn name(&self) -> &str;
    fn infer(&self, input: &str) -> Result<String>;
}

#[derive(Debug, Clone, Copy)]
pub enum RoutingStrategy {
    Sequential,
    Pinned,
}

pub struct ModelRouter {
    strategy: RoutingStrategy,
    pinned: Option<String>,
    cursor: Mutex<usize>,
}

impl ModelRouter {
    pub fn sequential() -> Self {
        Self {
            strategy: RoutingStrategy::Sequential,
            pinned: None,
            cursor: Mutex::new(0),
        }
    }

    pub fn pinned(model: impl Into<String>) -> Self {
        Self {
            strategy: RoutingStrategy::Pinned,
            pinned: Some(model.into()),
            cursor: Mutex::new(0),
        }
    }

    pub fn select(&self, candidates: &[String]) -> Result<String> {
        if candidates.is_empty() {
            return Err(anyhow!("no candidate models available"));
        }
        match self.strategy {
            RoutingStrategy::Pinned => {
                let p = self
                    .pinned
                    .clone()
                    .ok_or_else(|| anyhow!("pinned model missing"))?;
                if candidates.iter().any(|c| c == &p) {
                    Ok(p)
                } else {
                    Err(anyhow!("pinned model not found in candidates"))
                }
            }
            RoutingStrategy::Sequential => {
                let mut cur = self
                    .cursor
                    .lock()
                    .map_err(|_| anyhow!("router cursor lock poisoned"))?;
                let idx = *cur % candidates.len();
                *cur = cur.saturating_add(1);
                Ok(candidates[idx].clone())
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct EchoModelBackend {
    model_name: String,
}

impl EchoModelBackend {
    pub fn new(model_name: impl Into<String>) -> Self {
        Self {
            model_name: model_name.into(),
        }
    }
}

impl LlmBackend for EchoModelBackend {
    fn name(&self) -> &str {
        &self.model_name
    }

    fn infer(&self, input: &str) -> Result<String> {
        Ok(format!("model={} output={}", self.model_name, input))
    }
}

pub fn parse_model_candidates(
    local_constraints: &HashMap<String, String>,
    all_models: Vec<String>,
) -> Vec<String> {
    if let Some(raw) = local_constraints.get("model_candidates") {
        let v: Vec<String> = raw
            .split(',')
            .map(|x| x.trim())
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    all_models
}
