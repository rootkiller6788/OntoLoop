use anyhow::{Result, anyhow};

use crate::ir::ExecutionRecord;

pub trait ResultValidator: Send + Sync {
    fn validate(&self, record: &ExecutionRecord) -> Result<()>;
}

pub trait LearningWriteGate: Send + Sync {
    fn allow_write(&self, record: &ExecutionRecord) -> Result<()>;
}

pub trait RecoverySubsystem: Send + Sync {
    fn recover(&self, record: &ExecutionRecord) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct KeywordResultValidator {
    banned: Vec<String>,
}

impl KeywordResultValidator {
    pub fn new(banned: Vec<String>) -> Self {
        Self { banned }
    }
}

impl ResultValidator for KeywordResultValidator {
    fn validate(&self, record: &ExecutionRecord) -> Result<()> {
        let out = record
            .final_output
            .clone()
            .unwrap_or_default()
            .to_lowercase();
        if let Some(hit) = self.banned.iter().find(|b| out.contains(&b.to_lowercase())) {
            return Err(anyhow!("result policy violation: {}", hit));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct StatusLearningWriteGate;

impl LearningWriteGate for StatusLearningWriteGate {
    fn allow_write(&self, record: &ExecutionRecord) -> Result<()> {
        if record.final_status == "Completed" {
            Ok(())
        } else {
            Err(anyhow!("learning write denied for non-completed execution"))
        }
    }
}

#[derive(Debug, Clone)]
pub struct NoopRecoverySubsystem;

impl RecoverySubsystem for NoopRecoverySubsystem {
    fn recover(&self, _record: &ExecutionRecord) -> Result<()> {
        Ok(())
    }
}
