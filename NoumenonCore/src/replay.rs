use anyhow::{Result, anyhow};
use std::collections::HashMap;

use crate::ir::{
    ExecutionRecord, FingerprintMaterial, ReproducibleClosure, StepRecord, SupplyChainEnvelope,
    TrustedExecutionRequest,
};
use crate::ledger::EvidenceRecord;
use crate::trust::CryptoService;

pub trait RolloutGate: Send + Sync {
    fn admit(&self, request: &TrustedExecutionRequest) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct PercentRolloutGate {
    pub percentage: u8,
    pub salt: String,
}

impl PercentRolloutGate {
    pub fn new(percentage: u8, salt: impl Into<String>) -> Self {
        Self {
            percentage,
            salt: salt.into(),
        }
    }
}

impl RolloutGate for PercentRolloutGate {
    fn admit(&self, request: &TrustedExecutionRequest) -> Result<()> {
        let p = self.percentage.min(100);
        if p == 100 {
            return Ok(());
        }
        let seed = format!("{}:{}", request.execution_id, self.salt);
        let mut acc: u64 = 1469598103934665603;
        for b in seed.as_bytes() {
            acc ^= *b as u64;
            acc = acc.wrapping_mul(1099511628211);
        }
        let bucket = (acc % 100) as u8;
        if bucket < p {
            Ok(())
        } else {
            Err(anyhow!("execution blocked by rollout gate"))
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenRolloutGate;

impl RolloutGate for OpenRolloutGate {
    fn admit(&self, _request: &TrustedExecutionRequest) -> Result<()> {
        Ok(())
    }
}

pub trait ReplayExplainer: Send + Sync {
    fn explain(&self, record: &ExecutionRecord, actual_fingerprint: &str) -> String;
}

#[derive(Debug, Clone)]
pub struct DefaultReplayExplainer;

impl ReplayExplainer for DefaultReplayExplainer {
    fn explain(&self, record: &ExecutionRecord, actual_fingerprint: &str) -> String {
        if record.step_records.is_empty() {
            return "mismatch:no-step-records".to_string();
        }
        if record.evidence_bundle.execution_fingerprint.is_empty() {
            return "mismatch:missing-expected-fingerprint".to_string();
        }
        format!(
            "mismatch:fingerprint expected={} actual={} step_count={}",
            record.evidence_bundle.execution_fingerprint,
            actual_fingerprint,
            record.step_records.len()
        )
    }
}

pub fn build_fingerprint_material(
    closure: &ReproducibleClosure,
    supply_chain: &SupplyChainEnvelope,
    crypto: &dyn CryptoService,
) -> FingerprintMaterial {
    let mut canonical_store_paths = closure.store_paths.clone();
    canonical_store_paths.sort();
    let store_paths_digest = crypto.digest_hex(&canonical_store_paths.join("|"));

    FingerprintMaterial {
        flake_lock_digest: closure.flake_lock_digest.clone(),
        derivation_digest: closure.derivation_digest.clone(),
        canonical_store_paths,
        store_paths_digest,
        runtime_closure_hash: closure.runtime_closure_hash.clone(),
        policy_bundle_digest: supply_chain.policy_bundle_digest.clone(),
        wasm_plugin_digest: supply_chain.capability_package_digest.clone(),
        verifier_digest: supply_chain.verifier_digest.clone(),
        config_digest: closure.config_digest.clone(),
        generation_id: closure.generation_id.clone(),
    }
}

pub fn compute_execution_fingerprint(
    material: &FingerprintMaterial,
    output_chain_digest: &str,
    crypto: &dyn CryptoService,
) -> String {
    let canonical_store_paths = material.canonical_store_paths.join("|");
    let payload = [
        material.flake_lock_digest.as_str(),
        material.derivation_digest.as_str(),
        canonical_store_paths.as_str(),
        material.store_paths_digest.as_str(),
        material.runtime_closure_hash.as_str(),
        material.policy_bundle_digest.as_str(),
        material.wasm_plugin_digest.as_str(),
        material.verifier_digest.as_str(),
        material.config_digest.as_str(),
        material.generation_id.as_str(),
        output_chain_digest,
    ]
    .join("|");
    crypto.digest_hex(&payload)
}

pub fn output_chain_digest_from_step_records(
    step_records: &[StepRecord],
    crypto: &dyn CryptoService,
) -> String {
    let mut chain = String::new();
    for step in step_records {
        chain.push_str(&step.output_digest);
    }
    crypto.digest_hex(&chain)
}

pub fn output_chain_digest_from_evidence(
    events: &[EvidenceRecord],
    crypto: &dyn CryptoService,
) -> String {
    let mut by_prev_hash: HashMap<&str, &EvidenceRecord> = HashMap::with_capacity(events.len());
    for event in events {
        by_prev_hash.insert(event.prev_hash.as_str(), event);
    }

    let mut chain = String::new();
    let mut next_prev_hash = "genesis";
    let mut visited = 0usize;
    while let Some(event) = by_prev_hash.get(next_prev_hash) {
        if event.event_type == "STEP_EXECUTED" {
            chain.push_str(&event.payload_digest);
        }
        next_prev_hash = event.record_hash.as_str();
        visited += 1;
        if visited == events.len() {
            return crypto.digest_hex(&chain);
        }
    }

    // Fallback path for incomplete/corrupted chain structures.
    let mut ordered = events.to_vec();
    ordered.sort_by(|a, b| {
        a.timestamp_ms
            .cmp(&b.timestamp_ms)
            .then(a.record_id.cmp(&b.record_id))
    });
    chain.clear();
    for event in ordered {
        if event.event_type == "STEP_EXECUTED" {
            chain.push_str(&event.payload_digest);
        }
    }
    crypto.digest_hex(&chain)
}
