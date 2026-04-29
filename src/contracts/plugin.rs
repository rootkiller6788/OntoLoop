use std::collections::BTreeMap;

pub const PLUGIN_API_VERSION_V2: &str = "v2";
pub const PLUGIN_LIFECYCLE_CONTRACT_V2: &str = "plugin-lifecycle-v2";
pub const PLUGIN_EVENT_CONTRACT_V2: &str = "plugin-event-v2";
pub const PLUGIN_FACADE_CONTRACT_V2: &str = "plugin-facade-v2";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Installed,
    Enabled,
    Disabled,
    Deprecated,
    RolledBack,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    GraphProjection,
    VectorProjection,
    SearchProjection,
    SupermemoryFederation,
    SourceAdapter,
    ContextConstraint,
    SoftEstimator,
    Optimizer,
    Repair,
    Proof,
    Tool,
    Hook,
    Transport,
    Service,
    Other,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRisk {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRolloutMode {
    Shadow,
    Canary,
    Full,
    Rollback,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginIsolationMode {
    Subprocess,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginIsolationContract {
    pub mode: PluginIsolationMode,
    pub timeout_ms: u64,
    pub max_memory_mb: u32,
}

impl Default for PluginIsolationContract {
    fn default() -> Self {
        Self {
            mode: PluginIsolationMode::Subprocess,
            timeout_ms: 5_000,
            max_memory_mb: 256,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginFacadeContract {
    pub contract_version: String,
    pub facade_only: bool,
    pub allow_repo_direct: bool,
    pub allow_patch_state_direct: bool,
    #[serde(default)]
    pub allowed_facade_ops: Vec<String>,
}

impl Default for PluginFacadeContract {
    fn default() -> Self {
        Self {
            contract_version: PLUGIN_FACADE_CONTRACT_V2.to_string(),
            facade_only: true,
            allow_repo_direct: false,
            allow_patch_state_direct: false,
            allowed_facade_ops: vec![
                "recall.query".into(),
                "projection.read".into(),
                "mirror.export".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginCapabilityDescriptor {
    pub capability_id: String,
    pub description: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginCompatSpec {
    pub api_version: String,
    #[serde(default)]
    pub compatible_api_versions: Vec<String>,
    pub min_core_version: String,
    #[serde(default)]
    pub max_core_version: Option<String>,
}

impl PluginCompatSpec {
    pub fn supports_api_version(&self, host_api_version: &str) -> bool {
        self.api_version == host_api_version
            || self
                .compatible_api_versions
                .iter()
                .any(|candidate| candidate == host_api_version)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginErrorCode {
    UnsupportedApiVersion,
    InvalidInput,
    CapabilityDenied,
    ExecutionFailed,
    Timeout,
    Internal,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginExecutionError {
    pub code: PluginErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginApiNegotiationRequest {
    pub plugin_id: String,
    pub host_api_version: String,
    #[serde(default)]
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PluginApiNegotiationResult {
    pub accepted: bool,
    pub plugin_id: String,
    pub selected_api_version: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInvocationInput {
    pub invocation_id: String,
    pub plugin_id: String,
    pub session_id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub capability_id: String,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInvocationOutput {
    pub invocation_id: String,
    pub plugin_id: String,
    pub status: String,
    pub payload: serde_json::Value,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

pub trait PluginRuntimeContract: Send + Sync {
    fn manifest(&self) -> &PluginManifestContract;
    fn negotiate_api(&self, request: &PluginApiNegotiationRequest) -> PluginApiNegotiationResult;
    fn invoke(
        &self,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput, PluginExecutionError>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginManifestContract {
    pub id: String,
    #[serde(default)]
    pub plugin_id: String,
    pub version: String,
    pub kind: PluginKind,
    pub capability: PluginCapabilityDescriptor,
    pub risk: PluginRisk,
    pub compat: PluginCompatSpec,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub signature_ref: Option<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default = "default_event_contract_version")]
    pub event_contract_version: String,
    #[serde(default = "default_lifecycle_contract_version")]
    pub lifecycle_contract_version: String,
    #[serde(default)]
    pub isolation: PluginIsolationContract,
    #[serde(default)]
    pub facade: PluginFacadeContract,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginInstallRequest {
    pub plugin_id: String,
    pub source: String,
    pub requested_by: String,
    pub tenant_id: String,
    pub verify_signature: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginLifecycleEvent {
    pub plugin_id: String,
    pub from_state: Option<PluginState>,
    pub to_state: PluginState,
    pub reason: String,
    pub operator: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginVerificationVerdict {
    pub plugin_id: String,
    pub verified: bool,
    pub reason: String,
    pub provenance_ref: Option<String>,
    pub sbom_ref: Option<String>,
    pub checked_at_ms: u64,
}

fn default_event_contract_version() -> String {
    PLUGIN_EVENT_CONTRACT_V2.to_string()
}

fn default_lifecycle_contract_version() -> String {
    PLUGIN_LIFECYCLE_CONTRACT_V2.to_string()
}
