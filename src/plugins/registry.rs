use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Result, bail};
use tokio::sync::RwLock;

use crate::contracts::plugin::{PluginManifestContract, PluginRuntimeContract};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginSourceKind {
    BuiltIn,
    External,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginRegistration {
    pub manifest: PluginManifestContract,
    pub source_kind: PluginSourceKind,
    pub loaded_at_ms: u64,
    pub entrypoint: Option<String>,
}

#[derive(Clone, Default)]
pub struct PluginRegistry {
    registrations: Arc<RwLock<BTreeMap<String, PluginRegistration>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register_builtin(&self, manifest: PluginManifestContract) -> Result<()> {
        self.register(manifest, PluginSourceKind::BuiltIn, None)
            .await
    }

    pub async fn register_external(
        &self,
        manifest: PluginManifestContract,
        entrypoint: Option<String>,
    ) -> Result<()> {
        self.register(manifest, PluginSourceKind::External, entrypoint)
            .await
    }

    async fn register(
        &self,
        manifest: PluginManifestContract,
        source_kind: PluginSourceKind,
        entrypoint: Option<String>,
    ) -> Result<()> {
        if manifest.id.trim().is_empty() {
            bail!("plugin manifest id cannot be empty");
        }
        let id = manifest.id.clone();
        let record = PluginRegistration {
            manifest,
            source_kind,
            loaded_at_ms: current_time_ms(),
            entrypoint,
        };
        self.registrations.write().await.insert(id, record);
        Ok(())
    }

    pub async fn get(&self, plugin_id: &str) -> Option<PluginRegistration> {
        self.registrations.read().await.get(plugin_id).cloned()
    }

    pub async fn list(&self) -> Vec<PluginRegistration> {
        let mut records = self
            .registrations
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        records
    }

    pub async fn contains(&self, plugin_id: &str) -> bool {
        self.registrations.read().await.contains_key(plugin_id)
    }

    pub async fn register_runtime_contract(
        &self,
        plugin: &dyn PluginRuntimeContract,
    ) -> Result<()> {
        self.register_builtin(plugin.manifest().clone()).await
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
