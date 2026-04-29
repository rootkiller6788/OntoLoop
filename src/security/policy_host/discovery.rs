use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use reqwest::header::{ETAG, IF_NONE_MATCH};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::contracts::policy_pdp::PolicyVersion;

use super::bundle::{BundleActivationManager, LoadedPolicyBundle};
use super::verify::{
    PolicyBundleVerifier, PolicyBundleVerifyRequirements, PolicyBundleVerifyResult,
    enforce_verified,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDiscoveryConfig {
    pub discovery_url: String,
    pub poll_interval_ms: u64,
    pub max_backoff_ms: u64,
    pub request_timeout_ms: u64,
    pub canary_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyDiscoveryState {
    pub etag: Option<String>,
    pub consecutive_failures: u32,
    pub last_checked_at_ms: u64,
    pub last_stable_version: Option<PolicyVersion>,
    pub last_error: Option<String>,
}

impl Default for PolicyDiscoveryState {
    fn default() -> Self {
        Self {
            etag: None,
            consecutive_failures: 0,
            last_checked_at_ms: 0,
            last_stable_version: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanaryAssessment {
    pub passed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DiscoveryPollOutcome {
    NoChange,
    Activated {
        etag: Option<String>,
        policy_version: PolicyVersion,
        verify: PolicyBundleVerifyResult,
    },
    Rejected {
        etag: Option<String>,
        reasons: Vec<String>,
        verify: Option<PolicyBundleVerifyResult>,
    },
    RolledBack {
        etag: Option<String>,
        reason: String,
        policy_version: Option<PolicyVersion>,
    },
    FetchError {
        reason: String,
    },
}

#[async_trait]
pub trait CanaryEvaluator: Send + Sync {
    async fn evaluate(&self, bundle: &LoadedPolicyBundle) -> Result<CanaryAssessment>;
}

#[derive(Debug, Clone, Default)]
pub struct PassThroughCanary;

#[async_trait]
impl CanaryEvaluator for PassThroughCanary {
    async fn evaluate(&self, _bundle: &LoadedPolicyBundle) -> Result<CanaryAssessment> {
        Ok(CanaryAssessment {
            passed: true,
            reason: "canary accepted".into(),
        })
    }
}

pub struct PolicyDiscoveryService<V: PolicyBundleVerifier> {
    config: PolicyDiscoveryConfig,
    client: reqwest::Client,
    activation: BundleActivationManager,
    verifier: V,
    requirements: PolicyBundleVerifyRequirements,
    canary: Arc<dyn CanaryEvaluator>,
    state: Arc<Mutex<PolicyDiscoveryState>>,
}

impl<V: PolicyBundleVerifier> PolicyDiscoveryService<V> {
    pub fn new(
        config: PolicyDiscoveryConfig,
        activation: BundleActivationManager,
        verifier: V,
        requirements: PolicyBundleVerifyRequirements,
        canary: Arc<dyn CanaryEvaluator>,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(
                config.request_timeout_ms.max(1000),
            ))
            .build()
            .context("failed to build policy discovery http client")?;
        Ok(Self {
            config,
            client,
            activation,
            verifier,
            requirements,
            canary,
            state: Arc::new(Mutex::new(PolicyDiscoveryState::default())),
        })
    }

    pub fn state_handle(&self) -> Arc<Mutex<PolicyDiscoveryState>> {
        Arc::clone(&self.state)
    }

    pub async fn next_backoff_ms(&self) -> u64 {
        let state = self.state.lock().await;
        exponential_backoff_ms(
            self.config.poll_interval_ms,
            self.config.max_backoff_ms,
            state.consecutive_failures,
        )
    }

    pub async fn poll_once(&self) -> Result<DiscoveryPollOutcome> {
        let prior_etag = {
            let state = self.state.lock().await;
            state.etag.clone()
        };

        let mut request = self.client.get(&self.config.discovery_url);
        if let Some(etag) = prior_etag.as_deref() {
            request = request.header(IF_NONE_MATCH, etag);
        }

        let response = match request.send().await {
            Ok(value) => value,
            Err(error) => {
                self.record_failure(error.to_string()).await;
                return Ok(DiscoveryPollOutcome::FetchError {
                    reason: error.to_string(),
                });
            }
        };

        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            self.record_success(None, None).await;
            return Ok(DiscoveryPollOutcome::NoChange);
        }

        if !response.status().is_success() {
            let reason = format!("discovery fetch failed: http {}", response.status());
            self.record_failure(reason.clone()).await;
            return Ok(DiscoveryPollOutcome::FetchError { reason });
        }

        let etag = response
            .headers()
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        let bytes = response.bytes().await.context("failed to read bundle bytes")?;

        let archive_path = self.write_download_bundle(&bytes)?;
        let bundle = match self.activation.load_bundle(&archive_path) {
            Ok(value) => value,
            Err(error) => {
                self.record_failure(error.to_string()).await;
                return Ok(DiscoveryPollOutcome::Rejected {
                    etag,
                    reasons: vec![error.to_string()],
                    verify: None,
                });
            }
        };

        let verify = self.verifier.verify(&bundle, &self.requirements)?;
        if let Err(error) = enforce_verified(&verify) {
            self.record_failure(error.to_string()).await;
            return Ok(DiscoveryPollOutcome::Rejected {
                etag,
                reasons: verify.reasons.clone(),
                verify: Some(verify),
            });
        }

        let previous_stable = self.snapshot_current_to_stable_backup()?;
        self.activation.activate_bundle(&bundle)?;

        if self.config.canary_enabled {
            let canary = self.canary.evaluate(&bundle).await?;
            if !canary.passed {
                let rollback_reason = format!("canary failed: {}", canary.reason);
                self.restore_from_stable_backup(previous_stable.as_deref())?;
                self.record_failure(rollback_reason.clone()).await;
                return Ok(DiscoveryPollOutcome::RolledBack {
                    etag,
                    reason: rollback_reason,
                    policy_version: Some(bundle.manifest.policy_version.clone()),
                });
            }
        }

        self.promote_current_to_stable()?;
        self.record_success(etag.clone(), Some(bundle.manifest.policy_version.clone()))
            .await;
        Ok(DiscoveryPollOutcome::Activated {
            etag,
            policy_version: bundle.manifest.policy_version,
            verify,
        })
    }

    fn stable_dir(&self) -> PathBuf {
        self.activation.root_dir().join("stable")
    }

    fn stable_backup_dir(&self) -> PathBuf {
        self.activation.root_dir().join("stable_backup")
    }

    fn snapshot_current_to_stable_backup(&self) -> Result<Option<PathBuf>> {
        let current = self.activation.current_dir();
        if !current.exists() {
            return Ok(None);
        }

        let backup = self.stable_backup_dir();
        if backup.exists() {
            fs::remove_dir_all(&backup)?;
        }
        copy_dir_all(&current, &backup)?;
        Ok(Some(backup))
    }

    fn restore_from_stable_backup(&self, backup: Option<&Path>) -> Result<()> {
        let current = self.activation.current_dir();
        if current.exists() {
            fs::remove_dir_all(&current)?;
        }

        if let Some(backup) = backup {
            if backup.exists() {
                copy_dir_all(backup, &current)?;
                return Ok(());
            }
        }

        let stable = self.stable_dir();
        if stable.exists() {
            copy_dir_all(&stable, &current)?;
            return Ok(());
        }

        bail!("rollback requested but no stable backup available")
    }

    fn promote_current_to_stable(&self) -> Result<()> {
        let current = self.activation.current_dir();
        if !current.exists() {
            bail!("cannot promote stable: current bundle missing");
        }
        let stable = self.stable_dir();
        if stable.exists() {
            fs::remove_dir_all(&stable)?;
        }
        copy_dir_all(&current, &stable)?;
        Ok(())
    }

    fn write_download_bundle(&self, bytes: &[u8]) -> Result<PathBuf> {
        let download_dir = self.activation.staging_root().join("downloads");
        fs::create_dir_all(&download_dir)?;
        let target = download_dir.join(format!(
            "policy-discovery-{}-{}.tar.gz",
            current_time_ms(),
            std::process::id()
        ));
        fs::write(&target, bytes)
            .with_context(|| format!("failed to write downloaded bundle {}", target.display()))?;
        Ok(target)
    }

    async fn record_success(&self, etag: Option<String>, stable: Option<PolicyVersion>) {
        let mut state = self.state.lock().await;
        state.etag = etag.or_else(|| state.etag.clone());
        state.consecutive_failures = 0;
        state.last_checked_at_ms = current_time_ms();
        if let Some(stable) = stable {
            state.last_stable_version = Some(stable);
        }
        state.last_error = None;
    }

    async fn record_failure(&self, reason: String) {
        let mut state = self.state.lock().await;
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        state.last_checked_at_ms = current_time_ms();
        state.last_error = Some(reason);
    }
}

fn exponential_backoff_ms(base_ms: u64, max_ms: u64, failures: u32) -> u64 {
    if failures == 0 {
        return base_ms;
    }
    let shift = failures.saturating_sub(1).min(16);
    let growth = 1u64 << shift;
    base_ms.saturating_mul(growth).clamp(base_ms, max_ms.max(base_ms))
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path)?;
        }
    }
    Ok(())
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

