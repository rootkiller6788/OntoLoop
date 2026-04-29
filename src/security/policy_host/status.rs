use std::path::{Path, PathBuf};

use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::discovery::PolicyDiscoveryState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRuntimeHealth {
    pub status: String,
    pub mode: String,
    pub decisions_observed: u64,
    pub shadow_diffs_observed: u64,
    pub last_checked_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundleHealth {
    pub status: String,
    pub root_dir: String,
    pub current_exists: bool,
    pub candidate_exists: bool,
    pub rollback_exists: bool,
    pub manifest_present: bool,
    pub active_policy_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDiscoveryHealth {
    pub status: String,
    pub etag: Option<String>,
    pub consecutive_failures: u32,
    pub last_checked_at_ms: Option<u64>,
    pub last_stable_version: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyStatusReport {
    pub runtime: PolicyRuntimeHealth,
    pub bundle: PolicyBundleHealth,
    pub discovery: PolicyDiscoveryHealth,
    pub generated_at_ms: u64,
}

pub async fn collect_policy_status(db: &StateStore, bundle_root: impl AsRef<Path>) -> Result<PolicyStatusReport> {
    let runtime = collect_runtime_health(db).await?;
    let bundle = collect_bundle_health(bundle_root.as_ref())?;
    let discovery = collect_discovery_health(db).await?;

    Ok(PolicyStatusReport {
        runtime,
        bundle,
        discovery,
        generated_at_ms: current_time_ms(),
    })
}

async fn collect_runtime_health(db: &StateStore) -> Result<PolicyRuntimeHealth> {
    let mut decisions = db
        .list_knowledge_by_prefix("policy-pdp:evaluate:")
        .await?;
    let mut shadow_diffs = db
        .list_knowledge_by_prefix("policy-pdp:shadow-diff:")
        .await?;

    decisions.sort_by(|left, right| left.key.cmp(&right.key));
    shadow_diffs.sort_by(|left, right| left.key.cmp(&right.key));

    let latest_mode = decisions
        .last()
        .and_then(|record| serde_json::from_str::<Value>(&record.value).ok())
        .and_then(|value| value.get("mode").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string());
    let last_checked_at_ms = decisions
        .last()
        .map(|record| extract_timestamp_from_key(&record.key))
        .filter(|value| *value > 0);

    let status = if decisions.is_empty() {
        "degraded".to_string()
    } else {
        "healthy".to_string()
    };

    Ok(PolicyRuntimeHealth {
        status,
        mode: latest_mode,
        decisions_observed: decisions.len() as u64,
        shadow_diffs_observed: shadow_diffs.len() as u64,
        last_checked_at_ms,
    })
}

fn collect_bundle_health(root_dir: &Path) -> Result<PolicyBundleHealth> {
    let root_dir = root_dir.to_path_buf();
    let current_dir = root_dir.join("current");
    let candidate_dir = root_dir.join("candidate");
    let rollback_dir = root_dir.join("rollback");
    let manifest_path = current_dir.join("manifest.json");
    let active_policy_version = read_policy_version_from_manifest(&manifest_path);

    let current_exists = current_dir.exists();
    let candidate_exists = candidate_dir.exists();
    let rollback_exists = rollback_dir.exists();
    let manifest_present = manifest_path.exists();
    let status = if current_exists && manifest_present {
        "healthy"
    } else if current_exists {
        "degraded"
    } else {
        "offline"
    }
    .to_string();

    Ok(PolicyBundleHealth {
        status,
        root_dir: path_to_string(root_dir),
        current_exists,
        candidate_exists,
        rollback_exists,
        manifest_present,
        active_policy_version,
    })
}

async fn collect_discovery_health(db: &StateStore) -> Result<PolicyDiscoveryHealth> {
    let state = db
        .get_knowledge("policy:discovery:state:latest")
        .await?
        .and_then(|record| serde_json::from_str::<PolicyDiscoveryState>(&record.value).ok());

    if let Some(state) = state {
        let status = if state.consecutive_failures == 0 {
            "healthy"
        } else if state.consecutive_failures < 3 {
            "degraded"
        } else {
            "failing"
        }
        .to_string();
        return Ok(PolicyDiscoveryHealth {
            status,
            etag: state.etag,
            consecutive_failures: state.consecutive_failures,
            last_checked_at_ms: if state.last_checked_at_ms == 0 {
                None
            } else {
                Some(state.last_checked_at_ms)
            },
            last_stable_version: state.last_stable_version.map(|version| version.id),
            last_error: state.last_error,
        });
    }

    Ok(PolicyDiscoveryHealth {
        status: "degraded".to_string(),
        etag: None,
        consecutive_failures: 0,
        last_checked_at_ms: None,
        last_stable_version: None,
        last_error: Some("discovery state not initialized".to_string()),
    })
}

fn read_policy_version_from_manifest(path: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(path).ok()?;
    let manifest = serde_json::from_str::<Value>(&raw).ok()?;
    manifest
        .get("policy_version")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().to_string()
}

fn extract_timestamp_from_key(key: &str) -> u64 {
    key.rsplit(':')
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn status_report_collects_runtime_signals() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let now = current_time_ms();
        db.upsert_json_knowledge(
            format!("policy-pdp:evaluate:test-session:test-task:{now}"),
            &serde_json::json!({"mode":"shadow"}),
            "policy-pdp",
        )
        .await
        .expect("seed evaluate");

        let report = collect_policy_status(&db, std::env::temp_dir().join("policy-status-test"))
            .await
            .expect("collect status");
        assert!(report.runtime.decisions_observed >= 1);
    }
}

