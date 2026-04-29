use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::contracts::types::AttestationPolicy;
use anyhow::Result;
use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSection {
    pub name: String,
    pub workspace_root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeGateMode {
    Shadow,
    Canary,
    Full,
}

fn default_runtime_gate_mode() -> RuntimeGateMode {
    RuntimeGateMode::Shadow
}

fn default_runtime_gate_enforce_ratio() -> f32 {
    0.2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageBackend {
    PrimaryStore,
    Postgres,
}

fn default_storage_backend() -> StorageBackend {
    StorageBackend::PrimaryStore
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    Direct,
    Shadow,
    Enforced,
}

fn default_storage_mode() -> StorageMode {
    StorageMode::Shadow
}

fn default_storage_shadow_read_preference() -> String {
    "state_store".into()
}

fn default_storage_shadow_read_rollout_percent() -> u8 {
    0
}

fn default_storage_shadow_write_grace_hours() -> u64 {
    24
}

fn default_postgres_enabled() -> bool {
    false
}

fn default_postgres_uri() -> String {
    "postgres://postgres:postgres@localhost:5432/ontoloop".into()
}

fn default_postgres_pool_size() -> usize {
    16
}

fn default_postgres_schema() -> String {
    "public".into()
}

fn default_postgres_app_name() -> String {
    "ontoloop".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    Off,
    Shadow,
    Enforced,
}

fn default_runtime_policy_mode() -> PolicyMode {
    PolicyMode::Shadow
}

fn default_runtime_permission_mode() -> String {
    "strict".into()
}
fn default_runtime_budget_enforced() -> bool {
    true
}

fn default_runtime_default_budget_micros() -> u64 {
    5_000_000
}

fn default_runtime_quota_window_ms() -> u64 {
    3_600_000
}

fn default_runtime_quota_window_budget_micros() -> u64 {
    1_000_000
}

fn default_runtime_attestation_required() -> bool {
    false
}

fn default_runtime_attestation_secret_env() -> String {
    "AUTOLOOP_ATTESTATION_SECRET".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationBackend {
    Env,
    Remote,
    HardwareQuote,
    CertificateChain,
}

fn default_attestation_backend() -> AttestationBackend {
    AttestationBackend::Env
}

fn default_runtime_attestation_token_env() -> String {
    "AUTOLOOP_ATTESTATION_TOKEN".into()
}

fn default_runtime_trust_evidence_ledger_path() -> String {
    "deploy/runtime/trust/evidence.log".into()
}

fn default_runtime_trust_budget_ledger_path() -> String {
    "deploy/runtime/trust/budget.log".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLedgerConsistencyMode {
    BestEffort,
    Strong,
}

fn default_trust_ledger_consistency_mode() -> TrustLedgerConsistencyMode {
    TrustLedgerConsistencyMode::Strong
}

fn default_runtime_attestation_quote_env() -> String {
    "AUTOLOOP_ATTESTATION_QUOTE".into()
}

fn default_runtime_attestation_cert_chain_env() -> String {
    "AUTOLOOP_ATTESTATION_CERT_CHAIN".into()
}

fn default_runtime_attestation_policy() -> AttestationPolicy {
    AttestationPolicy {
        version: "v1".into(),
        strict: true,
        min_tcb_version: "1.0.0".into(),
        evidence_ttl_ms: 300_000,
        require_tenant_binding: true,
        require_nonce: true,
    }
}
fn default_runtime_attestation_cert_subject_allowlist() -> Vec<String> {
    Vec::new()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    pub max_parallel_agents: usize,
    pub max_memory_mb: u32,
    pub mcp_enabled: bool,
    pub allow_network_tools: bool,
    pub tool_breaker_failure_threshold: u32,
    pub tool_breaker_cooldown_ms: u64,
    pub mcp_breaker_failure_threshold: u32,
    pub mcp_breaker_cooldown_ms: u64,
    #[serde(default = "default_runtime_gate_mode")]
    pub gate_mode: RuntimeGateMode,
    #[serde(default = "default_runtime_gate_enforce_ratio")]
    pub gate_enforce_ratio: f32,
    #[serde(default = "default_runtime_permission_mode")]
    pub permission_mode: String,
    #[serde(default = "default_runtime_policy_mode")]
    pub policy_mode: PolicyMode,
    #[serde(default)]
    pub rollback_contract_version: Option<String>,
    #[serde(default = "default_runtime_budget_enforced")]
    pub budget_enforced: bool,
    #[serde(default = "default_runtime_default_budget_micros")]
    pub default_budget_micros: u64,
    #[serde(default = "default_runtime_quota_window_ms")]
    pub quota_window_ms: u64,
    #[serde(default = "default_runtime_quota_window_budget_micros")]
    pub quota_window_budget_micros: u64,
    #[serde(default = "default_runtime_attestation_required")]
    pub attestation_required: bool,
    #[serde(default = "default_runtime_attestation_secret_env")]
    pub attestation_secret_env: String,
    #[serde(default = "default_attestation_backend")]
    pub attestation_backend: AttestationBackend,
    #[serde(default)]
    pub attestation_remote_url: Option<String>,
    #[serde(default = "default_runtime_attestation_token_env")]
    pub attestation_token_env: String,
    #[serde(default = "default_runtime_trust_evidence_ledger_path")]
    pub trust_evidence_ledger_path: String,
    #[serde(default = "default_runtime_trust_budget_ledger_path")]
    pub trust_budget_ledger_path: String,
    #[serde(default = "default_trust_ledger_consistency_mode")]
    pub trust_ledger_consistency_mode: TrustLedgerConsistencyMode,
    #[serde(default = "default_runtime_attestation_quote_env")]
    pub attestation_quote_env: String,
    #[serde(default = "default_runtime_attestation_cert_chain_env")]
    pub attestation_cert_chain_env: String,
    #[serde(default = "default_runtime_attestation_cert_subject_allowlist")]
    pub attestation_cert_subject_allowlist: Vec<String>,
    #[serde(default = "default_runtime_attestation_policy")]
    pub attestation_policy: AttestationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostgresConfig {
    #[serde(default = "default_postgres_enabled")]
    pub enabled: bool,
    #[serde(default = "default_postgres_uri")]
    pub uri: String,
    #[serde(default = "default_postgres_pool_size")]
    pub pool_size: usize,
    #[serde(default = "default_postgres_schema")]
    pub schema: String,
    #[serde(default = "default_postgres_app_name")]
    pub app_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_storage_backend")]
    pub backend: StorageBackend,
    #[serde(default = "default_storage_mode")]
    pub mode: StorageMode,
    #[serde(default = "default_storage_shadow_read_preference")]
    pub shadow_read_preference: String,
    #[serde(default = "default_storage_shadow_read_rollout_percent")]
    pub shadow_read_rollout_percent: u8,
    #[serde(default = "default_storage_shadow_write_grace_hours")]
    pub shadow_write_grace_hours: u64,
    #[serde(default)]
    pub postgres: PostgresConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    pub profile: String,
    pub require_approval_for_exec: bool,
    pub ironclaw_compatible_rules: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub load_identity: bool,
    pub load_memory_md: bool,
    pub load_history_md: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningIndexKind {
    Flat,
    Hnsw,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantizationMode {
    None,
    Scalar,
    Product,
    Binary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningConfig {
    pub enabled: bool,
    pub sidecar_enabled: bool,
    pub index_kind: LearningIndexKind,
    pub embedding_dimensions: usize,
    pub top_k: usize,
    pub gray_routing_ratio: f32,
    pub routing_takeover_threshold: f32,
    pub hot_window_days: u32,
    pub cold_quantization: QuantizationMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagConfig {
    pub enable_graph_rag: bool,
    pub enable_first_principles_extraction: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchBackend {
    Auto,
    BrowserFetch,
    PlaywrightCli,
    Firecrawl,
    SelfHostedScraper,
    WebFetch,
    Synthetic,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchConfig {
    pub enabled: bool,
    pub backend: ResearchBackend,
    pub live_fetch_enabled: bool,
    pub max_queries_per_anchor: usize,
    pub max_candidates_per_query: usize,
    pub prefer_official_sources: bool,
    pub prefer_dynamic_render: bool,
    pub browser_render_url: Option<String>,
    pub browser_session_pool: Vec<String>,
    pub proxy_provider_name: String,
    pub proxy_pool: Vec<String>,
    pub anti_bot_profile: String,
    pub rotate_proxy_per_request: bool,
    pub proxy_request_quota_per_proxy: usize,
    pub proxy_breaker_failure_threshold: usize,
    pub proxy_breaker_cooldown_ms: u64,
    pub playwright_node_binary: String,
    pub playwright_launch_timeout_secs: u64,
    pub firecrawl_api_url: String,
    pub firecrawl_api_key_env: String,
    pub self_hosted_scraper_url: Option<String>,
    pub request_timeout_secs: u64,
    pub retry_attempts: usize,
    pub stability_backoff_ms: u64,
    pub user_agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub builtin: Vec<String>,
    pub default_model: String,
    pub screening_model: String,
    pub reasoning_model: String,
    pub judge_model: String,
    pub enable_tiered_routing: bool,
    pub prompt_cache_capacity: usize,
    pub api_base_url: String,
    pub api_key_env: String,
    pub request_timeout_secs: u64,
    pub mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    pub builtin: Vec<String>,
    pub allow_shell: bool,
    pub mcp_servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HooksConfig {
    pub builtin: Vec<String>,
    pub learning_hooks_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalPipelineMode {
    Off,
    Shadow,
    Enforced,
}

fn default_signal_pipeline_mode() -> SignalPipelineMode {
    SignalPipelineMode::Shadow
}

fn default_signal_pipeline_batch_size() -> usize {
    256
}

fn default_signal_pipeline_max_retries() -> u8 {
    2
}

fn default_signal_pipeline_retry_backoff_ms() -> u64 {
    25
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalPipelineConfig {
    #[serde(default = "default_signal_pipeline_mode")]
    pub mode: SignalPipelineMode,
    #[serde(default = "default_signal_pipeline_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_signal_pipeline_max_retries")]
    pub max_retries: u8,
    #[serde(default = "default_signal_pipeline_retry_backoff_ms")]
    pub retry_backoff_ms: u64,
}

impl Default for SignalPipelineConfig {
    fn default() -> Self {
        Self {
            mode: default_signal_pipeline_mode(),
            batch_size: default_signal_pipeline_batch_size(),
            max_retries: default_signal_pipeline_max_retries(),
            retry_backoff_ms: default_signal_pipeline_retry_backoff_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    pub enabled: bool,
    pub route_analytics_enabled: bool,
    pub failure_forensics_enabled: bool,
    pub dashboard_enabled: bool,
    pub report_top_k: usize,
    #[serde(default)]
    pub signal_pipeline: SignalPipelineConfig,
    #[serde(default)]
    pub alert_thresholds: AlertThresholds,
}

fn default_alert_p95_latency_ms() -> f64 {
    120_000.0
}

fn default_alert_error_rate() -> f64 {
    0.05
}

fn default_alert_mttr_ms() -> f64 {
    60_000.0
}

fn default_alert_open_circuit_count() -> u64 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertThresholds {
    #[serde(default = "default_alert_p95_latency_ms")]
    pub p95_latency_ms: f64,
    #[serde(default = "default_alert_error_rate")]
    pub error_rate: f64,
    #[serde(default = "default_alert_mttr_ms")]
    pub mttr_ms: f64,
    #[serde(default = "default_alert_open_circuit_count")]
    pub open_circuit_count: u64,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            p95_latency_ms: default_alert_p95_latency_ms(),
            error_rate: default_alert_error_rate(),
            mttr_ms: default_alert_mttr_ms(),
            open_circuit_count: default_alert_open_circuit_count(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentConfig {
    pub profile: String,
    pub enable_container_assets: bool,
    pub config_dir: PathBuf,
    pub backup_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub max_iterations: usize,
    pub memory_window: usize,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub app: AppSection,
    pub agent: AgentConfig,
    pub runtime: RuntimeConfig,
    pub security: SecurityConfig,
    pub memory: MemoryConfig,
    pub learning: LearningConfig,
    pub research: ResearchConfig,
    pub rag: RagConfig,
    pub providers: ProviderConfig,
    pub tools: ToolsConfig,
    pub hooks: HooksConfig,
    pub observability: ObservabilityConfig,
    pub deployment: DeploymentConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    pub state_store: StateStoreConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigProfile {
    Local,
    Production,
}

impl ConfigProfile {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" | "dev" | "development" => Some(Self::Local),
            "prod" | "production" => Some(Self::Production),
            _ => None,
        }
    }

    pub fn config_path(self) -> PathBuf {
        match self {
            Self::Local => PathBuf::from("deploy/config/autoloop.dev.toml"),
            Self::Production => PathBuf::from("deploy/config/autoloop.prod.toml"),
        }
    }
}

impl AppConfig {
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        let mut config: Self = toml::from_str(&raw)?;
        config.normalize_profile_fields();
        Ok(config)
    }

    pub fn load_with_profile_hint(
        config_path: Option<&Path>,
        profile_hint: Option<&str>,
    ) -> Result<Self> {
        if let Some(path) = config_path {
            return Self::load_from_path(path);
        }

        let requested_profile = profile_hint
            .and_then(ConfigProfile::parse)
            .or_else(|| {
                std::env::var("AUTOLOOP_PROFILE")
                    .ok()
                    .and_then(|value| ConfigProfile::parse(&value))
            });
        if let Some(profile) = requested_profile {
            let path = profile.config_path();
            if path.exists() {
                return Self::load_from_path(&path);
            }
        }

        for fallback in [ConfigProfile::Local, ConfigProfile::Production] {
            let path = fallback.config_path();
            if path.exists() {
                return Self::load_from_path(&path);
            }
        }

        Ok(Self::default())
    }

    fn normalize_profile_fields(&mut self) {
        if self
            .runtime
            .attestation_remote_url
            .as_deref()
            .map(str::trim)
            .is_some_and(str::is_empty)
        {
            self.runtime.attestation_remote_url = None;
        }
        if self
            .research
            .browser_render_url
            .as_deref()
            .map(str::trim)
            .is_some_and(str::is_empty)
        {
            self.research.browser_render_url = None;
        }
        if self
            .research
            .self_hosted_scraper_url
            .as_deref()
            .map(str::trim)
            .is_some_and(str::is_empty)
        {
            self.research.self_hosted_scraper_url = None;
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app: AppSection {
                name: "autoloop".into(),
                workspace_root: PathBuf::from("."),
            },
            agent: AgentConfig {
                max_iterations: 12,
                memory_window: 100,
                system_prompt: "You are AutoLoop, a lightweight autonomous assistant runtime."
                    .into(),
            },
            runtime: RuntimeConfig {
                max_parallel_agents: 4,
                max_memory_mb: 512,
                mcp_enabled: true,
                allow_network_tools: false,
                tool_breaker_failure_threshold: 3,
                tool_breaker_cooldown_ms: 180_000,
                mcp_breaker_failure_threshold: 2,
                mcp_breaker_cooldown_ms: 300_000,
                gate_mode: RuntimeGateMode::Shadow,
                gate_enforce_ratio: 0.2,
                permission_mode: default_runtime_permission_mode(),
                policy_mode: default_runtime_policy_mode(),
                rollback_contract_version: None,
                budget_enforced: true,
                default_budget_micros: 5_000_000,
                quota_window_ms: 3_600_000,
                quota_window_budget_micros: 1_000_000,
                attestation_required: false,
                attestation_secret_env: "AUTOLOOP_ATTESTATION_SECRET".into(),
                attestation_backend: AttestationBackend::Env,
                attestation_remote_url: None,
                attestation_token_env: "AUTOLOOP_ATTESTATION_TOKEN".into(),
                trust_evidence_ledger_path: "deploy/runtime/trust/evidence.log".into(),
                trust_budget_ledger_path: "deploy/runtime/trust/budget.log".into(),
                trust_ledger_consistency_mode: TrustLedgerConsistencyMode::Strong,
                attestation_quote_env: "AUTOLOOP_ATTESTATION_QUOTE".into(),
                attestation_cert_chain_env: "AUTOLOOP_ATTESTATION_CERT_CHAIN".into(),
                attestation_cert_subject_allowlist: Vec::new(),
                attestation_policy: default_runtime_attestation_policy(),
            },
            security: SecurityConfig {
                profile: "minimal-ironclaw".into(),
                require_approval_for_exec: true,
                ironclaw_compatible_rules: true,
            },
            memory: MemoryConfig {
                load_identity: true,
                load_memory_md: true,
                load_history_md: false,
            },
            learning: LearningConfig {
                enabled: true,
                sidecar_enabled: true,
                index_kind: LearningIndexKind::Flat,
                embedding_dimensions: 128,
                top_k: 4,
                gray_routing_ratio: 0.2,
                routing_takeover_threshold: 2.0,
                hot_window_days: 14,
                cold_quantization: QuantizationMode::Scalar,
            },
            research: ResearchConfig {
                enabled: true,
                backend: ResearchBackend::Auto,
                live_fetch_enabled: false,
                max_queries_per_anchor: 3,
                max_candidates_per_query: 4,
                prefer_official_sources: true,
                prefer_dynamic_render: true,
                browser_render_url: None,
                browser_session_pool: vec!["default-browser-session".into()],
                proxy_provider_name: "local-static-pool".into(),
                proxy_pool: Vec::new(),
                anti_bot_profile: "balanced-stealth".into(),
                rotate_proxy_per_request: true,
                proxy_request_quota_per_proxy: 12,
                proxy_breaker_failure_threshold: 3,
                proxy_breaker_cooldown_ms: 300_000,
                playwright_node_binary: "node".into(),
                playwright_launch_timeout_secs: 25,
                firecrawl_api_url: "https://api.firecrawl.dev".into(),
                firecrawl_api_key_env: "FIRECRAWL_API_KEY".into(),
                self_hosted_scraper_url: None,
                request_timeout_secs: 20,
                retry_attempts: 2,
                stability_backoff_ms: 350,
                user_agent: "autoloop-research/0.1".into(),
            },
            rag: RagConfig {
                enable_graph_rag: true,
                enable_first_principles_extraction: true,
            },
            providers: ProviderConfig {
                builtin: vec!["openai-compatible".into()],
                default_model: "gpt-4.1-mini".into(),
                screening_model: "gpt-4.1-nano".into(),
                reasoning_model: "gpt-4.1-mini".into(),
                judge_model: "gpt-5".into(),
                enable_tiered_routing: true,
                prompt_cache_capacity: 256,
                api_base_url: "https://api.openai.com/v1".into(),
                api_key_env: "OPENAI_API_KEY".into(),
                request_timeout_secs: 60,
                mcp_servers: vec!["local-mcp".into()],
            },
            tools: ToolsConfig {
                builtin: vec!["read_file".into(), "write_file".into(), "web_fetch".into()],
                allow_shell: false,
                mcp_servers: vec!["local-mcp".into()],
            },
            hooks: HooksConfig {
                builtin: vec!["self-learn".into()],
                learning_hooks_enabled: true,
            },
            observability: ObservabilityConfig {
                enabled: true,
                route_analytics_enabled: true,
                failure_forensics_enabled: true,
                dashboard_enabled: true,
                report_top_k: 8,
                signal_pipeline: SignalPipelineConfig::default(),
                alert_thresholds: AlertThresholds::default(),
            },
            deployment: DeploymentConfig {
                profile: "production-ready".into(),
                enable_container_assets: true,
                config_dir: PathBuf::from("deploy/config"),
                backup_dir: PathBuf::from("deploy/backup"),
            },
            storage: StorageConfig {
                backend: default_storage_backend(),
                mode: default_storage_mode(),
                shadow_read_preference: default_storage_shadow_read_preference(),
                shadow_read_rollout_percent: default_storage_shadow_read_rollout_percent(),
                shadow_write_grace_hours: default_storage_shadow_write_grace_hours(),
                postgres: PostgresConfig {
                    enabled: default_postgres_enabled(),
                    uri: default_postgres_uri(),
                    pool_size: default_postgres_pool_size(),
                    schema: default_postgres_schema(),
                    app_name: default_postgres_app_name(),
                },
            },
            state_store: StateStoreConfig {
                enabled: true,
                backend: StateStoreBackend::InMemory,
                uri: "http://state_store:3000".into(),
                module_name: "autoloop_core".into(),
                namespace: "autoloop".into(),
                pool_size: 8,
            },
        }
    }
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            enabled: default_postgres_enabled(),
            uri: default_postgres_uri(),
            pool_size: default_postgres_pool_size(),
            schema: default_postgres_schema(),
            app_name: default_postgres_app_name(),
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: default_storage_backend(),
            mode: default_storage_mode(),
            shadow_read_preference: default_storage_shadow_read_preference(),
            shadow_read_rollout_percent: default_storage_shadow_read_rollout_percent(),
            shadow_write_grace_hours: default_storage_shadow_write_grace_hours(),
            postgres: PostgresConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_renderer_loads_local_profile_when_requested() {
        let config =
            AppConfig::load_with_profile_hint(None, Some("local")).expect("load local profile");
        assert!(
            config.app.name.contains("ontoloop"),
            "profile renderer should load on-disk local profile when available"
        );
    }
}





