use std::{
    collections::BTreeMap,
    io::Write,
    process::{Command, Stdio},
    sync::Arc,
};

use anyhow::{Result, anyhow, bail};
use tokio::sync::RwLock;

use crate::contracts::plugin::{
    PluginApiNegotiationRequest, PluginApiNegotiationResult, PluginErrorCode, PluginExecutionError,
    PluginInvocationInput, PluginInvocationOutput, PluginManifestContract, PluginRuntimeContract,
};

use super::{
    facade_guard::{enforce_invocation_contract, enforce_manifest_contract},
    registry::PluginRegistry,
    resolver::{PluginResolveRequest, PluginResolver, ResolvedPlugin},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExternalPluginLoadRequest {
    pub manifest: PluginManifestContract,
    pub entrypoint: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

pub trait ExternalPluginLoader: Send + Sync {
    fn load(&self, request: &ExternalPluginLoadRequest) -> Result<Arc<dyn PluginRuntimeContract>>;
}

#[derive(Debug, Clone, Default)]
pub struct SubprocessPluginLoader;

impl ExternalPluginLoader for SubprocessPluginLoader {
    fn load(&self, request: &ExternalPluginLoadRequest) -> Result<Arc<dyn PluginRuntimeContract>> {
        let (command, args) = parse_entrypoint(&request.entrypoint)?;
        let runtime = SubprocessPluginRuntime {
            manifest: request.manifest.clone(),
            command,
            args,
        };
        Ok(Arc::new(runtime))
    }
}

#[derive(Debug, Clone)]
struct SubprocessPluginRuntime {
    manifest: PluginManifestContract,
    command: String,
    args: Vec<String>,
}

impl PluginRuntimeContract for SubprocessPluginRuntime {
    fn manifest(&self) -> &PluginManifestContract {
        &self.manifest
    }

    fn negotiate_api(&self, request: &PluginApiNegotiationRequest) -> PluginApiNegotiationResult {
        if !self
            .manifest
            .compat
            .supports_api_version(&request.host_api_version)
        {
            return PluginApiNegotiationResult {
                accepted: false,
                plugin_id: self.manifest.id.clone(),
                selected_api_version: self.manifest.compat.api_version.clone(),
                reason: format!(
                    "unsupported api_version host={} plugin={}",
                    request.host_api_version, self.manifest.compat.api_version
                ),
            };
        }
        PluginApiNegotiationResult {
            accepted: true,
            plugin_id: self.manifest.id.clone(),
            selected_api_version: request.host_api_version.clone(),
            reason: "api negotiation accepted".into(),
        }
    }

    fn invoke(
        &self,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput, PluginExecutionError> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                plugin_exec_error(PluginErrorCode::ExecutionFailed, error.to_string())
            })?;

        let payload = serde_json::to_vec(input)
            .map_err(|error| plugin_exec_error(PluginErrorCode::InvalidInput, error.to_string()))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&payload).map_err(|error| {
                plugin_exec_error(PluginErrorCode::ExecutionFailed, error.to_string())
            })?;
        }

        let output = child.wait_with_output().map_err(|error| {
            plugin_exec_error(PluginErrorCode::ExecutionFailed, error.to_string())
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(plugin_exec_error(
                PluginErrorCode::ExecutionFailed,
                format!(
                    "subprocess plugin failed status={} stderr={}",
                    output.status, stderr
                ),
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            return Ok(PluginInvocationOutput {
                invocation_id: input.invocation_id.clone(),
                plugin_id: self.manifest.id.clone(),
                status: "ok".into(),
                payload: serde_json::json!({"subprocess":"completed","output":""}),
                evidence_refs: Vec::new(),
                warnings: vec!["subprocess returned empty output; fallback payload used".into()],
            });
        }

        serde_json::from_str::<PluginInvocationOutput>(&stdout).map_err(|error| {
            plugin_exec_error(
                PluginErrorCode::ExecutionFailed,
                format!("subprocess output parse failed: {}", error),
            )
        })
    }
}

#[derive(Clone)]
pub struct PluginHost {
    registry: PluginRegistry,
    resolver: PluginResolver,
    plugins: Arc<RwLock<BTreeMap<String, Arc<dyn PluginRuntimeContract>>>>,
}

impl PluginHost {
    pub fn new(host_api_version: impl Into<String>) -> Self {
        let registry = PluginRegistry::new();
        let resolver = PluginResolver::new(registry.clone(), host_api_version);
        Self {
            registry,
            resolver,
            plugins: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    pub fn resolver(&self) -> &PluginResolver {
        &self.resolver
    }

    pub async fn register_builtin(&self, plugin: Arc<dyn PluginRuntimeContract>) -> Result<()> {
        let manifest = plugin.manifest().clone();
        enforce_manifest_contract(&manifest, true)?;
        self.registry.register_builtin(manifest.clone()).await?;
        self.plugins
            .write()
            .await
            .insert(manifest.id.clone(), plugin);
        Ok(())
    }

    pub async fn load_external(
        &self,
        request: ExternalPluginLoadRequest,
        loader: &dyn ExternalPluginLoader,
    ) -> Result<()> {
        let plugin = loader.load(&request)?;
        let runtime_manifest = plugin.manifest().clone();
        if runtime_manifest.id != request.manifest.id {
            bail!(
                "external loader manifest mismatch request={} runtime={}",
                request.manifest.id,
                runtime_manifest.id
            );
        }
        enforce_manifest_contract(&request.manifest, false)?;

        self.registry
            .register_external(request.manifest.clone(), Some(request.entrypoint.clone()))
            .await?;
        self.plugins
            .write()
            .await
            .insert(runtime_manifest.id.clone(), plugin);
        Ok(())
    }

    pub async fn invoke(
        &self,
        resolve_request: &PluginResolveRequest,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput> {
        let resolved = self.resolver.resolve(resolve_request).await?;
        self.invoke_resolved(&resolved, input).await
    }

    pub async fn invoke_by_id(
        &self,
        plugin_id: &str,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput> {
        let resolved = self
            .resolver
            .resolve(&PluginResolveRequest {
                plugin_id: Some(plugin_id.to_string()),
                capability_id: None,
                kind: None,
                required_scopes: Vec::new(),
            })
            .await?;
        self.invoke_resolved(&resolved, input).await
    }

    async fn invoke_resolved(
        &self,
        resolved: &ResolvedPlugin,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput> {
        let plugin = self
            .plugins
            .read()
            .await
            .get(&resolved.manifest.id)
            .cloned()
            .ok_or_else(|| anyhow!("plugin runtime not loaded: {}", resolved.manifest.id))?;

        enforce_invocation_contract(&resolved.manifest.facade, input)?;

        plugin.invoke(input).map_err(|error| {
            anyhow!(
                "plugin invoke failed [{}]: {}",
                resolved.manifest.id,
                error.message
            )
        })
    }

    pub async fn list_manifests(&self) -> Vec<PluginManifestContract> {
        self.registry
            .list()
            .await
            .into_iter()
            .map(|record| record.manifest)
            .collect()
    }
}

fn parse_entrypoint(entrypoint: &str) -> Result<(String, Vec<String>)> {
    let raw = entrypoint.trim();
    let value = raw.strip_prefix("proc://").unwrap_or(raw).trim();
    if value.is_empty() {
        bail!("plugin entrypoint cannot be empty");
    }
    let mut parts = value
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if parts.is_empty() {
        bail!("plugin entrypoint cannot be parsed");
    }
    let command = parts.remove(0);
    Ok((command, parts))
}

fn plugin_exec_error(code: PluginErrorCode, message: String) -> PluginExecutionError {
    PluginExecutionError {
        code,
        message,
        retryable: false,
        details: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::plugin::{
        PluginCapabilityDescriptor, PluginCompatSpec, PluginKind, PluginRisk,
    };

    struct DummyPlugin {
        manifest: PluginManifestContract,
    }

    impl PluginRuntimeContract for DummyPlugin {
        fn manifest(&self) -> &PluginManifestContract {
            &self.manifest
        }

        fn negotiate_api(
            &self,
            request: &PluginApiNegotiationRequest,
        ) -> PluginApiNegotiationResult {
            PluginApiNegotiationResult {
                accepted: true,
                plugin_id: self.manifest.id.clone(),
                selected_api_version: request.host_api_version.clone(),
                reason: "ok".into(),
            }
        }

        fn invoke(
            &self,
            input: &PluginInvocationInput,
        ) -> Result<PluginInvocationOutput, PluginExecutionError> {
            Ok(PluginInvocationOutput {
                invocation_id: input.invocation_id.clone(),
                plugin_id: self.manifest.id.clone(),
                status: "ok".into(),
                payload: serde_json::json!({"echo":true}),
                evidence_refs: Vec::new(),
                warnings: Vec::new(),
            })
        }
    }

    fn dummy_manifest() -> PluginManifestContract {
        PluginManifestContract {
            id: "plugin:test".into(),
            plugin_id: "plugin:test".into(),
            version: "v2".into(),
            kind: PluginKind::SourceAdapter,
            capability: PluginCapabilityDescriptor {
                capability_id: "plugin:test:invoke".into(),
                description: "test".into(),
                scopes: vec!["plugin.invoke".into()],
            },
            risk: PluginRisk::Low,
            compat: PluginCompatSpec {
                api_version: "v2".into(),
                compatible_api_versions: vec!["v2".into()],
                min_core_version: "v2".into(),
                max_core_version: None,
            },
            name: "plugin-test".into(),
            source: "builtin://dummy".into(),
            signature_ref: None,
            permissions: vec![],
            hooks: vec![],
            commands: vec![],
            event_contract_version: "plugin-event-v2".into(),
            lifecycle_contract_version: "plugin-lifecycle-v2".into(),
            isolation: crate::contracts::plugin::PluginIsolationContract::default(),
            facade: crate::contracts::plugin::PluginFacadeContract::default(),
            metadata: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn host_rejects_repo_bypass_payload() {
        let host = PluginHost::new("v2");
        host.register_builtin(Arc::new(DummyPlugin {
            manifest: dummy_manifest(),
        }))
        .await
        .expect("register plugin");

        let result = host
            .invoke_by_id(
                "plugin:test",
                &PluginInvocationInput {
                    invocation_id: "inv-1".into(),
                    plugin_id: "plugin:test".into(),
                    session_id: "s1".into(),
                    tenant_id: "t1".into(),
                    principal_id: "p1".into(),
                    capability_id: "plugin:test:invoke".into(),
                    payload: serde_json::json!({"repo_path":"D:/canonical"}),
                    metadata: BTreeMap::from([(
                        "facade_channel".into(),
                        "plugin-facade-v2".into(),
                    )]),
                },
            )
            .await;

        assert!(result.is_err());
        assert!(
            result
                .err()
                .expect("error")
                .to_string()
                .contains("facade contract violation")
        );
    }

    #[test]
    fn entrypoint_parsing_supports_proc_scheme() {
        let (command, args) = parse_entrypoint("proc://echo hello world").expect("parse");
        assert_eq!(command, "echo");
        assert_eq!(args, vec!["hello", "world"]);
    }
}
