pub use crate::admission_sigstore::{
    CommandOutput, CommandRunner, ProcessCommandRunner, SigstoreAdmissionConfig,
    SigstoreAdmissionVerifier,
};
pub use crate::execution::{CapabilityAdapter, CapabilityRegistry, EchoCapability, KernelExecutor};
pub use crate::governance::{
    KeywordResultValidator, LearningWriteGate, NoopRecoverySubsystem, RecoverySubsystem,
    ResultValidator, StatusLearningWriteGate,
};
pub use crate::ir::{
    ConstraintSet, EvidenceBundle, ExecutionPlan, ExecutionRecord, ExecutionStep, FailurePolicy,
    FingerprintMaterial, HardeningPolicy, IdentityContext, Intent, LedgerRefs, ReproducibleClosure,
    RuntimeIsland, StepAction, StepRecord, SupplyChainEnvelope, TraceContext,
    TrustedExecutionRequest, VerificationSpec,
};
pub use crate::kernel::NoumenonCore;
pub use crate::ledger::{
    BudgetAccount, BudgetLedger, EvidenceLedger, EvidenceRecord, FileBudgetLedger,
    FileEvidenceLedger, InMemoryBudgetLedger, InMemoryEvidenceLedger,
};
pub use crate::replay::{
    DefaultReplayExplainer, OpenRolloutGate, PercentRolloutGate, ReplayExplainer, RolloutGate,
};
pub use crate::resource::{
    ResourceGovernor, ResourceRequirement, ResourceReservation, SimpleResourceGovernor,
};
pub use crate::routing::{EchoModelBackend, LlmBackend, ModelRouter, RoutingStrategy};
pub use crate::state::{ExecutionStatus, StateMachine};
pub use crate::syscall::{
    InMemorySyscallQueue, SyscallKind, SyscallQueue, SyscallRequest, SyscallResponse,
    SyscallScheduler,
};
pub use crate::trust::{
    AttestationVerifier, CryptoService, HmacIdentityAuthority, IdentityAuthority,
    Sha256CryptoService, StaticAttestationVerifier, StaticSupplyChainVerifier,
    StaticTenantAuthority, SupplyChainVerifier, TenantAuthority, TrustLevel,
};

#[cfg(feature = "ext-evolution")]
pub use crate::evolution::{
    DegradationManager, FileRecoveryPolicyStore, PolicyAdvisor, RecoveryPolicy,
    RecoveryPolicyStore, SimplePolicyAdvisor, StaticDegradationManager,
};
#[cfg(feature = "ext-memory")]
pub use crate::memory_evolution::{
    EvolutionAction, EvolutionDecision, InMemoryMemoryStore, MemoryEvolutionEngine, MemoryNote,
    MemoryStore,
};
#[cfg(feature = "ext-observability")]
pub use crate::observability::{
    AlertEngine, AlertRule, InMemoryMetricsSink, MetricEvent, MetricsSink, ReportBuilder,
    SloAdvisor, SloTarget,
};
#[cfg(feature = "ext-memory")]
pub use crate::storage_vector::{InMemoryVectorStore, VectorPoint, VectorStore};
#[cfg(feature = "ext-tooling")]
pub use crate::tooling::{ToolAdapter, ToolOrchestrator, ToolRequest, VirtualEnvToolAdapter};
#[cfg(feature = "ext-truth")]
pub use crate::truth::{MismatchCategory, ReplayAnalysis, TruthEngine};
