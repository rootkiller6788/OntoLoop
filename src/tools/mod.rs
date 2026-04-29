mod cli_forge;

use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use autoloop_state_adapter::StateStore;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::ToolsConfig;
use crate::contracts::signal::SignalContext;

pub use cli_forge::{
    ApprovalStatus, CapabilityArtifact, CapabilityDeprecationTool, CapabilityRisk,
    CapabilityRollbackTool, CapabilityScope, CapabilityStatus, CapabilityVerifierTool,
    CliAnythingForgeTool, CliOutputMode, ForgeArgumentSpec, ForgedMcpToolManifest,
    ForgedToolCatalog, McpToolForgeRequest, Provenance, RenderedCommandSpec, Sbom, SbomComponent,
    Signature, SignatureAlgorithm, TrustPolicy, TrustStatus, build_command_spec, sanitize_segment,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionStep {
    ApplyPatch { target: String, summary: String },
    RunCommand { command: String, timeout_secs: u64 },
    ParseMetrics { source: String },
    Keep,
    Discard,
    Rollback { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStepResult {
    pub action: ExecutionStep,
    pub succeeded: bool,
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityLifecycleEntry {
    pub tool_name: String,
    pub lineage_key: String,
    pub active_version: Option<u32>,
    pub latest_version: u32,
    pub stable_version: Option<u32>,
    pub deprecated_versions: Vec<u32>,
    pub rolled_back_versions: Vec<u32>,
    pub average_health: f32,
    pub status_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityLifecycleReport {
    pub total_lineages: usize,
    pub active_capabilities: usize,
    pub deprecated_capabilities: usize,
    pub rollback_ready_capabilities: usize,
    pub entries: Vec<CapabilityLifecycleEntry>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    async fn execute(&self, arguments: &str) -> Result<ToolResult>;
}

#[derive(Debug)]
pub struct StubTool {
    name: String,
}

impl StubTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl Tool for StubTool {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        Ok(ToolResult {
            name: self.name.clone(),
            content: format!("[tool:{}] {}", self.name, arguments),
        })
    }
}

#[derive(Debug)]
pub struct McpToolAdapter {
    name: String,
    server: String,
}

impl McpToolAdapter {
    pub fn new(server: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            name: name.into(),
        }
    }
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        Ok(ToolResult {
            name: self.name.clone(),
            content: format!("[mcp-tool:{}:{}] {}", self.server, self.name, arguments),
        })
    }
}

#[derive(Clone)]
pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
    manifests: Arc<RwLock<HashMap<String, ForgedMcpToolManifest>>>,
    lineage_max_versions: Arc<RwLock<HashMap<String, u32>>>,
    state_store: Arc<RwLock<Option<StateStore>>>,
    pub allow_shell: bool,
}

impl ToolRegistry {
    pub const FORGED_TOOL_PREFIX: &str = "tooling:forged:";

    pub fn from_config(config: &ToolsConfig) -> Self {
        let registry = Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            manifests: Arc::new(RwLock::new(HashMap::new())),
            lineage_max_versions: Arc::new(RwLock::new(HashMap::new())),
            state_store: Arc::new(RwLock::new(None)),
            allow_shell: config.allow_shell,
        };

        for name in &config.builtin {
            let builtin: Arc<dyn Tool> = match name.as_str() {
                "read_file" => Arc::new(ReadFileTool),
                "write_file" => Arc::new(WriteFileTool),
                _ => Arc::new(StubTool::new(name.clone())),
            };
            registry.register_tool(builtin);
        }

        for server in &config.mcp_servers {
            let tool_name = format!("mcp::{server}::invoke");
            let tool = Arc::new(McpToolAdapter::new(server.clone(), tool_name.clone()));
            registry.register_tool(tool);
        }

        let default_server = config
            .mcp_servers
            .first()
            .cloned()
            .unwrap_or_else(|| "local-mcp".into());
        registry.register_tool(Arc::new(CliAnythingForgeTool::new(
            registry.clone(),
            default_server,
        )));
        registry.register_tool(Arc::new(ForgedToolCatalog::new(registry.clone())));
        registry.register_tool(Arc::new(CapabilityVerifierTool::new(registry.clone())));
        registry.register_tool(Arc::new(CapabilityDeprecationTool::new(registry.clone())));
        registry.register_tool(Arc::new(CapabilityRollbackTool::new(registry.clone())));

        registry
    }

    pub fn validate(&self) -> Result<()> {
        if self.tools.read().is_empty() {
            bail!("at least one tool must be registered");
        }
        Ok(())
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.read().contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.tools.read().len()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().get(name).cloned()
    }

    pub fn register_tool(&self, tool: Arc<dyn Tool>) {
        self.tools.write().insert(tool.name().to_string(), tool);
    }

    pub fn register_manifest(&self, manifest: ForgedMcpToolManifest) {
        {
            let mut lineage = self.lineage_max_versions.write();
            let entry = lineage.entry(manifest.lineage_key.clone()).or_insert(0);
            *entry = (*entry).max(manifest.version);
        }
        self.manifests
            .write()
            .insert(manifest.registered_tool_name.clone(), manifest);
    }

    pub fn attach_state_store(&self, db: StateStore) {
        *self.state_store.write() = Some(db);
    }

    pub fn state_store(&self) -> Option<StateStore> {
        self.state_store.read().clone()
    }

    pub fn manifests(&self) -> Vec<ForgedMcpToolManifest> {
        let mut manifests = self.manifests.read().values().cloned().collect::<Vec<_>>();
        manifests.sort_by(|left, right| left.registered_tool_name.cmp(&right.registered_tool_name));
        manifests
    }

    pub fn forged_tool_names(&self) -> Vec<String> {
        self.manifests()
            .into_iter()
            .filter(|manifest| manifest.is_executable())
            .map(|manifest| manifest.registered_tool_name)
            .collect()
    }

    pub fn lifecycle_report(&self) -> CapabilityLifecycleReport {
        let manifests = self.manifests();
        let mut by_lineage = HashMap::<String, Vec<ForgedMcpToolManifest>>::new();
        for manifest in manifests {
            by_lineage
                .entry(manifest.lineage_key.clone())
                .or_default()
                .push(manifest);
        }

        let mut entries = by_lineage
            .into_iter()
            .map(|(lineage_key, mut manifests)| {
                manifests.sort_by_key(|manifest| manifest.version);
                let latest_version = manifests
                    .last()
                    .map(|manifest| manifest.version)
                    .unwrap_or(1);
                let active = manifests
                    .iter()
                    .find(|manifest| manifest.status == CapabilityStatus::Active)
                    .cloned();
                let stable_version = manifests
                    .iter()
                    .filter(|manifest| {
                        manifest.status == CapabilityStatus::Active
                            && manifest.approval_status == ApprovalStatus::Verified
                            && manifest.health_score >= 0.7
                    })
                    .max_by_key(|manifest| manifest.version)
                    .map(|manifest| manifest.version);
                let deprecated_versions = manifests
                    .iter()
                    .filter(|manifest| manifest.status == CapabilityStatus::Deprecated)
                    .map(|manifest| manifest.version)
                    .collect::<Vec<_>>();
                let rolled_back_versions = manifests
                    .iter()
                    .filter(|manifest| manifest.status == CapabilityStatus::RolledBack)
                    .map(|manifest| manifest.version)
                    .collect::<Vec<_>>();
                let average_health = if manifests.is_empty() {
                    0.0
                } else {
                    manifests
                        .iter()
                        .map(|manifest| manifest.health_score)
                        .sum::<f32>()
                        / manifests.len() as f32
                };
                let tool_name = active
                    .as_ref()
                    .map(|manifest| manifest.registered_tool_name.clone())
                    .or_else(|| {
                        manifests
                            .last()
                            .map(|manifest| manifest.registered_tool_name.clone())
                    })
                    .unwrap_or_else(|| lineage_key.clone());
                let status_summary = if active.is_none() {
                    "no active version".to_string()
                } else if stable_version.is_some() {
                    "active and rollback-ready".to_string()
                } else {
                    "active but unstable".to_string()
                };

                CapabilityLifecycleEntry {
                    tool_name,
                    lineage_key,
                    active_version: active.as_ref().map(|manifest| manifest.version),
                    latest_version,
                    stable_version,
                    deprecated_versions,
                    rolled_back_versions,
                    average_health,
                    status_summary,
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.tool_name.cmp(&right.tool_name));

        CapabilityLifecycleReport {
            total_lineages: entries.len(),
            active_capabilities: entries
                .iter()
                .filter(|entry| entry.active_version.is_some())
                .count(),
            deprecated_capabilities: entries
                .iter()
                .filter(|entry| !entry.deprecated_versions.is_empty())
                .count(),
            rollback_ready_capabilities: entries
                .iter()
                .filter(|entry| entry.stable_version.is_some())
                .count(),
            entries,
        }
    }

    pub async fn auto_retire_unhealthy_capabilities(&self) -> Result<Vec<ForgedMcpToolManifest>> {
        let candidates = self
            .manifests()
            .into_iter()
            .filter(|manifest| {
                manifest.status == CapabilityStatus::Active
                    && manifest.approval_status == ApprovalStatus::Verified
                    && manifest.health_score < 0.45
            })
            .collect::<Vec<_>>();
        let mut changed = Vec::new();
        for candidate in candidates {
            if let Some(rolled_back) = self
                .rollback_capability(&candidate.registered_tool_name)
                .await?
            {
                changed.push(rolled_back);
            } else if let Some(deprecated) = self
                .deprecate_capability(
                    &candidate.registered_tool_name,
                    candidate.health_score.min(0.35),
                )
                .await?
            {
                changed.push(deprecated);
            }
        }
        Ok(changed)
    }

    pub async fn persist_manifest(&self, manifest: &ForgedMcpToolManifest) -> Result<()> {
        if let Some(db) = self.state_store() {
            db.upsert_json_knowledge(
                format!(
                    "{}{}",
                    Self::FORGED_TOOL_PREFIX,
                    manifest.registered_tool_name
                ),
                manifest,
                "cli-forge",
            )
            .await?;
        }
        Ok(())
    }

    pub async fn restore_persisted_manifests(&self) -> Result<usize> {
        let Some(db) = self.state_store() else {
            return Ok(0);
        };

        let records = db
            .list_knowledge_by_prefix(Self::FORGED_TOOL_PREFIX)
            .await?;
        let mut restored = 0usize;
        for record in records {
            let manifest = serde_json::from_str::<ForgedMcpToolManifest>(&record.value)?;
            self.hydrate_manifest(manifest);
            restored += 1;
        }
        Ok(restored)
    }

    pub fn hydrate_manifest(&self, manifest: ForgedMcpToolManifest) {
        if manifest.status != CapabilityStatus::Deprecated
            && manifest.status != CapabilityStatus::RolledBack
        {
            let tool = Arc::new(cli_forge::ForgedMcpTool::new(
                manifest.registered_tool_name.clone(),
                manifest.clone(),
            ));
            self.register_tool(tool);
        }
        self.register_manifest(manifest);
    }

    pub async fn upsert_governed_manifest(
        &self,
        mut manifest: ForgedMcpToolManifest,
    ) -> Result<()> {
        if manifest.lineage_key.is_empty() {
            manifest.lineage_key = manifest.capability_id.clone();
        }

        let next_version = self
            .manifests()
            .into_iter()
            .filter(|existing| existing.lineage_key == manifest.lineage_key)
            .map(|existing| existing.version)
            .max()
            .unwrap_or(0)
            + 1;
        manifest.version = next_version;
        manifest.updated_at_ms = manifest.updated_at_ms.max(manifest.created_at_ms);
        if manifest.capability_id.is_empty() {
            manifest.capability_id = format!("{}:v{}", manifest.lineage_key, manifest.version);
        }
        let trust_findings = self.supply_chain_findings(&manifest);
        manifest.trust_findings = trust_findings.clone();
        manifest.trust_status = if trust_findings.is_empty() {
            TrustStatus::Trusted
        } else {
            manifest.approval_status = ApprovalStatus::Rejected;
            manifest.status = CapabilityStatus::PendingVerification;
            TrustStatus::Rejected
        };

        self.hydrate_manifest(manifest.clone());
        self.persist_manifest(&manifest).await
    }

    pub async fn verify_capability(
        &self,
        tool_name: &str,
    ) -> Result<Option<ForgedMcpToolManifest>> {
        let Some(mut manifest) = self.manifests.read().get(tool_name).cloned() else {
            return Ok(None);
        };
        let trust_findings = self.supply_chain_findings(&manifest);
        manifest.trust_findings = trust_findings.clone();
        if !trust_findings.is_empty() {
            manifest.trust_status = TrustStatus::Rejected;
            manifest.approval_status = ApprovalStatus::Rejected;
            manifest.status = CapabilityStatus::PendingVerification;
            manifest.updated_at_ms = current_time_ms();
            self.register_manifest(manifest.clone());
            self.persist_manifest(&manifest).await?;
            bail!(
                "capability '{}' failed trusted supply-chain verification: {}",
                tool_name,
                trust_findings.join("; ")
            );
        }
        manifest.status = CapabilityStatus::Active;
        manifest.approval_status = ApprovalStatus::Verified;
        manifest.trust_status = TrustStatus::Trusted;
        manifest.health_score = manifest.health_score.max(0.8);
        manifest.approved_at_ms = Some(current_time_ms());
        manifest.updated_at_ms = current_time_ms();
        self.hydrate_manifest(manifest.clone());
        self.persist_manifest(&manifest).await?;
        Ok(Some(manifest))
    }

    pub async fn deprecate_capability(
        &self,
        tool_name: &str,
        health_score: f32,
    ) -> Result<Option<ForgedMcpToolManifest>> {
        let Some(mut manifest) = self.manifests.read().get(tool_name).cloned() else {
            return Ok(None);
        };
        manifest.status = CapabilityStatus::Deprecated;
        manifest.health_score = health_score;
        manifest.updated_at_ms = current_time_ms();
        self.tools.write().remove(tool_name);
        self.register_manifest(manifest.clone());
        self.persist_manifest(&manifest).await?;
        Ok(Some(manifest))
    }

    pub async fn rollback_capability(
        &self,
        tool_name: &str,
    ) -> Result<Option<ForgedMcpToolManifest>> {
        let Some(current) = self.manifests.read().get(tool_name).cloned() else {
            return Ok(None);
        };
        let previous = self
            .manifests()
            .into_iter()
            .filter(|manifest| {
                manifest.lineage_key == current.lineage_key
                    && manifest.version + 1 == current.version
            })
            .max_by_key(|manifest| manifest.version);
        let Some(mut previous) = previous else {
            return Ok(None);
        };
        let mut current = current;
        current.status = CapabilityStatus::RolledBack;
        current.rollback_to_version = Some(previous.version);
        current.updated_at_ms = current_time_ms();
        self.tools.write().remove(tool_name);
        self.register_manifest(current.clone());
        self.persist_manifest(&current).await?;

        previous.status = CapabilityStatus::Active;
        previous.approval_status = ApprovalStatus::Verified;
        previous.updated_at_ms = current_time_ms();
        self.hydrate_manifest(previous.clone());
        self.persist_manifest(&previous).await?;
        Ok(Some(previous))
    }

    pub fn names(&self) -> Vec<String> {
        let mut names = self.tools.read().keys().cloned().collect::<Vec<_>>();
        names.sort();
        names
    }

    pub fn mcp_tool_names(&self) -> Vec<String> {
        self.names()
            .into_iter()
            .filter(|name| name.starts_with("mcp::"))
            .collect()
    }

    pub async fn execute(&self, name: &str, arguments: &str) -> Result<ToolResult> {
        self.execute_with_context(name, arguments, None).await
    }

    pub async fn execute_with_context(
        &self,
        name: &str,
        arguments: &str,
        context: Option<&SignalContext>,
    ) -> Result<ToolResult> {
        if let Some(manifest) = self.manifests.read().get(name).cloned() {
            if !manifest.is_executable() {
                bail!(
                    "capability '{}' is not executable: status={:?} approval={:?} health={:.2}",
                    name,
                    manifest.status,
                    manifest.approval_status,
                    manifest.health_score
                );
            }
            if manifest.trust_status != TrustStatus::Trusted {
                bail!(
                    "capability '{}' is not trusted: trust_status={:?} findings={}",
                    name,
                    manifest.trust_status,
                    manifest.trust_findings.join("; ")
                );
            }
        }
        let tool = self
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {name}"))?;
        let _ = context;
        tool.execute(arguments).await
    }

    pub async fn execute_plan(
        &self,
        actions: &[ExecutionStep],
    ) -> Result<Vec<ExecutionStepResult>> {
        let mut results = Vec::new();

        for action in actions {
            let (succeeded, details) = match action {
                ExecutionStep::ApplyPatch { target, summary } => {
                    (true, format!("Prepared patch for {target}: {summary}"))
                }
                ExecutionStep::RunCommand {
                    command,
                    timeout_secs,
                } => (
                    true,
                    format!("Scheduled run `{command}` with timeout {timeout_secs}s"),
                ),
                ExecutionStep::ParseMetrics { source } => {
                    (true, format!("Parsed immutable metrics from {source}"))
                }
                ExecutionStep::Keep => (true, "Iteration marked as keep".into()),
                ExecutionStep::Discard => (true, "Iteration marked as discard".into()),
                ExecutionStep::Rollback { reason } => {
                    (true, format!("Rollback requested: {reason}"))
                }
            };

            results.push(ExecutionStepResult {
                action: action.clone(),
                succeeded,
                details,
            });
        }

        Ok(results)
    }

    fn supply_chain_findings(&self, manifest: &ForgedMcpToolManifest) -> Vec<String> {
        let mut findings = manifest.supply_chain_findings();
        let lineage_max_verified = self
            .lineage_max_versions
            .read()
            .get(&manifest.lineage_key)
            .copied()
            .unwrap_or(0);
        if lineage_max_verified > 0 && manifest.version < lineage_max_verified {
            findings.push(format!(
                "version rollback detected: {} < trusted {}",
                manifest.version, lineage_max_verified
            ));
        }
        if manifest.provenance.source_ref.trim().is_empty() {
            findings.push("provenance source_ref missing".into());
        }
        findings
    }
}

#[derive(Debug)]
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let path = parse_path_argument(arguments)?;
        let body = std::fs::read_to_string(&path)?;
        Ok(ToolResult {
            name: self.name().to_string(),
            content: serde_json::to_string(&serde_json::json!({
                "path": path,
                "size_bytes": body.as_bytes().len(),
                "content": body,
            }))?,
        })
    }
}

#[derive(Debug)]
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let (path, content, append) = parse_write_arguments(arguments)?;
        let file_path = std::path::Path::new(&path);
        if let Some(parent) = file_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        if append {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(file_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
        } else {
            std::fs::write(file_path, content.as_bytes())?;
        }

        let bytes = std::fs::read(file_path)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = format!("{:x}", hasher.finalize());

        Ok(ToolResult {
            name: self.name().to_string(),
            content: serde_json::to_string(&serde_json::json!({
                "path": path,
                "size_bytes": bytes.len(),
                "sha256": hash,
                "append": append,
                "created": !append,
            }))?,
        })
    }
}

fn parse_path_argument(arguments: &str) -> Result<String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) {
        if let Some(path) = extract_path_field(&value) {
            return Ok(path);
        }
    }

    let trimmed = arguments.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() {
        bail!("read_file requires a non-empty path argument");
    }
    Ok(trimmed.to_string())
}

fn parse_write_arguments(arguments: &str) -> Result<(String, String, bool)> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) {
        if let Some(path) = extract_path_field(&value) {
            let content = value
                .get("content")
                .or_else(|| value.get("text"))
                .or_else(|| value.get("body"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let append = value
                .get("append")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            return Ok((path, content, append));
        }
    }

    if let Some((path, content)) = arguments.split_once('\n') {
        let path = path.trim().trim_matches('"').trim_matches('\'').to_string();
        if !path.is_empty() {
            return Ok((path, content.to_string(), false));
        }
    }

    bail!("write_file expects JSON arguments with path and content")
}

fn extract_path_field(value: &serde_json::Value) -> Option<String> {
    value
        .get("path")
        .or_else(|| value.get("file_path"))
        .or_else(|| value.get("target_path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

