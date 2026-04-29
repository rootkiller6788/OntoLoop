use anyhow::Result;

use crate::execution::{CapabilityRegistry, KernelExecutor};
use crate::ir::TrustedExecutionRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MismatchCategory {
    MissingEvidence,
    FingerprintMismatch,
    StepCountMismatch,
    OutputDrift,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ReplayAnalysis {
    pub matched: bool,
    pub category: Option<MismatchCategory>,
    pub expected_fingerprint: String,
    pub actual_fingerprint: String,
    pub explanation: String,
}

pub struct TruthEngine;

impl TruthEngine {
    pub fn replay_and_classify(
        registry: &CapabilityRegistry,
        request: &TrustedExecutionRequest,
        expected_fingerprint: &str,
    ) -> Result<ReplayAnalysis> {
        let outputs = KernelExecutor::run_plan(registry, request)?;
        let mut aggregate = String::new();
        for out in &outputs {
            aggregate.push_str(&out.output);
        }
        let actual = hex::encode(aggregate.as_bytes());

        if actual == expected_fingerprint {
            return Ok(ReplayAnalysis {
                matched: true,
                category: None,
                expected_fingerprint: expected_fingerprint.to_string(),
                actual_fingerprint: actual,
                explanation: "replay-match".to_string(),
            });
        }

        let category = if outputs.is_empty() {
            MismatchCategory::MissingEvidence
        } else {
            MismatchCategory::FingerprintMismatch
        };

        Ok(ReplayAnalysis {
            matched: false,
            category: Some(category),
            expected_fingerprint: expected_fingerprint.to_string(),
            actual_fingerprint: actual,
            explanation: "replay-mismatch".to_string(),
        })
    }
}
