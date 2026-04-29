use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Result, anyhow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub record_id: String,
    pub stream_id: String,
    pub execution_id: String,
    pub step_id: String,
    pub event_type: String,
    pub payload_digest: String,
    pub prev_hash: String,
    pub record_hash: String,
    pub timestamp_ms: u128,
}

pub trait EvidenceLedger: Send + Sync {
    fn append(&self, record: EvidenceRecord) -> Result<()>;
    fn by_execution(&self, execution_id: &str) -> Vec<EvidenceRecord>;
}

#[derive(Debug, Default)]
pub struct InMemoryEvidenceLedger {
    pub records: Mutex<Vec<EvidenceRecord>>,
}

impl EvidenceLedger for InMemoryEvidenceLedger {
    fn append(&self, record: EvidenceRecord) -> Result<()> {
        let mut guard = self
            .records
            .lock()
            .map_err(|_| anyhow!("evidence ledger lock poisoned"))?;
        guard.push(record);
        Ok(())
    }

    fn by_execution(&self, execution_id: &str) -> Vec<EvidenceRecord> {
        self.records
            .lock()
            .map(|g| {
                g.iter()
                    .filter(|r| r.execution_id == execution_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct FileEvidenceLedger {
    file_path: PathBuf,
    lock: Mutex<()>,
}

impl FileEvidenceLedger {
    pub fn new(file_path: impl Into<PathBuf>) -> Result<Self> {
        let file_path = file_path.into();
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !file_path.exists() {
            fs::write(&file_path, b"")?;
        }
        Ok(Self {
            file_path,
            lock: Mutex::new(()),
        })
    }
}

impl EvidenceLedger for FileEvidenceLedger {
    fn append(&self, record: EvidenceRecord) -> Result<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| anyhow!("evidence file lock poisoned"))?;
        let mut file = OpenOptions::new().append(true).open(&self.file_path)?;
        let line = encode_evidence(&record);
        writeln!(file, "{}", line)?;
        Ok(())
    }

    fn by_execution(&self, execution_id: &str) -> Vec<EvidenceRecord> {
        let _g = match self.lock.lock() {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        let content = match fs::read_to_string(&self.file_path) {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        content
            .lines()
            .filter_map(decode_evidence)
            .filter(|r| r.execution_id == execution_id)
            .collect()
    }
}

fn encode_field(v: &str) -> String {
    hex::encode(v.as_bytes())
}

fn decode_field(v: &str) -> Option<String> {
    let bytes = hex::decode(v).ok()?;
    String::from_utf8(bytes).ok()
}

fn encode_evidence(r: &EvidenceRecord) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        encode_field(&r.record_id),
        encode_field(&r.stream_id),
        encode_field(&r.execution_id),
        encode_field(&r.step_id),
        encode_field(&r.event_type),
        encode_field(&r.payload_digest),
        encode_field(&r.prev_hash),
        encode_field(&r.record_hash),
        r.timestamp_ms
    )
}

fn decode_evidence(line: &str) -> Option<EvidenceRecord> {
    let p: Vec<&str> = line.split('|').collect();
    if p.len() != 9 {
        return None;
    }
    Some(EvidenceRecord {
        record_id: decode_field(p[0])?,
        stream_id: decode_field(p[1])?,
        execution_id: decode_field(p[2])?,
        step_id: decode_field(p[3])?,
        event_type: decode_field(p[4])?,
        payload_digest: decode_field(p[5])?,
        prev_hash: decode_field(p[6])?,
        record_hash: decode_field(p[7])?,
        timestamp_ms: p[8].parse().ok()?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetAccount {
    pub account_id: String,
    pub tenant_id: String,
    pub reserved_units: u64,
    pub consumed_units: u64,
}

pub trait BudgetLedger: Send + Sync {
    fn reserve(&self, account_id: &str, tenant_id: &str, units: u64) -> Result<()>;
    fn consume(&self, account_id: &str, units: u64) -> Result<()>;
    fn refund(&self, account_id: &str, units: u64) -> Result<()>;
    fn snapshot(&self, account_id: &str) -> Option<BudgetAccount>;
}

#[derive(Debug, Default)]
pub struct InMemoryBudgetLedger {
    accounts: Mutex<HashMap<String, BudgetAccount>>,
}

impl BudgetLedger for InMemoryBudgetLedger {
    fn reserve(&self, account_id: &str, tenant_id: &str, units: u64) -> Result<()> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| anyhow!("budget ledger lock poisoned"))?;
        let entry = guard
            .entry(account_id.to_string())
            .or_insert(BudgetAccount {
                account_id: account_id.to_string(),
                tenant_id: tenant_id.to_string(),
                reserved_units: 0,
                consumed_units: 0,
            });
        entry.reserved_units = entry.reserved_units.saturating_add(units);
        Ok(())
    }

    fn consume(&self, account_id: &str, units: u64) -> Result<()> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| anyhow!("budget ledger lock poisoned"))?;
        let entry = guard
            .get_mut(account_id)
            .ok_or_else(|| anyhow!("budget account not found"))?;
        if entry.reserved_units < units {
            return Err(anyhow!("insufficient reserved budget"));
        }
        entry.reserved_units -= units;
        entry.consumed_units = entry.consumed_units.saturating_add(units);
        Ok(())
    }

    fn refund(&self, account_id: &str, units: u64) -> Result<()> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| anyhow!("budget ledger lock poisoned"))?;
        let entry = guard
            .get_mut(account_id)
            .ok_or_else(|| anyhow!("budget account not found"))?;
        entry.reserved_units = entry.reserved_units.saturating_sub(units);
        Ok(())
    }

    fn snapshot(&self, account_id: &str) -> Option<BudgetAccount> {
        self.accounts
            .lock()
            .ok()
            .and_then(|g| g.get(account_id).cloned())
    }
}

#[derive(Debug)]
pub struct FileBudgetLedger {
    file_path: PathBuf,
    lock: Mutex<()>,
}

impl FileBudgetLedger {
    pub fn new(file_path: impl AsRef<Path>) -> Result<Self> {
        let file_path = file_path.as_ref().to_path_buf();
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !file_path.exists() {
            fs::write(&file_path, b"")?;
        }
        Ok(Self {
            file_path,
            lock: Mutex::new(()),
        })
    }

    fn append_event(
        &self,
        kind: &str,
        account_id: &str,
        tenant_id: &str,
        units: u64,
    ) -> Result<()> {
        let mut file = OpenOptions::new().append(true).open(&self.file_path)?;
        writeln!(
            file,
            "{}|{}|{}|{}",
            kind,
            encode_field(account_id),
            encode_field(tenant_id),
            units
        )?;
        Ok(())
    }

    fn load_accounts(&self) -> HashMap<String, BudgetAccount> {
        let content = match fs::read_to_string(&self.file_path) {
            Ok(v) => v,
            Err(_) => return HashMap::new(),
        };
        let mut m: HashMap<String, BudgetAccount> = HashMap::new();
        for line in content.lines() {
            let p: Vec<&str> = line.split('|').collect();
            if p.len() != 4 {
                continue;
            }
            let kind = p[0];
            let account_id = match decode_field(p[1]) {
                Some(v) => v,
                None => continue,
            };
            let tenant_id = match decode_field(p[2]) {
                Some(v) => v,
                None => continue,
            };
            let units: u64 = match p[3].parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            let entry = m.entry(account_id.clone()).or_insert(BudgetAccount {
                account_id,
                tenant_id,
                reserved_units: 0,
                consumed_units: 0,
            });
            match kind {
                "reserve" => entry.reserved_units = entry.reserved_units.saturating_add(units),
                "consume" => {
                    entry.reserved_units = entry.reserved_units.saturating_sub(units);
                    entry.consumed_units = entry.consumed_units.saturating_add(units);
                }
                "refund" => entry.reserved_units = entry.reserved_units.saturating_sub(units),
                _ => {}
            }
        }
        m
    }
}

impl BudgetLedger for FileBudgetLedger {
    fn reserve(&self, account_id: &str, tenant_id: &str, units: u64) -> Result<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| anyhow!("budget file lock poisoned"))?;
        self.append_event("reserve", account_id, tenant_id, units)
    }

    fn consume(&self, account_id: &str, units: u64) -> Result<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| anyhow!("budget file lock poisoned"))?;
        let snap = self.load_accounts();
        let current = snap
            .get(account_id)
            .ok_or_else(|| anyhow!("budget account not found"))?;
        if current.reserved_units < units {
            return Err(anyhow!("insufficient reserved budget"));
        }
        self.append_event("consume", account_id, &current.tenant_id, units)
    }

    fn refund(&self, account_id: &str, units: u64) -> Result<()> {
        let _g = self
            .lock
            .lock()
            .map_err(|_| anyhow!("budget file lock poisoned"))?;
        let snap = self.load_accounts();
        let current = snap
            .get(account_id)
            .ok_or_else(|| anyhow!("budget account not found"))?;
        self.append_event("refund", account_id, &current.tenant_id, units)
    }

    fn snapshot(&self, account_id: &str) -> Option<BudgetAccount> {
        let _g = self.lock.lock().ok()?;
        self.load_accounts().get(account_id).cloned()
    }
}
