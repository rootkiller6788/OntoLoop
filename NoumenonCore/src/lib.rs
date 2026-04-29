pub mod admission_sigstore;
pub mod execution;
pub mod governance;
pub mod ir;
pub mod kernel;
pub mod ledger;
pub mod prelude;
pub mod replay;
pub mod resource;
pub mod routing;
pub mod state;
pub mod syscall;
pub mod trust;

#[cfg(feature = "ext-evolution")]
pub mod evolution;
#[cfg(feature = "ext-memory")]
pub mod memory_evolution;
#[cfg(feature = "ext-observability")]
pub mod observability;
#[cfg(feature = "ext-memory")]
pub mod storage_vector;
#[cfg(feature = "ext-tooling")]
pub mod tooling;
#[cfg(feature = "ext-truth")]
pub mod truth;

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::admission_sigstore::{
        CommandOutput, CommandRunner, SigstoreAdmissionConfig, SigstoreAdmissionVerifier,
    };
    #[cfg(feature = "ext-evolution")]
    use crate::evolution::{FileRecoveryPolicyStore, RecoveryPolicy, RecoveryPolicyStore};
    use crate::execution::{CapabilityRegistry, EchoCapability};
    use crate::governance::{
        KeywordResultValidator, LearningWriteGate, NoopRecoverySubsystem, RecoverySubsystem,
        ResultValidator, StatusLearningWriteGate,
    };
    use crate::ir::{
        ConstraintSet, ExecutionPlan, ExecutionStep, FailurePolicy, HardeningPolicy,
        IdentityContext, Intent, ReproducibleClosure, RuntimeIsland, StepAction,
        SupplyChainEnvelope, TraceContext, TrustedExecutionRequest, VerificationSpec,
    };
    use crate::kernel::NoumenonCore;
    use crate::ledger::{
        BudgetAccount, BudgetLedger, EvidenceLedger, FileBudgetLedger, FileEvidenceLedger,
        InMemoryBudgetLedger, InMemoryEvidenceLedger,
    };
    #[cfg(feature = "ext-memory")]
    use crate::memory_evolution::{EvolutionAction, MemoryEvolutionEngine, MemoryNote};
    #[cfg(feature = "ext-observability")]
    use crate::observability::{
        AlertEngine, AlertRule, InMemoryMetricsSink, MetricEvent, MetricsSink, SloAdvisor,
        SloTarget,
    };
    use crate::replay::{DefaultReplayExplainer, OpenRolloutGate, PercentRolloutGate};
    use crate::resource::{ResourceGovernor, ResourceRequirement, SimpleResourceGovernor};
    use crate::routing::{EchoModelBackend, ModelRouter};
    #[cfg(feature = "ext-memory")]
    use crate::storage_vector::{InMemoryVectorStore, VectorStore};
    use crate::syscall::{InMemorySyscallQueue, SyscallQueue, SyscallScheduler};
    #[cfg(feature = "ext-tooling")]
    use crate::tooling::{ToolOrchestrator, ToolRequest, VirtualEnvToolAdapter};
    use crate::trust::TenantAuthority;
    use crate::trust::{
        HmacIdentityAuthority, Sha256CryptoService, StaticAttestationVerifier,
        StaticSupplyChainVerifier, StaticTenantAuthority, SupplyChainVerifier,
    };
    #[cfg(feature = "ext-truth")]
    use crate::truth::{MismatchCategory, TruthEngine};
    use anyhow::{Result, anyhow};

    fn now_ms() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    }

    fn make_request(sig: String) -> TrustedExecutionRequest {
        let step = ExecutionStep {
            step_id: "step-1".to_string(),
            action: StepAction::CapabilityCall,
            capability_ref: Some("llm.echo".to_string()),
            runtime_island: RuntimeIsland::Trusted,
            hardening: HardeningPolicy::default(),
            input: "hello".to_string(),
            dependencies: vec![],
            local_constraints: HashMap::new(),
            failure_policy: FailurePolicy::default(),
        };

        TrustedExecutionRequest {
            execution_id: "exec-1".to_string(),
            identity: IdentityContext {
                principal: "agent-1".to_string(),
                tenant_id: "tenant-1".to_string(),
                workspace: "ws-a".to_string(),
                signature: sig,
            },
            intent: Intent {
                operation: "execute".to_string(),
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
                trace_id: "trace-1".to_string(),
                parent_execution_id: None,
                submitted_at_ms: now_ms(),
            },
        }
    }

    fn make_kernel(
        attestation_ok: bool,
        banned: Vec<String>,
    ) -> (NoumenonCore, HmacIdentityAuthority) {
        make_kernel_with_supply(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            Arc::new(InMemoryEvidenceLedger::default()),
            Arc::new(KeywordResultValidator::new(banned)),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            attestation_ok,
            Arc::new(StaticSupplyChainVerifier),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn make_kernel_with(
        tenant_authority: Arc<dyn TenantAuthority>,
        resource_governor: Arc<dyn ResourceGovernor>,
        budget_ledger: Arc<dyn BudgetLedger>,
        evidence_ledger: Arc<dyn EvidenceLedger>,
        result_validator: Arc<dyn ResultValidator>,
        learning_gate: Arc<dyn LearningWriteGate>,
        recovery: Arc<dyn RecoverySubsystem>,
        attestation_ok: bool,
    ) -> (NoumenonCore, HmacIdentityAuthority) {
        make_kernel_with_supply(
            tenant_authority,
            resource_governor,
            budget_ledger,
            evidence_ledger,
            result_validator,
            learning_gate,
            recovery,
            attestation_ok,
            Arc::new(StaticSupplyChainVerifier),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn make_kernel_with_supply(
        tenant_authority: Arc<dyn TenantAuthority>,
        resource_governor: Arc<dyn ResourceGovernor>,
        budget_ledger: Arc<dyn BudgetLedger>,
        evidence_ledger: Arc<dyn EvidenceLedger>,
        result_validator: Arc<dyn ResultValidator>,
        learning_gate: Arc<dyn LearningWriteGate>,
        recovery: Arc<dyn RecoverySubsystem>,
        attestation_ok: bool,
        supply_chain_verifier: Arc<dyn SupplyChainVerifier>,
    ) -> (NoumenonCore, HmacIdentityAuthority) {
        let auth = HmacIdentityAuthority::new("top-secret");
        let mut registry = CapabilityRegistry::new();
        registry.register(Box::new(EchoCapability::new("llm.echo")));
        registry.register_llm_backend(Box::new(EchoModelBackend::new("model-a")));
        registry.register_llm_backend(Box::new(EchoModelBackend::new("model-b")));
        registry.set_router(ModelRouter::sequential());

        let kernel = NoumenonCore::new(
            Arc::new(auth.clone()),
            tenant_authority,
            Arc::new(StaticAttestationVerifier {
                trusted: attestation_ok,
            }),
            supply_chain_verifier,
            Arc::new(Sha256CryptoService),
            evidence_ledger,
            budget_ledger,
            result_validator,
            resource_governor,
            learning_gate,
            recovery,
            Arc::new(OpenRolloutGate),
            Arc::new(DefaultReplayExplainer),
            registry,
        );
        (kernel, auth)
    }

    struct DenyTenantAuthority;
    impl TenantAuthority for DenyTenantAuthority {
        fn authorize(
            &self,
            _identity: &crate::ir::IdentityContext,
            _capability: &str,
        ) -> Result<()> {
            Err(anyhow!("tenant denied by policy"))
        }
    }

    struct DenyResourceGovernor;
    impl ResourceGovernor for DenyResourceGovernor {
        fn reserve(&self, _req: &ResourceRequirement) -> Result<()> {
            Err(anyhow!("resource denied"))
        }
        fn release(&self, _req: &ResourceRequirement) {}
    }

    struct DenyBudgetLedger;
    impl BudgetLedger for DenyBudgetLedger {
        fn reserve(&self, _account_id: &str, _tenant_id: &str, _units: u64) -> Result<()> {
            Err(anyhow!("budget denied"))
        }
        fn consume(&self, _account_id: &str, _units: u64) -> Result<()> {
            Ok(())
        }
        fn refund(&self, _account_id: &str, _units: u64) -> Result<()> {
            Ok(())
        }
        fn snapshot(&self, _account_id: &str) -> Option<BudgetAccount> {
            None
        }
    }

    struct QueueRunner {
        outputs: std::sync::Mutex<VecDeque<CommandOutput>>,
    }

    impl QueueRunner {
        fn new(outputs: Vec<CommandOutput>) -> Self {
            Self {
                outputs: std::sync::Mutex::new(outputs.into()),
            }
        }
    }

    impl CommandRunner for QueueRunner {
        fn run(&self, _program: &str, _args: &[String]) -> Result<CommandOutput> {
            self.outputs
                .lock()
                .map_err(|_| anyhow!("runner lock poisoned"))?
                .pop_front()
                .ok_or_else(|| anyhow!("no queued cosign output"))
        }
    }

    struct TrackingResultValidator {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl TrackingResultValidator {
        fn new(calls: Arc<AtomicUsize>, fail: bool) -> Self {
            Self { calls, fail }
        }
    }

    impl ResultValidator for TrackingResultValidator {
        fn validate(&self, _record: &crate::ir::ExecutionRecord) -> Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(anyhow!("forced validation failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn syscall_scheduler_respects_priority() {
        let q: Arc<dyn SyscallQueue> = Arc::new(InMemorySyscallQueue::default());
        let sched = SyscallScheduler::new(q);
        sched
            .submit("s1".to_string(), "x".to_string(), "i1".to_string(), 1)
            .expect("submit s1");
        sched
            .submit("s2".to_string(), "x".to_string(), "i2".to_string(), 5)
            .expect("submit s2");
        let out = sched
            .drain(|req| Ok((format!("{}", req.step_id), 1)))
            .expect("drain");
        assert_eq!(out[0].step_id, "s2");
        assert_eq!(out[1].step_id, "s1");
    }

    #[test]
    fn model_router_selects_and_executes() {
        let auth = HmacIdentityAuthority::new("top-secret");
        let mut registry = CapabilityRegistry::new();
        registry.register(Box::new(EchoCapability::new("noop")));
        registry.register_llm_backend(Box::new(EchoModelBackend::new("model-a")));
        registry.register_llm_backend(Box::new(EchoModelBackend::new("model-b")));
        registry.set_router(ModelRouter::sequential());

        let mut req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        req.plan.steps[0].capability_ref = Some("llm.route".to_string());
        req.plan.steps[0].local_constraints.insert(
            "model_candidates".to_string(),
            "model-a,model-b".to_string(),
        );

        let out = crate::execution::KernelExecutor::run_plan(&registry, &req).expect("run plan");
        assert_eq!(out.len(), 1);
        assert!(out[0].output.contains("model="));
    }

    #[test]
    fn persistent_ledgers_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tk-ledger-{}", now_ms()));
        let e_path = dir.join("evidence.log");
        let b_path = dir.join("budget.log");
        let evidence = FileEvidenceLedger::new(&e_path).expect("file evidence");
        let budget = FileBudgetLedger::new(&b_path).expect("file budget");

        evidence
            .append(crate::ledger::EvidenceRecord {
                record_id: "r1".to_string(),
                stream_id: "s1".to_string(),
                execution_id: "e1".to_string(),
                step_id: "step-1".to_string(),
                event_type: "STEP_EXECUTED".to_string(),
                payload_digest: "d1".to_string(),
                prev_hash: "g".to_string(),
                record_hash: "h1".to_string(),
                timestamp_ms: now_ms(),
            })
            .expect("append evidence");

        let got = evidence.by_execution("e1");
        assert_eq!(got.len(), 1);

        budget.reserve("acct-1", "tenant-1", 5).expect("reserve");
        budget.consume("acct-1", 2).expect("consume");
        let snap = budget.snapshot("acct-1").expect("snapshot");
        assert_eq!(snap.reserved_units, 3);
        assert_eq!(snap.consumed_units, 2);
    }

    #[test]
    fn rollout_gate_can_reject() {
        let (mut kernel, auth) = make_kernel(true, vec![]);
        kernel.rollout_gate = Arc::new(PercentRolloutGate::new(0, "x"));
        let req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        let err = kernel.execute(&req).expect_err("must reject by rollout");
        assert!(err.to_string().contains("rollout gate"));
    }

    #[test]
    fn replay_explainer_message_on_mismatch() {
        let (kernel, auth) = make_kernel(true, vec!["never-hit".to_string()]);
        let req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        let mut record = kernel.execute(&req).expect("execute");
        record.evidence_bundle.execution_fingerprint = "tampered".to_string();
        let err = kernel.replay_check(&record).expect_err("mismatch expected");
        assert!(err.to_string().contains("mismatch"));
    }

    #[cfg(feature = "ext-memory")]
    #[test]
    fn memory_evolution_merges_neighbors() {
        let candidate = MemoryNote {
            id: "n-new".to_string(),
            content: "rust kernel trust execution".to_string(),
            context: "ctx".to_string(),
            tags: vec!["trust".to_string()],
            links: vec![],
        };
        let neighbor = MemoryNote {
            id: "n-1".to_string(),
            content: "rust trust execution engine".to_string(),
            context: "ctx2".to_string(),
            tags: vec!["kernel".to_string()],
            links: vec![],
        };
        let d = MemoryEvolutionEngine::evolve(candidate, vec![neighbor]);
        assert_eq!(d.action, EvolutionAction::Merge);
    }

    #[cfg(feature = "ext-tooling")]
    #[test]
    fn virtual_env_tool_orchestrator_works() {
        let mut o = ToolOrchestrator::new();
        o.register(Box::new(VirtualEnvToolAdapter::new("tool.build", "env-a")));
        let out = o
            .call(&ToolRequest {
                tool_name: "tool.build".to_string(),
                input: "make".to_string(),
                workspace: "ws".to_string(),
            })
            .expect("tool call");
        assert!(out.contains("venv=env-a"));
    }

    #[cfg(feature = "ext-memory")]
    #[test]
    fn vector_store_query_returns_best_match() {
        let mut vs = InMemoryVectorStore::default();
        vs.upsert("a".to_string(), "apple banana".to_string());
        vs.upsert("b".to_string(), "rust kernel trust".to_string());
        let top = vs.query("kernel trust", 1);
        assert_eq!(top[0].0, "b");
    }

    #[cfg(feature = "ext-truth")]
    #[test]
    fn truth_engine_classifies_mismatch() {
        let auth = HmacIdentityAuthority::new("top-secret");
        let mut registry = CapabilityRegistry::new();
        registry.register(Box::new(EchoCapability::new("llm.echo")));
        let req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        let analysis = TruthEngine::replay_and_classify(&registry, &req, "bad").expect("analysis");
        assert!(!analysis.matched);
        assert_eq!(
            analysis.category,
            Some(MismatchCategory::FingerprintMismatch)
        );
    }

    #[cfg(feature = "ext-observability")]
    #[test]
    fn observability_alert_and_slo_advice() {
        let sink = InMemoryMetricsSink::default();
        sink.record(MetricEvent {
            name: "execution.duration.ms".to_string(),
            value: 1500.0,
            labels: vec![],
        });
        let alerts = AlertEngine {
            rules: vec![AlertRule {
                metric: "execution.duration.ms".to_string(),
                threshold: 1000.0,
            }],
        }
        .evaluate(&sink);
        assert_eq!(alerts.len(), 1);

        let advice = SloAdvisor::advise(
            &SloTarget {
                metric: "execution.duration.ms".to_string(),
                max_value: 900.0,
            },
            &sink,
        )
        .expect("slo advice");
        assert!(advice.contains("violation"));
    }

    #[cfg(feature = "ext-evolution")]
    #[test]
    fn persistent_recovery_policy_store_roundtrip() {
        let path = std::env::temp_dir().join(format!("tk-policy-{}.log", now_ms()));
        let store = FileRecoveryPolicyStore::new(&path).expect("policy store");
        store
            .put(RecoveryPolicy {
                id: "p1".to_string(),
                max_retry: 3,
                rollback_on_fail: true,
            })
            .expect("put policy");
        let got = store.get("p1").expect("get policy");
        assert_eq!(got.max_retry, 3);
        assert!(got.rollback_on_fail);
    }

    #[test]
    fn rejects_invalid_identity() {
        let (kernel, _auth) = make_kernel(true, vec![]);
        let req = make_request("invalid-signature".to_string());
        let err = kernel
            .execute(&req)
            .expect_err("must reject invalid identity");
        assert!(err.to_string().contains("rejected"));
    }

    #[test]
    fn rejects_untrusted_attestation_in_hard_mode() {
        let (kernel, auth) = make_kernel(false, vec![]);
        let sig = auth.sign("agent-1", "tenant-1", "ws-a");
        let req = make_request(sig);
        let err = kernel
            .execute(&req)
            .expect_err("must reject untrusted environment");
        assert!(err.to_string().contains("attestation"));
    }

    #[test]
    fn executes_and_execution_fingerprint_matches() {
        let (kernel, auth) = make_kernel(true, vec!["forbidden".to_string()]);
        let sig = auth.sign("agent-1", "tenant-1", "ws-a");
        let req = make_request(sig);
        let record = kernel.execute(&req).expect("execution should succeed");
        assert_eq!(record.final_status, "Completed");
        assert_eq!(record.step_records.len(), 1);
        let replay = kernel.replay_check(&record).expect("replay should match");
        assert_eq!(replay, record.evidence_bundle.execution_fingerprint);
    }

    #[test]
    fn execution_fingerprint_changes_with_generation() {
        let (kernel, auth) = make_kernel(true, vec!["forbidden".to_string()]);
        let sig = auth.sign("agent-1", "tenant-1", "ws-a");
        let req_v1 = make_request(sig.clone());
        let mut req_v2 = make_request(sig);
        req_v2.execution_id = "exec-2".to_string();
        req_v2.closure.generation_id = "gen-2".to_string();

        let rec_v1 = kernel.execute(&req_v1).expect("gen-1 should execute");
        let rec_v2 = kernel.execute(&req_v2).expect("gen-2 should execute");

        assert_ne!(
            rec_v1.evidence_bundle.execution_fingerprint,
            rec_v2.evidence_bundle.execution_fingerprint
        );
    }

    #[test]
    fn execution_fingerprint_is_store_path_order_invariant() {
        let (kernel, auth) = make_kernel(true, vec!["forbidden".to_string()]);
        let sig = auth.sign("agent-1", "tenant-1", "ws-a");
        let req_a = make_request(sig.clone());
        let mut req_b = make_request(sig);
        req_b.execution_id = "exec-3".to_string();
        req_b.closure.store_paths.reverse();

        let rec_a = kernel.execute(&req_a).expect("baseline should execute");
        let rec_b = kernel
            .execute(&req_b)
            .expect("reordered store paths should execute");

        assert_eq!(
            rec_a.evidence_bundle.execution_fingerprint,
            rec_b.evidence_bundle.execution_fingerprint
        );
    }

    #[test]
    fn replay_check_recomputes_from_evidence_chain() {
        let (kernel, auth) = make_kernel(true, vec!["forbidden".to_string()]);
        let sig = auth.sign("agent-1", "tenant-1", "ws-a");
        let req = make_request(sig);
        let mut record = kernel.execute(&req).expect("execution should succeed");
        record.step_records[0].output_digest = "tampered-output-digest".to_string();

        let replay = kernel
            .replay_check(&record)
            .expect("replay should be reconstructed from evidence chain");
        assert_eq!(replay, record.evidence_bundle.execution_fingerprint);
    }

    #[test]
    fn records_execution_context_binding_in_evidence() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let (kernel, auth) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let mut req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        req.execution_id = "exec-evidence-context".to_string();
        let record = kernel.execute(&req).expect("execution should succeed");
        let events = evidence.by_execution(&req.execution_id);

        assert_eq!(events.len(), record.step_records.len() + 1);
        assert_eq!(events[0].event_type, "EXECUTION_CONTEXT_BOUND");
        assert_eq!(events[0].step_id, "admission-context");
        assert!(events.iter().any(|e| e.event_type == "STEP_EXECUTED"));
    }

    #[test]
    fn completed_implies_validate_pass_contract() {
        let calls_ok = Arc::new(AtomicUsize::new(0));
        let (kernel_ok, auth_ok) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            Arc::new(InMemoryEvidenceLedger::default()),
            Arc::new(TrackingResultValidator::new(calls_ok.clone(), false)),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let req_ok = make_request(auth_ok.sign("agent-1", "tenant-1", "ws-a"));
        let rec_ok = kernel_ok
            .execute(&req_ok)
            .expect("validation pass should complete");
        assert_eq!(rec_ok.final_status, "Completed");
        assert_eq!(calls_ok.load(Ordering::SeqCst), 1);

        let calls_fail = Arc::new(AtomicUsize::new(0));
        let (kernel_fail, auth_fail) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            Arc::new(InMemoryEvidenceLedger::default()),
            Arc::new(TrackingResultValidator::new(calls_fail.clone(), true)),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let mut req_fail = make_request(auth_fail.sign("agent-1", "tenant-1", "ws-a"));
        req_fail.execution_id = "exec-validate-fail".to_string();
        let rec_fail = kernel_fail
            .execute(&req_fail)
            .expect("validation fail should return rollback record");
        assert_eq!(rec_fail.final_status, "RolledBack");
        assert_eq!(calls_fail.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rolls_back_when_result_violates_policy() {
        let (kernel, auth) = make_kernel(true, vec!["output_digest".to_string()]);
        let sig = auth.sign("agent-1", "tenant-1", "ws-a");
        let req = make_request(sig);
        let record = kernel
            .execute(&req)
            .expect("should return record with rollback status");
        assert_eq!(record.final_status, "RolledBack");
        assert!(record.evidence_bundle.mismatch_explanation.is_some());
    }

    #[test]
    fn rejects_non_mandatory_operation_modes() {
        let (kernel, auth) = make_kernel(true, vec![]);
        let sig = auth.sign("agent-1", "tenant-1", "ws-a");

        let mut replay_req = make_request(sig.clone());
        replay_req.intent.operation = "replay_only".to_string();
        let replay_err = kernel
            .execute(&replay_req)
            .expect_err("replay_only must be rejected");
        assert!(
            replay_err
                .to_string()
                .contains("outside mandatory kernel path")
        );

        let mut audit_req = make_request(sig);
        audit_req.intent.operation = "audit_only".to_string();
        let audit_err = kernel
            .execute(&audit_req)
            .expect_err("audit_only must be rejected");
        assert!(
            audit_err
                .to_string()
                .contains("outside mandatory kernel path")
        );
    }

    #[test]
    fn rejects_missing_supply_chain_evidence_at_admission() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let (kernel, auth) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let mut req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        req.supply_chain.provenance_digest.clear();
        let err = kernel
            .execute(&req)
            .expect_err("missing provenance digest should reject");
        assert!(err.to_string().contains("supply-chain verifier"));
        assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
    }

    #[test]
    fn rejects_wasm_path_without_hardening_profile() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let (kernel, auth) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let mut req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        req.execution_id = "exec-hardening-profile".to_string();
        req.plan.steps[0].runtime_island = RuntimeIsland::Wasm;
        req.plan.steps[0].hardening.syscall_policy_ref = Some("syscall:min".to_string());
        req.plan.steps[0].hardening.max_runtime_ms = Some(1_000);
        req.plan.steps[0].hardening.max_memory_mb = Some(128);
        req.plan.steps[0].hardening.max_cpu_units = Some(1);

        let err = kernel
            .execute(&req)
            .expect_err("missing hardening profile should reject");
        assert!(err.to_string().contains("runtime hardening gate"));
        assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
    }

    #[test]
    fn rejects_wasm_path_without_syscall_constraints() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let (kernel, auth) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let mut req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        req.execution_id = "exec-hardening-syscall".to_string();
        req.plan.steps[0].runtime_island = RuntimeIsland::Wasm;
        req.plan.steps[0].hardening.hardening_profile = Some("hardened-v1".to_string());
        req.plan.steps[0].hardening.seccomp_profile_ref = Some("seccomp:v1".to_string());
        req.plan.steps[0].hardening.enforce_namespace_isolation = true;
        req.plan.steps[0].hardening.enforce_cgroup_isolation = true;
        req.plan.steps[0].hardening.enforce_fs_isolation = true;
        req.plan.steps[0].hardening.enforce_network_isolation = true;
        req.plan.steps[0].hardening.max_runtime_ms = Some(1_000);
        req.plan.steps[0].hardening.max_memory_mb = Some(128);
        req.plan.steps[0].hardening.max_cpu_units = Some(1);

        let err = kernel
            .execute(&req)
            .expect_err("missing syscall constraints should reject");
        assert!(err.to_string().contains("runtime hardening gate"));
        assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
    }

    #[test]
    fn executes_wasm_path_with_hardening_constraints() {
        let (kernel, auth) = make_kernel(true, vec![]);
        let mut req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        req.execution_id = "exec-hardening-ok".to_string();
        req.plan.steps[0].runtime_island = RuntimeIsland::Wasm;
        req.plan.steps[0].hardening = HardeningPolicy {
            hardening_profile: Some("hardened-v1".to_string()),
            syscall_policy_ref: Some("syscall:min".to_string()),
            syscall_allowlist: vec!["read".to_string(), "write".to_string()],
            seccomp_profile_ref: Some("seccomp:v1".to_string()),
            enforce_namespace_isolation: true,
            enforce_cgroup_isolation: true,
            enforce_fs_isolation: true,
            enforce_network_isolation: true,
            max_runtime_ms: Some(1_000),
            max_memory_mb: Some(128),
            max_cpu_units: Some(1),
        };
        req.plan.steps[0]
            .local_constraints
            .insert("isolation_backend".to_string(), "auto".to_string());

        let rec = kernel
            .execute(&req)
            .expect("hardened wasm path should execute");
        assert_eq!(rec.final_status, "Completed");
    }

    #[test]
    fn rejects_tenant_denial_without_evidence() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let (kernel, auth) = make_kernel_with(
            Arc::new(DenyTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        let err = kernel
            .execute(&req)
            .expect_err("tenant denial should reject");
        assert!(err.to_string().contains("tenant authority"));
        assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
    }

    #[test]
    fn rejects_resource_denial_without_evidence() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let (kernel, auth) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(DenyResourceGovernor),
            Arc::new(InMemoryBudgetLedger::default()),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        let err = kernel
            .execute(&req)
            .expect_err("resource denial should reject");
        assert!(err.to_string().contains("resource governor"));
        assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
    }

    #[test]
    fn rejects_budget_reservation_without_evidence() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let (kernel, auth) = make_kernel_with(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(DenyBudgetLedger),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
        );
        let req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        let err = kernel
            .execute(&req)
            .expect_err("budget denial should reject");
        assert!(err.to_string().contains("budget ledger reservation"));
        assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
    }

    #[test]
    fn rolls_back_on_execution_error_consistently() {
        let (kernel, auth) = make_kernel(true, vec![]);
        let mut req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        req.plan.steps[0].capability_ref = Some("missing.cap".to_string());
        let record = kernel
            .execute(&req)
            .expect("kernel should return rollback record");
        assert_eq!(record.final_status, "RolledBack");
        assert!(
            record
                .final_output
                .unwrap_or_default()
                .contains("capability not found")
        );
    }

    #[test]
    fn rejects_sigstore_failure_without_evidence() {
        let evidence = Arc::new(InMemoryEvidenceLedger::default());
        let sigstore = Arc::new(SigstoreAdmissionVerifier::with_runner(
            SigstoreAdmissionConfig::default(),
            Arc::new(QueueRunner::new(vec![CommandOutput {
                status: 1,
                stdout: String::new(),
                stderr: "signature invalid".to_string(),
            }])),
        ));
        let (kernel, auth) = make_kernel_with_supply(
            Arc::new(StaticTenantAuthority),
            Arc::new(SimpleResourceGovernor::new(8, 1024)),
            Arc::new(InMemoryBudgetLedger::default()),
            evidence.clone(),
            Arc::new(KeywordResultValidator::new(vec![])),
            Arc::new(StatusLearningWriteGate),
            Arc::new(NoopRecoverySubsystem),
            true,
            sigstore,
        );
        let req = make_request(auth.sign("agent-1", "tenant-1", "ws-a"));
        let err = kernel
            .execute(&req)
            .expect_err("sigstore failure should reject before execution");
        assert!(err.to_string().contains("supply-chain verifier"));
        assert_eq!(evidence.by_execution(&req.execution_id).len(), 0);
    }
}
