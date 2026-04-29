use std::{
    collections::{BTreeMap, HashMap, hash_map::DefaultHasher},
    env,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use parking_lot::RwLock;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::adaptive_framework::{PromptTemplateProfile, build_prompt_template_bundle};
use crate::config::ProviderConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationSignal {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationProposal {
    pub title: String,
    pub change_target: String,
    pub hypothesis: String,
    pub expected_gain: String,
    pub risk: String,
    pub patch_outline: Vec<String>,
    pub evaluation_focus: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptPolicyOverlay {
    pub preferred_model: Option<String>,
    pub directives: Vec<String>,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRouteStage {
    Screening,
    Reasoning,
    Judge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRouteDecision {
    pub stage: ProviderRouteStage,
    pub model: String,
    pub rationale: String,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderChatTrace {
    pub response: LlmResponse,
    pub route: ProviderRouteDecision,
    pub cache_key: String,
    #[serde(default)]
    pub adapter: Option<ApiAdapterTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiAdapterKind {
    OpenAiCompatible,
    AnthropicCompatible,
    McpAdapter,
    Stub,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetryDisposition {
    NotRetried,
    RetriedAndSucceeded,
    RetriedAndFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    None,
    RateLimit,
    Authentication,
    Permission,
    InvalidRequest,
    UpstreamUnavailable,
    Timeout,
    Network,
    Decode,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UsageStats {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorTaxonomy {
    pub category: ErrorCategory,
    pub code: Option<String>,
    pub http_status: Option<u16>,
    pub retryable: bool,
    pub message: Option<String>,
}

impl Default for ErrorTaxonomy {
    fn default() -> Self {
        Self {
            category: ErrorCategory::None,
            code: None,
            http_status: None,
            retryable: false,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiAdapterTrace {
    pub adapter: ApiAdapterKind,
    pub usage: UsageStats,
    pub attempts: u32,
    pub retry: RetryDisposition,
    pub error: ErrorTaxonomy,
}

impl ApiAdapterTrace {
    fn unknown() -> Self {
        Self {
            adapter: ApiAdapterKind::Unknown,
            usage: UsageStats::default(),
            attempts: 1,
            retry: RetryDisposition::NotRetried,
            error: ErrorTaxonomy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAdapterOutcome {
    pub response: LlmResponse,
    pub trace: ApiAdapterTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProvenance {
    pub provider_name: String,
    pub source: String,
    pub version_ref: String,
    pub trusted: bool,
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, messages: &[ChatMessage], model: &str) -> Result<LlmResponse>;

    async fn chat_with_adapter(
        &self,
        messages: &[ChatMessage],
        model: &str,
    ) -> Result<ProviderAdapterOutcome> {
        let response = self.chat(messages, model).await?;
        Ok(ProviderAdapterOutcome {
            response,
            trace: ApiAdapterTrace::unknown(),
        })
    }
}

#[async_trait]
pub trait Embedder: Send + Sync {
    fn name(&self) -> &str;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

#[async_trait]
pub trait VectorStore: Send + Sync {
    fn name(&self) -> &str;
    async fn upsert(
        &self,
        id: &str,
        vector: &[f32],
        metadata: &BTreeMap<String, String>,
    ) -> Result<()>;
    async fn query(&self, vector: &[f32], top_k: usize) -> Result<Vec<String>>;
}

#[async_trait]
pub trait GraphStore: Send + Sync {
    fn name(&self) -> &str;
    async fn upsert_edge(&self, source: &str, relation: &str, target: &str) -> Result<()>;
    async fn neighbors(&self, node: &str) -> Result<Vec<String>>;
}

#[async_trait]
pub trait Reranker: Send + Sync {
    fn name(&self) -> &str;
    async fn rerank(&self, query: &str, candidates: &[String]) -> Result<Vec<String>>;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FactoryComponentKind {
    Llm,
    Embedder,
    Vector,
    Graph,
    Reranker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactoryArtifact {
    pub artifact_id: String,
    pub kind: FactoryComponentKind,
    pub provider: String,
    pub version: String,
    pub source: String,
    pub active: bool,
    pub verified: bool,
    pub trusted: bool,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct FactoryRegistry {
    llm_factories: HashMap<
        String,
        Arc<
            dyn Fn(&ProviderConfig) -> Result<(Arc<dyn Provider>, ProviderProvenance)>
                + Send
                + Sync,
        >,
    >,
    embedder_factories: HashMap<String, Arc<dyn Fn() -> Result<Arc<dyn Embedder>> + Send + Sync>>,
    vector_factories: HashMap<String, Arc<dyn Fn() -> Result<Arc<dyn VectorStore>> + Send + Sync>>,
    graph_factories: HashMap<String, Arc<dyn Fn() -> Result<Arc<dyn GraphStore>> + Send + Sync>>,
    reranker_factories: HashMap<String, Arc<dyn Fn() -> Result<Arc<dyn Reranker>> + Send + Sync>>,
    artifacts: HashMap<String, FactoryArtifact>,
}

impl Clone for FactoryRegistry {
    fn clone(&self) -> Self {
        Self {
            llm_factories: self.llm_factories.clone(),
            embedder_factories: self.embedder_factories.clone(),
            vector_factories: self.vector_factories.clone(),
            graph_factories: self.graph_factories.clone(),
            reranker_factories: self.reranker_factories.clone(),
            artifacts: self.artifacts.clone(),
        }
    }
}

#[derive(Debug)]
pub struct StubProvider {
    name: String,
}

impl StubProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl Provider for StubProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, messages: &[ChatMessage], model: &str) -> Result<LlmResponse> {
        let last = messages.iter().rev().find(|msg| msg.role == "user");
        let content = last.map(|msg| format!("[provider:{}:{}] {}", self.name, model, msg.content));
        Ok(LlmResponse {
            content,
            tool_calls: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct HashEmbedder {
    name: String,
    dimensions: usize,
}

impl HashEmbedder {
    fn new(name: impl Into<String>, dimensions: usize) -> Self {
        Self {
            name: name.into(),
            dimensions: dimensions.max(8),
        }
    }
}

#[async_trait]
impl Embedder for HashEmbedder {
    fn name(&self) -> &str {
        &self.name
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for text in texts {
            let mut vector = vec![0.0_f32; self.dimensions];
            for (index, byte) in text.as_bytes().iter().enumerate() {
                let slot = index % self.dimensions;
                vector[slot] += (*byte as f32 / 255.0) * ((index % 17 + 1) as f32 / 17.0);
            }
            let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
            if norm > 0.0 {
                for value in &mut vector {
                    *value /= norm;
                }
            }
            vectors.push(vector);
        }
        Ok(vectors)
    }
}

#[derive(Debug)]
struct InMemoryVectorStore {
    name: String,
    entries: Arc<RwLock<HashMap<String, (Vec<f32>, BTreeMap<String, String>)>>>,
}

impl InMemoryVectorStore {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl VectorStore for InMemoryVectorStore {
    fn name(&self) -> &str {
        &self.name
    }

    async fn upsert(
        &self,
        id: &str,
        vector: &[f32],
        metadata: &BTreeMap<String, String>,
    ) -> Result<()> {
        self.entries
            .write()
            .insert(id.to_string(), (vector.to_vec(), metadata.clone()));
        Ok(())
    }

    async fn query(&self, vector: &[f32], top_k: usize) -> Result<Vec<String>> {
        let mut scored = self
            .entries
            .read()
            .iter()
            .map(|(id, (candidate, _))| {
                let score = cosine_similarity(vector, candidate);
                (id.clone(), score)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(scored.into_iter().take(top_k).map(|(id, _)| id).collect())
    }
}

#[derive(Debug)]
struct InMemoryGraphStore {
    name: String,
    edges: Arc<RwLock<HashMap<String, Vec<(String, String)>>>>,
}

impl InMemoryGraphStore {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            edges: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl GraphStore for InMemoryGraphStore {
    fn name(&self) -> &str {
        &self.name
    }

    async fn upsert_edge(&self, source: &str, relation: &str, target: &str) -> Result<()> {
        let mut guard = self.edges.write();
        let node_edges = guard.entry(source.to_string()).or_default();
        if !node_edges
            .iter()
            .any(|(rel, node)| rel == relation && node == target)
        {
            node_edges.push((relation.to_string(), target.to_string()));
        }
        Ok(())
    }

    async fn neighbors(&self, node: &str) -> Result<Vec<String>> {
        Ok(self
            .edges
            .read()
            .get(node)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|(relation, target)| format!("{relation}:{target}"))
            .collect())
    }
}

#[derive(Debug)]
struct LexicalReranker {
    name: String,
}

impl LexicalReranker {
    fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl Reranker for LexicalReranker {
    fn name(&self) -> &str {
        &self.name
    }

    async fn rerank(&self, query: &str, candidates: &[String]) -> Result<Vec<String>> {
        let query_terms = query
            .to_ascii_lowercase()
            .split_whitespace()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut scored = candidates
            .iter()
            .map(|candidate| {
                let lowered = candidate.to_ascii_lowercase();
                let score = query_terms
                    .iter()
                    .filter(|term| lowered.contains(term.as_str()))
                    .count();
                (candidate.clone(), score)
            })
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| right.1.cmp(&left.1));
        Ok(scored.into_iter().map(|(candidate, _)| candidate).collect())
    }
}

impl FactoryRegistry {
    pub fn with_defaults(config: &ProviderConfig) -> Self {
        let mut registry = Self::default();

        registry.register_llm_factory(
            "openai-compatible",
            FactoryArtifact {
                artifact_id: "factory:llm:openai-compatible:v1".into(),
                kind: FactoryComponentKind::Llm,
                provider: "openai-compatible".into(),
                version: "v1".into(),
                source: config.api_base_url.clone(),
                active: true,
                verified: true,
                trusted: config.api_base_url.starts_with("https://"),
                metadata: BTreeMap::from([("surface".into(), "provider-api".into())]),
            },
            Arc::new(|cfg| {
                let provider = OpenAiCompatibleProvider::try_new(cfg)?;
                Ok((
                    Arc::new(provider) as Arc<dyn Provider>,
                    ProviderProvenance {
                        provider_name: "openai-compatible".into(),
                        source: cfg.api_base_url.clone(),
                        version_ref: "openai-compatible-http".into(),
                        trusted: cfg.api_base_url.starts_with("https://"),
                    },
                ))
            }),
        );

        registry.register_llm_factory(
            "anthropic-compatible",
            FactoryArtifact {
                artifact_id: "factory:llm:anthropic-compatible:v1".into(),
                kind: FactoryComponentKind::Llm,
                provider: "anthropic-compatible".into(),
                version: "v1".into(),
                source: config.api_base_url.clone(),
                active: true,
                verified: true,
                trusted: config.api_base_url.starts_with("https://"),
                metadata: BTreeMap::from([("surface".into(), "provider-api".into())]),
            },
            Arc::new(|cfg| {
                let provider = AnthropicCompatibleProvider::try_new(cfg)?;
                Ok((
                    Arc::new(provider) as Arc<dyn Provider>,
                    ProviderProvenance {
                        provider_name: "anthropic-compatible".into(),
                        source: cfg.api_base_url.clone(),
                        version_ref: "anthropic-compatible-http".into(),
                        trusted: cfg.api_base_url.starts_with("https://"),
                    },
                ))
            }),
        );

        registry.register_llm_factory(
            "stub-llm",
            FactoryArtifact {
                artifact_id: "factory:llm:stub:v1".into(),
                kind: FactoryComponentKind::Llm,
                provider: "stub-llm".into(),
                version: "v1".into(),
                source: "builtin".into(),
                active: true,
                verified: true,
                trusted: true,
                metadata: BTreeMap::from([("surface".into(), "stub".into())]),
            },
            Arc::new(|_| {
                Ok((
                    Arc::new(StubProvider::new("stub-llm")) as Arc<dyn Provider>,
                    ProviderProvenance {
                        provider_name: "stub-llm".into(),
                        source: "builtin".into(),
                        version_ref: "stub".into(),
                        trusted: true,
                    },
                ))
            }),
        );

        registry.register_embedder_factory(
            "hash-embedder",
            FactoryArtifact {
                artifact_id: "factory:embedder:hash:v1".into(),
                kind: FactoryComponentKind::Embedder,
                provider: "hash-embedder".into(),
                version: "v1".into(),
                source: "builtin".into(),
                active: true,
                verified: true,
                trusted: true,
                metadata: BTreeMap::new(),
            },
            Arc::new(|| Ok(Arc::new(HashEmbedder::new("hash-embedder", 256)) as Arc<dyn Embedder>)),
        );

        registry.register_vector_factory(
            "in-memory-vector",
            FactoryArtifact {
                artifact_id: "factory:vector:in-memory:v1".into(),
                kind: FactoryComponentKind::Vector,
                provider: "in-memory-vector".into(),
                version: "v1".into(),
                source: "builtin".into(),
                active: true,
                verified: true,
                trusted: true,
                metadata: BTreeMap::new(),
            },
            Arc::new(|| {
                Ok(Arc::new(InMemoryVectorStore::new("in-memory-vector")) as Arc<dyn VectorStore>)
            }),
        );

        registry.register_graph_factory(
            "in-memory-graph",
            FactoryArtifact {
                artifact_id: "factory:graph:in-memory:v1".into(),
                kind: FactoryComponentKind::Graph,
                provider: "in-memory-graph".into(),
                version: "v1".into(),
                source: "builtin".into(),
                active: true,
                verified: true,
                trusted: true,
                metadata: BTreeMap::new(),
            },
            Arc::new(|| {
                Ok(Arc::new(InMemoryGraphStore::new("in-memory-graph")) as Arc<dyn GraphStore>)
            }),
        );

        registry.register_reranker_factory(
            "lexical-reranker",
            FactoryArtifact {
                artifact_id: "factory:reranker:lexical:v1".into(),
                kind: FactoryComponentKind::Reranker,
                provider: "lexical-reranker".into(),
                version: "v1".into(),
                source: "builtin".into(),
                active: true,
                verified: true,
                trusted: true,
                metadata: BTreeMap::new(),
            },
            Arc::new(
                || Ok(Arc::new(LexicalReranker::new("lexical-reranker")) as Arc<dyn Reranker>),
            ),
        );

        for server in &config.mcp_servers {
            let key = format!("mcp:{server}");
            let server_name = server.clone();
            registry.register_llm_factory(
                &key,
                FactoryArtifact {
                    artifact_id: format!("factory:llm:mcp:{server}:v1"),
                    kind: FactoryComponentKind::Llm,
                    provider: key.clone(),
                    version: "v1".into(),
                    source: format!("mcp://{server}"),
                    active: true,
                    verified: true,
                    trusted: true,
                    metadata: BTreeMap::from([("server".into(), server.clone())]),
                },
                Arc::new(move |_| {
                    Ok((
                        Arc::new(McpProviderAdapter::new(server_name.clone())) as Arc<dyn Provider>,
                        ProviderProvenance {
                            provider_name: format!("mcp:{}", server_name),
                            source: format!("mcp://{}", server_name),
                            version_ref: "adapter-v1".into(),
                            trusted: true,
                        },
                    ))
                }),
            );
        }

        registry
    }

    pub fn register_llm_factory(
        &mut self,
        name: &str,
        artifact: FactoryArtifact,
        constructor: Arc<
            dyn Fn(&ProviderConfig) -> Result<(Arc<dyn Provider>, ProviderProvenance)>
                + Send
                + Sync,
        >,
    ) {
        self.artifacts.insert(name.to_string(), artifact);
        self.llm_factories.insert(name.to_string(), constructor);
    }

    pub fn register_embedder_factory(
        &mut self,
        name: &str,
        artifact: FactoryArtifact,
        constructor: Arc<dyn Fn() -> Result<Arc<dyn Embedder>> + Send + Sync>,
    ) {
        self.artifacts.insert(name.to_string(), artifact);
        self.embedder_factories
            .insert(name.to_string(), constructor);
    }

    pub fn register_vector_factory(
        &mut self,
        name: &str,
        artifact: FactoryArtifact,
        constructor: Arc<dyn Fn() -> Result<Arc<dyn VectorStore>> + Send + Sync>,
    ) {
        self.artifacts.insert(name.to_string(), artifact);
        self.vector_factories.insert(name.to_string(), constructor);
    }

    pub fn register_graph_factory(
        &mut self,
        name: &str,
        artifact: FactoryArtifact,
        constructor: Arc<dyn Fn() -> Result<Arc<dyn GraphStore>> + Send + Sync>,
    ) {
        self.artifacts.insert(name.to_string(), artifact);
        self.graph_factories.insert(name.to_string(), constructor);
    }

    pub fn register_reranker_factory(
        &mut self,
        name: &str,
        artifact: FactoryArtifact,
        constructor: Arc<dyn Fn() -> Result<Arc<dyn Reranker>> + Send + Sync>,
    ) {
        self.artifacts.insert(name.to_string(), artifact);
        self.reranker_factories
            .insert(name.to_string(), constructor);
    }

    pub fn create_llm(
        &self,
        name: &str,
        config: &ProviderConfig,
    ) -> Result<(Arc<dyn Provider>, ProviderProvenance)> {
        let constructor = self
            .llm_factories
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("llm factory '{name}' not registered"))?;
        constructor(config)
    }

    pub fn create_embedder(&self, name: &str) -> Result<Arc<dyn Embedder>> {
        let constructor = self
            .embedder_factories
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("embedder factory '{name}' not registered"))?;
        constructor()
    }

    pub fn create_vector_store(&self, name: &str) -> Result<Arc<dyn VectorStore>> {
        let constructor = self
            .vector_factories
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("vector factory '{name}' not registered"))?;
        constructor()
    }

    pub fn create_graph_store(&self, name: &str) -> Result<Arc<dyn GraphStore>> {
        let constructor = self
            .graph_factories
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("graph factory '{name}' not registered"))?;
        constructor()
    }

    pub fn create_reranker(&self, name: &str) -> Result<Arc<dyn Reranker>> {
        let constructor = self
            .reranker_factories
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("reranker factory '{name}' not registered"))?;
        constructor()
    }

    pub fn artifact(&self, name: &str) -> Option<&FactoryArtifact> {
        self.artifacts.get(name)
    }

    pub fn artifacts(&self) -> Vec<FactoryArtifact> {
        self.artifacts.values().cloned().collect()
    }

    pub fn trusted_active_artifacts(&self, kind: FactoryComponentKind) -> Vec<FactoryArtifact> {
        self.artifacts
            .values()
            .filter(|artifact| {
                artifact.kind == kind && artifact.active && artifact.verified && artifact.trusted
            })
            .cloned()
            .collect()
    }

    pub fn is_trusted_active(&self, name: &str) -> bool {
        self.artifact(name)
            .is_some_and(|artifact| artifact.active && artifact.verified && artifact.trusted)
    }
}

fn render_error_category(category: &ErrorCategory) -> &'static str {
    match category {
        ErrorCategory::None => "none",
        ErrorCategory::RateLimit => "rate_limit",
        ErrorCategory::Authentication => "authentication",
        ErrorCategory::Permission => "permission",
        ErrorCategory::InvalidRequest => "invalid_request",
        ErrorCategory::UpstreamUnavailable => "upstream_unavailable",
        ErrorCategory::Timeout => "timeout",
        ErrorCategory::Network => "network",
        ErrorCategory::Decode => "decode",
        ErrorCategory::Unknown => "unknown",
    }
}

fn classify_http_error(status: u16, message: Option<String>) -> ErrorTaxonomy {
    let category = match status {
        400 | 404 | 422 => ErrorCategory::InvalidRequest,
        401 => ErrorCategory::Authentication,
        403 => ErrorCategory::Permission,
        408 => ErrorCategory::Timeout,
        429 => ErrorCategory::RateLimit,
        500..=599 => ErrorCategory::UpstreamUnavailable,
        _ => ErrorCategory::Unknown,
    };
    let retryable = matches!(status, 408 | 429 | 500..=599);
    ErrorTaxonomy {
        category,
        code: Some(format!("http_{status}")),
        http_status: Some(status),
        retryable,
        message,
    }
}

fn classify_transport_error(error: &reqwest::Error) -> ErrorTaxonomy {
    let category = if error.is_timeout() {
        ErrorCategory::Timeout
    } else if error.is_connect() || error.is_request() {
        ErrorCategory::Network
    } else if error.is_decode() {
        ErrorCategory::Decode
    } else {
        ErrorCategory::Unknown
    };
    let retryable = matches!(category, ErrorCategory::Timeout | ErrorCategory::Network);
    ErrorTaxonomy {
        category,
        code: Some("transport_error".to_string()),
        http_status: None,
        retryable,
        message: Some(error.to_string()),
    }
}
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let size = a.len().min(b.len());
    let mut dot = 0.0_f32;
    let mut norm_a = 0.0_f32;
    let mut norm_b = 0.0_f32;
    for idx in 0..size {
        dot += a[idx] * b[idx];
        norm_a += a[idx] * a[idx];
        norm_b += b[idx] * b[idx];
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiChatRequest {
    model: String,
    messages: Vec<OpenAiChatMessage>,
    temperature: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    #[serde(default)]
    total_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiChoice {
    message: OpenAiAssistantMessage,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiAssistantMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCall>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(default)]
    function: Option<OpenAiFunctionCall>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicChatRequest {
    model: String,
    #[serde(rename = "max_tokens")]
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicChatResponse {
    #[serde(default)]
    content: Vec<AnthropicTextBlock>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicTextBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

#[derive(Debug)]
pub struct AnthropicCompatibleProvider {
    name: String,
    client: reqwest::Client,
    base_url: String,
}

impl AnthropicCompatibleProvider {
    pub fn try_new(config: &ProviderConfig) -> Result<Self> {
        let api_key = env::var(&config.api_key_env)
            .with_context(|| format!("missing provider api key env {}", config.api_key_env))?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&api_key).context("invalid anthropic x-api-key header")?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static("2023-06-01"),
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .context("failed to build anthropic-compatible client")?;

        Ok(Self {
            name: "anthropic-compatible".into(),
            client,
            base_url: config.api_base_url.trim_end_matches('/').into(),
        })
    }
}

#[async_trait]
impl Provider for AnthropicCompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, messages: &[ChatMessage], model: &str) -> Result<LlmResponse> {
        Ok(self.chat_with_adapter(messages, model).await?.response)
    }

    async fn chat_with_adapter(
        &self,
        messages: &[ChatMessage],
        model: &str,
    ) -> Result<ProviderAdapterOutcome> {
        let request = AnthropicChatRequest {
            model: model.to_string(),
            max_tokens: 1024,
            messages: messages
                .iter()
                .map(|message| AnthropicMessage {
                    role: if message.role == "assistant" {
                        "assistant".to_string()
                    } else {
                        "user".to_string()
                    },
                    content: message.content.clone(),
                })
                .collect(),
        };

        let mut attempts: u32 = 0;
        let mut last_error: Option<ErrorTaxonomy> = None;
        let max_attempts: u32 = 2;
        loop {
            attempts += 1;
            let response_result = self
                .client
                .post(format!("{}/messages", self.base_url))
                .json(&request)
                .send()
                .await;

            let response = match response_result {
                Ok(response) => response,
                Err(error) => {
                    let taxonomy = classify_transport_error(&error);
                    let retryable = taxonomy.retryable && attempts < max_attempts;
                    last_error = Some(taxonomy.clone());
                    if retryable {
                        tokio::time::sleep(Duration::from_millis(300 * attempts as u64)).await;
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "provider request failed [{}]: {}",
                        render_error_category(&taxonomy.category),
                        error
                    ));
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<body unavailable>".to_string());
                let taxonomy = classify_http_error(status, Some(body));
                let retryable = taxonomy.retryable && attempts < max_attempts;
                last_error = Some(taxonomy.clone());
                if retryable {
                    tokio::time::sleep(Duration::from_millis(350 * attempts as u64)).await;
                    continue;
                }
                return Err(anyhow::anyhow!(
                    "provider returned non-success status {} [{}]",
                    status,
                    render_error_category(&taxonomy.category)
                ));
            }

            let body = response
                .json::<AnthropicChatResponse>()
                .await
                .context("failed to decode anthropic-compatible response")?;

            let content = body
                .content
                .iter()
                .find(|block| block.block_type == "text")
                .and_then(|block| block.text.clone());

            let usage = body
                .usage
                .as_ref()
                .map(|usage| UsageStats {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_tokens: match (usage.input_tokens, usage.output_tokens) {
                        (Some(input), Some(output)) => Some(input + output),
                        _ => None,
                    },
                })
                .unwrap_or_default();

            return Ok(ProviderAdapterOutcome {
                response: LlmResponse {
                    content,
                    tool_calls: Vec::new(),
                },
                trace: ApiAdapterTrace {
                    adapter: ApiAdapterKind::AnthropicCompatible,
                    usage,
                    attempts,
                    retry: if attempts > 1 {
                        RetryDisposition::RetriedAndSucceeded
                    } else {
                        RetryDisposition::NotRetried
                    },
                    error: last_error.unwrap_or_default(),
                },
            });
        }
    }
}
#[derive(Debug)]
pub struct OpenAiCompatibleProvider {
    name: String,
    client: reqwest::Client,
    base_url: String,
}

impl OpenAiCompatibleProvider {
    pub fn try_new(config: &ProviderConfig) -> Result<Self> {
        let api_key = env::var(&config.api_key_env)
            .with_context(|| format!("missing provider api key env {}", config.api_key_env))?;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_key}"))
                .context("invalid provider api key for authorization header")?,
        );
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .context("failed to build openai-compatible client")?;

        Ok(Self {
            name: "openai-compatible".into(),
            client,
            base_url: config.api_base_url.trim_end_matches('/').into(),
        })
    }
}

#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, messages: &[ChatMessage], model: &str) -> Result<LlmResponse> {
        Ok(self.chat_with_adapter(messages, model).await?.response)
    }

    async fn chat_with_adapter(
        &self,
        messages: &[ChatMessage],
        model: &str,
    ) -> Result<ProviderAdapterOutcome> {
        let request = OpenAiChatRequest {
            model: model.to_string(),
            messages: messages
                .iter()
                .map(|message| OpenAiChatMessage {
                    role: message.role.clone(),
                    content: message.content.clone(),
                })
                .collect(),
            temperature: 0.2,
        };

        let mut attempts: u32 = 0;
        let mut last_error: Option<ErrorTaxonomy> = None;
        let max_attempts: u32 = 2;
        loop {
            attempts += 1;
            let response_result = self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .json(&request)
                .send()
                .await;

            let response = match response_result {
                Ok(response) => response,
                Err(error) => {
                    let taxonomy = classify_transport_error(&error);
                    let retryable = taxonomy.retryable && attempts < max_attempts;
                    last_error = Some(taxonomy.clone());
                    if retryable {
                        tokio::time::sleep(Duration::from_millis(300 * attempts as u64)).await;
                        continue;
                    }
                    return Err(anyhow::anyhow!(
                        "provider request failed [{}]: {}",
                        render_error_category(&taxonomy.category),
                        error
                    ));
                }
            };

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "<body unavailable>".to_string());
                let taxonomy = classify_http_error(status, Some(body));
                let retryable = taxonomy.retryable && attempts < max_attempts;
                last_error = Some(taxonomy.clone());
                if retryable {
                    tokio::time::sleep(Duration::from_millis(350 * attempts as u64)).await;
                    continue;
                }
                return Err(anyhow::anyhow!(
                    "provider returned non-success status {} [{}]",
                    status,
                    render_error_category(&taxonomy.category)
                ));
            }

            let body = response
                .json::<OpenAiChatResponse>()
                .await
                .context("failed to decode provider response")?;
            let usage = body
                .usage
                .as_ref()
                .map(|usage| UsageStats {
                    input_tokens: usage.prompt_tokens,
                    output_tokens: usage.completion_tokens,
                    total_tokens: usage.total_tokens,
                })
                .unwrap_or_default();
            let choice = body
                .choices
                .into_iter()
                .next()
                .context("provider returned no choices")?;

            return Ok(ProviderAdapterOutcome {
                response: LlmResponse {
                    content: choice.message.content,
                    tool_calls: choice
                        .message
                        .tool_calls
                        .into_iter()
                        .filter_map(|call| {
                            call.function.map(|function| ToolCall {
                                id: call.id,
                                name: function.name,
                                arguments: function.arguments,
                            })
                        })
                        .collect(),
                },
                trace: ApiAdapterTrace {
                    adapter: ApiAdapterKind::OpenAiCompatible,
                    usage,
                    attempts,
                    retry: if attempts > 1 {
                        RetryDisposition::RetriedAndSucceeded
                    } else {
                        RetryDisposition::NotRetried
                    },
                    error: last_error.unwrap_or_default(),
                },
            });
        }
    }
}

#[derive(Debug)]
pub struct McpProviderAdapter {
    name: String,
    server: String,
}

impl McpProviderAdapter {
    pub fn new(server: impl Into<String>) -> Self {
        let server = server.into();
        Self {
            name: format!("mcp:{server}"),
            server,
        }
    }
}

#[async_trait]
impl Provider for McpProviderAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn chat(&self, messages: &[ChatMessage], model: &str) -> Result<LlmResponse> {
        let last = messages.iter().rev().find(|msg| msg.role == "user");
        let content =
            last.map(|msg| format!("[mcp-provider:{}:{}] {}", self.server, model, msg.content));
        Ok(LlmResponse {
            content,
            tool_calls: Vec::new(),
        })
    }
}

#[derive(Clone)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
    provider_provenance: HashMap<String, ProviderProvenance>,
    default_provider: String,
    pub default_model: String,
    screening_model: String,
    reasoning_model: String,
    judge_model: String,
    enable_tiered_routing: bool,
    prompt_cache_capacity: usize,
    prompt_cache: Arc<RwLock<HashMap<String, LlmResponse>>>,
    factory_registry: FactoryRegistry,
}

impl ProviderRegistry {
    pub fn from_config(config: &ProviderConfig) -> Self {
        let mut providers: HashMap<String, Arc<dyn Provider>> = HashMap::new();
        let mut provider_provenance: HashMap<String, ProviderProvenance> = HashMap::new();
        let factory_registry = FactoryRegistry::with_defaults(config);

        for name in &config.builtin {
            if let Ok((provider, provenance)) = factory_registry.create_llm(name, config) {
                providers.insert(name.clone(), provider);
                provider_provenance.insert(name.clone(), provenance);
            } else {
                providers.insert(name.clone(), Arc::new(StubProvider::new(name.clone())));
                provider_provenance.insert(
                    name.clone(),
                    ProviderProvenance {
                        provider_name: name.clone(),
                        source: "stub-fallback".into(),
                        version_ref: "stub".into(),
                        trusted: true,
                    },
                );
            }
        }

        for server in &config.mcp_servers {
            let provider_key = format!("mcp:{server}");
            if let Ok((provider, provenance)) = factory_registry.create_llm(&provider_key, config) {
                providers.insert(provider_key.clone(), provider);
                provider_provenance.insert(provider_key.clone(), provenance);
            }
        }

        let default_provider = config
            .builtin
            .first()
            .cloned()
            .or_else(|| {
                config
                    .mcp_servers
                    .first()
                    .map(|server| format!("mcp:{server}"))
            })
            .unwrap_or_else(|| "openai-compatible".into());

        Self {
            providers,
            provider_provenance,
            default_provider,
            default_model: config.default_model.clone(),
            screening_model: config.screening_model.clone(),
            reasoning_model: config.reasoning_model.clone(),
            judge_model: config.judge_model.clone(),
            enable_tiered_routing: config.enable_tiered_routing,
            prompt_cache_capacity: config.prompt_cache_capacity,
            prompt_cache: Arc::new(RwLock::new(HashMap::new())),
            factory_registry,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.providers.is_empty() {
            bail!("at least one provider must be registered");
        }
        if self.default_model.trim().is_empty() {
            bail!("providers.default_model must not be empty");
        }
        if self.prompt_cache_capacity == 0 {
            bail!("providers.prompt_cache_capacity must be greater than 0");
        }
        if !self.providers.contains_key(&self.default_provider) {
            bail!(
                "default provider '{}' is not registered",
                self.default_provider
            );
        }
        if let Some(default_meta) = self.provider_provenance.get(&self.default_provider) {
            if !default_meta.trusted {
                bail!(
                    "default provider '{}' is not trusted by provenance policy",
                    self.default_provider
                );
            }
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn default_provider(&self) -> &str {
        &self.default_provider
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Provider>> {
        self.providers.get(name).cloned()
    }

    pub fn provenance(&self, name: &str) -> Option<&ProviderProvenance> {
        self.provider_provenance.get(name)
    }

    pub fn factory_artifacts(&self) -> Vec<FactoryArtifact> {
        self.factory_registry.artifacts()
    }

    pub fn trusted_factory_artifacts(&self, kind: FactoryComponentKind) -> Vec<FactoryArtifact> {
        self.factory_registry.trusted_active_artifacts(kind)
    }

    pub fn create_embedder(&self, name: &str) -> Result<Arc<dyn Embedder>> {
        self.factory_registry.create_embedder(name)
    }

    pub fn create_vector_store(&self, name: &str) -> Result<Arc<dyn VectorStore>> {
        self.factory_registry.create_vector_store(name)
    }

    pub fn create_graph_store(&self, name: &str) -> Result<Arc<dyn GraphStore>> {
        self.factory_registry.create_graph_store(name)
    }

    pub fn create_reranker(&self, name: &str) -> Result<Arc<dyn Reranker>> {
        self.factory_registry.create_reranker(name)
    }

    pub async fn chat(&self, messages: &[ChatMessage]) -> Result<LlmResponse> {
        self.chat_with_policy(messages, None).await
    }

    pub async fn chat_with_policy(
        &self,
        messages: &[ChatMessage],
        preferred_model: Option<&str>,
    ) -> Result<LlmResponse> {
        Ok(self
            .chat_with_trace(messages, preferred_model)
            .await?
            .response)
    }

    pub async fn chat_with_trace(
        &self,
        messages: &[ChatMessage],
        preferred_model: Option<&str>,
    ) -> Result<ProviderChatTrace> {
        let normalized_messages = normalize_messages(messages);
        let mut route = self.route_for_messages(&normalized_messages, preferred_model);
        let cache_key = self.cache_key(&normalized_messages, &route.model);
        if let Some(cached) = self.prompt_cache.read().get(&cache_key).cloned() {
            route.cache_hit = true;
            return Ok(ProviderChatTrace {
                response: cached,
                route,
                cache_key,
                adapter: None,
            });
        }
        if !self
            .factory_registry
            .is_trusted_active(&self.default_provider)
        {
            bail!(
                "default provider '{}' is not trusted+verified+active in factory supply chain",
                self.default_provider
            );
        }

        let provider = self
            .get(&self.default_provider)
            .ok_or_else(|| anyhow::anyhow!("provider '{}' not found", self.default_provider))?;
        let outcome = provider
            .chat_with_adapter(&normalized_messages, &route.model)
            .await?;
        self.insert_cache(cache_key.clone(), outcome.response.clone());
        route.cache_hit = false;
        Ok(ProviderChatTrace {
            response: outcome.response,
            route,
            cache_key,
            adapter: Some(outcome.trace),
        })
    }

    pub fn derive_prompt_policy(
        &self,
        objective: &str,
        evolution_summary: Option<&str>,
        research_summary: Option<&str>,
        capability_hints: &[String],
    ) -> PromptPolicyOverlay {
        let bundle = build_prompt_template_bundle(
            Some(PromptTemplateProfile {
                stage: "api-policy-adaptation".into(),
                adaptation_type: "prompt-route-tool-policy".into(),
                preferred_surface: "provider-api".into(),
                rollout_budget_ms: 1,
            }),
            evolution_summary,
            research_summary,
            capability_hints,
        );
        let mut directives = bundle.all_directives();

        if objective.to_ascii_lowercase().contains("research")
            || objective.to_ascii_lowercase().contains("crawl")
        {
            directives.push(
                "Bias toward evidence expansion, freshness checks, and official sources before finalizing the answer."
                    .into(),
            );
        }

        PromptPolicyOverlay {
            preferred_model: None,
            directives: dedupe_directives(&directives),
            rationale: if bundle.all_rationales().is_empty() {
                "Derived from self-evolution reports, research evidence, and active capability catalog."
                    .into()
            } else {
                bundle.all_rationales().join(" ")
            },
        }
    }

    pub fn route_for_messages(
        &self,
        messages: &[ChatMessage],
        preferred_model: Option<&str>,
    ) -> ProviderRouteDecision {
        if let Some(model) = preferred_model {
            return ProviderRouteDecision {
                stage: ProviderRouteStage::Reasoning,
                model: model.to_string(),
                rationale: "explicit preferred model requested".into(),
                cache_hit: false,
            };
        }

        if !self.enable_tiered_routing {
            return ProviderRouteDecision {
                stage: ProviderRouteStage::Reasoning,
                model: self.default_model.clone(),
                rationale: "tiered routing disabled".into(),
                cache_hit: false,
            };
        }

        let combined = messages
            .iter()
            .map(|message| message.content.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        let total_chars = combined.len();
        let stage = if combined.contains("judge")
            || combined.contains("verifier")
            || combined.contains("arbitrate")
            || combined.contains("regression")
        {
            ProviderRouteStage::Judge
        } else if total_chars > 1400
            || combined.contains("graph")
            || combined.contains("research")
            || combined.contains("plan")
            || combined.contains("swarm")
            || combined.contains("capability")
        {
            ProviderRouteStage::Reasoning
        } else {
            ProviderRouteStage::Screening
        };

        let model = match stage {
            ProviderRouteStage::Screening => self.screening_model.clone(),
            ProviderRouteStage::Reasoning => self.reasoning_model.clone(),
            ProviderRouteStage::Judge => self.judge_model.clone(),
        };

        ProviderRouteDecision {
            stage,
            model,
            rationale: format!(
                "tiered routing selected model using {} characters of prompt context",
                total_chars
            ),
            cache_hit: false,
        }
    }

    fn cache_key(&self, messages: &[ChatMessage], model: &str) -> String {
        let mut hasher = DefaultHasher::new();
        model.hash(&mut hasher);
        for message in messages {
            message.role.hash(&mut hasher);
            message.content.hash(&mut hasher);
        }
        format!("provider-cache:{:x}", hasher.finish())
    }

    fn insert_cache(&self, key: String, response: LlmResponse) {
        let mut cache = self.prompt_cache.write();
        if cache.len() >= self.prompt_cache_capacity {
            if let Some(oldest_key) = cache.keys().next().cloned() {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(key, response);
    }

    pub async fn propose_next_iteration(
        &self,
        objective: &str,
        history_summary: &str,
        optimization_signals: &[OptimizationSignal],
    ) -> Result<OptimizationProposal> {
        let signal_text = optimization_signals
            .iter()
            .map(|signal| format!("{}={}", signal.key, signal.value))
            .collect::<Vec<_>>()
            .join(", ");
        let prompt = vec![
            ChatMessage {
                role: "system".into(),
                content: "You are the strategy layer for autonomous optimization. Propose the next iteration, focusing on the highest-leverage bounded change, expected gain, risk, and evaluation focus.".into(),
            },
            ChatMessage {
                role: "user".into(),
                content: format!(
                    "Objective: {objective}\nHistory: {history_summary}\nSignals: {signal_text}"
                ),
            },
        ];
        let response = self.chat(&prompt).await?;
        let fallback_hypothesis = response.content.unwrap_or_else(|| {
            format!(
                "Use strategy signals ({signal_text}) to improve the fixed-budget evaluation outcome."
            )
        });

        Ok(OptimizationProposal {
            title: "Autonomous optimization iteration".into(),
            change_target: "current target surface".into(),
            hypothesis: fallback_hypothesis,
            expected_gain:
                "Improve the immutable objective while preserving simplicity and bounded risk."
                    .into(),
            risk: if history_summary.to_ascii_lowercase().contains("crash") {
                "Medium: recent failures suggest guarding against unstable structural changes."
                    .into()
            } else {
                "Low to medium: prefer bounded changes within the current evaluation budget.".into()
            },
            patch_outline: vec![
                "Adjust one coherent slice of the system.".into(),
                "Keep the evaluation protocol read-only.".into(),
                "Capture observable outcomes and compare against the current incumbent.".into(),
            ],
            evaluation_focus: "immutable evaluation protocol or equivalent fixed success criteria"
                .into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_prompt_policy_uses_evolution_research_and_capabilities() {
        let registry =
            ProviderRegistry::from_config(&crate::config::AppConfig::default().providers);
        let overlay = registry.derive_prompt_policy(
            "research anchor drift",
            Some("improve verifier score with bounded prompt changes"),
            Some("official sources were recovered"),
            &["mcp::local-mcp::deploy".into()],
        );

        assert!(!overlay.directives.is_empty());
        assert!(
            overlay
                .directives
                .iter()
                .any(|line| line.contains("Self-evolution guidance"))
        );
        assert!(
            overlay
                .directives
                .iter()
                .any(|line| line.contains("Verified capability hints"))
        );
    }

    #[test]
    fn factory_registry_registers_all_five_component_kinds() {
        let config = crate::config::AppConfig::default().providers;
        let registry = FactoryRegistry::with_defaults(&config);

        assert!(
            !registry
                .trusted_active_artifacts(FactoryComponentKind::Llm)
                .is_empty()
        );
        assert!(
            !registry
                .trusted_active_artifacts(FactoryComponentKind::Embedder)
                .is_empty()
        );
        assert!(
            !registry
                .trusted_active_artifacts(FactoryComponentKind::Vector)
                .is_empty()
        );
        assert!(
            !registry
                .trusted_active_artifacts(FactoryComponentKind::Graph)
                .is_empty()
        );
        assert!(
            !registry
                .trusted_active_artifacts(FactoryComponentKind::Reranker)
                .is_empty()
        );
    }

    #[tokio::test]
    async fn provider_registry_exposes_factory_plugins_for_embedder_vector_graph_reranker() {
        let config = crate::config::AppConfig::default().providers;
        let registry = ProviderRegistry::from_config(&config);

        let embedder = registry.create_embedder("hash-embedder").expect("embedder");
        let vectors = embedder
            .embed(&["factory pipeline".to_string()])
            .await
            .expect("embed");
        assert_eq!(vectors.len(), 1);

        let vector_store = registry
            .create_vector_store("in-memory-vector")
            .expect("vector store");
        vector_store
            .upsert(
                "doc-1",
                &vectors[0],
                &BTreeMap::from([("scope".to_string(), "factory".to_string())]),
            )
            .await
            .expect("vector upsert");
        let matches = vector_store
            .query(&vectors[0], 1)
            .await
            .expect("vector query");
        assert_eq!(matches.first().map(String::as_str), Some("doc-1"));

        let graph_store = registry
            .create_graph_store("in-memory-graph")
            .expect("graph store");
        graph_store
            .upsert_edge("autoloop", "uses", "factory")
            .await
            .expect("graph upsert");
        let neighbors = graph_store
            .neighbors("autoloop")
            .await
            .expect("graph neighbors");
        assert!(neighbors.iter().any(|edge| edge == "uses:factory"));

        let reranker = registry
            .create_reranker("lexical-reranker")
            .expect("reranker");
        let reranked = reranker
            .rerank(
                "trusted factory",
                &[
                    "runtime guard only".to_string(),
                    "trusted factory pipeline".to_string(),
                ],
            )
            .await
            .expect("rerank");
        assert_eq!(
            reranked.first().map(String::as_str),
            Some("trusted factory pipeline")
        );
    }

    #[test]
    fn http_error_taxonomy_maps_retryable_statuses() {
        let rate_limit = classify_http_error(429, Some("throttled".into()));
        assert_eq!(rate_limit.category, ErrorCategory::RateLimit);
        assert!(rate_limit.retryable);

        let invalid = classify_http_error(400, Some("bad request".into()));
        assert_eq!(invalid.category, ErrorCategory::InvalidRequest);
        assert!(!invalid.retryable);
    }

    #[test]
    fn factory_registry_exposes_dual_api_llm_adapters() {
        let config = crate::config::AppConfig::default().providers;
        let registry = FactoryRegistry::with_defaults(&config);
        let llm_artifacts = registry.trusted_active_artifacts(FactoryComponentKind::Llm);

        assert!(llm_artifacts.iter().any(|artifact| artifact.provider == "openai-compatible"));
        assert!(llm_artifacts.iter().any(|artifact| artifact.provider == "anthropic-compatible"));
    }

    #[test]
    fn provider_routes_small_and_judge_prompts_to_different_models() {
        let registry =
            ProviderRegistry::from_config(&crate::config::AppConfig::default().providers);
        let screening = registry.route_for_messages(
            &[ChatMessage {
                role: "user".into(),
                content: "say hello".into(),
            }],
            None,
        );
        let judge = registry.route_for_messages(
            &[ChatMessage {
                role: "user".into(),
                content: "Judge route correctness and verifier regression output".into(),
            }],
            None,
        );

        assert_eq!(screening.stage, ProviderRouteStage::Screening);
        assert_eq!(judge.stage, ProviderRouteStage::Judge);
        assert_ne!(screening.model, judge.model);
    }
}

fn normalize_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut normalized = Vec::new();
    for message in messages {
        let normalized_content = dedupe_text(&message.content);
        if normalized.last().is_some_and(|last: &ChatMessage| {
            last.role == message.role && last.content == normalized_content
        }) {
            continue;
        }
        normalized.push(ChatMessage {
            role: message.role.clone(),
            content: compress_text(&normalized_content, 1800),
        });
    }
    normalized
}

fn dedupe_directives(directives: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    directives
        .iter()
        .filter_map(|directive| {
            let normalized = directive.trim().to_string();
            if normalized.is_empty() || !seen.insert(normalized.clone()) {
                None
            } else {
                Some(normalized)
            }
        })
        .collect()
}

fn dedupe_text(text: &str) -> String {
    let mut seen = std::collections::BTreeSet::new();
    text.lines()
        .filter_map(|line| {
            let normalized = line.trim();
            if normalized.is_empty() || !seen.insert(normalized.to_string()) {
                None
            } else {
                Some(normalized.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compress_text(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        text.to_string()
    } else {
        format!("{}...", &text[..limit.saturating_sub(3)])
    }
}
