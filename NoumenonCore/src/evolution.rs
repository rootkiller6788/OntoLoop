use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use crate::ir::TrustedExecutionRequest;

pub trait PolicyAdvisor: Send + Sync {
    fn suggest(&self, request: &TrustedExecutionRequest) -> String;
}

#[derive(Debug, Clone)]
pub struct SimplePolicyAdvisor;

impl PolicyAdvisor for SimplePolicyAdvisor {
    fn suggest(&self, request: &TrustedExecutionRequest) -> String {
        if request.constraints.max_runtime_ms > 60_000 {
            "suggest:reduce-runtime-budget".to_string()
        } else {
            "suggest:policy-ok".to_string()
        }
    }
}

pub trait DegradationManager: Send + Sync {
    fn fallback_capability(&self, capability: &str) -> Option<String>;
}

#[derive(Debug, Clone)]
pub struct StaticDegradationManager {
    fallbacks: HashMap<String, String>,
}

impl StaticDegradationManager {
    pub fn new(fallbacks: HashMap<String, String>) -> Self {
        Self { fallbacks }
    }
}

impl DegradationManager for StaticDegradationManager {
    fn fallback_capability(&self, capability: &str) -> Option<String> {
        self.fallbacks.get(capability).cloned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryPolicy {
    pub id: String,
    pub max_retry: u8,
    pub rollback_on_fail: bool,
}

pub trait RecoveryPolicyStore: Send + Sync {
    fn put(&self, policy: RecoveryPolicy) -> Result<()>;
    fn get(&self, id: &str) -> Option<RecoveryPolicy>;
}

#[derive(Debug)]
pub struct FileRecoveryPolicyStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileRecoveryPolicyStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::write(&path, b"")?;
        }
        Ok(Self {
            path,
            lock: Mutex::new(()),
        })
    }
}

impl RecoveryPolicyStore for FileRecoveryPolicyStore {
    fn put(&self, policy: RecoveryPolicy) -> Result<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| anyhow!("policy store lock poisoned"))?;
        let line = format!(
            "{}|{}|{}\n",
            hex::encode(policy.id.as_bytes()),
            policy.max_retry,
            if policy.rollback_on_fail { 1 } else { 0 }
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&self.path)?
            .write_all(line.as_bytes())
            .map_err(|e| anyhow!(e))
    }

    fn get(&self, id: &str) -> Option<RecoveryPolicy> {
        let _g = self.lock.lock().ok()?;
        let content = fs::read_to_string(&self.path).ok()?;
        for line in content.lines().rev() {
            let p: Vec<&str> = line.split('|').collect();
            if p.len() != 3 {
                continue;
            }
            let pid = String::from_utf8(hex::decode(p[0]).ok()?).ok()?;
            if pid == id {
                return Some(RecoveryPolicy {
                    id: pid,
                    max_retry: p[1].parse().ok()?,
                    rollback_on_fail: p[2] == "1",
                });
            }
        }
        None
    }
}

use std::io::Write;
