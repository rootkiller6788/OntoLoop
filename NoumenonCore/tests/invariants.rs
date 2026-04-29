use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use trustkernel::execution::{CapabilityRegistry, EchoCapability};
use trustkernel::governance::{NoopRecoverySubsystem, ResultValidator, StatusLearningWriteGate};
use trustkernel::ir::{
    ConstraintSet, ExecutionPlan, ExecutionStep, FailurePolicy, HardeningPolicy, IdentityContext,
    Intent, ReproducibleClosure, RuntimeIsland, StepAction, SupplyChainEnvelope, TraceContext,
    TrustedExecutionRequest, VerificationSpec,
};
use trustkernel::kernel::NoumenonCore;
use trustkernel::ledger::{EvidenceLedger, InMemoryBudgetLedger, InMemoryEvidenceLedger};
use trustkernel::replay::{DefaultReplayExplainer, OpenRolloutGate};
use trustkernel::resource::SimpleResourceGovernor;
use trustkernel::state::{ExecutionStatus, StateMachine};
use trustkernel::trust::{
    HmacIdentityAuthority, Sha256CryptoService, StaticAttestationVerifier,
    StaticSupplyChainVerifier, StaticTenantAuthority, SupplyChainVerifier, TenantAuthority,
};

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn make_request(
    auth: &HmacIdentityAuthority,
    execution_id: &str,
    operation: &str,
    capability: Option<&str>,
) -> TrustedExecutionRequest {
    let cap_ref = capability.map(|v| v.to_string());
    let step = ExecutionStep {
        step_id: "step-1".to_string(),
        action: StepAction::CapabilityCall,
        capability_ref: cap_ref,
        runtime_island: RuntimeIsland::Trusted,
        hardening: HardeningPolicy::default(),
        input: "hello".to_string(),
        dependencies: vec![],
        local_constraints: HashMap::new(),
        failure_policy: FailurePolicy::default(),
    };

    TrustedExecutionRequest {
        execution_id: execution_id.to_string(),
        identity: IdentityContext {
            principal: "agent-1".to_string(),
            tenant_id: "tenant-1".to_string(),
            workspace: "ws-a".to_string(),
            signature: auth.sign("agent-1", "tenant-1", "ws-a"),
        },
        intent: Intent {
            operation: operation.to_string(),
            payload: "task".to_string(),
        },
        plan: ExecutionPlan {
            plan_id: "plan-1".to_string(),
            version: 1,
            strategy: "sequential".to_string(),
            steps: vec![step],
        },
        constraints: ConstraintSet {
            max_runtime_ms: 30_000,
            max_cpu_units: 2,
            max_memory_mb: 256,
            policy_refs: vec!["sha256:policy-v1".to_string()],
        },
        verification_spec: VerificationSpec {
            verify_environment: true,
            verify_identity: true,
            validate_output: true,
            trust_level: "hard".to_string(),
        },
        supply_chain: SupplyChainEnvelope {
            executor_digest: "sha256:executor-a".to_string(),
            verifier_digest: "sha256:verifier-a".to_string(),
            policy_bundle_digest: "sha256:policy-v1".to_string(),
            capability_package_digest: "sha256:wasm-pack-v1".to_string(),
            provenance_digest: "sha256:provenance-v1".to_string(),
            signer_identity_ref: "sigstore://issuer/subject".to_string(),
            signer_oidc_issuer: "https://issuer.example".to_string(),
            rekor_url: "https://rekor.sigstore.dev".to_string(),
            rekor_log_ref: "rekor://log/entry-1".to_string(),
            rekor_inclusion_proof: "proof-1".to_string(),
            provenance_type: "slsaprovenance".to_string(),
        },
        closure: ReproducibleClosure {
            flake_ref: "github:autoloop/trustkernel".to_string(),
            flake_lock_digest: "sha256:flake-lock-v1".to_string(),
            derivation_digest: "sha256:drv-v1".to_string(),
            store_paths: vec![
                "/nix/store/aaa-kernel".to_string(),
                "/nix/store/bbb-runtime".to_string(),
            ],
            runtime_closure_hash: "sha256:closure-v1".to_string(),
            generation_id: "gen-1".to_string(),
            profile_id: "profile-prod".to_string(),
            config_digest: "sha256:cfg-v1".to_string(),
        },
        trace_context: TraceContext {
            trace_id: format!("trace-{}", execution_id),
            parent_execution_id: None,
            submitted_at_ms: now_ms(),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn make_kernel_with(
    tenant_authority: Arc<dyn TenantAuthority>,
    result_validator: Arc<dyn ResultValidator>,
    evidence_ledger: Arc<InMemoryEvidenceLedger>,
    supply_chain_verifier: Arc<dyn SupplyChainVerifier>,
    attestation_ok: bool,
) -> (NoumenonCore, HmacIdentityAuthority) {
    let auth = HmacIdentityAuthority::new("top-secret");
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(EchoCapability::new("llm.echo")));

    let kernel = NoumenonCore::new(
        Arc::new(auth.clone()),
        tenant_authority,
        Arc::new(StaticAttestationVerifier {
            trusted: attestation_ok,
        }),
        supply_chain_verifier,
        Arc::new(Sha256CryptoService),
        evidence_ledger,
        Arc::new(InMemoryBudgetLedger::default()),
        result_validator,
        Arc::new(SimpleResourceGovernor::new(8, 1024)),
        Arc::new(StatusLearningWriteGate),
        Arc::new(NoopRecoverySubsystem),
        Arc::new(OpenRolloutGate),
        Arc::new(DefaultReplayExplainer),
        registry,
    );
    (kernel, auth)
}

#[derive(Debug)]
struct DenyTenantAuthority;

impl TenantAuthority for DenyTenantAuthority {
    fn authorize(&self, _identity: &IdentityContext, _capability: &str) -> Result<()> {
        Err(anyhow!("tenant denied by policy"))
    }
}

#[derive(Debug)]
struct TrackingValidator {
    calls: Arc<AtomicUsize>,
    fail: bool,
}

impl TrackingValidator {
    fn new(calls: Arc<AtomicUsize>, fail: bool) -> Self {
        Self { calls, fail }
    }
}

impl ResultValidator for TrackingValidator {
    fn validate(&self, _record: &trustkernel::ir::ExecutionRecord) -> Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(anyhow!("forced validation failure"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn i1_unadmitted_request_never_reaches_enforcing() {
    assert!(!StateMachine::can_transition(
        &ExecutionStatus::PendingAdmission,
        &ExecutionStatus::Enforcing
    ));

    let evidence = Arc::new(InMemoryEvidenceLedger::default());
    let (kernel, auth) = make_kernel_with(
        Arc::new(StaticTenantAuthority),
        Arc::new(TrackingValidator::new(Arc::new(AtomicUsize::new(0)), false)),
        evidence.clone(),
        Arc::new(StaticSupplyChainVerifier),
        true,
    );
    let mut req = make_request(&auth, "inv-i1", "execute", Some("llm.echo"));
    req.identity.signature = "invalid".to_string();

    let err = kernel
        .execute(&req)
        .expect_err("invalid identity must reject before enforcement");
    assert!(err.to_string().contains("identity authority"));
    assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
}

#[test]
fn i2_side_effect_gate_only_in_executing() {
    let mut sm = StateMachine::new();
    assert!(!sm.allows_side_effect());
    sm.transit(ExecutionStatus::Admitted).expect("admit");
    sm.transit(ExecutionStatus::Enforcing).expect("enforce");
    assert!(!sm.allows_side_effect());
    sm.transit(ExecutionStatus::Executing).expect("execute");
    assert!(sm.allows_side_effect());
    sm.transit(ExecutionStatus::RecordingEvidence)
        .expect("recording");
    assert!(!sm.allows_side_effect());
}

#[test]
fn i3_ledger_gate_only_in_recording_evidence() {
    let mut sm = StateMachine::new();
    assert!(!sm.allows_ledger_write());
    sm.transit(ExecutionStatus::Admitted).expect("admit");
    sm.transit(ExecutionStatus::Enforcing).expect("enforce");
    sm.transit(ExecutionStatus::Executing).expect("execute");
    assert!(!sm.allows_ledger_write());
    sm.transit(ExecutionStatus::RecordingEvidence)
        .expect("recording");
    assert!(sm.allows_ledger_write());
    sm.transit(ExecutionStatus::Verifying).expect("verify");
    assert!(!sm.allows_ledger_write());
}

#[test]
fn i4_failed_path_only_allows_rollback_transition() {
    let mut sm = StateMachine::new();
    sm.transit(ExecutionStatus::Admitted).expect("admit");
    sm.transit(ExecutionStatus::Enforcing).expect("enforce");
    sm.transit(ExecutionStatus::Executing).expect("execute");
    sm.transit(ExecutionStatus::Failed).expect("fail");

    let invalid = sm
        .transit(ExecutionStatus::Completed)
        .expect_err("failed must not jump to completed");
    assert!(invalid.to_string().contains("invalid state transition"));

    sm.transit(ExecutionStatus::RolledBack)
        .expect("rollback after failure");
    assert!(sm.is_terminal());
}

#[test]
fn i5_completed_implies_validate_pass() {
    let calls_ok = Arc::new(AtomicUsize::new(0));
    let evidence_ok = Arc::new(InMemoryEvidenceLedger::default());
    let (kernel_ok, auth_ok) = make_kernel_with(
        Arc::new(StaticTenantAuthority),
        Arc::new(TrackingValidator::new(calls_ok.clone(), false)),
        evidence_ok,
        Arc::new(StaticSupplyChainVerifier),
        true,
    );
    let req_ok = make_request(&auth_ok, "inv-i5-ok", "execute", Some("llm.echo"));
    let rec_ok = kernel_ok
        .execute(&req_ok)
        .expect("validation pass must complete");
    assert_eq!(rec_ok.final_status, "Completed");
    assert_eq!(calls_ok.load(Ordering::SeqCst), 1);

    let calls_fail = Arc::new(AtomicUsize::new(0));
    let evidence_fail = Arc::new(InMemoryEvidenceLedger::default());
    let (kernel_fail, auth_fail) = make_kernel_with(
        Arc::new(StaticTenantAuthority),
        Arc::new(TrackingValidator::new(calls_fail.clone(), true)),
        evidence_fail,
        Arc::new(StaticSupplyChainVerifier),
        true,
    );
    let req_fail = make_request(&auth_fail, "inv-i5-fail", "execute", Some("llm.echo"));
    let rec_fail = kernel_fail
        .execute(&req_fail)
        .expect("validation failure should rollback");
    assert_eq!(rec_fail.final_status, "RolledBack");
    assert_eq!(calls_fail.load(Ordering::SeqCst), 1);
}

#[test]
fn i6_replaycheck_is_not_execution_mode() {
    let evidence = Arc::new(InMemoryEvidenceLedger::default());
    let (kernel, auth) = make_kernel_with(
        Arc::new(StaticTenantAuthority),
        Arc::new(TrackingValidator::new(Arc::new(AtomicUsize::new(0)), false)),
        evidence.clone(),
        Arc::new(StaticSupplyChainVerifier),
        true,
    );

    let replay_req = make_request(&auth, "inv-i6-replay", "replay_only", Some("llm.echo"));
    let replay_err = kernel
        .execute(&replay_req)
        .expect_err("replay mode must be rejected");
    assert!(
        replay_err
            .to_string()
            .contains("outside mandatory kernel path")
    );
    assert_eq!(evidence.by_execution(&replay_req.execution_id).len(), 0);

    let audit_req = make_request(&auth, "inv-i6-audit", "audit_only", Some("llm.echo"));
    let audit_err = kernel
        .execute(&audit_req)
        .expect_err("audit mode must be rejected");
    assert!(
        audit_err
            .to_string()
            .contains("outside mandatory kernel path")
    );
    assert_eq!(evidence.by_execution(&audit_req.execution_id).len(), 0);
}

#[test]
fn i7_execution_is_bound_to_capability_and_tenant_scope() {
    let evidence_deny = Arc::new(InMemoryEvidenceLedger::default());
    let (kernel_deny, auth_deny) = make_kernel_with(
        Arc::new(DenyTenantAuthority),
        Arc::new(TrackingValidator::new(Arc::new(AtomicUsize::new(0)), false)),
        evidence_deny.clone(),
        Arc::new(StaticSupplyChainVerifier),
        true,
    );
    let req_deny = make_request(&auth_deny, "inv-i7-deny", "execute", Some("llm.echo"));
    let deny_err = kernel_deny
        .execute(&req_deny)
        .expect_err("tenant denial must reject before execution");
    assert!(deny_err.to_string().contains("tenant authority"));
    assert_eq!(evidence_deny.by_execution(&req_deny.execution_id).len(), 0);

    let evidence_cap = Arc::new(InMemoryEvidenceLedger::default());
    let (kernel_cap, auth_cap) = make_kernel_with(
        Arc::new(StaticTenantAuthority),
        Arc::new(TrackingValidator::new(Arc::new(AtomicUsize::new(0)), false)),
        evidence_cap,
        Arc::new(StaticSupplyChainVerifier),
        true,
    );
    let req_cap = make_request(&auth_cap, "inv-i7-cap", "execute", None);
    let rec_cap = kernel_cap
        .execute(&req_cap)
        .expect("missing capability must rollback");
    assert_eq!(rec_cap.final_status, "RolledBack");
    assert!(
        rec_cap
            .final_output
            .unwrap_or_default()
            .contains("missing capability_ref")
    );
}

#[test]
fn i8_execution_requires_signed_artifacts_and_closure_identity() {
    let evidence = Arc::new(InMemoryEvidenceLedger::default());
    let (kernel, auth) = make_kernel_with(
        Arc::new(StaticTenantAuthority),
        Arc::new(TrackingValidator::new(Arc::new(AtomicUsize::new(0)), false)),
        evidence.clone(),
        Arc::new(StaticSupplyChainVerifier),
        true,
    );

    let mut req_artifact = make_request(&auth, "inv-i8-artifact", "execute", Some("llm.echo"));
    req_artifact.supply_chain.executor_digest.clear();
    let artifact_err = kernel
        .execute(&req_artifact)
        .expect_err("missing signed artifact digest must reject");
    assert!(artifact_err.to_string().contains("supply-chain verifier"));
    assert_eq!(evidence.by_execution(&req_artifact.execution_id).len(), 0);

    let mut req_closure = make_request(&auth, "inv-i8-closure", "execute", Some("llm.echo"));
    req_closure.closure.flake_lock_digest.clear();
    let closure_err = kernel
        .execute(&req_closure)
        .expect_err("missing closure identity must reject");
    assert!(closure_err.to_string().contains("supply-chain verifier"));
    assert_eq!(evidence.by_execution(&req_closure.execution_id).len(), 0);
}
