use async_trait::async_trait;

use super::{
    capability::{CapabilityAdmissionDecision, CapabilityCandidate, CapabilityIntent},
    context::{KnowledgeContext, UnifiedQueryView},
    errors::ContractError,
    events::DomainEvent,
    flow::{FlowMapSnapshot, FlowStatePatch, RuntimeModeDecision},
    focus_trigger::{FocusBoard, TriggerSpec},
    identity::AgentWorkspaceSnapshot,
    ids::{SessionId, TaskId},
    org::OrganizationContext,
    plugin::{
        PluginInstallRequest, PluginLifecycleEvent, PluginManifestContract,
        PluginVerificationVerdict,
    },
    query::{
        QueryContinuation, QueryFrame, QueryTurn, QueryTurnOutcome, SessionCheckpoint,
        SessionResumeOutcome, SessionResumeRequest,
    },
    services::{ServiceCall, ServiceHealthSnapshot, ServiceResult},
    transport::{BridgeControlDecision, TransportEnvelope},
    types::{
        ConstraintSet, ExecutionPlan, Intent, LearningDelta, PolicyDecision, ReportArtifact,
        RunReceipt, TaskEnvelope, VerificationVerdict,
    },
};

#[async_trait]
pub trait OperatorControlPlane: Send + Sync {
    async fn approve_intent(&self, intent: &Intent) -> Result<bool, ContractError>;
    async fn veto_session(&self, session_id: &SessionId, reason: &str)
    -> Result<(), ContractError>;
}

#[async_trait]
pub trait PolicyRuleEngine: Send + Sync {
    async fn evaluate_intent(
        &self,
        intent: &Intent,
        constraints: &ConstraintSet,
    ) -> Result<PolicyDecision, ContractError>;
}

#[async_trait]
pub trait OrganizationContextInjector: Send + Sync {
    async fn inject_context(
        &self,
        session_id: &SessionId,
    ) -> Result<OrganizationContext, ContractError>;
}

#[async_trait]
pub trait KnowledgeContextInjector: Send + Sync {
    async fn inject_knowledge_context(
        &self,
        session_id: &SessionId,
    ) -> Result<KnowledgeContext, ContractError>;
}

#[async_trait]
pub trait AgentWorkspaceLoader: Send + Sync {
    async fn load_workspace(
        &self,
        session_id: &SessionId,
    ) -> Result<AgentWorkspaceSnapshot, ContractError>;
}

#[async_trait]
pub trait FocusBoardBuilderPort: Send + Sync {
    async fn build_focus_board(&self, intent: &Intent) -> Result<FocusBoard, ContractError>;
}

#[async_trait]
pub trait TriggerRuntimePort: Send + Sync {
    async fn register_trigger(
        &self,
        session_id: &SessionId,
        trigger: &TriggerSpec,
    ) -> Result<(), ContractError>;
}

#[async_trait]
pub trait CapabilityIntentSelectorPort: Send + Sync {
    async fn select_candidates(
        &self,
        intent: &CapabilityIntent,
    ) -> Result<Vec<CapabilityCandidate>, ContractError>;
}

#[async_trait]
pub trait CapabilityAdmissionPort: Send + Sync {
    async fn admit(
        &self,
        session_id: &SessionId,
        candidates: &[CapabilityCandidate],
    ) -> Result<CapabilityAdmissionDecision, ContractError>;
}

#[async_trait]
pub trait RuntimeModeDispatcherPort: Send + Sync {
    async fn dispatch_mode(
        &self,
        envelope: &TaskEnvelope,
        has_degrade_profile: bool,
        is_replay: bool,
    ) -> Result<RuntimeModeDecision, ContractError>;
}

#[async_trait]
pub trait FlowStateEnginePort: Send + Sync {
    async fn upsert_flow_map(&self, snapshot: &FlowMapSnapshot) -> Result<(), ContractError>;
    async fn apply_patch(&self, patch: &FlowStatePatch) -> Result<(), ContractError>;
    async fn fetch_flow_map(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<FlowMapSnapshot>, ContractError>;
}

#[async_trait]
pub trait QueryPlanePort: Send + Sync {
    async fn query_unified(
        &self,
        session_id: &SessionId,
        trace_id: Option<&str>,
    ) -> Result<UnifiedQueryView, ContractError>;
}

#[async_trait]
pub trait QueryEnginePort: Send + Sync {
    async fn submit_turn(
        &self,
        frame: &QueryFrame,
        turn: &QueryTurn,
    ) -> Result<QueryTurnOutcome, ContractError>;
    async fn continue_turn(
        &self,
        continuation: &QueryContinuation,
    ) -> Result<QueryTurnOutcome, ContractError>;
}

#[async_trait]
pub trait SessionRuntimePort: Send + Sync {
    async fn checkpoint(&self, checkpoint: &SessionCheckpoint) -> Result<(), ContractError>;
    async fn resume(
        &self,
        request: &SessionResumeRequest,
    ) -> Result<SessionResumeOutcome, ContractError>;
}

#[async_trait]
pub trait TransportBridgePort: Send + Sync {
    async fn ingest(&self, envelope: &TransportEnvelope) -> Result<(), ContractError>;
    async fn dispatch(&self, envelope: &TransportEnvelope) -> Result<(), ContractError>;
    async fn decide(&self, decision: &BridgeControlDecision) -> Result<(), ContractError>;
}

#[async_trait]
pub trait PluginLifecyclePort: Send + Sync {
    async fn install_plugin(
        &self,
        request: &PluginInstallRequest,
    ) -> Result<PluginManifestContract, ContractError>;
    async fn apply_lifecycle_event(
        &self,
        event: &PluginLifecycleEvent,
    ) -> Result<(), ContractError>;
    async fn verify_plugin(
        &self,
        plugin_id: &str,
    ) -> Result<PluginVerificationVerdict, ContractError>;
}

#[async_trait]
pub trait ServiceMediatorPort: Send + Sync {
    async fn mediate(&self, call: &ServiceCall) -> Result<ServiceResult, ContractError>;
    async fn health(&self) -> Result<Vec<ServiceHealthSnapshot>, ContractError>;
}

#[async_trait]
pub trait ExecutionFabricPort: Send + Sync {
    async fn persist_execution_fabric(
        &self,
        receipt: &RunReceipt,
        verdict: &VerificationVerdict,
    ) -> Result<(), ContractError>;
}

#[async_trait]
pub trait EvidenceLedgerPort: Send + Sync {
    async fn append_evidence_step(&self, event: &DomainEvent) -> Result<(), ContractError>;
}

#[async_trait]
pub trait SharedKnowledgePublisherPort: Send + Sync {
    async fn publish_shared_knowledge(
        &self,
        session_id: &SessionId,
        delta: &LearningDelta,
    ) -> Result<(), ContractError>;
}

#[async_trait]
pub trait StrategyUpdaterPort: Send + Sync {
    async fn update_strategy(
        &self,
        session_id: &SessionId,
        verdict: &VerificationVerdict,
        delta: &LearningDelta,
    ) -> Result<(), ContractError>;
}

#[async_trait]
pub trait OrchestratorScheduler: Send + Sync {
    async fn build_plan(
        &self,
        intent: &Intent,
        decision: &PolicyDecision,
    ) -> Result<ExecutionPlan, ContractError>;
    async fn schedule_task(&self, plan: &ExecutionPlan) -> Result<Vec<TaskId>, ContractError>;
}

#[async_trait]
pub trait ExecutionPool: Send + Sync {
    async fn execute_task(&self, envelope: &TaskEnvelope) -> Result<RunReceipt, ContractError>;
}

#[async_trait]
pub trait RuntimeKernel: Send + Sync {
    async fn guard_and_execute(
        &self,
        envelope: &TaskEnvelope,
        pool: &dyn ExecutionPool,
    ) -> Result<RunReceipt, ContractError>;
}

#[async_trait]
pub trait VerifierAuditPipeline: Send + Sync {
    async fn verify(&self, receipt: &RunReceipt) -> Result<VerificationVerdict, ContractError>;
    async fn record_event(&self, event: &DomainEvent) -> Result<(), ContractError>;
}

#[async_trait]
pub trait LearningGraphEngine: Send + Sync {
    async fn apply_learning(
        &self,
        receipt: &RunReceipt,
        verdict: &VerificationVerdict,
    ) -> Result<LearningDelta, ContractError>;
}

#[async_trait]
pub trait ReportingObservability: Send + Sync {
    async fn emit_report(
        &self,
        verdict: &VerificationVerdict,
        delta: &LearningDelta,
    ) -> Result<ReportArtifact, ContractError>;
}
