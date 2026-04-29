use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityContext {
    pub principal: String,
    pub tenant_id: String,
    pub workspace: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub operation: String,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    CapabilityCall,
    Transform,
    Verify,
    Branch,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RuntimeIsland {
    #[default]
    Trusted,
    Wasm,
    Ffi,
    Plugin,
}

impl RuntimeIsland {
    pub fn requires_hardening(&self) -> bool {
        matches!(
            self,
            RuntimeIsland::Wasm | RuntimeIsland::Ffi | RuntimeIsland::Plugin
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HardeningPolicy {
    pub hardening_profile: Option<String>,
    pub syscall_policy_ref: Option<String>,
    pub syscall_allowlist: Vec<String>,
    pub seccomp_profile_ref: Option<String>,
    pub enforce_namespace_isolation: bool,
    pub enforce_cgroup_isolation: bool,
    pub enforce_fs_isolation: bool,
    pub enforce_network_isolation: bool,
    pub max_runtime_ms: Option<u64>,
    pub max_memory_mb: Option<u32>,
    pub max_cpu_units: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailurePolicy {
    pub retry_max: u8,
    pub allow_degrade: bool,
    pub rollback_on_fail: bool,
}

impl Default for FailurePolicy {
    fn default() -> Self {
        Self {
            retry_max: 0,
            allow_degrade: false,
            rollback_on_fail: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionStep {
    pub step_id: String,
    pub action: StepAction,
    pub capability_ref: Option<String>,
    pub runtime_island: RuntimeIsland,
    pub hardening: HardeningPolicy,
    pub input: String,
    pub dependencies: Vec<String>,
    pub local_constraints: HashMap<String, String>,
    pub failure_policy: FailurePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    pub plan_id: String,
    pub version: u32,
    pub strategy: String,
    pub steps: Vec<ExecutionStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConstraintSet {
    pub max_runtime_ms: u64,
    pub max_cpu_units: u32,
    pub max_memory_mb: u32,
    pub policy_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VerificationSpec {
    pub verify_environment: bool,
    pub verify_identity: bool,
    pub validate_output: bool,
    pub trust_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SupplyChainEnvelope {
    pub executor_digest: String,
    pub verifier_digest: String,
    pub policy_bundle_digest: String,
    pub capability_package_digest: String,
    pub provenance_digest: String,
    pub signer_identity_ref: String,
    pub signer_oidc_issuer: String,
    pub rekor_url: String,
    pub rekor_log_ref: String,
    pub rekor_inclusion_proof: String,
    pub provenance_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReproducibleClosure {
    pub flake_ref: String,
    pub flake_lock_digest: String,
    pub derivation_digest: String,
    pub store_paths: Vec<String>,
    pub runtime_closure_hash: String,
    pub generation_id: String,
    pub profile_id: String,
    pub config_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceContext {
    pub trace_id: String,
    pub parent_execution_id: Option<String>,
    pub submitted_at_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedExecutionRequest {
    pub execution_id: String,
    pub identity: IdentityContext,
    pub intent: Intent,
    pub plan: ExecutionPlan,
    pub constraints: ConstraintSet,
    pub verification_spec: VerificationSpec,
    pub supply_chain: SupplyChainEnvelope,
    pub closure: ReproducibleClosure,
    pub trace_context: TraceContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRecord {
    pub step_id: String,
    pub capability_ref: Option<String>,
    pub input_digest: String,
    pub output_digest: String,
    pub success: bool,
    pub error: Option<String>,
    pub cost_units: u64,
    pub timestamp_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FingerprintMaterial {
    pub flake_lock_digest: String,
    pub derivation_digest: String,
    pub canonical_store_paths: Vec<String>,
    pub store_paths_digest: String,
    pub runtime_closure_hash: String,
    pub policy_bundle_digest: String,
    pub wasm_plugin_digest: String,
    pub verifier_digest: String,
    pub config_digest: String,
    pub generation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBundle {
    pub evidence_record_ids: Vec<String>,
    pub execution_fingerprint: String,
    pub fingerprint_material: FingerprintMaterial,
    pub mismatch_explanation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRefs {
    pub budget_account_id: String,
    pub evidence_stream_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRecord {
    pub execution_id: String,
    pub final_status: String,
    pub final_output: Option<String>,
    pub step_records: Vec<StepRecord>,
    pub evidence_bundle: EvidenceBundle,
    pub ledger_refs: LedgerRefs,
}
