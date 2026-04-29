use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};

use crate::execution::{CapabilityRegistry, KernelExecutor};
use crate::governance::{LearningWriteGate, RecoverySubsystem, ResultValidator};
use crate::ir::{
    EvidenceBundle, ExecutionRecord, FingerprintMaterial, LedgerRefs, StepRecord,
    TrustedExecutionRequest,
};
use crate::ledger::{BudgetLedger, EvidenceLedger, EvidenceRecord};
use crate::replay::{
    ReplayExplainer, RolloutGate, build_fingerprint_material, compute_execution_fingerprint,
    output_chain_digest_from_evidence, output_chain_digest_from_step_records,
};
use crate::resource::{
    ResourceGovernor, ResourceRequirement, ResourceReservation, enforce_runtime_island_hardening,
};
use crate::state::{ExecutionStatus, StateMachine};
use crate::trust::{
    AttestationVerifier, CryptoService, IdentityAuthority, SupplyChainVerifier, TenantAuthority,
    TrustLevel,
};

pub struct NoumenonCore {
    pub identity_authority: Arc<dyn IdentityAuthority>,
    pub tenant_authority: Arc<dyn TenantAuthority>,
    pub attestation_verifier: Arc<dyn AttestationVerifier>,
    pub supply_chain_verifier: Arc<dyn SupplyChainVerifier>,
    pub crypto: Arc<dyn CryptoService>,
    pub evidence_ledger: Arc<dyn EvidenceLedger>,
    pub budget_ledger: Arc<dyn BudgetLedger>,
    pub result_validator: Arc<dyn ResultValidator>,
    pub resource_governor: Arc<dyn ResourceGovernor>,
    pub learning_gate: Arc<dyn LearningWriteGate>,
    pub recovery: Arc<dyn RecoverySubsystem>,
    pub rollout_gate: Arc<dyn RolloutGate>,
    pub replay_explainer: Arc<dyn ReplayExplainer>,
    pub capabilities: CapabilityRegistry,
}

impl NoumenonCore {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity_authority: Arc<dyn IdentityAuthority>,
        tenant_authority: Arc<dyn TenantAuthority>,
        attestation_verifier: Arc<dyn AttestationVerifier>,
        supply_chain_verifier: Arc<dyn SupplyChainVerifier>,
        crypto: Arc<dyn CryptoService>,
        evidence_ledger: Arc<dyn EvidenceLedger>,
        budget_ledger: Arc<dyn BudgetLedger>,
        result_validator: Arc<dyn ResultValidator>,
        resource_governor: Arc<dyn ResourceGovernor>,
        learning_gate: Arc<dyn LearningWriteGate>,
        recovery: Arc<dyn RecoverySubsystem>,
        rollout_gate: Arc<dyn RolloutGate>,
        replay_explainer: Arc<dyn ReplayExplainer>,
        capabilities: CapabilityRegistry,
    ) -> Self {
        Self {
            identity_authority,
            tenant_authority,
            attestation_verifier,
            supply_chain_verifier,
            crypto,
            evidence_ledger,
            budget_ledger,
            result_validator,
            resource_governor,
            learning_gate,
            recovery,
            rollout_gate,
            replay_explainer,
            capabilities,
        }
    }

    pub fn execute(&self, request: &TrustedExecutionRequest) -> Result<ExecutionRecord> {
        let mut sm = StateMachine::new();

        if request.intent.operation == "replay_only" || request.intent.operation == "audit_only" {
            sm.transit(ExecutionStatus::Rejected)?;
            return Err(anyhow!(
                "execution rejected: operation mode is outside mandatory kernel path"
            ));
        }

        if let Err(e) = self.identity_authority.authenticate(&request.identity) {
            sm.transit(ExecutionStatus::Rejected)?;
            return Err(anyhow!("execution rejected by identity authority: {e}"));
        }

        if let Err(e) = self.supply_chain_verifier.verify_admission(request) {
            sm.transit(ExecutionStatus::Rejected)?;
            return Err(anyhow!("execution rejected by supply-chain verifier: {e}"));
        }

        if let Err(e) = self.rollout_gate.admit(request) {
            sm.transit(ExecutionStatus::Rejected)?;
            return Err(anyhow!("execution rejected by rollout gate: {e}"));
        }

        sm.transit(ExecutionStatus::Admitted)?;
        sm.transit(ExecutionStatus::Enforcing)?;

        let trust_level = if request
            .verification_spec
            .trust_level
            .eq_ignore_ascii_case("hard")
        {
            TrustLevel::Hard
        } else {
            TrustLevel::Soft
        };

        if request.verification_spec.verify_environment && matches!(trust_level, TrustLevel::Hard) {
            if let Err(e) = self.attestation_verifier.verify() {
                sm.transit(ExecutionStatus::Rejected)?;
                return Err(anyhow!("execution rejected by attestation verifier: {e}"));
            }
        }

        for step in &request.plan.steps {
            if let Some(cap) = &step.capability_ref {
                if let Err(e) = self.tenant_authority.authorize(&request.identity, cap) {
                    sm.transit(ExecutionStatus::Rejected)?;
                    return Err(anyhow!("execution rejected by tenant authority: {e}"));
                }
            }

            if let Err(e) = enforce_runtime_island_hardening(step, &request.constraints) {
                sm.transit(ExecutionStatus::Rejected)?;
                return Err(anyhow!("execution rejected by runtime hardening gate: {e}"));
            }
        }

        let resource_req = ResourceRequirement::from_constraints(&request.constraints);
        let _resource_reservation = if let Ok(reservation) =
            ResourceReservation::new(self.resource_governor.as_ref(), resource_req)
        {
            reservation
        } else {
            sm.transit(ExecutionStatus::Rejected)?;
            return Err(anyhow!("execution rejected by resource governor"));
        };

        let budget_id = format!("budget:{}", request.identity.tenant_id);
        if let Err(e) = self.budget_ledger.reserve(
            &budget_id,
            &request.identity.tenant_id,
            request.plan.steps.len() as u64,
        ) {
            sm.transit(ExecutionStatus::Rejected)?;
            return Err(anyhow!(
                "execution rejected by budget ledger reservation: {e}"
            ));
        }

        sm.transit(ExecutionStatus::Executing)?;

        let run_outputs = match KernelExecutor::run_plan(&self.capabilities, request) {
            Ok(v) => v,
            Err(exec_err) => {
                sm.transit(ExecutionStatus::Failed)?;
                let mut rec = self.empty_record(request, "Failed");
                rec.final_output = Some(exec_err.to_string());
                self.recovery.recover(&rec)?;
                sm.transit(ExecutionStatus::RolledBack)?;
                rec.final_status = "RolledBack".to_string();
                return Ok(rec);
            }
        };

        sm.transit(ExecutionStatus::RecordingEvidence)?;

        let mut step_records = Vec::with_capacity(run_outputs.len());
        let mut evidence_ids = Vec::with_capacity(run_outputs.len());
        let stream_id = format!("evidence:{}", request.execution_id);
        let mut prev_hash = "genesis".to_string();

        let context_payload = self.build_execution_context_payload(request);
        let context_payload_digest = self.crypto.digest_hex(&context_payload);
        let context_record_hash = self.crypto.digest_hex(&format!(
            "{}:{}:{}:{}:{}",
            request.execution_id, "admission-context", "context", context_payload_digest, prev_hash
        ));
        let context_evidence_id = format!("{}:admission-context", request.execution_id);
        self.evidence_ledger.append(EvidenceRecord {
            record_id: context_evidence_id.clone(),
            stream_id: stream_id.clone(),
            execution_id: request.execution_id.clone(),
            step_id: "admission-context".to_string(),
            event_type: "EXECUTION_CONTEXT_BOUND".to_string(),
            payload_digest: context_payload_digest,
            prev_hash: prev_hash.clone(),
            record_hash: context_record_hash.clone(),
            timestamp_ms: now_ms(),
        })?;
        prev_hash = context_record_hash;
        evidence_ids.push(context_evidence_id);

        for out in run_outputs {
            let step = request
                .plan
                .steps
                .iter()
                .find(|s| s.step_id == out.step_id)
                .ok_or_else(|| anyhow!("step not found during record build"))?;
            let input_digest = self.crypto.digest_hex(&step.input);
            let output_digest = self.crypto.digest_hex(&out.output);

            let record_hash = self.crypto.digest_hex(&format!(
                "{}:{}:{}:{}:{}",
                request.execution_id, out.step_id, input_digest, output_digest, prev_hash
            ));
            let evidence_id = format!("{}:{}", request.execution_id, out.step_id);
            let evidence = EvidenceRecord {
                record_id: evidence_id.clone(),
                stream_id: stream_id.clone(),
                execution_id: request.execution_id.clone(),
                step_id: out.step_id.clone(),
                event_type: "STEP_EXECUTED".to_string(),
                payload_digest: output_digest.clone(),
                prev_hash: prev_hash.clone(),
                record_hash: record_hash.clone(),
                timestamp_ms: now_ms(),
            };
            self.evidence_ledger.append(evidence)?;
            prev_hash = record_hash;
            evidence_ids.push(evidence_id);

            step_records.push(StepRecord {
                step_id: out.step_id,
                capability_ref: step.capability_ref.clone(),
                input_digest,
                output_digest,
                success: true,
                error: None,
                cost_units: out.cost_units,
                timestamp_ms: now_ms(),
            });
        }

        let total_cost = step_records.iter().map(|s| s.cost_units).sum();
        self.budget_ledger.consume(&budget_id, total_cost)?;

        let final_output = step_records
            .last()
            .map(|last| format!("output_digest:{}", last.output_digest));

        let fingerprint_material = build_fingerprint_material(
            &request.closure,
            &request.supply_chain,
            self.crypto.as_ref(),
        );
        let output_chain_digest =
            output_chain_digest_from_step_records(&step_records, self.crypto.as_ref());
        let execution_fingerprint = compute_execution_fingerprint(
            &fingerprint_material,
            &output_chain_digest,
            self.crypto.as_ref(),
        );

        let mut record = ExecutionRecord {
            execution_id: request.execution_id.clone(),
            final_status: "Completed".to_string(),
            final_output,
            step_records,
            evidence_bundle: EvidenceBundle {
                evidence_record_ids: evidence_ids,
                execution_fingerprint,
                fingerprint_material,
                mismatch_explanation: None,
            },
            ledger_refs: LedgerRefs {
                budget_account_id: budget_id,
                evidence_stream_id: stream_id,
            },
        };

        sm.transit(ExecutionStatus::Verifying)?;

        if request.verification_spec.validate_output {
            if let Err(e) = self.result_validator.validate(&record) {
                sm.transit(ExecutionStatus::Failed)?;
                record.final_status = "RolledBack".to_string();
                record.evidence_bundle.mismatch_explanation = Some(e.to_string());
                self.recovery.recover(&record)?;
                sm.transit(ExecutionStatus::RolledBack)?;
                return Ok(record);
            }
        }

        if let Err(e) = self.learning_gate.allow_write(&record) {
            sm.transit(ExecutionStatus::Failed)?;
            record.final_status = "RolledBack".to_string();
            record.evidence_bundle.mismatch_explanation = Some(e.to_string());
            self.recovery.recover(&record)?;
            sm.transit(ExecutionStatus::RolledBack)?;
            return Ok(record);
        }

        sm.transit(ExecutionStatus::Completed)?;
        Ok(record)
    }

    pub fn replay_check(&self, record: &ExecutionRecord) -> Result<String> {
        let events = self.evidence_ledger.by_execution(&record.execution_id);
        if events.is_empty() {
            return Err(anyhow!("no evidence records for execution"));
        }
        let output_chain_digest = output_chain_digest_from_evidence(&events, self.crypto.as_ref());
        let replay = compute_execution_fingerprint(
            &record.evidence_bundle.fingerprint_material,
            &output_chain_digest,
            self.crypto.as_ref(),
        );
        if replay == record.evidence_bundle.execution_fingerprint {
            Ok(replay)
        } else {
            let explanation = self.replay_explainer.explain(record, &replay);
            Err(anyhow!(explanation))
        }
    }

    fn empty_record(&self, request: &TrustedExecutionRequest, status: &str) -> ExecutionRecord {
        ExecutionRecord {
            execution_id: request.execution_id.clone(),
            final_status: status.to_string(),
            final_output: None,
            step_records: vec![],
            evidence_bundle: EvidenceBundle {
                evidence_record_ids: vec![],
                execution_fingerprint: String::new(),
                fingerprint_material: FingerprintMaterial::default(),
                mismatch_explanation: None,
            },
            ledger_refs: LedgerRefs {
                budget_account_id: format!("budget:{}", request.identity.tenant_id),
                evidence_stream_id: format!("evidence:{}", request.execution_id),
            },
        }
    }

    fn build_execution_context_payload(&self, request: &TrustedExecutionRequest) -> String {
        let mut store_paths = request.closure.store_paths.clone();
        store_paths.sort();

        let mut capability_set = request
            .plan
            .steps
            .iter()
            .filter_map(|s| s.capability_ref.clone())
            .collect::<Vec<_>>();
        capability_set.sort();
        capability_set.dedup();

        let mut hardening_view = request
            .plan
            .steps
            .iter()
            .map(|s| {
                format!(
                    "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
                    s.step_id,
                    s.capability_ref
                        .clone()
                        .unwrap_or_else(|| "<missing-capability>".to_string()),
                    format!("{:?}", s.runtime_island),
                    s.hardening
                        .hardening_profile
                        .clone()
                        .unwrap_or_else(|| "<none>".to_string()),
                    s.hardening
                        .seccomp_profile_ref
                        .clone()
                        .unwrap_or_else(|| "<none>".to_string()),
                    s.hardening
                        .syscall_policy_ref
                        .clone()
                        .unwrap_or_else(|| "<none>".to_string()),
                    s.hardening.enforce_namespace_isolation,
                    s.hardening.enforce_cgroup_isolation,
                    s.hardening.enforce_fs_isolation,
                    s.hardening.enforce_network_isolation
                )
            })
            .collect::<Vec<_>>();
        hardening_view.sort();

        [
            format!("exec_id={}", request.execution_id),
            format!("tenant={}", request.identity.tenant_id),
            format!("principal={}", request.identity.principal),
            format!("operation={}", request.intent.operation),
            format!("policy={}", request.supply_chain.policy_bundle_digest),
            format!("executor={}", request.supply_chain.executor_digest),
            format!("verifier={}", request.supply_chain.verifier_digest),
            format!(
                "wasm_plugin={}",
                request.supply_chain.capability_package_digest
            ),
            format!("provenance={}", request.supply_chain.provenance_digest),
            format!("rekor_ref={}", request.supply_chain.rekor_log_ref),
            format!("rekor_proof={}", request.supply_chain.rekor_inclusion_proof),
            format!("flake_lock={}", request.closure.flake_lock_digest),
            format!("drv={}", request.closure.derivation_digest),
            format!("closure={}", request.closure.runtime_closure_hash),
            format!("config={}", request.closure.config_digest),
            format!("generation={}", request.closure.generation_id),
            format!("profile={}", request.closure.profile_id),
            format!("store_paths={}", store_paths.join(",")),
            format!("capability_set={}", capability_set.join(",")),
            format!("hardening={}", hardening_view.join(";")),
        ]
        .join("|")
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
