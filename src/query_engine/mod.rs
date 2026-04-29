pub mod compactor;
pub mod context_compiler;
pub mod context_runtime_kernel;
pub mod continuation;
pub mod default_plugins;
pub mod diff_patch_engine;
pub mod failure_experience;
pub mod git_checkpoint_layer;
pub mod iteration_controller;
pub mod repo_context_compiler;
pub mod r#loop;
pub mod shell_execution_loop;
pub mod test_verifier;
pub mod turn_state;

use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use autoloop_state_adapter::StateStore;

use crate::{
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    providers::{ChatMessage, LlmResponse, ProviderRegistry, ToolCall},
    runtime::{GuardDecision, RuntimeKernel, decision_protocol::ExecutionGuardObservation},
    security::SecurityPolicy,
    tools::ToolRegistry,
};

pub use self::compactor::{CompactionBoundary, CompactionStrategy, ContextCompactor};
pub use self::context_compiler::{CompiledContext, ContextCompiler, TokenBudgetFrame};
pub use self::context_runtime_kernel::{
    ContextRuntimeKernel, ContextRuntimeKernelMode, ContextRuntimeRun,
};
pub use self::continuation::{
    ContinuationCheckpoint, ContinuationProtocol, ContinuationRequest, build_continuation_protocol,
    build_replay_fingerprint,
};
pub use self::default_plugins::{
    DefaultContextCompilerPlugins, ObjectiveScore, ObjectiveWeights, ProofResult,
};
pub use self::diff_patch_engine::{DiffPatchEngine, PatchExecutionReport, PatchStepRecord, PatchStepStatus};
pub use self::failure_experience::{
    FailureExperienceEntry, load_failure_experience_hints, merge_failure_experience_hints,
    record_failure_experience,
};
pub use self::git_checkpoint_layer::{
    GitCheckpointLayer, GitCheckpointOperationInput, GitCheckpointReport, GitCheckpointRequest,
    GitCheckpointStepRecord,
};
pub use self::iteration_controller::{
    FailureCategory, IterationAttemptRecord, IterationControllerConfig, IterationControllerReport,
    RepairStrategy, TerminationReason, classify_failure, select_repair_strategy, should_retry,
    should_retry_for_failure, stage_from_error,
};
pub use self::repo_context_compiler::RepoContextCompiler;
pub use self::shell_execution_loop::{ShellExecutionLoopEngine, ShellLoopReport, ShellLoopRequest, ShellLoopStepInput};
pub use self::test_verifier::{TestRunnerKind, TestRunnerSpec, TestVerifierEngine, TestVerifierReport, TestVerifierRequest};
pub use self::r#loop::{
    PluginExecutionTrace, QueryLoopBackend, QueryLoopConfig, QueryLoopEngine, QueryLoopInput,
    QueryLoopOutput, QueryLoopStreamEvent, ToolExecutionEvent,
};

#[derive(Clone)]
pub struct RuntimeQueryLoopBackend {
    runtime: RuntimeKernel,
    state_store: StateStore,
    tools: ToolRegistry,
    providers: ProviderRegistry,
    security: SecurityPolicy,
    execution_identity: ExecutionIdentity,
    provider_constraints: ConstraintSet,
    tool_constraints: ConstraintSet,
    preferred_model: Option<String>,
    guard_observations: Arc<Mutex<Vec<ExecutionGuardObservation>>>,
}

impl RuntimeQueryLoopBackend {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime: RuntimeKernel,
        state_store: StateStore,
        tools: ToolRegistry,
        providers: ProviderRegistry,
        security: SecurityPolicy,
        execution_identity: ExecutionIdentity,
        provider_constraints: ConstraintSet,
        tool_constraints: ConstraintSet,
        preferred_model: Option<String>,
    ) -> Self {
        Self {
            runtime,
            state_store,
            tools,
            providers,
            security,
            execution_identity,
            provider_constraints,
            tool_constraints,
            preferred_model,
            guard_observations: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn push_guard_observation(
        &self,
        surface: &str,
        capability_id: &str,
        guard_report: &crate::runtime::RuntimeGuardReport,
    ) {
        let decision = match guard_report.decision {
            GuardDecision::Allow => "allow",
            GuardDecision::RequiresApproval => "requires_approval",
            GuardDecision::Blocked => "blocked",
        };
        if let Ok(mut sink) = self.guard_observations.lock() {
            sink.push(ExecutionGuardObservation {
                surface: surface.to_string(),
                capability_id: capability_id.to_string(),
                decision: decision.to_string(),
                reason: guard_report.reason.clone(),
            });
        }
    }
}

#[async_trait::async_trait]
impl QueryLoopBackend for RuntimeQueryLoopBackend {
    async fn provider_step(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
        hardgate_pass_token: &str,
    ) -> Result<LlmResponse> {
        let provider_envelope = TaskEnvelope {
            session_id: SessionId::from(session_id),
            trace_id: TraceId::from(format!(
                "{}:provider-loop:{}",
                session_id,
                current_time_ms()
            )),
            task_id: TaskId::from("agent-provider-loop"),
            capability_id: CapabilityId::from("provider:default"),
            identity: self.execution_identity.clone(),
            payload: serde_json::json!({"messages": messages, "hardgate_required": true, "hardgate_pass_token": hardgate_pass_token}),
            constraints: self.provider_constraints.clone(),
            trust_plan: None,
        };

        let result = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &provider_envelope,
                None,
                self.preferred_model.as_deref(),
            )
            .await?;
        self.push_guard_observation("provider", "provider:default", &result.guard_report);

        Ok(result.provider_response.unwrap_or(LlmResponse {
            content: None,
            tool_calls: Vec::new(),
        }))
    }

    async fn tool_step(
        &self,
        session_id: &str,
        call: &ToolCall,
        hardgate_pass_token: &str,
    ) -> Result<String> {
        let tool_report = self
            .security
            .inspect_tool_call(&self.state_store, session_id, &call.name, &call.arguments)
            .await?;
        if tool_report.blocked {
            let details = tool_report
                .findings
                .into_iter()
                .map(|finding| finding.detail)
                .collect::<Vec<_>>()
                .join("; ");
            bail!(
                "Tool call '{}' blocked by security policy: {}",
                call.name,
                details
            );
        }

        let manifest_owned = self
            .tools
            .manifests()
            .into_iter()
            .find(|manifest| manifest.registered_tool_name == call.name);

        let envelope = TaskEnvelope {
            session_id: SessionId::from(session_id),
            trace_id: TraceId::from(format!(
                "{}:{}:{}",
                session_id,
                call.name,
                current_time_ms()
            )),
            task_id: TaskId::from(format!("agent-tool-{}", current_time_ms())),
            capability_id: CapabilityId::from(call.name.as_str()),
            identity: self.execution_identity.clone(),
            payload: serde_json::json!({"arguments": call.arguments.clone(), "hardgate_required": true, "hardgate_pass_token": hardgate_pass_token}),
            constraints: self.tool_constraints.clone(),
            trust_plan: None,
        };

        let executed = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &envelope,
                manifest_owned.as_ref(),
                None,
            )
            .await?;
        self.push_guard_observation("tool", &call.name, &executed.guard_report);

        Ok(executed.content)
    }

    fn guard_observations(&self) -> Vec<ExecutionGuardObservation> {
        self.guard_observations
            .lock()
            .map(|entries| entries.clone())
            .unwrap_or_default()
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}



