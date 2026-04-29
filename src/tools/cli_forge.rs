use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Tool, ToolRegistry, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedCommandSpec {
    pub executable: String,
    pub args: Vec<String>,
    pub display_command: String,
    pub working_directory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliOutputMode {
    Human,
    Json,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    PendingVerification,
    Active,
    Deprecated,
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Verified,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityScope {
    Session,
    TaskFamily,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignatureAlgorithm {
    DeterministicV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustStatus {
    Pending,
    Trusted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityArtifact {
    pub artifact_id: String,
    pub digest_sha256: String,
    pub source_uri: String,
    pub build_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub signer: String,
    pub algorithm: SignatureAlgorithm,
    pub signed_payload_hash: String,
    pub signature: String,
    pub signed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub source_repo: String,
    pub source_ref: String,
    pub builder: String,
    pub generated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbomComponent {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sbom {
    pub components: Vec<SbomComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustPolicy {
    pub required_signers: Vec<String>,
    pub blocked_dependencies: Vec<String>,
    pub min_provenance_ref_len: usize,
}

impl Default for CliOutputMode {
    fn default() -> Self {
        Self::Json
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeArgumentSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub example: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolForgeRequest {
    #[serde(default)]
    pub server: Option<String>,
    pub capability_name: String,
    pub purpose: String,
    pub executable: String,
    #[serde(default)]
    pub subcommands: Vec<String>,
    #[serde(default)]
    pub json_flag: Option<String>,
    #[serde(default)]
    pub arguments: Vec<ForgeArgumentSpec>,
    #[serde(default)]
    pub output_mode: CliOutputMode,
    #[serde(default)]
    pub success_signal: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub scope: Option<CapabilityScope>,
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub signer: Option<String>,
    #[serde(default)]
    pub source_repo: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    #[serde(default)]
    pub sbom_components: Vec<SbomComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgedMcpToolManifest {
    #[serde(default)]
    pub capability_id: String,
    pub registered_tool_name: String,
    pub delegate_tool_name: String,
    pub server: String,
    pub capability_name: String,
    pub purpose: String,
    pub executable: String,
    pub command_template: String,
    pub payload_template: Value,
    pub output_mode: CliOutputMode,
    pub working_directory: Option<String>,
    pub success_signal: Option<String>,
    pub help_text: String,
    pub skill_markdown: String,
    pub examples: Vec<String>,
    #[serde(default = "default_capability_version")]
    pub version: u32,
    #[serde(default)]
    pub lineage_key: String,
    #[serde(default = "default_capability_status")]
    pub status: CapabilityStatus,
    #[serde(default = "default_approval_status")]
    pub approval_status: ApprovalStatus,
    #[serde(default = "default_health_score")]
    pub health_score: f32,
    #[serde(default = "default_scope")]
    pub scope: CapabilityScope,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_risk")]
    pub risk: CapabilityRisk,
    #[serde(default = "default_requested_by")]
    pub requested_by: String,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub approved_at_ms: Option<u64>,
    #[serde(default)]
    pub rollback_to_version: Option<u32>,
    #[serde(default = "default_trust_status")]
    pub trust_status: TrustStatus,
    #[serde(default)]
    pub trust_findings: Vec<String>,
    #[serde(default = "default_capability_artifact")]
    pub artifact: CapabilityArtifact,
    #[serde(default = "default_signature")]
    pub signature: Signature,
    #[serde(default = "default_provenance")]
    pub provenance: Provenance,
    #[serde(default = "default_sbom")]
    pub sbom: Sbom,
    #[serde(default = "default_trust_policy")]
    pub trust_policy: TrustPolicy,
}

impl ForgedMcpToolManifest {
    pub fn is_executable(&self) -> bool {
        self.status == CapabilityStatus::Active
            && self.approval_status == ApprovalStatus::Verified
            && self.trust_status == TrustStatus::Trusted
            && self.health_score >= 0.55
    }

    pub fn requires_gate(&self) -> bool {
        matches!(self.risk, CapabilityRisk::High | CapabilityRisk::Medium)
    }

    pub fn supply_chain_findings(&self) -> Vec<String> {
        let mut findings = Vec::new();
        let payload_hash = supply_chain_payload_hash(self);
        if self.signature.signed_payload_hash != payload_hash {
            findings.push("signature payload hash mismatch".into());
        }
        let expected_signature = signature_value_for(
            &self.signature.signer,
            &self.signature.algorithm,
            &payload_hash,
        );
        if self.signature.signature != expected_signature {
            findings.push("signature verification failed".into());
        }
        if !self
            .trust_policy
            .required_signers
            .iter()
            .any(|signer| signer == &self.signature.signer)
        {
            findings.push(format!(
                "signer '{}' is not allowed by trust policy",
                self.signature.signer
            ));
        }
        if self.provenance.source_ref.len() < self.trust_policy.min_provenance_ref_len {
            findings.push("provenance source_ref is too short".into());
        }
        for component in &self.sbom.components {
            if self
                .trust_policy
                .blocked_dependencies
                .iter()
                .any(|blocked| component.name.eq_ignore_ascii_case(blocked))
            {
                findings.push(format!("blocked dependency detected: {}", component.name));
            }
            if component.checksum.trim().is_empty() {
                findings.push(format!("dependency '{}' missing checksum", component.name));
            }
        }
        findings
    }
}

impl Default for ForgedMcpToolManifest {
    fn default() -> Self {
        Self {
            capability_id: "capability:default".into(),
            registered_tool_name: "mcp::local-mcp::default".into(),
            delegate_tool_name: "mcp::local-mcp::invoke".into(),
            server: "local-mcp".into(),
            capability_name: "default".into(),
            purpose: "default capability".into(),
            executable: "autoloop-cli".into(),
            command_template: "autoloop-cli".into(),
            payload_template: json!({}),
            output_mode: CliOutputMode::Json,
            working_directory: Some(".".into()),
            success_signal: None,
            help_text: String::new(),
            skill_markdown: String::new(),
            examples: Vec::new(),
            version: 1,
            lineage_key: "capability:default".into(),
            status: CapabilityStatus::Active,
            approval_status: ApprovalStatus::Verified,
            health_score: 0.8,
            scope: CapabilityScope::TaskFamily,
            tags: Vec::new(),
            risk: CapabilityRisk::Low,
            requested_by: "cli-agent".into(),
            created_at_ms: 0,
            updated_at_ms: 0,
            approved_at_ms: None,
            rollback_to_version: None,
            trust_status: TrustStatus::Trusted,
            trust_findings: Vec::new(),
            artifact: default_capability_artifact(),
            signature: default_signature(),
            provenance: default_provenance(),
            sbom: default_sbom(),
            trust_policy: default_trust_policy(),
        }
    }
}

pub struct CliAnythingForgeTool {
    registry: ToolRegistry,
    default_server: String,
}

impl CliAnythingForgeTool {
    pub fn new(registry: ToolRegistry, default_server: impl Into<String>) -> Self {
        Self {
            registry,
            default_server: default_server.into(),
        }
    }
}

#[async_trait]
impl Tool for CliAnythingForgeTool {
    fn name(&self) -> &str {
        "cli::forge_mcp_tool"
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let request: McpToolForgeRequest = serde_json::from_str(arguments)
            .map_err(|error| anyhow!("forge request must be valid JSON: {error}"))?;
        let server = request
            .server
            .clone()
            .unwrap_or_else(|| self.default_server.clone());
        let capability_slug = sanitize_segment(&request.capability_name);
        if capability_slug.is_empty() {
            bail!("capability_name must contain at least one alphanumeric character");
        }
        let tool_name = format!("mcp::{server}::{capability_slug}");
        let manifest = build_manifest(tool_name.clone(), server.clone(), request)?;
        self.registry
            .upsert_governed_manifest(manifest.clone())
            .await?;

        Ok(ToolResult {
            name: self.name().to_string(),
            content: serde_json::to_string_pretty(&manifest)?,
        })
    }
}

pub struct ForgedToolCatalog {
    registry: ToolRegistry,
}

impl ForgedToolCatalog {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ForgedToolCatalog {
    fn name(&self) -> &str {
        "cli::list_forged_tools"
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let requested_server = arguments.trim();
        let manifests = self
            .registry
            .manifests()
            .into_iter()
            .filter(|manifest| requested_server.is_empty() || manifest.server == requested_server)
            .collect::<Vec<_>>();

        Ok(ToolResult {
            name: self.name().to_string(),
            content: serde_json::to_string_pretty(&manifests)?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CapabilityMutationRequest {
    tool_name: String,
    #[serde(default)]
    health_score: Option<f32>,
}

pub struct CapabilityVerifierTool {
    registry: ToolRegistry,
}

impl CapabilityVerifierTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for CapabilityVerifierTool {
    fn name(&self) -> &str {
        "cli::verify_capability"
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let request: CapabilityMutationRequest = serde_json::from_str(arguments)?;
        let manifest = self
            .registry
            .verify_capability(&request.tool_name)
            .await?
            .ok_or_else(|| anyhow!("unknown capability: {}", request.tool_name))?;
        Ok(ToolResult {
            name: self.name().into(),
            content: serde_json::to_string_pretty(&manifest)?,
        })
    }
}

pub struct CapabilityDeprecationTool {
    registry: ToolRegistry,
}

impl CapabilityDeprecationTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for CapabilityDeprecationTool {
    fn name(&self) -> &str {
        "cli::deprecate_capability"
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let request: CapabilityMutationRequest = serde_json::from_str(arguments)?;
        let manifest = self
            .registry
            .deprecate_capability(&request.tool_name, request.health_score.unwrap_or(0.25))
            .await?
            .ok_or_else(|| anyhow!("unknown capability: {}", request.tool_name))?;
        Ok(ToolResult {
            name: self.name().into(),
            content: serde_json::to_string_pretty(&manifest)?,
        })
    }
}

pub struct CapabilityRollbackTool {
    registry: ToolRegistry,
}

impl CapabilityRollbackTool {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for CapabilityRollbackTool {
    fn name(&self) -> &str {
        "cli::rollback_capability"
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let request: CapabilityMutationRequest = serde_json::from_str(arguments)?;
        let manifest = self
            .registry
            .rollback_capability(&request.tool_name)
            .await?
            .ok_or_else(|| anyhow!("no rollback target for capability: {}", request.tool_name))?;
        Ok(ToolResult {
            name: self.name().into(),
            content: serde_json::to_string_pretty(&manifest)?,
        })
    }
}

#[derive(Debug)]
pub struct ForgedMcpTool {
    name: String,
    manifest: ForgedMcpToolManifest,
}

impl ForgedMcpTool {
    pub fn new(name: String, manifest: ForgedMcpToolManifest) -> Self {
        Self { name, manifest }
    }
}

#[async_trait]
impl Tool for ForgedMcpTool {
    fn name(&self) -> &str {
        &self.name
    }

    async fn execute(&self, arguments: &str) -> Result<ToolResult> {
        let payload = parse_invocation(arguments)?;
        let rendered_command = render_command(&self.manifest, &payload)?;
        let response = json!({
            "delegate_tool": self.manifest.delegate_tool_name,
            "server": self.manifest.server,
            "capability_name": self.manifest.capability_name,
            "command": rendered_command,
            "arguments": payload,
            "output_mode": self.manifest.output_mode,
            "working_directory": self.manifest.working_directory,
            "success_signal": self.manifest.success_signal,
            "help_text": self.manifest.help_text,
        });

        Ok(ToolResult {
            name: self.name.clone(),
            content: serde_json::to_string_pretty(&response)?,
        })
    }
}

fn build_manifest(
    tool_name: String,
    server: String,
    request: McpToolForgeRequest,
) -> Result<ForgedMcpToolManifest> {
    if request.executable.trim().is_empty() {
        bail!("executable must not be empty");
    }

    let delegate_tool_name = format!("mcp::{server}::invoke");
    let command_template = build_command_template(&request);
    let payload_template = json!({
        "server": server,
        "capability_name": request.capability_name,
        "command": command_template,
        "arguments": request.arguments.iter().map(|argument| {
            json!({
                "name": argument.name,
                "required": argument.required,
                "example": argument.example,
            })
        }).collect::<Vec<_>>(),
        "output_mode": request.output_mode,
        "working_directory": request.working_directory,
        "success_signal": request.success_signal,
    });
    let help_text = build_help_text(&tool_name, &request);
    let skill_markdown = build_skill_markdown(&tool_name, &server, &request);
    let risk = infer_risk(&request);
    let now_ms = current_time_ms();
    let capability_id = format!("{server}:{}", sanitize_segment(&request.capability_name));
    let status = if matches!(risk, CapabilityRisk::Low) {
        CapabilityStatus::Active
    } else {
        CapabilityStatus::PendingVerification
    };
    let approval_status = if matches!(risk, CapabilityRisk::Low) {
        ApprovalStatus::Verified
    } else {
        ApprovalStatus::Pending
    };
    let approved_at_ms = matches!(approval_status, ApprovalStatus::Verified).then_some(now_ms);
    let signer = request.signer.unwrap_or_else(|| "autoloop-ci".into());
    let source_repo = request
        .source_repo
        .unwrap_or_else(|| "autoloop/forged-capability".into());
    let source_ref = request.source_ref.unwrap_or_else(|| format!("v{now_ms}"));
    let artifact = CapabilityArtifact {
        artifact_id: format!("artifact:{capability_id}:{now_ms}"),
        digest_sha256: format!(
            "{:016x}",
            hash_seed(&format!("{capability_id}:{command_template}:{now_ms}"))
        ),
        source_uri: format!(
            "mcp://{server}/{}",
            sanitize_segment(&request.capability_name)
        ),
        build_epoch: now_ms,
    };
    let provenance = Provenance {
        source_repo,
        source_ref,
        builder: "autoloop-cli-forge".into(),
        generated_by: request
            .requested_by
            .clone()
            .unwrap_or_else(|| "cli-agent".into()),
    };
    let sbom = Sbom {
        components: if request.sbom_components.is_empty() {
            vec![SbomComponent {
                name: request.executable.clone(),
                version: "latest".into(),
                source: "local".into(),
                checksum: format!("{:016x}", hash_seed(&request.executable)),
            }]
        } else {
            request.sbom_components.clone()
        },
    };
    let trust_policy = default_trust_policy();
    let mut manifest = ForgedMcpToolManifest {
        capability_id: capability_id.clone(),
        registered_tool_name: tool_name,
        delegate_tool_name,
        server,
        capability_name: request.capability_name,
        purpose: request.purpose,
        executable: request.executable,
        command_template,
        payload_template,
        output_mode: request.output_mode,
        working_directory: request.working_directory,
        success_signal: request.success_signal,
        help_text,
        skill_markdown,
        examples: request.examples,
        version: 1,
        lineage_key: capability_id,
        status,
        approval_status,
        health_score: initial_health_score(&risk),
        scope: request.scope.unwrap_or(CapabilityScope::TaskFamily),
        tags: request.tags,
        risk,
        requested_by: request.requested_by.unwrap_or_else(|| "cli-agent".into()),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
        approved_at_ms,
        rollback_to_version: None,
        trust_status: TrustStatus::Pending,
        trust_findings: Vec::new(),
        artifact,
        signature: default_signature(),
        provenance,
        sbom,
        trust_policy,
    };
    let payload_hash = supply_chain_payload_hash(&manifest);
    manifest.signature = Signature {
        signer,
        algorithm: SignatureAlgorithm::DeterministicV1,
        signed_payload_hash: payload_hash.clone(),
        signature: signature_value_for(
            manifest.signature.signer.as_str(),
            &SignatureAlgorithm::DeterministicV1,
            &payload_hash,
        ),
        signed_at_ms: now_ms,
    };
    manifest.trust_findings = manifest.supply_chain_findings();
    manifest.trust_status = if manifest.trust_findings.is_empty() {
        TrustStatus::Trusted
    } else {
        TrustStatus::Rejected
    };
    if manifest.trust_status == TrustStatus::Rejected {
        manifest.approval_status = ApprovalStatus::Rejected;
        manifest.status = CapabilityStatus::PendingVerification;
    }
    Ok(manifest)
}

fn build_command_template(request: &McpToolForgeRequest) -> String {
    let mut parts = vec![request.executable.clone()];
    parts.extend(request.subcommands.clone());
    if let Some(json_flag) = &request.json_flag {
        parts.push(json_flag.clone());
    }
    for argument in &request.arguments {
        let placeholder = format!("{{{{{}}}}}", sanitize_segment(&argument.name));
        parts.push(format!(
            "--{} {placeholder}",
            sanitize_segment(&argument.name)
        ));
    }
    parts.join(" ")
}

fn build_help_text(tool_name: &str, request: &McpToolForgeRequest) -> String {
    let arg_list = request
        .arguments
        .iter()
        .map(|argument| {
            format!(
                "- {}{}: {}",
                argument.name,
                if argument.required { " (required)" } else { "" },
                argument.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "{tool_name}\nPurpose: {}\nExecutable: {}\nOutput mode: {:?}\nArguments:\n{}",
        request.purpose,
        request.executable,
        request.output_mode,
        if arg_list.is_empty() {
            "- none".into()
        } else {
            arg_list
        }
    )
}

fn build_skill_markdown(tool_name: &str, server: &str, request: &McpToolForgeRequest) -> String {
    let examples = if request.examples.is_empty() {
        "- No examples provided yet.".to_string()
    } else {
        request
            .examples
            .iter()
            .map(|example| format!("- {example}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "# {tool_name}\n\nUse this forged MCP tool through `{server}` to satisfy: {}\n\n## Contract\n- Deterministic JSON-first CLI wrapper\n- Self-describing argument schema\n- Suitable for CLI agents that need reusable command surfaces\n\n## Examples\n{}",
        request.purpose, examples
    )
}

fn parse_invocation(arguments: &str) -> Result<Value> {
    if arguments.trim().is_empty() {
        return Ok(json!({}));
    }

    let parsed = serde_json::from_str::<Value>(arguments)
        .map_err(|error| anyhow!("forged tool invocation must be valid JSON: {error}"))?;
    if !parsed.is_object() {
        bail!("forged tool invocation must be a JSON object");
    }
    Ok(parsed)
}

pub fn build_command_spec(
    manifest: &ForgedMcpToolManifest,
    arguments: &str,
) -> Result<RenderedCommandSpec> {
    let payload = parse_invocation(arguments)?;
    let rendered = render_command(manifest, &payload)?;
    let object = payload
        .as_object()
        .ok_or_else(|| anyhow!("forged tool invocation must be a JSON object"))?;

    let arg_specs = manifest
        .payload_template
        .get("arguments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut args = manifest.subcommand_segments();
    for spec in arg_specs {
        let name = spec
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("argument schema missing name"))?;
        let required = spec
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let key = sanitize_segment(name);
        match object.get(name).or_else(|| object.get(&key)) {
            Some(value) => {
                args.push(format!("--{key}"));
                args.push(render_value(value));
            }
            None if required => bail!("missing required forged tool argument: {name}"),
            None => {}
        }
    }
    Ok(RenderedCommandSpec {
        executable: manifest.executable.clone(),
        args,
        display_command: rendered,
        working_directory: manifest.working_directory.clone(),
    })
}

fn render_command(manifest: &ForgedMcpToolManifest, payload: &Value) -> Result<String> {
    let mut rendered = manifest.command_template.clone();
    let object = payload
        .as_object()
        .ok_or_else(|| anyhow!("forged tool invocation must be a JSON object"))?;

    let arg_specs = manifest
        .payload_template
        .get("arguments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for spec in arg_specs {
        let name = spec
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("argument schema missing name"))?;
        let required = spec
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let key = sanitize_segment(name);
        let placeholder = format!("{{{{{key}}}}}");
        match object.get(name).or_else(|| object.get(&key)) {
            Some(value) => {
                let value = render_value(value);
                rendered = rendered.replace(&placeholder, &shell_escape(&value));
            }
            None if required => bail!("missing required forged tool argument: {name}"),
            None => {
                rendered = rendered.replace(&format!(" --{key} {placeholder}"), "");
            }
        }
    }

    Ok(rendered)
}

impl ForgedMcpToolManifest {
    fn subcommand_segments(&self) -> Vec<String> {
        let command = self.command_template.trim();
        let executable = self.executable.trim();
        let tokens = command
            .strip_prefix(executable)
            .unwrap_or(command)
            .split_whitespace()
            .filter(|segment| !segment.is_empty())
            .map(|segment| segment.to_string())
            .collect::<Vec<_>>();
        let mut segments = Vec::new();
        let mut index = 0usize;
        while index < tokens.len() {
            let token = &tokens[index];
            if token == executable {
                index += 1;
                continue;
            }
            if token.contains("{{") {
                index += 1;
                continue;
            }
            if token.starts_with("--")
                && tokens
                    .get(index + 1)
                    .is_some_and(|next| next.contains("{{"))
            {
                index += 2;
                continue;
            }
            segments.push(token.clone());
            index += 1;
        }
        segments
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(render_value)
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => value.to_string(),
    }
}

fn shell_escape(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "-_./:".contains(ch))
    {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

pub fn sanitize_segment(value: &str) -> String {
    let mut sanitized = String::new();
    let mut previous_dash = false;

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            sanitized.push('-');
            previous_dash = true;
        }
    }

    sanitized.trim_matches('-').to_string()
}

fn infer_risk(request: &McpToolForgeRequest) -> CapabilityRisk {
    let purpose = request.purpose.to_ascii_lowercase();
    let executable = request.executable.to_ascii_lowercase();
    if purpose.contains("delete")
        || purpose.contains("deploy")
        || purpose.contains("network")
        || executable.contains("powershell")
        || executable.contains("bash")
    {
        CapabilityRisk::High
    } else if purpose.contains("write") || purpose.contains("modify") || purpose.contains("patch") {
        CapabilityRisk::Medium
    } else {
        CapabilityRisk::Low
    }
}

fn initial_health_score(risk: &CapabilityRisk) -> f32 {
    match risk {
        CapabilityRisk::Low => 0.82,
        CapabilityRisk::Medium => 0.65,
        CapabilityRisk::High => 0.45,
    }
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn default_capability_version() -> u32 {
    1
}
fn default_capability_status() -> CapabilityStatus {
    CapabilityStatus::Active
}
fn default_approval_status() -> ApprovalStatus {
    ApprovalStatus::Verified
}
fn default_health_score() -> f32 {
    0.8
}
fn default_scope() -> CapabilityScope {
    CapabilityScope::TaskFamily
}
fn default_risk() -> CapabilityRisk {
    CapabilityRisk::Low
}
fn default_requested_by() -> String {
    "cli-agent".into()
}
fn default_trust_status() -> TrustStatus {
    TrustStatus::Pending
}
fn default_capability_artifact() -> CapabilityArtifact {
    CapabilityArtifact {
        artifact_id: "artifact:default".into(),
        digest_sha256: "0000000000000000".into(),
        source_uri: "mcp://local/default".into(),
        build_epoch: 0,
    }
}
fn default_signature() -> Signature {
    Signature {
        signer: "autoloop-ci".into(),
        algorithm: SignatureAlgorithm::DeterministicV1,
        signed_payload_hash: "0000000000000000".into(),
        signature: "sig:autoloop-ci:deterministic_v1:0000000000000000".into(),
        signed_at_ms: 0,
    }
}
fn default_provenance() -> Provenance {
    Provenance {
        source_repo: "autoloop/forged-capability".into(),
        source_ref: "unknown".into(),
        builder: "autoloop-cli-forge".into(),
        generated_by: "cli-agent".into(),
    }
}
fn default_sbom() -> Sbom {
    Sbom {
        components: Vec::new(),
    }
}
fn default_trust_policy() -> TrustPolicy {
    TrustPolicy {
        required_signers: vec!["autoloop-ci".into(), "autoloop-operator".into()],
        blocked_dependencies: vec!["evil-lib".into(), "malware-kit".into()],
        min_provenance_ref_len: 3,
    }
}

fn hash_seed(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn supply_chain_payload_hash(manifest: &ForgedMcpToolManifest) -> String {
    let mut hasher = DefaultHasher::new();
    manifest.capability_id.hash(&mut hasher);
    manifest.registered_tool_name.hash(&mut hasher);
    manifest.delegate_tool_name.hash(&mut hasher);
    manifest.server.hash(&mut hasher);
    manifest.executable.hash(&mut hasher);
    manifest.command_template.hash(&mut hasher);
    manifest.version.hash(&mut hasher);
    manifest.lineage_key.hash(&mut hasher);
    manifest.artifact.artifact_id.hash(&mut hasher);
    manifest.artifact.digest_sha256.hash(&mut hasher);
    manifest.artifact.source_uri.hash(&mut hasher);
    manifest.artifact.build_epoch.hash(&mut hasher);
    manifest.provenance.source_repo.hash(&mut hasher);
    manifest.provenance.source_ref.hash(&mut hasher);
    manifest.provenance.builder.hash(&mut hasher);
    for component in &manifest.sbom.components {
        component.name.hash(&mut hasher);
        component.version.hash(&mut hasher);
        component.source.hash(&mut hasher);
        component.checksum.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn signature_value_for(signer: &str, algorithm: &SignatureAlgorithm, payload_hash: &str) -> String {
    let algorithm_name = match algorithm {
        SignatureAlgorithm::DeterministicV1 => "deterministic_v1",
    };
    format!("sig:{signer}:{algorithm_name}:{payload_hash}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ToolsConfig;
    use autoloop_state_adapter::{StateStoreBackend, StateStore, StateStoreConfig};

    #[tokio::test]
    async fn forge_tool_registers_new_mcp_capability() {
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });

        let result = registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"image batch export",
                    "purpose":"Turn batch exports into a reusable MCP surface",
                    "executable":"image-cli",
                    "subcommands":["batch","export"],
                    "json_flag":"--json",
                    "arguments":[
                        {"name":"input","description":"input directory","required":true},
                        {"name":"format","description":"output format","required":false,"example":"png"}
                    ],
                    "output_mode":"json",
                    "success_signal":"completed"
                }"#,
            )
            .await
            .expect("forge tool");

        assert!(
            result
                .content
                .contains("\"registered_tool_name\": \"mcp::local-mcp::image-batch-export\"")
        );
        assert!(registry.has_tool("mcp::local-mcp::image-batch-export"));
    }

    #[tokio::test]
    async fn forged_tool_renders_command_payload() {
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });
        registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"diagram export",
                    "purpose":"Export diagrams through a stable CLI wrapper",
                    "executable":"diagram-cli",
                    "subcommands":["export"],
                    "arguments":[
                        {"name":"project","description":"project path","required":true},
                        {"name":"theme","description":"theme name","required":false}
                    ],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge tool");

        let result = registry
            .execute(
                "mcp::local-mcp::diagram-export",
                r#"{"project":"D:/demo/project.drawio","theme":"clean light"}"#,
            )
            .await
            .expect("execute forged tool");

        assert!(
            result
                .content
                .contains("\"delegate_tool\": \"mcp::local-mcp::invoke\"")
        );
        assert!(result.content.contains("diagram-cli export"));
        assert!(result.content.contains("--project D:/demo/project.drawio"));
    }

    #[tokio::test]
    async fn forged_tools_persist_and_restore_from_state_store() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });
        registry.attach_state_store(db.clone());

        registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"session replay",
                    "purpose":"Rebuild session-level CLI tooling from persisted manifests",
                    "executable":"autoloop-cli",
                    "subcommands":["session","replay"],
                    "arguments":[
                        {"name":"session","description":"session id","required":true}
                    ],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge and persist");

        let recovered = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });
        recovered.attach_state_store(db);
        let restored = recovered
            .restore_persisted_manifests()
            .await
            .expect("restore");

        assert_eq!(restored, 1);
        assert!(recovered.has_tool("mcp::local-mcp::session-replay"));
        assert_eq!(recovered.manifests().len(), 1);
    }

    #[tokio::test]
    async fn governance_tools_change_capability_execution_state() {
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });

        registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"network deploy",
                    "purpose":"Deploy over network to remote target",
                    "executable":"deploy-cli",
                    "arguments":[{"name":"target","description":"host","required":true}],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge");

        assert!(
            registry
                .execute("mcp::local-mcp::network-deploy", r#"{"target":"prod"}"#)
                .await
                .is_err()
        );

        registry
            .execute(
                "cli::verify_capability",
                r#"{"tool_name":"mcp::local-mcp::network-deploy"}"#,
            )
            .await
            .expect("verify");
        assert!(
            registry
                .execute("mcp::local-mcp::network-deploy", r#"{"target":"prod"}"#)
                .await
                .is_ok()
        );

        registry
            .execute(
                "cli::deprecate_capability",
                r#"{"tool_name":"mcp::local-mcp::network-deploy","health_score":0.2}"#,
            )
            .await
            .expect("deprecate");
        assert!(
            registry
                .execute("mcp::local-mcp::network-deploy", r#"{"target":"prod"}"#)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn p9_signature_forgery_is_rejected() {
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });
        registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"trusted-read",
                    "purpose":"trusted local read tool",
                    "executable":"read-cli",
                    "arguments":[{"name":"path","description":"path","required":true}],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge");
        let mut manifest = registry
            .manifests()
            .into_iter()
            .find(|item| item.registered_tool_name == "mcp::local-mcp::trusted-read")
            .expect("manifest");
        manifest.signature.signature = "sig:forged".into();
        registry.hydrate_manifest(manifest);
        let verify = registry
            .verify_capability("mcp::local-mcp::trusted-read")
            .await;
        assert!(verify.is_err());
    }

    #[tokio::test]
    async fn p9_version_rollback_attack_is_rejected() {
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });
        registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"rollback-check",
                    "purpose":"rollback detection",
                    "executable":"tool-cli",
                    "arguments":[{"name":"arg","description":"arg","required":true}],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge v1");
        let mut v2 = registry
            .manifests()
            .into_iter()
            .find(|item| item.registered_tool_name == "mcp::local-mcp::rollback-check")
            .expect("manifest");
        v2.version = 2;
        v2.updated_at_ms = current_time_ms().saturating_add(1);
        let payload_hash = supply_chain_payload_hash(&v2);
        v2.signature.signed_payload_hash = payload_hash.clone();
        v2.signature.signature =
            signature_value_for(&v2.signature.signer, &v2.signature.algorithm, &payload_hash);
        registry.hydrate_manifest(v2.clone());
        registry
            .verify_capability("mcp::local-mcp::rollback-check")
            .await
            .expect("verify v2");

        let mut rollback = v2;
        rollback.version = 1;
        rollback.updated_at_ms = current_time_ms().saturating_add(2);
        let rollback_hash = supply_chain_payload_hash(&rollback);
        rollback.signature.signed_payload_hash = rollback_hash.clone();
        rollback.signature.signature = signature_value_for(
            &rollback.signature.signer,
            &rollback.signature.algorithm,
            &rollback_hash,
        );
        registry.hydrate_manifest(rollback);
        let verify = registry
            .verify_capability("mcp::local-mcp::rollback-check")
            .await;
        assert!(verify.is_err());
    }

    #[tokio::test]
    async fn p9_dependency_poisoning_is_rejected() {
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });
        let forge = registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"poisoned-tool",
                    "purpose":"test dependency poisoning guard",
                    "executable":"tool-cli",
                    "arguments":[{"name":"arg","description":"arg","required":true}],
                    "sbom_components":[{"name":"evil-lib","version":"1.0.0","source":"third-party","checksum":"deadbeef"}],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge poisoned");
        assert!(forge.content.contains("\"trust_status\": \"rejected\""));
        let verify = registry
            .verify_capability("mcp::local-mcp::poisoned-tool")
            .await;
        assert!(verify.is_err());
    }

    #[tokio::test]
    async fn p9_unapproved_or_untrusted_capability_cannot_execute() {
        let registry = ToolRegistry::from_config(&ToolsConfig {
            builtin: vec!["read_file".into()],
            allow_shell: false,
            mcp_servers: vec!["local-mcp".into()],
        });
        registry
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"strict-exec-gate",
                    "purpose":"verify trusted gate",
                    "executable":"tool-cli",
                    "arguments":[{"name":"arg","description":"arg","required":true}],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge");
        let mut manifest = registry
            .manifests()
            .into_iter()
            .find(|item| item.registered_tool_name == "mcp::local-mcp::strict-exec-gate")
            .expect("manifest");
        manifest.trust_status = TrustStatus::Rejected;
        manifest.trust_findings = vec!["manual reject".into()];
        registry.hydrate_manifest(manifest);
        let result = registry
            .execute("mcp::local-mcp::strict-exec-gate", r#"{"arg":"x"}"#)
            .await;
        assert!(result.is_err());
    }
}

