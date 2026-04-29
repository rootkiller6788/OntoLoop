use anyhow::{Result, bail};

use crate::contracts::plugin::{PluginApiNegotiationResult, PluginKind, PluginManifestContract};

use super::registry::{PluginRegistration, PluginRegistry};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PluginResolveRequest {
    pub plugin_id: Option<String>,
    pub capability_id: Option<String>,
    pub kind: Option<PluginKind>,
    #[serde(default)]
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ResolvedPlugin {
    pub manifest: PluginManifestContract,
    pub negotiation: PluginApiNegotiationResult,
}

#[derive(Clone)]
pub struct PluginResolver {
    registry: PluginRegistry,
    host_api_version: String,
}

impl PluginResolver {
    pub fn new(registry: PluginRegistry, host_api_version: impl Into<String>) -> Self {
        Self {
            registry,
            host_api_version: host_api_version.into(),
        }
    }

    pub fn host_api_version(&self) -> &str {
        &self.host_api_version
    }

    pub async fn resolve(&self, request: &PluginResolveRequest) -> Result<ResolvedPlugin> {
        let candidates = self.candidates(request).await;
        let Some(chosen) = candidates.into_iter().next() else {
            bail!("no plugin candidate matched resolve request");
        };

        let negotiation = negotiate_with_manifest(
            &chosen.manifest,
            &self.host_api_version,
            &request.required_scopes,
        );
        if !negotiation.accepted {
            bail!("plugin api negotiation rejected: {}", negotiation.reason);
        }

        Ok(ResolvedPlugin {
            manifest: chosen.manifest,
            negotiation,
        })
    }

    async fn candidates(&self, request: &PluginResolveRequest) -> Vec<PluginRegistration> {
        let mut records = self.registry.list().await;

        if let Some(plugin_id) = request.plugin_id.as_deref() {
            records.retain(|record| record.manifest.id == plugin_id);
        }
        if let Some(capability_id) = request.capability_id.as_deref() {
            records.retain(|record| record.manifest.capability.capability_id == capability_id);
        }
        if let Some(kind) = request.kind.as_ref() {
            records.retain(|record| &record.manifest.kind == kind);
        }
        if !request.required_scopes.is_empty() {
            records.retain(|record| {
                request.required_scopes.iter().all(|scope| {
                    record
                        .manifest
                        .capability
                        .scopes
                        .iter()
                        .any(|candidate| candidate == scope)
                })
            });
        }

        records
    }
}

pub fn negotiate_with_manifest(
    manifest: &PluginManifestContract,
    host_api_version: &str,
    required_scopes: &[String],
) -> PluginApiNegotiationResult {
    if !manifest.compat.supports_api_version(host_api_version) {
        return PluginApiNegotiationResult {
            accepted: false,
            plugin_id: manifest.id.clone(),
            selected_api_version: manifest.compat.api_version.clone(),
            reason: format!(
                "unsupported api_version host={} plugin={}",
                host_api_version, manifest.compat.api_version
            ),
        };
    }

    let missing_scope = required_scopes.iter().find(|scope| {
        !manifest
            .capability
            .scopes
            .iter()
            .any(|candidate| candidate == *scope)
    });
    if let Some(scope) = missing_scope {
        return PluginApiNegotiationResult {
            accepted: false,
            plugin_id: manifest.id.clone(),
            selected_api_version: host_api_version.to_string(),
            reason: format!("required scope '{}' not granted by plugin", scope),
        };
    }

    PluginApiNegotiationResult {
        accepted: true,
        plugin_id: manifest.id.clone(),
        selected_api_version: host_api_version.to_string(),
        reason: "api negotiation accepted".into(),
    }
}
