pub mod focus_board;
pub mod governance_telemetry_scope;
pub mod knowledge_context;
pub mod org_context;
pub mod response_builder;
use std::collections::BTreeMap;
use std::collections::HashMap;

use anyhow::Result;
use autoloop_state_adapter::{KnowledgeRecord, StateStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    adaptive_framework::{PromptTemplateProfile, build_prompt_template_bundle},
    agent::workspace_loader::WorkspaceLoader,
    agentevolver_task_core::AgentEvolverTaskPack,
    contracts::{
        capability::CapabilityIntent,
        context::{GovernanceContext, KnowledgeContext},
        focus_trigger::{FocusBoard, TriggerRef},
        identity::AgentWorkspaceSnapshot,
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        org::OrganizationContext,
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    memory::{
        JointRoutingEvidence, LearningAssetKind, LearningScorer, MemorySubsystem,
        RetrievalEvidence, WeightedLearningScorer,
    },
    providers::{
        ChatMessage, OptimizationProposal, OptimizationSignal, PromptPolicyOverlay,
        ProviderRegistry,
    },
    rag::{GraphKnowledgeUpdate, GraphRoutingSignals, RagSubsystem},
    runtime::{DegradeProfileKind, RuntimeKernel, VerifierReport, VerifierVerdict},
    session::{
        SessionStore, audit::StateAuditSink, machine::WorkflowMachine, signal::WorkflowSignal,
    },
    skills::foundry::{extract_first_principles, normalize_intake, promotion_suggestion, route_layer},
    tools::{CapabilityRisk, ToolRegistry},
};

use self::focus_board::FocusBoardBuilder;
use self::knowledge_context::KnowledgeContextResolver;
use self::org_context::OrganizationContextResolver;
use crate::contracts::ports::{
    CapabilityIntentSelectorPort, KnowledgeContextInjector, OrganizationContextInjector,
};
use crate::plugins::gitmemory_core::{GitmemoryCoreKernel, GovernancePhase};
use crate::runtime::execution_fabric::{ExecutionFabricTrace, persist_execution_fabric_trace};
use crate::runtime::trigger_runtime::TriggerRuntimeEngine;
use crate::security::capability_admission::{CapabilityAdmissionEngine, CapabilityIntentSelector};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementBrief {
    pub anchor_id: String,
    pub original_request: String,
    pub clarified_goal: String,
    pub frozen_scope: String,
    pub open_questions: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub clarification_turns: Vec<RequirementTurn>,
    pub confirmation_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementTurn {
    pub turn_index: usize,
    pub question: String,
    pub inferred_answer: String,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmDeliberation {
    pub planner_notes: String,
    pub critic_notes: String,
    pub planner_rebuttal: String,
    pub judge_notes: String,
    pub arbitration_summary: String,
    pub round_count: usize,
    pub rounds: Vec<DebateRound>,
    pub final_execution_order: Vec<String>,
    pub consensus_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateRound {
    pub round_index: usize,
    pub speaker: String,
    pub stance: String,
    pub summary: String,
    pub supporting_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTask {
    pub task_id: String,
    pub agent_name: String,
    pub role: String,
    pub objective: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPoolKind {
    Stable,
    Adaptive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPoolQueue {
    pub pool: ExecutionPoolKind,
    pub tasks: Vec<SwarmTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub task: SwarmTask,
    pub output: String,
    pub tool_used: Option<String>,
    pub mcp_server: Option<String>,
    pub invocation_payload: Option<String>,
    pub outcome_score: i32,
    pub route_variant: String,
    pub control_score: i32,
    pub treatment_score: i32,
    pub guard_decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub ready: bool,
    pub summary: String,
    pub follow_up_tasks: Vec<SwarmTask>,
    pub verifier_summary: String,
}

#[derive(Debug, Clone)]
struct GovernanceBlockSummary {
    reason: String,
    evidence_ref: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoutingContext {
    pub history_records: Vec<KnowledgeRecord>,
    pub execution_metrics: Vec<ExecutionStats>,
    pub graph_signals: GraphRoutingSignals,
    pub pending_event_count: usize,
    pub learning_evidence: Vec<RetrievalEvidence>,
    pub skill_success_rate: f32,
    pub causal_confidence: f32,
    pub forged_tool_coverage: usize,
    pub session_ab_stats: Option<AbRoutingStats>,
    pub task_ab_stats: HashMap<String, AbRoutingStats>,
    pub tool_ab_stats: HashMap<String, AbRoutingStats>,
    pub server_ab_stats: HashMap<String, AbRoutingStats>,
    pub agent_reputations: HashMap<String, AgentReputation>,
    pub route_biases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentReputation {
    pub agent_name: String,
    pub runs: u32,
    pub average_score: f32,
    pub verifier_alignment: f32,
    pub trust: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolRoutingDecision {
    pub tool_name: Option<String>,
    pub mcp_server: Option<String>,
    pub invocation_payload: Option<String>,
    pub score: i32,
    pub control_score: i32,
    pub treatment_score: i32,
    pub route_variant: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStats {
    pub tool_name: String,
    pub mcp_server: Option<String>,
    pub attempts: u32,
    pub successes: u32,
    pub failures: u32,
    pub cumulative_score: i32,
    pub success_rate: f32,
    pub effective_success_rate: f32,
    pub effective_score: f32,
    pub last_updated_ms: u64,
    pub last_payload: Option<String>,
    pub last_outcome: String,
    pub samples: Vec<ExecutionSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionSample {
    pub observed_at_ms: u64,
    pub outcome_score: i32,
    pub success: bool,
    pub mcp_server: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AbRoutingStats {
    pub scope: String,
    pub attempts: u32,
    pub treatment_attempts: u32,
    pub treatment_wins: u32,
    pub treatment_losses: u32,
    pub control_wins: u32,
    pub cumulative_lift: f32,
    pub effective_win_rate: f32,
    pub effective_lift: f32,
    pub last_updated_ms: u64,
    pub samples: Vec<AbRoutingSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbRoutingSample {
    pub observed_at_ms: u64,
    pub route_variant: String,
    pub treatment_win: bool,
    pub control_win: bool,
    pub lift: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwarmOutcome {
    pub brief: RequirementBrief,
    pub optimization_proposal: OptimizationProposal,
    pub routing_context: RoutingContext,
    pub ceo_summary: String,
    pub deliberation: SwarmDeliberation,
    pub tasks: Vec<SwarmTask>,
    pub execution_reports: Vec<ExecutionReport>,
    pub verifier_report: VerifierReport,
    pub validation: ValidationReport,
    pub knowledge_update: GraphKnowledgeUpdate,
}

#[derive(Clone)]
pub struct OrchestrationKernel {
    providers: ProviderRegistry,
    tools: ToolRegistry,
    sessions: SessionStore,
    memory: MemorySubsystem,
    rag: RagSubsystem,
    runtime: RuntimeKernel,
    state_store: StateStore,
    gray_routing_ratio: f32,
    routing_takeover_threshold: f32,
}

impl OrchestrationKernel {
    pub fn new(
        providers: ProviderRegistry,
        tools: ToolRegistry,
        sessions: SessionStore,
        memory: MemorySubsystem,
        rag: RagSubsystem,
        runtime: RuntimeKernel,
        state_store: StateStore,
        gray_routing_ratio: f32,
        routing_takeover_threshold: f32,
    ) -> Self {
        Self {
            providers,
            tools,
            sessions,
            memory,
            rag,
            runtime,
            state_store,
            gray_routing_ratio,
            routing_takeover_threshold,
        }
    }

    pub async fn run_requirement_swarm(
        &self,
        session_id: &str,
        request: &str,
    ) -> Result<SwarmOutcome> {
        let mut workflow = WorkflowMachine::new(
            session_id,
            Arc::new(StateAuditSink::with_source(
                self.state_store.clone(),
                "orchestration-workflow",
            )),
        );
        let _ = workflow
            .apply(
                WorkflowSignal::IntentReceived,
                Some("requirement received".into()),
            )
            .await;
        let mut effective_request = request.to_string();
        if let Some(policy_reason) = policy_requires_revision(request) {
            let _ = workflow
                .apply(
                    WorkflowSignal::PolicyRejected,
                    Some(format!("policy rejected: {policy_reason}")),
                )
                .await;
            effective_request = format!(
                "Policy revise required before planning. Keep intent, remove unsafe instruction patterns. Original request:\n{}",
                request
            );
            let _ = workflow
                .apply(
                    WorkflowSignal::PolicyApproved,
                    Some("policy revised and approved".into()),
                )
                .await;
        } else {
            let _ = workflow
                .apply(
                    WorkflowSignal::PolicyApproved,
                    Some("policy baseline approved".into()),
                )
                .await;
        }
        let brief = self
            .requirement_agent_dialogue(session_id, &effective_request)
            .await?;
        let focus_board = self.build_focus_board(session_id, &brief).await?;
        let trigger_refs = self
            .register_focus_triggers(session_id, &focus_board)
            .await?;
        self.persist_focus_runtime_snapshots(session_id, &focus_board, &trigger_refs)
            .await?;
        let routing_context = self
            .load_routing_context(session_id, &effective_request)
            .await?;
        let optimization_proposal = self.optimization_proposal(&brief, &routing_context).await?;
        let ceo_summary = self
            .ceo_summary(session_id, &brief, &routing_context)
            .await?;
        let _ = workflow
            .apply(
                WorkflowSignal::PlanCommitted,
                Some("swarm plan committed".into()),
            )
            .await;
        let tasks = self.build_swarm_team(
            &brief,
            &routing_context,
            &ceo_summary,
            &optimization_proposal,
        );
        let _ = workflow
            .apply(
                WorkflowSignal::TaskScheduled,
                Some("tasks scheduled".into()),
            )
            .await;
        let deliberation = self
            .deliberate_swarm(session_id, &brief, &routing_context, &tasks, &ceo_summary)
            .await?;
        let _ = workflow
            .apply(
                WorkflowSignal::ExecutionStarted,
                Some("swarm execution started".into()),
            )
            .await;
        let execution_reports = self
            .execute_swarm(session_id, &tasks, &brief, &routing_context)
            .await?;
        for report in &execution_reports {
            let signal = self.runtime.workflow_signal_from_execution_report(report);
            let _ = workflow
                .apply(signal, Some("runtime execution outcome".into()))
                .await;
        }
        let verifier_report = self.runtime.verify_swarm_outcome(
            &brief,
            &routing_context,
            &execution_reports,
            &self.tools,
        );
        let verifier_signal = if verifier_report.verdict == VerifierVerdict::Pass {
            WorkflowSignal::VerifyPassed
        } else {
            WorkflowSignal::VerifyRejected
        };
        let _ = workflow
            .apply(verifier_signal, Some("verifier completed".into()))
            .await;
        let _ = workflow
            .apply(WorkflowSignal::Closed, Some("workflow closed".into()))
            .await;
        let validation = self.validate_outcome(
            &brief,
            &routing_context,
            &execution_reports,
            &verifier_report,
        );
        let knowledge_update = self.rag.build_knowledge_update(
            session_id,
            &brief.original_request,
            &format!("{ceo_summary}\n{}", deliberation.arbitration_summary),
            &tasks,
            &execution_reports,
            &validation,
        );

        Ok(SwarmOutcome {
            brief,
            optimization_proposal,
            routing_context,
            ceo_summary,
            deliberation,
            tasks,
            execution_reports,
            verifier_report,
            validation,
            knowledge_update,
        })
    }

    async fn inject_organization_context(&self, session_id: &str) -> Result<OrganizationContext> {
        let resolver = OrganizationContextResolver::new(self.state_store.clone());
        let injector: &dyn OrganizationContextInjector = &resolver;
        injector
            .inject_context(&SessionId::from(session_id))
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    fn governance_from_org_context(&self, org_context: &OrganizationContext) -> GovernanceContext {
        GovernanceContext {
            session_id: org_context.session_id.clone(),
            tenant_id: org_context.tenant_id.clone(),
            principal_id: org_context.principal_id.clone(),
            policy_id: org_context.policy_id.clone(),
            role: org_context.role.clone(),
            approval_policy: org_context.approval_policy.clone(),
            risk_tier: org_context
                .metadata
                .get("risk_tier")
                .cloned()
                .unwrap_or_else(|| "balanced".to_string()),
            route_policy: org_context
                .metadata
                .get("route_policy")
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec!["policy-guided-routing".to_string()]),
            quotas: org_context.quotas.clone(),
            metadata: org_context.metadata.clone(),
        }
    }

    async fn inject_governance_context(&self, session_id: &str) -> Result<GovernanceContext> {
        let org_context = self.inject_organization_context(session_id).await?;
        Ok(self.governance_from_org_context(&org_context))
    }

    async fn inject_knowledge_context(&self, session_id: &str) -> Result<KnowledgeContext> {
        let resolver = KnowledgeContextResolver::new(self.state_store.clone());
        let injector: &dyn KnowledgeContextInjector = &resolver;
        injector
            .inject_knowledge_context(&SessionId::from(session_id))
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }
    async fn load_workspace_snapshot(&self, session_id: &str) -> Result<AgentWorkspaceSnapshot> {
        let loader = WorkspaceLoader::new(self.state_store.clone());
        loader.load(session_id).await
    }

    async fn persist_dual_context_snapshots(
        &self,
        session_id: &str,
        governance_context: &GovernanceContext,
        knowledge_context: &KnowledgeContext,
        workspace_snapshot: &AgentWorkspaceSnapshot,
    ) -> Result<()> {
        let compatibility_org_context = OrganizationContext {
            session_id: governance_context.session_id.clone(),
            tenant_id: governance_context.tenant_id.clone(),
            principal_id: governance_context.principal_id.clone(),
            policy_id: governance_context.policy_id.clone(),
            role: governance_context.role.clone(),
            approval_policy: governance_context.approval_policy.clone(),
            kb_refs: knowledge_context.kb_refs.clone(),
            plaza_refs: knowledge_context.plaza_refs.clone(),
            quotas: governance_context.quotas.clone(),
            metadata: governance_context.metadata.clone(),
        };

        self.state_store
            .upsert_knowledge(
                format!("org-context:{session_id}:latest"),
                serde_json::to_string(&compatibility_org_context)?,
                "orchestration:org-context".to_string(),
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("governance-context:{session_id}:latest"),
                serde_json::to_string(governance_context)?,
                "orchestration:governance-context".to_string(),
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("knowledge-context:{session_id}:latest"),
                serde_json::to_string(knowledge_context)?,
                "orchestration:knowledge-context".to_string(),
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("workspace-context:{session_id}:latest"),
                serde_json::to_string(workspace_snapshot)?,
                "orchestration:workspace-loader".to_string(),
            )
            .await?;
        Ok(())
    }
    async fn build_focus_board(
        &self,
        session_id: &str,
        brief: &RequirementBrief,
    ) -> Result<FocusBoard> {
        let builder = FocusBoardBuilder::new();
        Ok(builder.build(session_id, brief))
    }

    async fn register_focus_triggers(
        &self,
        session_id: &str,
        board: &FocusBoard,
    ) -> Result<Vec<TriggerRef>> {
        let runtime = TriggerRuntimeEngine::new(self.state_store.clone());
        runtime
            .register_focus_triggers(session_id, board, "orchestrator")
            .await
    }

    async fn persist_focus_runtime_snapshots(
        &self,
        session_id: &str,
        board: &FocusBoard,
        trigger_refs: &[TriggerRef],
    ) -> Result<()> {
        self.state_store
            .upsert_knowledge(
                format!("focus-board:{session_id}:latest"),
                serde_json::to_string(board)?,
                "orchestration:focus-board".to_string(),
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("trigger-runtime:{session_id}:latest"),
                serde_json::to_string(trigger_refs)?,
                "runtime:trigger-runtime".to_string(),
            )
            .await?;
        Ok(())
    }
    async fn requirement_agent_dialogue(
        &self,
        session_id: &str,
        request: &str,
    ) -> Result<RequirementBrief> {
        self.sessions.append_user_message(session_id, request).await;
        let governance_context = self.inject_governance_context(session_id).await?;
        let knowledge_context = self.inject_knowledge_context(session_id).await?;
        let workspace_snapshot = self.load_workspace_snapshot(session_id).await?;
        self.persist_dual_context_snapshots(
            session_id,
            &governance_context,
            &knowledge_context,
            &workspace_snapshot,
        )
        .await?;

        let open_questions = infer_open_questions(request);
        let clarification_turns = open_questions
            .iter()
            .enumerate()
            .map(|(index, question)| RequirementTurn {
                turn_index: index + 1,
                question: question.clone(),
                inferred_answer: infer_requirement_answer(question, request),
                resolved: !requires_user_confirmation(question, request),
            })
            .collect::<Vec<_>>();
        let prompt = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".into(),
                content: "You are the requirement agent. Run a bounded clarification loop, freeze the scope, and rewrite the request into an actionable brief with explicit acceptance criteria.".into(),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: serde_json::json!({
                    "request": request,
                    "clarification_turns": clarification_turns,
                    "governance_context": governance_context,
                    "knowledge_context": knowledge_context,
                    "workspace_snapshot": workspace_snapshot,
                })
                .to_string(),
            },
        ];
        let envelope = self.provider_envelope(
            session_id,
            "requirement-dialogue",
            self.execution_identity_for_session(session_id).await?,
            &prompt,
            self.default_provider_constraints(),
        );
        let reply = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &envelope,
                None,
                None,
            )
            .await?
            .provider_response
            .unwrap_or(crate::providers::LlmResponse {
                content: None,
                tool_calls: Vec::new(),
            });
        let clarified_goal = reply
            .content
            .unwrap_or_else(|| format!("Clarified requirement: {request}"));
        self.sessions
            .append_assistant_message(session_id, &clarified_goal)
            .await;

        Ok(RequirementBrief {
            anchor_id: format!("anchor:{}", session_id),
            original_request: request.to_string(),
            clarified_goal,
            frozen_scope: infer_frozen_scope(request),
            open_questions,
            acceptance_criteria: infer_acceptance_criteria(request),
            clarification_turns: clarification_turns.clone(),
            confirmation_required: clarification_turns.iter().any(|turn| !turn.resolved),
        })
    }

    async fn deliberate_swarm(
        &self,
        session_id: &str,
        brief: &RequirementBrief,
        routing: &RoutingContext,
        tasks: &[SwarmTask],
        ceo_summary: &str,
    ) -> Result<SwarmDeliberation> {
        let overlay = self.adaptive_overlay(&brief.clarified_goal, routing);
        let task_pack = self.adaptive_task_pack(&brief.clarified_goal, routing);
        let execution_order = topological_tasks(tasks)
            .into_iter()
            .map(|task| task.task_id)
            .collect::<Vec<_>>();
        let reputation_summary = summarize_agent_reputations(&routing.agent_reputations);
        let verifier_signal_summary = summarize_verifier_signals(&routing.history_records);
        let ops_signal_summary = summarize_ops_signals(routing);
        let planner_prompt = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".into(),
                content: format!(
                    "You are the swarm planner. Sequence the team, identify dependencies, and propose a bounded round-based plan.\n\n{}\n\n{}",
                    overlay_as_text(&overlay),
                    task_pack_as_text(&task_pack)
                ),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: serde_json::json!({
                    "goal": brief.clarified_goal,
                    "frozen_scope": brief.frozen_scope,
                    "tasks": tasks,
                    "proposed_execution_order": execution_order,
                    "ceo_summary": ceo_summary,
                    "agent_reputations": reputation_summary,
                })
                .to_string(),
            },
        ];
        let planner_envelope = self.provider_envelope(
            session_id,
            "planner-stage",
            self.execution_identity_for_session(session_id).await?,
            &planner_prompt,
            self.default_provider_constraints(),
        );
        let planner_notes = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &planner_envelope,
                None,
                overlay.preferred_model.as_deref(),
            )
            .await?
            .provider_response
            .unwrap_or(crate::providers::LlmResponse {
                content: None,
                tool_calls: Vec::new(),
            })
            .content
            .unwrap_or_else(|| {
                format!(
                    "Planner sequences {} tasks in two bounded rounds.",
                    tasks.len()
                )
            });

        let critic_prompt = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".into(),
                content: format!(
                    "You are the swarm critic. Find missing constraints, risky assumptions, and route weaknesses in the current plan.\n\n{}\n\n{}",
                    overlay_as_text(&overlay),
                    task_pack_as_text(&task_pack)
                ),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: serde_json::json!({
                    "goal": brief.clarified_goal,
                    "open_questions": brief.open_questions,
                    "planner_notes": planner_notes,
                    "route_biases": routing.route_biases,
                    "agent_reputations": routing.agent_reputations,
                    "graph_signals": routing.graph_signals,
                    "ops_signals": ops_signal_summary,
                    "verifier_signals": verifier_signal_summary,
                })
                .to_string(),
            },
        ];
        let critic_envelope = self.provider_envelope(
            session_id,
            "critic-stage",
            self.execution_identity_for_session(session_id).await?,
            &critic_prompt,
            self.default_provider_constraints(),
        );
        let critic_notes = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &critic_envelope,
                None,
                overlay.preferred_model.as_deref(),
            )
            .await?
            .provider_response
            .unwrap_or(crate::providers::LlmResponse {
                content: None,
                tool_calls: Vec::new(),
            })
            .content
            .unwrap_or_else(|| "Critic requests explicit verifier checkpoints, capability reuse, and graph-grounded execution.".into());

        let rebuttal_prompt = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".into(),
                content: format!(
                    "You are the swarm planner responding to a critic. Reconcile valid concerns, preserve necessary momentum, and update the round plan with minimal churn.\n\n{}\n\n{}",
                    overlay_as_text(&overlay),
                    task_pack_as_text(&task_pack)
                ),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: serde_json::json!({
                    "goal": brief.clarified_goal,
                    "planner_notes": planner_notes,
                    "critic_notes": critic_notes,
                    "execution_order": execution_order,
                    "agent_reputations": reputation_summary,
                    "verifier_signals": verifier_signal_summary,
                })
                .to_string(),
            },
        ];
        let rebuttal_envelope = self.provider_envelope(
            session_id,
            "planner-rebuttal-stage",
            self.execution_identity_for_session(session_id).await?,
            &rebuttal_prompt,
            self.default_provider_constraints(),
        );
        let planner_rebuttal = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &rebuttal_envelope,
                None,
                overlay.preferred_model.as_deref(),
            )
            .await?
            .provider_response
            .unwrap_or(crate::providers::LlmResponse {
                content: None,
                tool_calls: Vec::new(),
            })
            .content
            .unwrap_or_else(|| {
                "Planner accepts verifier guardrails, keeps the dependency order, and narrows execution to the highest-trust agents first.".into()
            });

        let judge_prompt = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".into(),
                content: format!(
                    "You are the swarm judge. Arbitrate between planner and critic, choose the final route, and freeze the execution order.\n\n{}\n\n{}",
                    overlay_as_text(&overlay),
                    task_pack_as_text(&task_pack)
                ),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: serde_json::json!({
                    "goal": brief.clarified_goal,
                    "planner_notes": planner_notes,
                    "critic_notes": critic_notes,
                    "planner_rebuttal": planner_rebuttal,
                    "graph_signals": routing.graph_signals,
                    "pending_events": routing.pending_event_count,
                    "agent_reputations": routing.agent_reputations,
                    "ops_signals": {
                        "route_biases": routing.route_biases,
                        "session_ab": routing.session_ab_stats,
                        "forged_tool_coverage": routing.forged_tool_coverage,
                        "summary": ops_signal_summary,
                    },
                    "verifier_signals": verifier_signal_summary,
                    "execution_metrics_summary": summarize_execution_metrics(&routing.execution_metrics),
                })
                .to_string(),
            },
        ];
        let judge_envelope = self.provider_envelope(
            session_id,
            "judge-stage",
            self.execution_identity_for_session(session_id).await?,
            &judge_prompt,
            self.default_provider_constraints(),
        );
        let judge_notes = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &judge_envelope,
                None,
                overlay.preferred_model.as_deref(),
            )
            .await?
            .provider_response
            .unwrap_or(crate::providers::LlmResponse {
                content: None,
                tool_calls: Vec::new(),
            })
            .content
            .unwrap_or_else(|| "Judge keeps the planner skeleton, accepts critic safeguards, and freezes a verifier-gated execution order.".into());

        let consensus_signals =
            build_consensus_signals(routing, &verifier_signal_summary, &ops_signal_summary);
        let rounds = vec![
            DebateRound {
                round_index: 1,
                speaker: "planner".into(),
                stance: "proposal".into(),
                summary: compress_text(&planner_notes, 240),
                supporting_signals: vec![
                    format!("execution_order={}", execution_order.join("->")),
                    format!("agent_reputations={}", reputation_summary),
                ],
            },
            DebateRound {
                round_index: 2,
                speaker: "critic".into(),
                stance: "challenge".into(),
                summary: compress_text(&critic_notes, 240),
                supporting_signals: vec![
                    format!("ops={ops_signal_summary}"),
                    format!("verifier={verifier_signal_summary}"),
                ],
            },
            DebateRound {
                round_index: 3,
                speaker: "planner".into(),
                stance: "rebuttal".into(),
                summary: compress_text(&planner_rebuttal, 240),
                supporting_signals: vec![
                    format!("reputation={reputation_summary}"),
                    format!(
                        "execution_metrics={}",
                        summarize_execution_metrics(&routing.execution_metrics)
                    ),
                ],
            },
            DebateRound {
                round_index: 4,
                speaker: "judge".into(),
                stance: "arbitration".into(),
                summary: compress_text(&judge_notes, 240),
                supporting_signals: consensus_signals.clone(),
            },
        ];

        Ok(SwarmDeliberation {
            planner_notes: planner_notes.clone(),
            critic_notes: critic_notes.clone(),
            planner_rebuttal: planner_rebuttal.clone(),
            judge_notes: judge_notes.clone(),
            arbitration_summary: format!(
                "Planner proposed: {} Critic warned: {} Planner rebutted: {} Judge finalized: {}",
                compress_text(&planner_notes, 180),
                compress_text(&critic_notes, 180),
                compress_text(&planner_rebuttal, 180),
                compress_text(&judge_notes, 180)
            ),
            round_count: rounds.len(),
            rounds,
            final_execution_order: execution_order,
            consensus_signals,
        })
    }

    async fn load_routing_context(
        &self,
        session_id: &str,
        request: &str,
    ) -> Result<RoutingContext> {
        let history_prefix = format!("conversation:{session_id}:");
        let mut history_records = self
            .state_store
            .list_knowledge_by_prefix(&history_prefix)
            .await?;
        history_records.extend(
            self.state_store
                .list_knowledge_by_prefix(&format!("research:{session_id}:"))
                .await?,
        );
        let execution_metrics = self
            .state_store
            .list_knowledge_by_prefix("metrics:execution:")
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<ExecutionStats>(&record.value).ok())
            .collect::<Vec<_>>();
        let pending_event_count = self
            .state_store
            .list_schedule_events(session_id)
            .await?
            .into_iter()
            .filter(|event| event.status != "completed")
            .count();

        let graph_signals = self
            .state_store
            .get_knowledge(&format!("graph:{session_id}:snapshot"))
            .await?
            .map(|record| self.rag.graph_routing_signals(&record.value))
            .unwrap_or_default();

        let route_biases =
            infer_route_biases(&history_records, &graph_signals, pending_event_count);
        let learning_evidence = self
            .memory
            .retrieve_learning_evidence(&self.state_store, session_id, request, 4)
            .await
            .unwrap_or_default();
        let forged_tool_coverage = learning_evidence
            .iter()
            .filter(|item| item.document.asset_kind == LearningAssetKind::ForgedToolManifest)
            .count();
        let skill_success_rate = aggregate_skill_success_rate(
            &self
                .state_store
                .list_skill_library_records(session_id)
                .await
                .unwrap_or_default(),
        );
        let causal_confidence = aggregate_causal_confidence(
            &self
                .state_store
                .list_causal_edge_records(session_id)
                .await
                .unwrap_or_default(),
        );
        let session_ab_stats = self
            .state_store
            .get_knowledge(&format!("metrics:ab:session:{session_id}"))
            .await?
            .and_then(|record| serde_json::from_str::<AbRoutingStats>(&record.value).ok());
        let task_ab_stats = self
            .state_store
            .list_knowledge_by_prefix("metrics:ab:task:")
            .await?
            .into_iter()
            .filter_map(|record| {
                let stats = serde_json::from_str::<AbRoutingStats>(&record.value).ok()?;
                Some((
                    record
                        .key
                        .trim_start_matches("metrics:ab:task:")
                        .to_string(),
                    stats,
                ))
            })
            .collect::<HashMap<_, _>>();
        let tool_ab_stats = self
            .state_store
            .list_knowledge_by_prefix("metrics:ab:tool:")
            .await?
            .into_iter()
            .filter_map(|record| {
                let stats = serde_json::from_str::<AbRoutingStats>(&record.value).ok()?;
                Some((
                    record
                        .key
                        .trim_start_matches("metrics:ab:tool:")
                        .to_string(),
                    stats,
                ))
            })
            .collect::<HashMap<_, _>>();
        let server_ab_stats = self
            .state_store
            .list_knowledge_by_prefix("metrics:ab:server:")
            .await?
            .into_iter()
            .filter_map(|record| {
                let stats = serde_json::from_str::<AbRoutingStats>(&record.value).ok()?;
                Some((
                    record
                        .key
                        .trim_start_matches("metrics:ab:server:")
                        .to_string(),
                    stats,
                ))
            })
            .collect::<HashMap<_, _>>();
        let agent_reputations = aggregate_agent_reputations(&history_records);

        Ok(RoutingContext {
            history_records,
            execution_metrics,
            graph_signals,
            pending_event_count,
            learning_evidence,
            skill_success_rate,
            causal_confidence,
            forged_tool_coverage,
            session_ab_stats,
            task_ab_stats,
            tool_ab_stats,
            server_ab_stats,
            agent_reputations,
            route_biases,
        })
    }

    async fn ceo_summary(
        &self,
        session_id: &str,
        brief: &RequirementBrief,
        routing: &RoutingContext,
    ) -> Result<String> {
        let overlay = self.adaptive_overlay(&brief.clarified_goal, routing);
        let prompt = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".into(),
                content: format!(
                    "You are the CEO agent. Use graph density, historical execution results, pending events, and adaptive policy hints to decide the specialist team and execution order.\n\n{}",
                    overlay_as_text(&overlay)
                ),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".into(),
                content: serde_json::to_string(&(brief, routing))?,
            },
        ];
        let envelope = self.provider_envelope(
            session_id,
            "strategy-summary-stage",
            self.execution_identity_for_session(session_id).await?,
            &prompt,
            self.default_provider_constraints(),
        );
        let reply = self
            .runtime
            .execute(
                &self.state_store,
                &self.tools,
                &self.providers,
                session_id,
                &envelope,
                None,
                overlay.preferred_model.as_deref(),
            )
            .await?
            .provider_response
            .unwrap_or(crate::providers::LlmResponse {
                content: None,
                tool_calls: Vec::new(),
            });
        Ok(reply.content.unwrap_or_else(|| {
            format!(
                "CEO routed with biases {:?}. Dense graph: {}. Pending events: {}. Agent reputation entries: {}.",
                routing.route_biases,
                routing.graph_signals.has_dense_graph,
                routing.pending_event_count,
                routing.agent_reputations.len()
            )
        }))
    }

    async fn optimization_proposal(
        &self,
        brief: &RequirementBrief,
        routing: &RoutingContext,
    ) -> Result<OptimizationProposal> {
        let overlay = self.adaptive_overlay(&brief.clarified_goal, routing);
        let history_summary = routing
            .history_records
            .iter()
            .rev()
            .take(4)
            .map(|record| format!("{}:{}", record.source, record.key))
            .collect::<Vec<_>>()
            .join(", ");
        let signals = vec![
            OptimizationSignal {
                key: "graph_entities".into(),
                value: routing.graph_signals.entity_count.to_string(),
            },
            OptimizationSignal {
                key: "graph_relationships".into(),
                value: routing.graph_signals.relationship_count.to_string(),
            },
            OptimizationSignal {
                key: "pending_events".into(),
                value: routing.pending_event_count.to_string(),
            },
            OptimizationSignal {
                key: "route_biases".into(),
                value: routing.route_biases.join("|"),
            },
            OptimizationSignal {
                key: "learning_hits".into(),
                value: routing.learning_evidence.len().to_string(),
            },
            OptimizationSignal {
                key: "forged_tool_hits".into(),
                value: routing.forged_tool_coverage.to_string(),
            },
            OptimizationSignal {
                key: "adaptive_policy".into(),
                value: overlay.directives.join(" | "),
            },
        ];

        self.providers
            .propose_next_iteration(&brief.clarified_goal, &history_summary, &signals)
            .await
    }

    fn adaptive_overlay(&self, objective: &str, routing: &RoutingContext) -> PromptPolicyOverlay {
        let evolution_summary = routing
            .history_records
            .iter()
            .rev()
            .find(|record| record.key.contains(":self-evolution"))
            .map(|record| compress_text(&record.value, 320));
        let research_summary = routing
            .history_records
            .iter()
            .rev()
            .find(|record| record.key.contains("research:") && record.key.ends_with(":report"))
            .map(|record| compress_text(&record.value, 320));
        let capability_hints = routing
            .learning_evidence
            .iter()
            .filter(|item| item.document.asset_kind == LearningAssetKind::ForgedToolManifest)
            .map(|item| item.document.id.clone())
            .take(4)
            .collect::<Vec<_>>();
        self.providers.derive_prompt_policy(
            objective,
            evolution_summary.as_deref(),
            research_summary.as_deref(),
            &capability_hints,
        )
    }

    fn adaptive_task_pack(
        &self,
        objective: &str,
        routing: &RoutingContext,
    ) -> AgentEvolverTaskPack {
        let evolution_summary = routing
            .history_records
            .iter()
            .rev()
            .find(|record| record.key.contains(":self-evolution"))
            .map(|record| compress_text(&record.value, 320));
        let research_summary = routing
            .history_records
            .iter()
            .rev()
            .find(|record| record.key.contains("research:") && record.key.ends_with(":report"))
            .map(|record| compress_text(&record.value, 320));
        let capability_hints = routing
            .learning_evidence
            .iter()
            .filter(|item| item.document.asset_kind == LearningAssetKind::ForgedToolManifest)
            .map(|item| item.document.id.clone())
            .take(4)
            .collect::<Vec<_>>();
        let bundle = build_prompt_template_bundle(
            Some(PromptTemplateProfile {
                stage: "api-policy-adaptation".into(),
                adaptation_type: format!("prompt-route-tool-policy:{objective}"),
                preferred_surface: "provider-api".into(),
                rollout_budget_ms: 1,
            }),
            evolution_summary.as_deref(),
            research_summary.as_deref(),
            &capability_hints,
        );
        AgentEvolverTaskPack::from_bundle(&bundle)
    }

    fn build_swarm_team(
        &self,
        brief: &RequirementBrief,
        routing: &RoutingContext,
        ceo_summary: &str,
        optimization_proposal: &OptimizationProposal,
    ) -> Vec<SwarmTask> {
        let skill_constraints = learned_skill_constraints(&routing.history_records);
        let mut tasks = vec![SwarmTask {
            task_id: "architecture".into(),
            agent_name: "architecture-agent".into(),
            role: "Architecture".into(),
            objective: format!(
                "Design a Rust + StateStore workflow for: {}. Optimization hypothesis: {}. Learned constraints: {}",
                brief.clarified_goal, optimization_proposal.hypothesis, skill_constraints
            ),
            depends_on: Vec::new(),
        }];

        if routing.graph_signals.needs_more_extraction
            || request_needs_graph_focus(&brief.original_request)
        {
            tasks.push(SwarmTask {
                task_id: "knowledge".into(),
                agent_name: "knowledge-agent".into(),
                role: "GraphRAG".into(),
                objective: format!(
                    "Improve graph coverage. Current entities={} relationships={}. Extract missing entities, relationships, and retrieval summaries. Evaluation focus: {}. Learned constraints: {}.",
                    routing.graph_signals.entity_count,
                    routing.graph_signals.relationship_count,
                    optimization_proposal.evaluation_focus,
                    skill_constraints
                ),
                depends_on: vec!["architecture".into()],
            });
        }

        if request_needs_execution_focus(&brief.original_request)
            || !history_contains_source(&routing.history_records, "swarm-execution")
        {
            tasks.push(SwarmTask {
                task_id: "capability-forge".into(),
                agent_name: "cli-agent".into(),
                role: "CapabilityForge".into(),
                objective: format!(
                    "Inspect graph and memory evidence, then produce or update forged MCP capabilities for: {}. Existing capability hits: {}. Learned constraints: {}",
                    brief.clarified_goal, routing.forged_tool_coverage, skill_constraints
                ),
                depends_on: vec!["architecture".into()],
            });
            tasks.push(SwarmTask {
                task_id: "execution".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: format!(
                    "Select only from the forged MCP capability catalog, execute the plan, and report task-level feedback. If the catalog is insufficient, request one bounded capability forge. Change outline: {}. Learned constraints: {}",
                    optimization_proposal.patch_outline.join(" | "),
                    skill_constraints
                ),
                depends_on: vec!["capability-forge".into()],
            });
        }

        if routing.pending_event_count > 0 || ceo_summary.to_ascii_lowercase().contains("iteration")
        {
            tasks.push(SwarmTask {
                task_id: "operations".into(),
                agent_name: "ops-agent".into(),
                role: "Operations".into(),
                objective: format!(
                    "Drain or reconcile {} pending scheduled events before further expansion.",
                    routing.pending_event_count
                ),
                depends_on: vec!["execution".into()],
            });
        }

        if ceo_summary.to_ascii_lowercase().contains("security")
            || history_contains_security_risk(&routing.history_records)
            || brief
                .original_request
                .to_ascii_lowercase()
                .contains("permission")
        {
            tasks.push(SwarmTask {
                task_id: "security".into(),
                agent_name: "security-agent".into(),
                role: "Security".into(),
                objective:
                    "Review permissions, prompt safety, and execution boundaries before rollout."
                        .into(),
                depends_on: vec!["architecture".into()],
            });
        }

        if routing.graph_signals.has_dense_graph && !routing.history_records.is_empty() {
            tasks.push(SwarmTask {
                task_id: "retrieval".into(),
                agent_name: "retrieval-agent".into(),
                role: "Retrieval".into(),
                objective: "Use the existing graph memory to ground the execution plan and avoid redundant work.".into(),
                depends_on: vec!["knowledge".into(), "architecture".into()],
            });
        }

        tasks
    }

    async fn admit_capability_for_task(
        &self,
        session_id: &str,
        task: &SwarmTask,
        decision: &ToolRoutingDecision,
        identity: &ExecutionIdentity,
    ) -> Result<(Option<String>, Option<String>, String)> {
        let Some(tool_name) = decision.tool_name.as_deref() else {
            return Ok((None, None, "not_applicable".to_string()));
        };

        let engine = CapabilityAdmissionEngine::with_policy_mode(self.runtime.policy_mode());
        let selector = CapabilityIntentSelector::new(self.tools.clone());
        let selector_port: &dyn CapabilityIntentSelectorPort = &selector;
        let intent = CapabilityIntent {
            session_id: session_id.to_string(),
            objective: task.objective.clone(),
            required_tags: vec![task.role.clone()],
            preferred_servers: decision.mcp_server.clone().into_iter().collect::<Vec<_>>(),
        };
        self.record_foundry_router_shadow(session_id, task, &intent, decision)
            .await?;
        let candidates = selector_port
            .select_candidates(&intent)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let selector_allows_tool = candidates.iter().any(|candidate| candidate.tool == tool_name);
        let artifact_write_override = tool_name == "write_file"
            && objective_requires_local_artifact_write(&task.objective)
            && self.tools.has_tool("write_file");
        if !selector_allows_tool && !artifact_write_override {
            return Ok((
                Some("tool not selected by capability intent selector".to_string()),
                None,
                "rejected".to_string(),
            ));
        }
        let preferred_server = candidates
            .iter()
            .find(|candidate| candidate.tool == tool_name)
            .and_then(|candidate| candidate.server.clone());

        let provider_factory_artifacts = self.providers.factory_artifacts();
        let admission = engine
            .admit_selected(
                &self.state_store,
                &self.tools,
                &provider_factory_artifacts,
                session_id,
                &task.task_id,
                identity,
                &intent,
                tool_name,
                preferred_server
                    .as_deref()
                    .or(decision.mcp_server.as_deref()),
            )
            .await?;

        if admission.allowed {
            if let Some(blocked) = self
                .enforce_admission_governance(session_id, task, identity, tool_name)
                .await?
            {
                return Ok((
                    Some(blocked.reason),
                    Some(blocked.evidence_ref),
                    "rejected".to_string(),
                ));
            }
            Ok((None, admission.evidence_ref, "admitted".to_string()))
        } else {
            Ok((
                Some(admission.reason),
                admission.evidence_ref,
                "rejected".to_string(),
            ))
        }
    }
    fn foundry_router_shadow_enabled(&self) -> bool {
        let raw = std::env::var("AUTOLOOP_FOUNDRY_ROUTER_ENABLED")
            .unwrap_or_else(|_| "enabled".to_string())
            .to_ascii_lowercase();
        !matches!(raw.as_str(), "disabled" | "off" | "false" | "0")
    }

    async fn record_foundry_router_shadow(
        &self,
        session_id: &str,
        task: &SwarmTask,
        intent: &CapabilityIntent,
        decision: &ToolRoutingDecision,
    ) -> Result<()> {
        if !self.foundry_router_shadow_enabled() {
            return Ok(());
        }
        let now_ms = current_time_ms();
        let intake = normalize_intake(crate::contracts::skill_foundry::FoundryIntake {
            intake_id: format!("intake:foundry-shadow:{}:{}", task.task_id, now_ms),
            task_name: format!("{}:{}", task.role, task.task_id),
            concrete_examples: vec![task.objective.clone()],
            negative_examples: vec![],
            expected_output: "capability-routing-suggestion".to_string(),
            existing_software: vec![decision
                .tool_name
                .clone()
                .unwrap_or_else(|| "tool:none".to_string())],
            existing_apis: vec![],
            existing_scripts: vec![],
            requested_by: "orchestrator-shadow".to_string(),
            session_id: session_id.to_string(),
            created_at_ms: now_ms,
        });
        let extraction = extract_first_principles(&intake);
        let route = route_layer(&extraction, now_ms);
        let suggestion = promotion_suggestion(&extraction, &route, now_ms);
        let key = format!("foundry-router:shadow:{session_id}:{}:latest", task.task_id);
        let payload = serde_json::json!({
            "session_id": session_id,
            "task_id": task.task_id,
            "shadow": true,
            "feature_flag": "AUTOLOOP_FOUNDRY_ROUTER_ENABLED",
            "intent": intent,
            "current_tool_decision": {
                "tool_name": decision.tool_name,
                "mcp_server": decision.mcp_server,
                "route_variant": decision.route_variant,
            },
            "foundry": {
                "route": route,
                "suggestion": suggestion,
            },
            "created_at_ms": now_ms,
        });
        self.state_store
            .upsert_json_knowledge(key, &payload, "foundry-router-shadow")
            .await?;
        Ok(())
    }


    fn governance_repo_root(&self) -> std::path::PathBuf {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("D:\\AutoLoop\\autoloop-app"))
    }

    fn governance_action_and_sensitivity(&self, tool_name: &str) -> (&'static str, &'static str) {
        let risk = self
            .tools
            .manifests()
            .into_iter()
            .find(|manifest| manifest.registered_tool_name == tool_name)
            .map(|manifest| manifest.risk)
            .unwrap_or(CapabilityRisk::Medium);
        match risk {
            CapabilityRisk::Low => ("read", "low"),
            CapabilityRisk::Medium => ("read", "medium"),
            CapabilityRisk::High => ("write", "high"),
        }
    }

    async fn enforce_admission_governance(
        &self,
        session_id: &str,
        task: &SwarmTask,
        identity: &ExecutionIdentity,
        tool_name: &str,
    ) -> Result<Option<GovernanceBlockSummary>> {
        let (action, sensitivity) = self.governance_action_and_sensitivity(tool_name);
        let trace_id = format!(
            "trace:{session_id}:{}:governance-admission:{}",
            task.task_id,
            current_time_ms()
        );
        let governance = GitmemoryCoreKernel::new()
            .run_phase4_advanced_governance(
                &self.state_store,
                &self.governance_repo_root(),
                session_id,
                &identity.tenant_id,
                &trace_id,
                GovernancePhase::Admission,
                &identity.principal_id,
                action,
                "memory:runtime",
                sensitivity,
            )
            .await?;

        if governance.allowed {
            return Ok(None);
        }

        let block_rule_id = governance
            .rule_id
            .clone()
            .unwrap_or_else(|| "governance:unknown".to_string());
        let evidence_ref = self
            .runtime
            .tag_external_stage(
                &self.state_store,
                session_id,
                &trace_id,
                Some(&task.task_id),
                Some(tool_name),
                crate::runtime::evidence_tagger::EvidenceTagStage::Admission,
                "governance.admission.blocked",
                serde_json::json!({
                    "rule_id": block_rule_id,
                    "policy_version": governance.policy_version,
                    "replay_fp": governance.replay_fp,
                    "summary": governance.summary,
                    "source": "phase4-advanced-governance",
                }),
            )
            .await?;

        let reason = serde_json::json!({
            "code": "governance_admission_blocked",
            "rule_id": governance.rule_id,
            "policy_version": governance.policy_version,
            "evidence_ref": evidence_ref,
            "replay_fp": governance.replay_fp,
            "summary": governance.summary,
        });
        self.state_store
            .upsert_json_knowledge(
                format!(
                    "policy-reject:{session_id}:{}:{}",
                    task.task_id,
                    current_time_ms()
                ),
                &serde_json::json!({
                    "session_id": session_id,
                    "task_id": task.task_id,
                    "trace_id": trace_id,
                    "tenant_id": identity.tenant_id,
                    "principal_id": identity.principal_id,
                    "policy_id": identity.policy_id,
                    "tool_name": tool_name,
                    "rule_id": governance.rule_id,
                    "policy_version": governance.policy_version,
                    "evidence_ref": evidence_ref,
                    "replay_fp": governance.replay_fp,
                    "summary": governance.summary,
                }),
                "phase4-governance",
            )
            .await?;

        Ok(Some(GovernanceBlockSummary {
            reason: reason.to_string(),
            evidence_ref,
        }))
    }
    async fn execute_swarm(
        &self,
        session_id: &str,
        tasks: &[SwarmTask],
        brief: &RequirementBrief,
        routing: &RoutingContext,
    ) -> Result<Vec<ExecutionReport>> {
        let mut reports = Vec::new();
        let execution_identity = self.execution_identity_for_session(session_id).await?;

        let ordered_tasks = topological_tasks(tasks);
        let pool_queues = self.materialize_execution_pools(&ordered_tasks, routing);
        let pool_lookup = pool_queues
            .iter()
            .flat_map(|queue| {
                queue
                    .tasks
                    .iter()
                    .map(|task| (task.task_id.clone(), queue.pool.clone()))
            })
            .collect::<HashMap<_, _>>();
        if ordered_tasks.len() > self.runtime.limits.max_parallel_agents.saturating_mul(4) {
            let trigger = format!("queue-congestion:{}:{}", session_id, current_time_ms());
            let reason = format!(
                "queue congestion detected: task_count={} max_parallel_agents={}",
                ordered_tasks.len(),
                self.runtime.limits.max_parallel_agents
            );
            let _ = self
                .runtime
                .apply_degrade_profile(
                    &self.state_store,
                    session_id,
                    &trigger,
                    DegradeProfileKind::QueueThrottle,
                    &reason,
                )
                .await;
            let _ = self
                .runtime
                .build_recovery_plan(
                    &self.state_store,
                    session_id,
                    &trigger,
                    DegradeProfileKind::QueueThrottle,
                )
                .await;
        }
        let _ = self
            .state_store
            .upsert_json_knowledge(
                format!("execution-pools:{session_id}:{}", current_time_ms()),
                &pool_queues,
                "orchestration",
            )
            .await;

        for task in &ordered_tasks {
            let pool_kind = pool_lookup
                .get(&task.task_id)
                .cloned()
                .unwrap_or(ExecutionPoolKind::Stable);
            let decision = self.select_tool(session_id, task, routing);
            let mut tool_used = decision.tool_name.clone();
            let mut selected_server = decision.mcp_server.clone();
            let mut invocation_payload = decision.invocation_payload.clone();
            let mut guard_decision = "provider".to_string();
            let mut guard_reason = "provider fallback path".to_string();
            let mut admission_status = "not_applicable".to_string();
            let mut admission_reason: Option<String> = None;
            let mut admission_evidence_ref: Option<String> = None;

            if tool_used.is_some() {
                let (rejected_reason, evidence_ref, status) = self
                    .admit_capability_for_task(session_id, task, &decision, &execution_identity)
                    .await?;
                admission_status = status;
                admission_evidence_ref = evidence_ref;
                if let Some(reason) = rejected_reason {
                    admission_reason = Some(reason.clone());
                    guard_reason = reason.clone();
                    guard_decision = format!("capability_rejected:{reason}");
                    tool_used = None;
                    selected_server = None;
                    invocation_payload = None;
                }
            }
            let manifest_for_tool = tool_used.as_deref().and_then(|tool_name| {
                self.tools
                    .manifests()
                    .into_iter()
                    .find(|manifest| manifest.registered_tool_name == tool_name)
            });
            let output = if let Some(tool_name) = &tool_used {
                let arguments = invocation_payload
                    .as_deref()
                    .unwrap_or(task.objective.as_str());
                let envelope = TaskEnvelope {
                    session_id: SessionId::from(session_id),
                    trace_id: TraceId::from(format!(
                        "{}:{}:{}",
                        session_id,
                        task.task_id,
                        current_time_ms()
                    )),
                    task_id: TaskId::from(task.task_id.as_str()),
                    capability_id: CapabilityId::from(tool_name.as_str()),
                    identity: execution_identity.clone(),
                    payload: serde_json::Value::String(arguments.to_string()),
                    constraints: self.default_task_constraints(),
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
                        manifest_for_tool.as_ref(),
                        None,
                    )
                    .await?;
                guard_decision = format!("{:?}", executed.guard_report.decision);
                guard_reason = executed.guard_report.reason.clone();
                format!(
                    "{}\n[routing] {}\n[guard] {}\n[pool] {:?}",
                    executed.content, decision.rationale, executed.guard_report.reason, pool_kind
                )
            } else {
                let delegated = if task.role == "Execution" {
                    if let Some(peer) = self.select_peer_for_delegation(session_id).await? {
                        selected_server = Some(format!("peer::{peer}"));
                        guard_decision = "Delegated".into();
                        guard_reason = format!("peer delegation to {peer}");
                        let delegate_messages = [
                            ChatMessage { tool_call_id: None, tool_calls: None,
                                role: "system".into(),
                                content: format!(
                                    "You are peer delegate {}. Execute the delegated task with bounded reasoning and return actionable output.",
                                    peer
                                ),
                            },
                            ChatMessage { tool_call_id: None, tool_calls: None,
                                role: "user".into(),
                                content: format!(
                                    "Goal: {}\nDelegated Task: {}",
                                    brief.clarified_goal, task.objective
                                ),
                            },
                        ];
                        let delegate_envelope = self.provider_envelope(
                            session_id,
                            &format!("peer-delegate:{}", task.task_id),
                            execution_identity.clone(),
                            &delegate_messages,
                            self.default_provider_constraints(),
                        );
                        let reply = self
                            .runtime
                            .execute(
                                &self.state_store,
                                &self.tools,
                                &self.providers,
                                session_id,
                                &delegate_envelope,
                                None,
                                None,
                            )
                            .await?
                            .provider_response
                            .unwrap_or(crate::providers::LlmResponse {
                                content: None,
                                tool_calls: Vec::new(),
                            });
                        let delegated_output = reply
                            .content
                            .unwrap_or_else(|| format!("peer {} completed delegated task.", peer));
                        let _ = self
                            .state_store
                            .upsert_json_knowledge(
                                format!("peer-delegation:{session_id}:{}:latest", task.task_id),
                                &serde_json::json!({
                                    "session_id": session_id,
                                    "task_id": task.task_id,
                                    "peer": peer,
                                    "delegated": true,
                                    "created_at_ms": current_time_ms(),
                                }),
                                "orchestration",
                            )
                            .await;
                        Some(format!(
                            "{}\n[delegation] peer={}\n[pool] {:?}",
                            delegated_output, peer, pool_kind
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };

                if let Some(peer_output) = delegated {
                    peer_output
                } else {
                    let overlay = self.adaptive_overlay(&task.objective, routing);
                    let task_pack = self.adaptive_task_pack(&task.objective, routing);
                    let provider_messages = [
                        ChatMessage { tool_call_id: None, tool_calls: None,
                            role: "system".into(),
                            content: format!(
                                "You are {}.\n\n{}\n\n{}",
                                task.agent_name,
                                overlay_as_text(&overlay),
                                task_pack_as_text(&task_pack)
                            ),
                        },
                        ChatMessage { tool_call_id: None, tool_calls: None,
                            role: "user".into(),
                            content: format!(
                                "Goal: {}\nTask: {}",
                                brief.clarified_goal, task.objective
                            ),
                        },
                    ];
                    let envelope = self.provider_envelope(
                        session_id,
                        &format!("provider-task:{}", task.task_id),
                        execution_identity.clone(),
                        &provider_messages,
                        self.default_provider_constraints(),
                    );
                    let reply = self
                        .runtime
                        .execute(
                            &self.state_store,
                            &self.tools,
                            &self.providers,
                            session_id,
                            &envelope,
                            None,
                            overlay.preferred_model.as_deref(),
                        )
                        .await?
                        .provider_response
                        .unwrap_or(crate::providers::LlmResponse {
                            content: None,
                            tool_calls: Vec::new(),
                        });
                    format!(
                        "{}\n[pool] {:?}",
                        reply
                            .content
                            .unwrap_or_else(|| format!("{} completed.", task.agent_name)),
                        pool_kind
                    )
                }
            };

            self.sessions
                .append_tool_message(session_id, &task.agent_name, &output)
                .await;
            let outcome_score = execution_outcome_score(&output);

            reports.push(ExecutionReport {
                task: task.clone(),
                output,
                tool_used,
                mcp_server: selected_server,
                invocation_payload,
                outcome_score,
                route_variant: decision.route_variant,
                control_score: decision.control_score,
                treatment_score: decision.treatment_score,
                guard_decision: guard_decision.clone(),
            });
            if let Some(report) = reports.last() {
                self.runtime
                    .record_execution_outcome(&self.state_store, report)
                    .await?;

                let pool_name = match pool_kind {
                    ExecutionPoolKind::Stable => "stable",
                    ExecutionPoolKind::Adaptive => "adaptive",
                };
                let trace = ExecutionFabricTrace {
                    session_id: session_id.to_string(),
                    task_id: task.task_id.clone(),
                    trace_id: format!(
                        "fabric:{}:{}:{}",
                        session_id,
                        task.task_id,
                        current_time_ms()
                    ),
                    pool: pool_name.to_string(),
                    route_variant: report.route_variant.clone(),
                    tool_name: report.tool_used.clone(),
                    mcp_server: report.mcp_server.clone(),
                    admission_status: admission_status.clone(),
                    admission_reason: admission_reason.clone(),
                    admission_evidence_ref: admission_evidence_ref.clone(),
                    guard_decision: guard_decision.clone(),
                    guard_reason: guard_reason.clone(),
                    outcome_score: report.outcome_score,
                    created_at_ms: current_time_ms(),
                };
                let _ = persist_execution_fabric_trace(&self.state_store, &trace).await;
            }
        }

        Ok(reports)
    }

    fn default_task_constraints(&self) -> ConstraintSet {
        ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: self.runtime.limits.max_memory_mb,
            timeout_ms: 120_000,
            max_retries: 2,
            max_tokens: 16_000,
            io_allow_paths: vec![".".into()],
            io_deny_paths: vec!["/etc".into(), "/root".into()],
            sandbox_profile: "standard".into(),
            requires_human_approval: false,
        }
    }

    fn default_provider_constraints(&self) -> ConstraintSet {
        ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: self.runtime.limits.max_memory_mb,
            timeout_ms: 120_000,
            max_retries: 1,
            max_tokens: 8_000,
            io_allow_paths: vec![".".into()],
            io_deny_paths: vec!["/etc".into(), "/root".into()],
            sandbox_profile: "provider".into(),
            requires_human_approval: false,
        }
    }

    fn materialize_execution_pools(
        &self,
        ordered_tasks: &[SwarmTask],
        routing: &RoutingContext,
    ) -> Vec<ExecutionPoolQueue> {
        let mut by_pool = BTreeMap::<String, Vec<SwarmTask>>::new();
        for task in ordered_tasks {
            let pool = self.classify_execution_pool(task, routing);
            let key = match pool {
                ExecutionPoolKind::Stable => "stable",
                ExecutionPoolKind::Adaptive => "adaptive",
            };
            by_pool.entry(key.into()).or_default().push(task.clone());
        }
        let stable = ExecutionPoolQueue {
            pool: ExecutionPoolKind::Stable,
            tasks: by_pool.remove("stable").unwrap_or_default(),
        };
        let adaptive = ExecutionPoolQueue {
            pool: ExecutionPoolKind::Adaptive,
            tasks: by_pool.remove("adaptive").unwrap_or_default(),
        };
        vec![stable, adaptive]
    }

    fn classify_execution_pool(
        &self,
        task: &SwarmTask,
        routing: &RoutingContext,
    ) -> ExecutionPoolKind {
        let role = task.role.to_ascii_lowercase();
        let objective = task.objective.to_ascii_lowercase();
        let adaptive_hint = routing
            .route_biases
            .iter()
            .any(|bias| bias.to_ascii_lowercase().contains("adaptive"));
        let is_adaptive_role = role.contains("research")
            || role.contains("retrieval")
            || role.contains("capabilityforge")
            || role.contains("learning")
            || role.contains("optimizer");
        let is_adaptive_objective = objective.contains("explore")
            || objective.contains("learn")
            || objective.contains("optimiz")
            || objective.contains("evolv")
            || objective.contains("discover");
        if is_adaptive_role || is_adaptive_objective || adaptive_hint {
            ExecutionPoolKind::Adaptive
        } else {
            ExecutionPoolKind::Stable
        }
    }

    async fn execution_identity_for_session(&self, session_id: &str) -> Result<ExecutionIdentity> {
        let identity = self
            .sessions
            .identity(session_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("missing session identity for {session_id}"))?;
        Ok(ExecutionIdentity {
            tenant_id: identity.tenant_id,
            principal_id: identity.principal_id,
            policy_id: identity.policy_id,
            lease_token: identity.lease_token,
        })
    }

    async fn select_peer_for_delegation(&self, session_id: &str) -> Result<Option<String>> {
        let key = format!("workspace-context:{session_id}:latest");
        let Some(record) = self.state_store.get_knowledge(&key).await? else {
            return Ok(None);
        };
        let snapshot: AgentWorkspaceSnapshot = match serde_json::from_str(&record.value) {
            Ok(item) => item,
            Err(_) => return Ok(None),
        };
        Ok(snapshot
            .peers
            .into_iter()
            .find(|peer| !peer.trim().is_empty()))
    }

    fn provider_envelope(
        &self,
        session_id: &str,
        task_key: &str,
        identity: ExecutionIdentity,
        messages: &[ChatMessage],
        constraints: ConstraintSet,
    ) -> TaskEnvelope {
        TaskEnvelope {
            session_id: SessionId::from(session_id),
            trace_id: TraceId::from(format!("{}:{}:{}", session_id, task_key, current_time_ms())),
            task_id: TaskId::from(task_key),
            capability_id: CapabilityId::from("provider:default"),
            identity,
            payload: serde_json::to_value(messages).unwrap_or_else(|_| serde_json::json!([])),
            constraints,
            trust_plan: None,
        }
    }

    fn validate_outcome(
        &self,
        brief: &RequirementBrief,
        routing: &RoutingContext,
        reports: &[ExecutionReport],
        verifier_report: &VerifierReport,
    ) -> ValidationReport {
        let combined = reports
            .iter()
            .map(|report| report.output.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        let missing_criteria = brief
            .acceptance_criteria
            .iter()
            .filter(|criterion| {
                !combined
                    .to_ascii_lowercase()
                    .contains(&criterion.to_ascii_lowercase())
            })
            .cloned()
            .collect::<Vec<_>>();

        if missing_criteria.is_empty()
            && routing.pending_event_count == 0
            && verifier_report.verdict == VerifierVerdict::Pass
        {
            return ValidationReport {
                ready: true,
                summary:
                    "All acceptance criteria have matching evidence in the swarm execution outputs."
                        .into(),
                follow_up_tasks: Vec::new(),
                verifier_summary: verifier_report.summary.clone(),
            };
        }

        let mut follow_up_tasks = missing_criteria
            .into_iter()
            .map(|criterion| SwarmTask {
                task_id: format!(
                    "validation-criterion-{}",
                    sanitize_task_id_fragment(&criterion)
                ),
                agent_name: "iteration-agent".into(),
                role: "Validation".into(),
                objective: format!(
                    "Produce evidence or implementation detail for acceptance criterion: {criterion}"
                ),
                depends_on: vec!["execution".into()],
            })
            .collect::<Vec<_>>();

        if routing.pending_event_count > 0 {
            follow_up_tasks.push(SwarmTask {
                task_id: "validation-ops".into(),
                agent_name: "ops-agent".into(),
                role: "Operations".into(),
                objective: format!(
                    "Resolve {} pending scheduled events before marking the objective complete.",
                    routing.pending_event_count
                ),
                depends_on: vec!["execution".into()],
            });
        }
        if verifier_report.verdict != VerifierVerdict::Pass {
            follow_up_tasks.push(SwarmTask {
                task_id: "validation-verifier".into(),
                agent_name: "verifier-agent".into(),
                role: "Verification".into(),
                objective: verifier_report
                    .recommended_actions
                    .first()
                    .cloned()
                    .unwrap_or_else(|| {
                        "Re-run verifier checks and resolve catalog/regression issues.".into()
                    }),
                depends_on: vec!["execution".into()],
            });
        }

        ValidationReport {
            ready: false,
            summary: if routing.pending_event_count > 0 {
                format!(
                    "Global validation found pending work and missing coverage. Pending events: {}.",
                    routing.pending_event_count
                )
            } else {
                "Global validation found missing acceptance coverage or verifier findings.".into()
            },
            follow_up_tasks,
            verifier_summary: verifier_report.summary.clone(),
        }
    }

    fn select_tool(
        &self,
        session_id: &str,
        task: &SwarmTask,
        routing: &RoutingContext,
    ) -> ToolRoutingDecision {
        if task.role == "GraphRAG" && self.tools.has_tool("write_file") {
            let overlay = self.adaptive_overlay(&task.objective, routing);
            return ToolRoutingDecision {
                tool_name: Some("write_file".into()),
                mcp_server: None,
                invocation_payload: Some(build_local_payload(task, "write_file", &overlay)),
                score: 100,
                control_score: 100,
                treatment_score: 100,
                route_variant: "fixed".into(),
                rationale:
                    "GraphRAG tasks prefer deterministic local persistence for extracted artifacts."
                        .into(),
            };
        }

        if task.role == "CapabilityForge" {
            return ToolRoutingDecision {
                tool_name: Some("cli::forge_mcp_tool".into()),
                mcp_server: None,
                invocation_payload: Some(build_cli_forge_payload(
                    task,
                    &routing.route_biases,
                    &self.adaptive_overlay(&task.objective, routing),
                )),
                score: 120,
                control_score: 120,
                treatment_score: 120,
                route_variant: "fixed".into(),
                rationale:
                    "cli-agent is dedicated to maintaining the forged MCP capability catalog."
                        .into(),
            };
        }

        if task.role != "Execution" && task.role != "Operations" {
            return ToolRoutingDecision {
                tool_name: None,
                mcp_server: None,
                invocation_payload: None,
                score: 0,
                control_score: 0,
                treatment_score: 0,
                route_variant: "provider".into(),
                rationale: "This specialist task is better served by provider reasoning than a direct tool call.".into(),
            };
        }

        let mut best = ToolRoutingDecision {
            tool_name: None,
            mcp_server: None,
            invocation_payload: None,
            score: i32::MIN,
            control_score: i32::MIN,
            treatment_score: i32::MIN,
            route_variant: "control".into(),
            rationale: "No suitable tool candidates were available.".into(),
        };
        let catalog_tools = self.tools.forged_tool_names();
        let catalog_is_empty = catalog_tools.is_empty();

        for tool_name in self.tools.names() {
            let artifact_objective = objective_requires_local_artifact_write(&task.objective);
            if task.role == "Execution"
                && tool_name != "cli::forge_mcp_tool"
                && !catalog_tools
                    .iter()
                    .any(|catalog_name| catalog_name == &tool_name)
            {
                let artifact_local_write_fallback = tool_name == "write_file" && artifact_objective;
                if !artifact_local_write_fallback {
                    continue;
                }
            }
            let mut control_score = 0i32;
            let mut reasons = Vec::new();
            let candidate_server = parse_mcp_server(&tool_name);

            if tool_name == "cli::forge_mcp_tool" {
                control_score += 5;
                reasons.push(
                    "CLI-Anything style forge tool can synthesize reusable MCP wrappers"
                        .to_string(),
                );
                if artifact_objective {
                    control_score -= 25;
                    reasons.push(
                        "artifact objective prefers deterministic local write_file over forge path"
                            .to_string(),
                    );
                }
                if task.role == "Execution" && !catalog_is_empty {
                    control_score -= 20;
                    reasons.push(
                        "execution-agent must prefer catalog tools before forging new capability"
                            .to_string(),
                    );
                }
                if request_needs_cli_tooling(&task.objective) || catalog_is_empty {
                    control_score += 10;
                    reasons.push(
                        "task objective explicitly asks for a custom CLI/MCP tool surface"
                            .to_string(),
                    );
                }
                if routing.forged_tool_coverage > 0 && objective_prefers_reuse(&task.objective) {
                    control_score -= 12;
                    reasons.push(
                        "existing forged capability evidence suggests reusing a prior MCP wrapper"
                            .to_string(),
                    );
                }
                if routing.graph_signals.prefers_cli_execution {
                    control_score += 4;
                    reasons.push("graph entities point toward CLI-first execution".to_string());
                }
            } else if tool_name.starts_with("mcp::") {
                control_score += 4;
                reasons.push("available MCP endpoint".to_string());
                if task.role == "Execution" {
                    control_score += 6;
                    reasons.push(
                        "execution-agent is restricted to forged capability catalog tools"
                            .to_string(),
                    );
                }
                if routing.graph_signals.prefers_mcp_execution {
                    control_score += 5;
                    reasons.push("graph entities point toward MCP/server execution".to_string());
                }
                let forged_bonus = forged_manifest_bonus(
                    &routing.learning_evidence,
                    &tool_name,
                    candidate_server.as_deref(),
                );
                if forged_bonus > 0 {
                    control_score += forged_bonus;
                    reasons.push(format!(
                        "retrieved forged MCP capability evidence +{forged_bonus}"
                    ));
                }
            } else if tool_name == "read_file" || tool_name == "write_file" || tool_name == "shell"
            {
                control_score += 2;
                reasons.push("local CLI-capable tool".to_string());
                if tool_name == "write_file" && artifact_objective {
                    control_score += 35;
                    reasons.push(
                        "artifact objective hard-boosts deterministic local write_file path"
                            .to_string(),
                    );
                }
                if routing.graph_signals.prefers_cli_execution {
                    control_score += 5;
                    reasons.push("graph entities point toward CLI/file execution".to_string());
                }
            }

            if task.role == "Operations" && tool_name.starts_with("mcp::") {
                control_score += 3;
                reasons.push("ops tasks benefit from scheduled MCP execution".to_string());
            }

            if let Some(metric) = metric_for_tool(&routing.execution_metrics, &tool_name) {
                let effective_score = metric.effective_score.round() as i32;
                let success_bonus = (metric.effective_success_rate * 10.0).round() as i32;
                control_score += effective_score + success_bonus;
                reasons.push(format!(
                    "effective score {} and effective success rate {:.2}",
                    metric.effective_score, metric.effective_success_rate
                ));
            } else {
                let fallback_score = fallback_history_score(&routing.history_records, &tool_name);
                if fallback_score != 0 {
                    control_score += fallback_score;
                    reasons.push(format!("fallback history score {fallback_score}"));
                }
            }

            let scorer = WeightedLearningScorer;
            let tool_success_rate = metric_for_tool(&routing.execution_metrics, &tool_name)
                .map(|metric| metric.effective_success_rate)
                .unwrap_or(0.0);
            let learning_score = scorer.score_route(&JointRoutingEvidence {
                retrieved: routing.learning_evidence.clone(),
                skill_success_rate: routing.skill_success_rate,
                causal_confidence: routing.causal_confidence,
                tool_success_rate,
            });
            let learning_bonus = (learning_score * 10.0).round() as i32;
            if learning_bonus != 0 {
                reasons.push(format!(
                    "learning score {:.2} from retrieved evidence/skills/causal traces",
                    learning_score
                ));
            }

            if task.objective.to_ascii_lowercase().contains("mcp") && tool_name.starts_with("mcp::")
            {
                control_score += 4;
                reasons.push("task objective explicitly mentions MCP".to_string());
            }
            if task.objective.to_ascii_lowercase().contains("file")
                && (tool_name == "read_file" || tool_name == "write_file")
            {
                control_score += 3;
                reasons.push("task objective explicitly mentions files".to_string());
            }
            if request_needs_cli_tooling(&task.objective) && tool_name == "cli::forge_mcp_tool" {
                control_score += 6;
                reasons.push("custom CLI tooling request boosts the forge path".to_string());
            }

            let treatment_score = control_score + learning_bonus;
            let effective_ratio = blended_gray_ratio(
                self.gray_routing_ratio,
                routing.session_ab_stats.as_ref(),
                routing.task_ab_stats.get(&task.role),
                routing.tool_ab_stats.get(&tool_name),
                candidate_server
                    .as_deref()
                    .and_then(|server| routing.server_ab_stats.get(server)),
            );
            let in_treatment = routing_bucket(session_id, &task.objective) < effective_ratio;
            let use_treatment = in_treatment
                && (treatment_score as f32)
                    >= (control_score as f32 + self.routing_takeover_threshold);
            let final_score = if use_treatment {
                treatment_score
            } else {
                control_score
            };
            let route_variant = if use_treatment {
                "treatment"
            } else if in_treatment {
                "control_fallback"
            } else {
                "control"
            };
            reasons.push(format!(
                "ab route={route_variant} control={control_score} treatment={treatment_score} ratio={effective_ratio:.2}"
            ));

            if final_score > best.score {
                let mcp_server = candidate_server.clone();
                let overlay = self.adaptive_overlay(&task.objective, routing);
                let invocation_payload = if tool_name == "cli::forge_mcp_tool" {
                    Some(build_cli_forge_payload(
                        task,
                        &routing.route_biases,
                        &overlay,
                    ))
                } else if let Some(server) = mcp_server.as_deref() {
                    Some(build_mcp_payload(
                        task,
                        server,
                        &routing.route_biases,
                        &overlay,
                    ))
                } else {
                    Some(build_local_payload(task, &tool_name, &overlay))
                };
                best = ToolRoutingDecision {
                    tool_name: Some(tool_name),
                    mcp_server,
                    invocation_payload,
                    score: final_score,
                    control_score,
                    treatment_score,
                    route_variant: route_variant.into(),
                    rationale: reasons.join("; "),
                };
            }
        }

        if best.score <= 0 {
            ToolRoutingDecision {
                tool_name: None,
                mcp_server: None,
                invocation_payload: None,
                score: best.score,
                control_score: best.control_score,
                treatment_score: best.treatment_score,
                route_variant: best.route_variant,
                rationale:
                    "No tool earned a positive routing score, so provider reasoning stays in control."
                        .into(),
            }
        } else {
            best
        }
    }
}

fn build_cli_forge_payload(
    task: &SwarmTask,
    route_biases: &[String],
    overlay: &PromptPolicyOverlay,
) -> String {
    let capability_name = infer_capability_name(task);
    serde_json::json!({
        "server": "local-mcp",
        "capability_name": capability_name,
        "purpose": task.objective,
        "executable": "autoloop-cli",
        "subcommands": ["task", sanitize_task_role(&task.role)],
        "json_flag": "--json",
        "arguments": [
            {
                "name": "objective",
                "description": "primary objective to execute through the forged MCP surface",
                "required": true,
                "example": task.objective,
            },
            {
                "name": "session",
                "description": "session identifier",
                "required": false,
                "example": task.agent_name,
            },
            {
                "name": "biases",
                "description": "routing biases that should shape the generated wrapper",
                "required": false,
                "example": route_biases.join(","),
            },
            {
                "name": "adaptive_policy",
                "description": "self-evolution and research hints that should influence the forged wrapper",
                "required": false,
                "example": overlay.directives.join(" | "),
            }
        ],
        "output_mode": "json",
        "success_signal": "completed",
        "working_directory": ".",
        "tags": route_biases,
        "examples": [
            format!("mcp::local-mcp::{} --objective \"{}\"", infer_capability_name(task), task.objective),
            "Prefer deterministic JSON output for downstream agents".to_string(),
            format!("Adaptive hints: {}", overlay.rationale),
        ]
    })
    .to_string()
}

fn infer_open_questions(request: &str) -> Vec<String> {
    let mut questions = Vec::new();
    let lowered = request.to_ascii_lowercase();
    if !lowered.contains("mcp") {
        questions.push(
            "Which MCP servers or custom MCP endpoints should the execution layer prefer?".into(),
        );
    }
    if !lowered.contains("success") && !lowered.contains("acceptance") {
        questions.push("What exact success criteria should the validation layer enforce?".into());
    }
    if !lowered.contains("schedule") && !lowered.contains("cron") {
        questions.push("Which work should run synchronously and which should be scheduled as background events?".into());
    }
    questions
}

fn infer_requirement_answer(question: &str, request: &str) -> String {
    let lowered = request.to_ascii_lowercase();
    if question.to_ascii_lowercase().contains("mcp") {
        if lowered.contains("mcp") {
            "Prefer existing forged MCP capabilities first, then local-mcp forge fallback.".into()
        } else {
            "No MCP preference was stated; default to existing catalog, then forge if capability coverage is missing.".into()
        }
    } else if question.to_ascii_lowercase().contains("success criteria") {
        "Freeze acceptance around graph memory coverage, capability-governed execution, and verifier-approved completion.".into()
    } else if question.to_ascii_lowercase().contains("scheduled") {
        "Run requirement/swarm execution synchronously and send retries, learning, and reconciliation into scheduled events.".into()
    } else {
        format!(
            "Infer a bounded default from request: {}",
            compress_text(request, 96)
        )
    }
}

fn requires_user_confirmation(question: &str, request: &str) -> bool {
    let lowered = request.to_ascii_lowercase();
    (question.to_ascii_lowercase().contains("success criteria")
        && !lowered.contains("acceptance")
        && !lowered.contains("success"))
        || (question.to_ascii_lowercase().contains("mcp") && !lowered.contains("mcp"))
}

fn infer_frozen_scope(request: &str) -> String {
    let lowered = request.to_ascii_lowercase();
    let mut scopes = vec!["bounded single-session delivery".to_string()];
    if lowered.contains("swarm") {
        scopes.push("multi-agent coordination".into());
    }
    if lowered.contains("graph") || lowered.contains("rag") {
        scopes.push("graph memory persistence".into());
    }
    if lowered.contains("mcp") || lowered.contains("tool") {
        scopes.push("capability-catalog execution".into());
    }
    scopes.join(", ")
}

fn learned_skill_constraints(history_records: &[KnowledgeRecord]) -> String {
    let constraints = history_records
        .iter()
        .filter(|record| record.key.contains(":skill:") || record.key.contains(":consolidation"))
        .filter_map(|record| {
            let value = record.value.to_ascii_lowercase();
            if value.contains("rollback") {
                Some("prefer rollback-safe bounded changes".to_string())
            } else if value.contains("official") || value.contains("authority") {
                Some("prefer official evidence and first-party sources".to_string())
            } else if value.contains("catalog") || value.contains("capability") {
                Some("prefer verified capability reuse before new forge".to_string())
            } else {
                None
            }
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(3)
        .collect::<Vec<_>>();

    if constraints.is_empty() {
        "no explicit learned constraints".into()
    } else {
        constraints.join("; ")
    }
}

fn sanitize_task_id_fragment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();

    let collapsed = sanitized
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");

    if collapsed.is_empty() {
        "follow-up".into()
    } else {
        collapsed
    }
}

fn summarize_agent_reputations(agent_reputations: &HashMap<String, AgentReputation>) -> String {
    let mut reputations = agent_reputations.values().cloned().collect::<Vec<_>>();
    reputations.sort_by(|left, right| {
        right
            .trust
            .partial_cmp(&left.trust)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let summary = reputations
        .into_iter()
        .take(4)
        .map(|item| {
            format!(
                "{} trust={:.2} avg={:.1} verifier={:.2} runs={}",
                item.agent_name, item.trust, item.average_score, item.verifier_alignment, item.runs
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    if summary.is_empty() {
        "no_agent_reputation_history".into()
    } else {
        summary
    }
}

fn summarize_verifier_signals(history_records: &[KnowledgeRecord]) -> String {
    history_records
        .iter()
        .rev()
        .find(|record| record.key.contains(":verifier-report"))
        .map(|record| compress_text(&record.value, 220))
        .unwrap_or_else(|| "no_recent_verifier_report".into())
}

fn summarize_ops_signals(routing: &RoutingContext) -> String {
    format!(
        "pending_events={} biases={} forged_coverage={} session_ab={}",
        routing.pending_event_count,
        routing.route_biases.join("|"),
        routing.forged_tool_coverage,
        routing
            .session_ab_stats
            .as_ref()
            .map(|stats| format!(
                "win_rate={:.2},lift={:.2}",
                stats.effective_win_rate, stats.effective_lift
            ))
            .unwrap_or_else(|| "none".into())
    )
}

fn summarize_execution_metrics(metrics: &[ExecutionStats]) -> String {
    let mut metrics = metrics.to_vec();
    metrics.sort_by(|left, right| {
        right
            .effective_score
            .partial_cmp(&left.effective_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let summary = metrics
        .into_iter()
        .take(4)
        .map(|item| {
            format!(
                "{} rate={:.2} score={:.2}",
                item.tool_name, item.effective_success_rate, item.effective_score
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    if summary.is_empty() {
        "no_execution_metrics".into()
    } else {
        summary
    }
}

fn build_consensus_signals(
    routing: &RoutingContext,
    verifier_signal_summary: &str,
    ops_signal_summary: &str,
) -> Vec<String> {
    let mut signals = vec![
        format!("graph_entities={}", routing.graph_signals.entity_count),
        format!(
            "graph_relationships={}",
            routing.graph_signals.relationship_count
        ),
        format!("ops={ops_signal_summary}"),
        format!("verifier={verifier_signal_summary}"),
    ];
    if routing.graph_signals.prefers_mcp_execution {
        signals.push("graph_prefers_mcp".into());
    }
    if routing.graph_signals.prefers_cli_execution {
        signals.push("graph_prefers_cli".into());
    }
    signals
}

fn aggregate_agent_reputations(
    history_records: &[KnowledgeRecord],
) -> HashMap<String, AgentReputation> {
    let mut buckets = HashMap::<String, Vec<ExecutionReport>>::new();
    for record in history_records {
        if !record.key.ends_with(":swarm") {
            continue;
        }
        if let Ok(reports) = serde_json::from_str::<Vec<ExecutionReport>>(&record.value) {
            for report in reports {
                buckets
                    .entry(report.task.agent_name.clone())
                    .or_default()
                    .push(report);
            }
        }
    }

    buckets
        .into_iter()
        .map(|(agent_name, reports)| {
            let runs = reports.len() as u32;
            let average_score = if reports.is_empty() {
                0.0
            } else {
                reports
                    .iter()
                    .map(|report| report.outcome_score as f32)
                    .sum::<f32>()
                    / reports.len() as f32
            };
            let verifier_alignment = if reports.is_empty() {
                0.0
            } else {
                reports
                    .iter()
                    .filter(|report| {
                        report.outcome_score > 0
                            && !report.guard_decision.eq_ignore_ascii_case("blocked")
                    })
                    .count() as f32
                    / reports.len() as f32
            };
            let trust =
                ((average_score.max(0.0) / 5.0) * 0.6 + verifier_alignment * 0.4).clamp(0.0, 1.0);

            (
                agent_name.clone(),
                AgentReputation {
                    agent_name,
                    runs,
                    average_score,
                    verifier_alignment,
                    trust,
                },
            )
        })
        .collect()
}

fn topological_tasks(tasks: &[SwarmTask]) -> Vec<SwarmTask> {
    let mut remaining = tasks.to_vec();
    let mut ordered = Vec::new();
    let mut satisfied = std::collections::BTreeSet::<String>::new();

    while !remaining.is_empty() {
        let mut progressed = false;
        let mut index = 0usize;
        while index < remaining.len() {
            let ready = remaining[index]
                .depends_on
                .iter()
                .all(|dependency| satisfied.contains(dependency));
            if ready {
                let task = remaining.remove(index);
                satisfied.insert(task.task_id.clone());
                ordered.push(task);
                progressed = true;
            } else {
                index += 1;
            }
        }

        if !progressed {
            ordered.extend(remaining.into_iter());
            break;
        }
    }

    ordered
}

fn compress_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        text.to_string()
    } else {
        format!("{}...", text.chars().take(max_len).collect::<String>())
    }
}

fn overlay_as_text(overlay: &PromptPolicyOverlay) -> String {
    if overlay.directives.is_empty() && overlay.rationale.trim().is_empty() {
        return "Adaptive policy: none.".into();
    }

    let directives = if overlay.directives.is_empty() {
        "no explicit directives".into()
    } else {
        overlay.directives.join(" | ")
    };
    let rationale = if overlay.rationale.trim().is_empty() {
        "no rationale recorded".into()
    } else {
        compress_text(&overlay.rationale, 240)
    };
    format!("Adaptive policy directives: {directives}\nAdaptive policy rationale: {rationale}")
}

fn task_pack_as_text(task_pack: &AgentEvolverTaskPack) -> String {
    let mut sections = Vec::new();
    if !task_pack.routing_prompt.is_empty() {
        sections.push(format!(
            "# Routing Template\n{}",
            task_pack.routing_prompt.join("\n")
        ));
    }
    if !task_pack.tool_prompt.is_empty() {
        sections.push(format!(
            "# Tool Template\n{}",
            task_pack.tool_prompt.join("\n")
        ));
    }
    if !task_pack.forge_prompt.is_empty() {
        sections.push(format!(
            "# Forge Template\n{}",
            task_pack.forge_prompt.join("\n")
        ));
    }

    if sections.is_empty() {
        "No task templates.".into()
    } else {
        sections.join("\n\n")
    }
}

fn infer_acceptance_criteria(request: &str) -> Vec<String> {
    let mut criteria = vec!["state_store".into(), "graph".into(), "agent".into()];
    let lowered = request.to_ascii_lowercase();
    if lowered.contains("mcp") {
        criteria.push("mcp".into());
    }
    if lowered.contains("swarm") {
        criteria.push("swarm".into());
    }
    criteria
}

fn infer_route_biases(
    history_records: &[KnowledgeRecord],
    graph_signals: &GraphRoutingSignals,
    pending_event_count: usize,
) -> Vec<String> {
    let mut biases = Vec::new();
    if history_contains_source(history_records, "graph-rag") || graph_signals.has_dense_graph {
        biases.push("reuse_graph_memory".into());
    }
    if history_contains_source(history_records, "autonomous-research") {
        biases.push("reuse_web_findings".into());
    }
    if graph_signals.needs_more_extraction {
        biases.push("expand_graph_extraction".into());
    }
    if !history_contains_source(history_records, "swarm-execution") {
        biases.push("favor_execution_agent".into());
    }
    if pending_event_count > 0 {
        biases.push("drain_pending_events".into());
    }
    if history_contains_security_risk(history_records) {
        biases.push("security_review".into());
    }
    biases
}

fn request_needs_graph_focus(request: &str) -> bool {
    let lowered = request.to_ascii_lowercase();
    lowered.contains("graph") || lowered.contains("knowledge") || lowered.contains("rag")
}

fn request_needs_execution_focus(request: &str) -> bool {
    let lowered = request.to_ascii_lowercase();
    lowered.contains("mcp") || lowered.contains("cli") || lowered.contains("execute")
}

fn request_needs_cli_tooling(request: &str) -> bool {
    let lowered = request.to_ascii_lowercase();
    lowered.contains("forge")
        || lowered.contains("custom tool")
        || lowered.contains("tooling")
        || lowered.contains("build a cli")
        || lowered.contains("build cli")
        || lowered.contains("custom mcp")
        || lowered.contains("agent-native")
}

fn objective_prefers_reuse(request: &str) -> bool {
    let lowered = request.to_ascii_lowercase();
    lowered.contains("existing")
        || lowered.contains("reuse")
        || lowered.contains("already forged")
        || lowered.contains("existing forged")
}

fn objective_requires_local_artifact_write(request: &str) -> bool {
    let lowered = request.to_ascii_lowercase();
    let mentions_write_action = lowered.contains("write_file")
        || lowered.contains("write file")
        || lowered.contains("write to")
        || lowered.contains("save ")
        || lowered.contains("persist")
        || request.contains("写入")
        || request.contains("落盘")
        || request.contains("生成文件");
    let mentions_file_target = lowered.contains(".html")
        || lowered.contains(".htm")
        || lowered.contains(".txt")
        || lowered.contains(".md")
        || lowered.contains(".json")
        || lowered.contains("output\\")
        || lowered.contains("d:\\")
        || lowered.contains("d:/")
        || lowered.contains("file");
    mentions_write_action && mentions_file_target
}

fn infer_capability_name(task: &SwarmTask) -> String {
    let mut tokens = task
        .objective
        .split_whitespace()
        .filter_map(|token| {
            let token = token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                .to_ascii_lowercase();
            if token.len() >= 3 && token != "mcp" && token != "cli" && token != "tool" {
                Some(token)
            } else {
                None
            }
        })
        .take(3)
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        tokens.push(task.role.to_ascii_lowercase());
    }
    tokens.join("-")
}

fn sanitize_task_role(role: &str) -> String {
    role.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn history_contains_source(history_records: &[KnowledgeRecord], source: &str) -> bool {
    history_records.iter().any(|record| record.source == source)
}

fn history_contains_security_risk(history_records: &[KnowledgeRecord]) -> bool {
    history_records.iter().any(|record| {
        record.source == "security-agent"
            || record.value.to_ascii_lowercase().contains("permission")
            || record.value.to_ascii_lowercase().contains("credential")
    })
}

fn aggregate_skill_success_rate(
    skills: &[autoloop_state_adapter::SkillLibraryRecord],
) -> f32 {
    if skills.is_empty() {
        return 0.0;
    }
    skills.iter().map(|skill| skill.success_rate).sum::<f32>() / skills.len() as f32
}

fn aggregate_causal_confidence(edges: &[autoloop_state_adapter::CausalEdgeRecord]) -> f32 {
    if edges.is_empty() {
        return 0.0;
    }
    edges.iter().map(|edge| edge.confidence).sum::<f32>() / edges.len() as f32
}

fn collect_execution_history(history_records: &[KnowledgeRecord]) -> Vec<ExecutionReport> {
    history_records
        .iter()
        .filter(|record| record.source == "swarm-execution")
        .flat_map(|record| {
            serde_json::from_str::<Vec<ExecutionReport>>(&record.value).unwrap_or_default()
        })
        .collect()
}

fn metric_for_tool<'a>(
    metrics: &'a [ExecutionStats],
    tool_name: &str,
) -> Option<&'a ExecutionStats> {
    let server = parse_mcp_server(tool_name);
    metrics.iter().find(|metric| {
        if metric.tool_name == tool_name {
            return true;
        }
        if let Some(server) = server.as_deref() {
            return metric.mcp_server.as_deref() == Some(server);
        }
        false
    })
}

fn fallback_history_score(history_records: &[KnowledgeRecord], tool_name: &str) -> i32 {
    let history_reports = collect_execution_history(history_records);
    let feedback_records = history_records
        .iter()
        .filter(|record| record.source == "execution-feedback")
        .collect::<Vec<_>>();

    history_reports
        .iter()
        .filter(|report| report.tool_used.as_deref() == Some(tool_name))
        .map(|report| execution_outcome_score(&report.output))
        .sum::<i32>()
        + feedback_records
            .iter()
            .filter(|record| record.value.contains(tool_name))
            .map(|record| execution_outcome_score(&record.value))
            .sum::<i32>()
}

fn forged_manifest_bonus(
    evidence: &[RetrievalEvidence],
    tool_name: &str,
    server: Option<&str>,
) -> i32 {
    evidence
        .iter()
        .filter(|item| item.document.asset_kind == LearningAssetKind::ForgedToolManifest)
        .find_map(|item| {
            let tool_match = item
                .document
                .metadata
                .get("tool_name")
                .is_some_and(|name| name == tool_name);
            let server_match = server.is_some_and(|server_name| {
                item.document
                    .metadata
                    .get("server")
                    .is_some_and(|name| name == server_name)
            });
            if tool_match {
                Some(8)
            } else if server_match {
                Some(4)
            } else {
                None
            }
        })
        .unwrap_or(0)
}

fn execution_outcome_score(text: &str) -> i32 {
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("failed") || lowered.contains("error") || lowered.contains("blocked") {
        -6
    } else if lowered.contains("completed")
        || lowered.contains("success")
        || lowered.contains("ready")
    {
        4
    } else {
        1
    }
}

pub fn parse_mcp_server(tool_name: &str) -> Option<String> {
    let parts = tool_name.split("::").collect::<Vec<_>>();
    if parts.len() >= 3 && parts.first() == Some(&"mcp") {
        Some(parts[1].to_string())
    } else {
        None
    }
}

fn build_mcp_payload(
    task: &SwarmTask,
    server: &str,
    route_biases: &[String],
    overlay: &PromptPolicyOverlay,
) -> String {
    serde_json::json!({
        "server": server,
        "agent": task.agent_name,
        "role": task.role,
        "objective": task.objective,
        "routing_biases": route_biases,
        "adaptive_policy": overlay.directives,
        "adaptive_rationale": overlay.rationale,
        "mode": "structured-mcp-dispatch",
    })
    .to_string()
}

fn build_local_payload(task: &SwarmTask, tool_name: &str, overlay: &PromptPolicyOverlay) -> String {
    serde_json::json!({
        "tool": tool_name,
        "agent": task.agent_name,
        "role": task.role,
        "objective": task.objective,
        "adaptive_policy": overlay.directives,
        "adaptive_rationale": overlay.rationale,
        "mode": "structured-local-execution",
    })
    .to_string()
}

pub fn update_execution_stats(
    existing: Option<ExecutionStats>,
    report: &ExecutionReport,
    observed_at_ms: u64,
) -> ExecutionStats {
    let mut stats = existing.unwrap_or_else(|| ExecutionStats {
        tool_name: report
            .tool_used
            .clone()
            .unwrap_or_else(|| "provider-only".into()),
        mcp_server: report.mcp_server.clone(),
        attempts: 0,
        successes: 0,
        failures: 0,
        cumulative_score: 0,
        success_rate: 0.0,
        effective_success_rate: 0.0,
        effective_score: 0.0,
        last_updated_ms: observed_at_ms,
        last_payload: None,
        last_outcome: String::new(),
        samples: Vec::new(),
    });

    stats.samples.push(ExecutionSample {
        observed_at_ms,
        outcome_score: report.outcome_score,
        success: report.outcome_score > 0,
        mcp_server: report.mcp_server.clone(),
    });
    stats.samples = retain_recent_samples(&stats.samples, observed_at_ms);
    stats.attempts += 1;
    stats.cumulative_score += report.outcome_score;
    if report.outcome_score > 0 {
        stats.successes += 1;
    } else {
        stats.failures += 1;
    }
    stats.success_rate = stats.successes as f32 / stats.attempts as f32;
    let (effective_success_rate, effective_score) =
        compute_decay_metrics(&stats.samples, observed_at_ms);
    stats.effective_success_rate = effective_success_rate;
    stats.effective_score = effective_score;
    stats.last_updated_ms = observed_at_ms;
    stats.last_payload = report.invocation_payload.clone();
    stats.last_outcome = report.output.clone();
    stats.mcp_server = report.mcp_server.clone();
    stats
}

const EXECUTION_STATS_WINDOW_MS: u64 = 1000 * 60 * 60 * 24 * 14;
const EXECUTION_STATS_HALF_LIFE_MS: f32 = 1000.0 * 60.0 * 60.0 * 24.0 * 3.0;
const EXECUTION_STATS_MAX_SAMPLES: usize = 64;

fn retain_recent_samples(samples: &[ExecutionSample], observed_at_ms: u64) -> Vec<ExecutionSample> {
    let cutoff = observed_at_ms.saturating_sub(EXECUTION_STATS_WINDOW_MS);
    let mut filtered = samples
        .iter()
        .filter(|sample| sample.observed_at_ms >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    if filtered.len() > EXECUTION_STATS_MAX_SAMPLES {
        filtered = filtered[filtered.len() - EXECUTION_STATS_MAX_SAMPLES..].to_vec();
    }
    filtered
}

fn compute_decay_metrics(samples: &[ExecutionSample], observed_at_ms: u64) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }

    let mut weighted_success = 0.0f32;
    let mut weighted_score = 0.0f32;
    let mut total_weight = 0.0f32;

    for sample in samples {
        let age_ms = observed_at_ms.saturating_sub(sample.observed_at_ms) as f32;
        let weight = 0.5f32.powf(age_ms / EXECUTION_STATS_HALF_LIFE_MS);
        total_weight += weight;
        weighted_score += sample.outcome_score as f32 * weight;
        if sample.success {
            weighted_success += weight;
        }
    }

    if total_weight <= f32::EPSILON {
        return (0.0, 0.0);
    }

    (
        weighted_success / total_weight,
        weighted_score / total_weight,
    )
}

pub fn update_ab_routing_stats(
    existing: Option<AbRoutingStats>,
    report: &ExecutionReport,
    observed_at_ms: u64,
) -> AbRoutingStats {
    let mut stats = existing.unwrap_or_else(|| AbRoutingStats {
        scope: String::new(),
        ..AbRoutingStats::default()
    });

    let lift = (report.treatment_score - report.control_score) as f32;
    stats.samples.push(AbRoutingSample {
        observed_at_ms,
        route_variant: report.route_variant.clone(),
        treatment_win: report.route_variant == "treatment" && report.outcome_score > 0,
        control_win: report.route_variant != "treatment" && report.outcome_score > 0,
        lift,
    });
    stats.samples = retain_recent_ab_samples(&stats.samples, observed_at_ms);
    stats.attempts += 1;
    if report.route_variant == "treatment" {
        stats.treatment_attempts += 1;
        stats.cumulative_lift += lift;
        if report.outcome_score > 0 {
            stats.treatment_wins += 1;
        } else {
            stats.treatment_losses += 1;
        }
    } else if report.outcome_score > 0 {
        stats.control_wins += 1;
    }

    let (effective_win_rate, effective_lift) =
        compute_ab_decay_metrics(&stats.samples, observed_at_ms);
    stats.effective_win_rate = effective_win_rate;
    stats.effective_lift = effective_lift;
    stats.last_updated_ms = observed_at_ms;
    stats
}

fn blended_gray_ratio(
    base_ratio: f32,
    session_stats: Option<&AbRoutingStats>,
    task_stats: Option<&AbRoutingStats>,
    tool_stats: Option<&AbRoutingStats>,
    server_stats: Option<&AbRoutingStats>,
) -> f32 {
    let session_ratio = adaptive_ratio_from_stats(base_ratio, session_stats);
    let task_ratio = adaptive_ratio_from_stats(base_ratio, task_stats);
    let tool_ratio = adaptive_ratio_from_stats(base_ratio, tool_stats);
    let server_ratio = adaptive_ratio_from_stats(base_ratio, server_stats);
    ((session_ratio + task_ratio + tool_ratio + server_ratio) / 4.0).clamp(0.05, 0.95)
}

fn adaptive_ratio_from_stats(base_ratio: f32, stats: Option<&AbRoutingStats>) -> f32 {
    let Some(stats) = stats else {
        return base_ratio;
    };
    let decayed_treatment_attempts = stats
        .samples
        .iter()
        .filter(|sample| sample.route_variant == "treatment")
        .count();
    if decayed_treatment_attempts < 3 {
        return base_ratio;
    }

    let mut ratio = base_ratio;
    if stats.effective_win_rate >= 0.65 && stats.effective_lift > 0.5 {
        ratio += 0.15;
    } else if stats.effective_win_rate >= 0.55 && stats.effective_lift > 0.0 {
        ratio += 0.05;
    } else if stats.effective_win_rate <= 0.35 || stats.effective_lift < -0.5 {
        ratio -= 0.15;
    } else if stats.effective_win_rate <= 0.45 {
        ratio -= 0.05;
    }

    ratio.clamp(0.05, 0.95)
}

const AB_STATS_WINDOW_MS: u64 = 1000 * 60 * 60 * 24 * 21;
const AB_STATS_HALF_LIFE_MS: f32 = 1000.0 * 60.0 * 60.0 * 24.0 * 5.0;
const AB_STATS_MAX_SAMPLES: usize = 96;

fn retain_recent_ab_samples(
    samples: &[AbRoutingSample],
    observed_at_ms: u64,
) -> Vec<AbRoutingSample> {
    let cutoff = observed_at_ms.saturating_sub(AB_STATS_WINDOW_MS);
    let mut filtered = samples
        .iter()
        .filter(|sample| sample.observed_at_ms >= cutoff)
        .cloned()
        .collect::<Vec<_>>();
    if filtered.len() > AB_STATS_MAX_SAMPLES {
        filtered = filtered[filtered.len() - AB_STATS_MAX_SAMPLES..].to_vec();
    }
    filtered
}

fn compute_ab_decay_metrics(samples: &[AbRoutingSample], observed_at_ms: u64) -> (f32, f32) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }

    let mut weighted_treatment_wins = 0.0f32;
    let mut weighted_treatment_attempts = 0.0f32;
    let mut weighted_lift = 0.0f32;
    let mut weighted_lift_total = 0.0f32;

    for sample in samples {
        let age_ms = observed_at_ms.saturating_sub(sample.observed_at_ms) as f32;
        let weight = 0.5f32.powf(age_ms / AB_STATS_HALF_LIFE_MS);
        if sample.route_variant == "treatment" {
            weighted_treatment_attempts += weight;
            if sample.treatment_win {
                weighted_treatment_wins += weight;
            }
            weighted_lift += sample.lift * weight;
            weighted_lift_total += weight;
        }
    }

    let win_rate = if weighted_treatment_attempts <= f32::EPSILON {
        0.0
    } else {
        weighted_treatment_wins / weighted_treatment_attempts
    };
    let lift = if weighted_lift_total <= f32::EPSILON {
        0.0
    } else {
        weighted_lift / weighted_lift_total
    };

    (win_rate, lift)
}

fn routing_bucket(session_id: &str, objective: &str) -> f32 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    objective.hash(&mut hasher);
    let bucket = (hasher.finish() % 10_000) as f32;
    bucket / 10_000.0
}

pub fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn policy_requires_revision(request: &str) -> Option<String> {
    let lowered = request.to_ascii_lowercase();
    let banned = [
        "ignore previous instructions",
        "bypass safety",
        "disable verifier",
        "print sk-",
        "exfiltrate",
    ];
    banned
        .iter()
        .find(|item| lowered.contains(**item))
        .map(|item| format!("matched banned policy phrase: {item}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::AppConfig,
        memory::MemorySubsystem,
        providers::ProviderRegistry,
        rag::RagSubsystem,
        session::{SessionIdentity, SessionStore},
        tools::{ApprovalStatus, CapabilityRisk, CapabilityStatus, ToolRegistry, TrustStatus},
    };
    use autoloop_state_adapter::{
        PermissionAction, PolicyBinding, Principal, RoleBinding, SessionLease, StateStoreBackend,
        StateStoreConfig, Tenant,
    };

    #[tokio::test]
    async fn orchestration_builds_swarm_outcome() {
        let config = AppConfig::default();
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.grant_permissions(
            "session-1",
            vec![PermissionAction::Dispatch, PermissionAction::Write],
        )
        .await
        .expect("grant");

        let tools = ToolRegistry::from_config(&config.tools);
        tools.attach_state_store(db.clone());
        tools
            .restore_persisted_manifests()
            .await
            .expect("restore catalog");
        let sessions = SessionStore::new(32);
        seed_session_identity(&db, &sessions, "session-1").await;
        let kernel = OrchestrationKernel::new(
            ProviderRegistry::from_config(&config.providers),
            tools,
            sessions,
            MemorySubsystem::from_config(&config.memory, &config.learning),
            RagSubsystem::from_config(&config.rag),
            crate::runtime::RuntimeKernel::from_config(&config.runtime),
            db,
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );

        let outcome = kernel
            .run_requirement_swarm(
                "session-1",
                "Build a StateStore-native swarm with MCP execution and graph memory.",
            )
            .await
            .expect("swarm outcome");

        assert!(!outcome.tasks.is_empty());
        assert!(
            outcome
                .tasks
                .iter()
                .any(|task| task.agent_name == "cli-agent")
        );
        assert!(
            outcome
                .tasks
                .iter()
                .any(|task| task.agent_name == "execution-agent")
        );
        assert!(!outcome.brief.clarification_turns.is_empty());
        assert_eq!(outcome.deliberation.round_count, 4);
        assert_eq!(outcome.deliberation.rounds.len(), 4);
        assert!(!outcome.deliberation.final_execution_order.is_empty());
        assert!(!outcome.deliberation.consensus_signals.is_empty());
        assert!(outcome.knowledge_update.snapshot_json.contains("documents"));
    }

    #[tokio::test]
    async fn orchestration_uses_history_and_graph_for_dynamic_routing() {
        let config = AppConfig::default();
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.upsert_knowledge(
            "conversation:session-2:swarm".into(),
            serde_json::to_string(&vec![ExecutionReport {
                task: SwarmTask {
                    task_id: "execution-history".into(),
                    agent_name: "execution-agent".into(),
                    role: "Execution".into(),
                    objective: "Run MCP".into(),
                    depends_on: Vec::new(),
                },
                output: "mcp execution completed successfully".into(),
                tool_used: Some("mcp::local-mcp::invoke".into()),
                mcp_server: Some("local-mcp".into()),
                invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
                outcome_score: 4,
                route_variant: "control".into(),
                control_score: 4,
                treatment_score: 4,
                guard_decision: "Allow".into(),
            }])
            .expect("serialize"),
            "swarm-execution".into(),
        )
        .await
        .expect("swarm history");
        db.upsert_knowledge(
            "graph:session-2:snapshot".into(),
            r#"{"documents":[{"id":1,"title":"doc","source_uri":"x","raw_text":"t","status":"GraphReady","created_at_ms":0,"chunk_count":2,"entity_count":8,"relationship_count":6}],"chunks":[],"entities":[{"id":1,"canonical_name":"AutoLoop","normalized_name":"autoloop","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1},{"id":2,"canonical_name":"StateStore","normalized_name":"state_store","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1},{"id":3,"canonical_name":"MCP Server","normalized_name":"mcpserver","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1},{"id":4,"canonical_name":"GraphRAG","normalized_name":"graphrag","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1},{"id":5,"canonical_name":"CEO","normalized_name":"ceo","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1},{"id":6,"canonical_name":"Swarm","normalized_name":"swarm","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1}],"mentions":[],"relationships":[{"id":1,"document_id":1,"source_entity_id":1,"target_entity_id":2,"relation_type":"USES","weight":10,"confidence":80,"evidence_chunk_ids":[1],"description":"x"},{"id":2,"document_id":1,"source_entity_id":2,"target_entity_id":3,"relation_type":"USES","weight":10,"confidence":80,"evidence_chunk_ids":[1],"description":"x"},{"id":3,"document_id":1,"source_entity_id":3,"target_entity_id":4,"relation_type":"USES","weight":10,"confidence":80,"evidence_chunk_ids":[1],"description":"x"},{"id":4,"document_id":1,"source_entity_id":4,"target_entity_id":5,"relation_type":"USES","weight":10,"confidence":80,"evidence_chunk_ids":[1],"description":"x"}],"communities":[{"id":1,"document_id":1,"label":"AutoLoop","member_entity_ids":[1,2,3,4,5,6],"relationship_ids":[1,2,3,4],"rank":42,"summary":"dense"}]}"#.into(),
            "graph-rag".into(),
        )
        .await
        .expect("graph history");
        db.create_schedule_event(
            "session-2".into(),
            "validation.iteration".into(),
            "mcp::local-mcp::invoke".into(),
            "{}".into(),
            "session-2".into(),
        )
        .await
        .expect("event");

        let tools = ToolRegistry::from_config(&config.tools);
        tools.attach_state_store(db.clone());
        tools
            .restore_persisted_manifests()
            .await
            .expect("restore catalog");
        let sessions = SessionStore::new(32);
        seed_session_identity(&db, &sessions, "session-2").await;
        let kernel = OrchestrationKernel::new(
            ProviderRegistry::from_config(&config.providers),
            tools,
            sessions,
            MemorySubsystem::from_config(&config.memory, &config.learning),
            RagSubsystem::from_config(&config.rag),
            crate::runtime::RuntimeKernel::from_config(&config.runtime),
            db,
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );

        let outcome = kernel
            .run_requirement_swarm(
                "session-2",
                "Refine the swarm plan using the existing graph memory and close remaining gaps.",
            )
            .await
            .expect("swarm outcome");

        assert!(
            outcome
                .routing_context
                .route_biases
                .contains(&"reuse_graph_memory".to_string())
        );
        assert!(
            outcome
                .tasks
                .iter()
                .any(|task| task.agent_name == "retrieval-agent")
        );
        assert!(
            outcome
                .tasks
                .iter()
                .any(|task| task.agent_name == "ops-agent")
        );
    }

    async fn seed_session_identity(db: &StateStore, sessions: &SessionStore, session_id: &str) {
        let now = current_time_ms();
        let tenant_id = "tenant:test";
        let principal_id = format!("principal:{session_id}");
        let policy_id = "policy:test";
        let lease_token = format!("lease:{session_id}");

        db.upsert_tenant(Tenant {
            tenant_id: tenant_id.into(),
            name: tenant_id.into(),
            status: "active".into(),
            created_at_ms: now,
        })
        .await
        .expect("tenant");
        db.upsert_principal(Principal {
            principal_id: principal_id.clone(),
            tenant_id: tenant_id.into(),
            principal_type: "user".into(),
            status: "active".into(),
            created_at_ms: now,
        })
        .await
        .expect("principal");
        db.upsert_role_binding(RoleBinding {
            tenant_id: tenant_id.into(),
            principal_id: principal_id.clone(),
            role: "operator".into(),
            updated_at_ms: now,
        })
        .await
        .expect("role");
        db.upsert_policy_binding(PolicyBinding {
            policy_id: policy_id.into(),
            tenant_id: tenant_id.into(),
            role: "operator".into(),
            allowed_actions: vec![],
            capability_prefixes: vec!["".into()],
            max_memory_mb: 2048,
            max_tokens: 32000,
            updated_at_ms: now,
        })
        .await
        .expect("policy");
        db.upsert_session_lease(SessionLease {
            lease_token: lease_token.clone(),
            session_id: session_id.into(),
            tenant_id: tenant_id.into(),
            principal_id: principal_id.clone(),
            policy_id: policy_id.into(),
            expires_at_ms: now.saturating_add(60_000),
            issued_at_ms: now,
        })
        .await
        .expect("lease");
        sessions
            .bind_identity(
                session_id,
                SessionIdentity {
                    tenant_id: tenant_id.into(),
                    principal_id,
                    policy_id: policy_id.into(),
                    lease_token,
                    expires_at_ms: now.saturating_add(60_000),
                },
            )
            .await;
    }

    #[tokio::test]
    async fn admission_governance_block_returns_uniform_evidence_fields() {
        let config = AppConfig::default();
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let tools = ToolRegistry::from_config(&config.tools);
        tools.attach_state_store(db.clone());
        tools
            .restore_persisted_manifests()
            .await
            .expect("restore catalog");

        if tools.manifests().is_empty() {
            let _ = tools
                .execute(
                    "cli::forge_mcp_tool",
                    r#"{
                        "server":"local-mcp",
                        "capability_name":"governance block probe",
                        "purpose":"Probe governance admission blocking path",
                        "executable":"autoloop-cli",
                        "subcommands":["task","run"],
                        "arguments":[{"name":"objective","description":"objective","required":true}],
                        "output_mode":"json"
                    }"#,
                )
                .await
                .expect("forge fallback manifest");
        }

        let mut manifest = tools
            .manifests()
            .into_iter()
            .find(|item| item.server == "local-mcp")
            .or_else(|| tools.manifests().into_iter().next())
            .expect("mcp manifest");
        manifest.risk = CapabilityRisk::High;
        manifest.status = CapabilityStatus::Active;
        manifest.approval_status = ApprovalStatus::Verified;
        manifest.trust_status = TrustStatus::Trusted;
        tools.register_manifest(manifest.clone());
        tools
            .persist_manifest(&manifest)
            .await
            .expect("persist manifest");

        let sessions = SessionStore::new(32);
        let session_id = "session-governance-block";
        seed_session_identity(&db, &sessions, session_id).await;

        db.upsert_json_knowledge(
            format!(
                "approval:capability:{session_id}:task-governance-block:{}",
                manifest.registered_tool_name
            ),
            &serde_json::json!({"approved": true}),
            "test-suite",
        )
        .await
        .expect("approval");

        let kernel = OrchestrationKernel::new(
            ProviderRegistry::from_config(&config.providers),
            tools,
            sessions,
            MemorySubsystem::from_config(&config.memory, &config.learning),
            RagSubsystem::from_config(&config.rag),
            crate::runtime::RuntimeKernel::from_config(&config.runtime),
            db.clone(),
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );

        let identity = kernel
            .execution_identity_for_session(session_id)
            .await
            .expect("identity");
        let task = SwarmTask {
            task_id: "task-governance-block".into(),
            agent_name: "execution-agent".into(),
            role: "Execution".into(),
            objective: "Execute high-risk capability".into(),
            depends_on: Vec::new(),
        };
        let decision = ToolRoutingDecision {
            tool_name: Some(manifest.registered_tool_name.clone()),
            mcp_server: Some(manifest.server.clone()),
            invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
            score: 90,
            control_score: 90,
            treatment_score: 90,
            route_variant: "control".into(),
            rationale: "forced high-risk test route".into(),
        };

        let (reason, evidence_ref, status) = kernel
            .admit_capability_for_task(session_id, &task, &decision, &identity)
            .await
            .expect("admission decision");

        assert_eq!(status, "rejected");
        let reason = reason.expect("reason json");
        let reason_value: serde_json::Value = serde_json::from_str(&reason).expect("reason json");
        let evidence_ref = evidence_ref.expect("evidence ref");

        assert!(
            reason_value
                .get("rule_id")
                .and_then(|v| v.as_str())
                .is_some()
        );
        assert!(
            reason_value
                .get("policy_version")
                .and_then(|v| v.as_str())
                .is_some()
        );
        assert_eq!(
            reason_value
                .get("evidence_ref")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            evidence_ref
        );
        assert!(
            reason_value
                .get("replay_fp")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .starts_with("replay-fp:")
        );
    }

    #[tokio::test]
    async fn execution_agent_prefers_tool_with_better_history_and_graph_alignment() {
        let config = AppConfig::default();
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.upsert_knowledge(
            "conversation:session-3:swarm".into(),
            serde_json::to_string(&vec![
                ExecutionReport {
                    task: SwarmTask {
                        task_id: "execution-history-success".into(),
                        agent_name: "execution-agent".into(),
                        role: "Execution".into(),
                        objective: "Run MCP".into(),
                        depends_on: Vec::new(),
                    },
                    output: "mcp execution completed successfully".into(),
                    tool_used: Some("mcp::local-mcp::invoke".into()),
                    mcp_server: Some("local-mcp".into()),
                    invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
                    outcome_score: 4,
                    route_variant: "control".into(),
                    control_score: 4,
                    treatment_score: 4,
                    guard_decision: "Allow".into(),
                },
                ExecutionReport {
                    task: SwarmTask {
                        task_id: "execution-history-failure".into(),
                        agent_name: "execution-agent".into(),
                        role: "Execution".into(),
                        objective: "Run files".into(),
                        depends_on: Vec::new(),
                    },
                    output: "write_file failed with an error".into(),
                    tool_used: Some("write_file".into()),
                    mcp_server: None,
                    invocation_payload: Some("{\"tool\":\"write_file\"}".into()),
                    outcome_score: -6,
                    route_variant: "control".into(),
                    control_score: -6,
                    treatment_score: -6,
                    guard_decision: "Allow".into(),
                },
            ])
            .expect("serialize"),
            "swarm-execution".into(),
        )
        .await
        .expect("history");
        db.upsert_knowledge(
            "conversation:session-3:feedback".into(),
            "tool=mcp::local-mcp::invoke success ready".into(),
            "execution-feedback".into(),
        )
        .await
        .expect("feedback");
        db.upsert_knowledge(
            "graph:session-3:snapshot".into(),
            r#"{"documents":[{"id":1,"title":"doc","source_uri":"x","raw_text":"t","status":"GraphReady","created_at_ms":0,"chunk_count":2,"entity_count":8,"relationship_count":6}],"chunks":[],"entities":[{"id":1,"canonical_name":"MCP","normalized_name":"mcp","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1},{"id":2,"canonical_name":"Server","normalized_name":"server","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1}],"mentions":[],"relationships":[],"communities":[]}"#.into(),
            "graph-rag".into(),
        )
        .await
        .expect("graph");
        db.upsert_knowledge(
            format!(
                "{}mcp::local-mcp::invoke",
                crate::tools::ToolRegistry::FORGED_TOOL_PREFIX
            ),
            serde_json::json!({
                "registered_tool_name":"mcp::local-mcp::invoke",
                "delegate_tool_name":"mcp::local-mcp::invoke",
                "server":"local-mcp",
                "capability_name":"invoke",
                "purpose":"Use local mcp for execution",
                "executable":"autoloop-cli",
                "command_template":"autoloop-cli task execution --objective {{objective}}",
                "payload_template":{"server":"local-mcp"},
                "output_mode":"json",
                "working_directory":".",
                "success_signal":"completed",
                "help_text":"help",
                "skill_markdown":"# invoke",
                "examples":["autoloop-cli task execution --objective run"]
            })
            .to_string(),
            "cli-forge".into(),
        )
        .await
        .expect("catalog");

        let tools = ToolRegistry::from_config(&config.tools);
        tools.attach_state_store(db.clone());
        tools
            .restore_persisted_manifests()
            .await
            .expect("restore catalog");
        let kernel = OrchestrationKernel::new(
            ProviderRegistry::from_config(&config.providers),
            tools,
            SessionStore::new(32),
            MemorySubsystem::from_config(&config.memory, &config.learning),
            RagSubsystem::from_config(&config.rag),
            crate::runtime::RuntimeKernel::from_config(&config.runtime),
            db.clone(),
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );
        let routing = kernel
            .load_routing_context("session-3", "Choose the best MCP-capable executor")
            .await
            .expect("routing");
        let decision = kernel.select_tool(
            "session-3",
            &SwarmTask {
                task_id: "execution-select-tool".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Choose the best MCP-capable executor".into(),
                depends_on: Vec::new(),
            },
            &routing,
        );

        assert_eq!(decision.tool_name.as_deref(), Some("cli::forge_mcp_tool"));
        assert!(decision.score > 0);
    }

    #[tokio::test]
    async fn execution_agent_uses_forged_tool_evidence_in_routing() {
        let config = AppConfig::default();
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.upsert_knowledge(
            "graph:session-forged:snapshot".into(),
            r#"{"documents":[{"id":1,"title":"doc","source_uri":"x","raw_text":"t","status":"GraphReady","created_at_ms":0,"chunk_count":2,"entity_count":8,"relationship_count":6}],"chunks":[],"entities":[{"id":1,"canonical_name":"MCP","normalized_name":"mcp","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1},{"id":2,"canonical_name":"CLI","normalized_name":"cli","entity_type":"Concept","description":"x","salience":1,"mention_count":1,"degree":1,"weight":10,"first_document_id":1}],"mentions":[],"relationships":[],"communities":[]}"#.into(),
            "graph-rag".into(),
        )
        .await
        .expect("graph");
        db.upsert_knowledge(
            format!(
                "{}mcp::local-mcp::invoke",
                crate::tools::ToolRegistry::FORGED_TOOL_PREFIX
            ),
            serde_json::json!({
                "registered_tool_name":"mcp::local-mcp::invoke",
                "delegate_tool_name":"mcp::local-mcp::invoke",
                "server":"local-mcp",
                "capability_name":"invoke",
                "purpose":"Use local mcp for execution",
                "executable":"autoloop-cli",
                "command_template":"autoloop-cli task execution --objective {{objective}}",
                "payload_template":{"server":"local-mcp"},
                "output_mode":"json",
                "working_directory":".",
                "success_signal":"completed",
                "help_text":"help",
                "skill_markdown":"# invoke",
                "examples":["autoloop-cli task execution --objective run"]
            })
            .to_string(),
            "cli-forge".into(),
        )
        .await
        .expect("forged manifest");

        let tools = ToolRegistry::from_config(&config.tools);
        tools.attach_state_store(db.clone());
        tools
            .restore_persisted_manifests()
            .await
            .expect("restore catalog");
        let kernel = OrchestrationKernel::new(
            ProviderRegistry::from_config(&config.providers),
            tools,
            SessionStore::new(32),
            MemorySubsystem::from_config(&config.memory, &config.learning),
            RagSubsystem::from_config(&config.rag),
            crate::runtime::RuntimeKernel::from_config(&config.runtime),
            db.clone(),
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );
        let routing = kernel
            .load_routing_context(
                "session-forged",
                "use the existing forged mcp capability to execute",
            )
            .await
            .expect("routing");
        let decision = kernel.select_tool(
            "session-forged",
            &SwarmTask {
                task_id: "execution-forged-evidence".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Use existing forged MCP capability for execution".into(),
                depends_on: Vec::new(),
            },
            &routing,
        );

        assert!(routing.forged_tool_coverage >= 1);
        assert_eq!(decision.tool_name.as_deref(), Some("cli::forge_mcp_tool"));
        assert!(
            decision
                .rationale
                .contains("forged")
                || decision.rationale.contains("capability")
        );
    }

    #[tokio::test]
    async fn execution_agent_only_selects_catalog_or_forge_fallback() {
        let config = AppConfig::default();
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let tools = ToolRegistry::from_config(&config.tools);
        let kernel = OrchestrationKernel::new(
            ProviderRegistry::from_config(&config.providers),
            tools.clone(),
            SessionStore::new(32),
            MemorySubsystem::from_config(&config.memory, &config.learning),
            RagSubsystem::from_config(&config.rag),
            crate::runtime::RuntimeKernel::from_config(&config.runtime),
            db.clone(),
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );

        let routing = kernel
            .load_routing_context(
                "session-catalog",
                "execute through existing forged mcp capability",
            )
            .await
            .expect("routing");
        let first_decision = kernel.select_tool(
            "session-catalog",
            &SwarmTask {
                task_id: "execution-catalog-first".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Execute using existing forged capability".into(),
                depends_on: Vec::new(),
            },
            &routing,
        );
        assert_eq!(
            first_decision.tool_name.as_deref(),
            Some("cli::forge_mcp_tool")
        );

        tools
            .execute(
                "cli::forge_mcp_tool",
                r#"{
                    "server":"local-mcp",
                    "capability_name":"catalog-exec",
                    "purpose":"execute task via forged catalog tool",
                    "executable":"autoloop-cli",
                    "subcommands":["task","execution"],
                    "arguments":[{"name":"objective","description":"goal","required":true}],
                    "output_mode":"json"
                }"#,
            )
            .await
            .expect("forge");

        let routing_after_catalog = kernel
            .load_routing_context(
                "session-catalog",
                "execute through existing forged mcp capability",
            )
            .await
            .expect("routing");
        let second_decision = kernel.select_tool(
            "session-catalog",
            &SwarmTask {
                task_id: "execution-catalog-second".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Execute using existing forged capability".into(),
                depends_on: Vec::new(),
            },
            &routing_after_catalog,
        );
        assert_eq!(
            second_decision.tool_name.as_deref(),
            Some("mcp::local-mcp::catalog-exec")
        );
    }

    #[tokio::test]
    async fn execution_agent_allows_write_file_for_artifact_objective_without_catalog() {
        let config = AppConfig::default();
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let kernel = OrchestrationKernel::new(
            ProviderRegistry::from_config(&config.providers),
            ToolRegistry::from_config(&config.tools),
            SessionStore::new(32),
            MemorySubsystem::from_config(&config.memory, &config.learning),
            RagSubsystem::from_config(&config.rag),
            crate::runtime::RuntimeKernel::from_config(&config.runtime),
            db,
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );

        let routing = kernel
            .load_routing_context(
                "session-artifact-local",
                "write file artifact to D:\\AutoLoop\\output\\artifact.html",
            )
            .await
            .expect("routing");
        let decision = kernel.select_tool(
            "session-artifact-local",
            &SwarmTask {
                task_id: "execution-artifact-local".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Write file artifact to D:\\AutoLoop\\output\\artifact.html".into(),
                depends_on: Vec::new(),
            },
            &routing,
        );

        assert_eq!(decision.tool_name.as_deref(), Some("write_file"));
    }

    #[test]
    fn execution_stats_keep_decayed_long_term_memory() {
        let base_time = 1_700_000_000_000u64;
        let old_failure = ExecutionReport {
            task: SwarmTask {
                task_id: "execution-old-failure".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "old attempt".into(),
                depends_on: Vec::new(),
            },
            output: "failed with an error".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
            outcome_score: -6,
            route_variant: "control".into(),
            control_score: -6,
            treatment_score: -6,
            guard_decision: "Allow".into(),
        };
        let new_success = ExecutionReport {
            task: SwarmTask {
                task_id: "execution-new-success".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "new attempt".into(),
                depends_on: Vec::new(),
            },
            output: "completed successfully".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
            outcome_score: 4,
            route_variant: "control".into(),
            control_score: 4,
            treatment_score: 4,
            guard_decision: "Allow".into(),
        };

        let after_failure = update_execution_stats(None, &old_failure, base_time);
        let after_success = update_execution_stats(
            Some(after_failure),
            &new_success,
            base_time + EXECUTION_STATS_WINDOW_MS - 1,
        );

        assert_eq!(after_success.attempts, 2);
        assert_eq!(after_success.samples.len(), 2);
        assert!(after_success.effective_success_rate > 0.5);
        assert!(after_success.effective_score > 0.0);
    }

    #[test]
    fn execution_stats_drop_samples_outside_window() {
        let base_time = 1_700_000_000_000u64;
        let report = ExecutionReport {
            task: SwarmTask {
                task_id: "execution-window-attempt".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "attempt".into(),
                depends_on: Vec::new(),
            },
            output: "completed successfully".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
            outcome_score: 4,
            route_variant: "control".into(),
            control_score: 4,
            treatment_score: 4,
            guard_decision: "Allow".into(),
        };

        let stats = update_execution_stats(None, &report, base_time);
        let updated = update_execution_stats(
            Some(stats),
            &report,
            base_time + EXECUTION_STATS_WINDOW_MS + 1,
        );

        assert_eq!(updated.samples.len(), 1);
        assert_eq!(updated.attempts, 2);
    }

    #[test]
    fn ab_routing_stats_track_treatment_win_rate_and_lift() {
        let report = ExecutionReport {
            task: SwarmTask {
                task_id: "ab-treatment-win".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "attempt".into(),
                depends_on: Vec::new(),
            },
            output: "completed successfully".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
            outcome_score: 4,
            route_variant: "treatment".into(),
            control_score: 6,
            treatment_score: 9,
            guard_decision: "Allow".into(),
        };

        let stats = update_ab_routing_stats(None, &report, 100);

        assert_eq!(stats.treatment_attempts, 1);
        assert_eq!(stats.treatment_wins, 1);
        assert!(stats.effective_win_rate > 0.9);
        assert!(stats.effective_lift > 0.0);
        assert_eq!(stats.samples.len(), 1);
    }

    #[test]
    fn blended_gray_ratio_increases_for_strong_treatment_history() {
        let session_stats = AbRoutingStats {
            scope: "metrics:ab:session:s1".into(),
            attempts: 10,
            treatment_attempts: 8,
            treatment_wins: 6,
            treatment_losses: 2,
            control_wins: 1,
            cumulative_lift: 16.0,
            effective_win_rate: 0.75,
            effective_lift: 2.0,
            last_updated_ms: 100,
            samples: vec![
                AbRoutingSample {
                    observed_at_ms: 90,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 2.2,
                },
                AbRoutingSample {
                    observed_at_ms: 95,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.8,
                },
                AbRoutingSample {
                    observed_at_ms: 99,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 2.0,
                },
            ],
        };
        let task_stats = AbRoutingStats {
            scope: "metrics:ab:task:execution".into(),
            attempts: 8,
            treatment_attempts: 6,
            treatment_wins: 4,
            treatment_losses: 2,
            control_wins: 1,
            cumulative_lift: 9.0,
            effective_win_rate: 0.66,
            effective_lift: 1.5,
            last_updated_ms: 100,
            samples: vec![
                AbRoutingSample {
                    observed_at_ms: 92,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.4,
                },
                AbRoutingSample {
                    observed_at_ms: 97,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.6,
                },
                AbRoutingSample {
                    observed_at_ms: 100,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.5,
                },
            ],
        };

        let tool_stats = AbRoutingStats {
            scope: "metrics:ab:tool:mcp::local-mcp::invoke".into(),
            attempts: 8,
            treatment_attempts: 7,
            treatment_wins: 5,
            treatment_losses: 2,
            control_wins: 1,
            cumulative_lift: 8.0,
            effective_win_rate: 0.71,
            effective_lift: 1.1,
            last_updated_ms: 100,
            samples: vec![
                AbRoutingSample {
                    observed_at_ms: 96,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.1,
                },
                AbRoutingSample {
                    observed_at_ms: 98,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.0,
                },
                AbRoutingSample {
                    observed_at_ms: 100,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.2,
                },
            ],
        };
        let server_stats = AbRoutingStats {
            scope: "metrics:ab:server:local-mcp".into(),
            attempts: 8,
            treatment_attempts: 6,
            treatment_wins: 4,
            treatment_losses: 2,
            control_wins: 1,
            cumulative_lift: 7.0,
            effective_win_rate: 0.66,
            effective_lift: 1.0,
            last_updated_ms: 100,
            samples: vec![
                AbRoutingSample {
                    observed_at_ms: 94,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.0,
                },
                AbRoutingSample {
                    observed_at_ms: 99,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 1.1,
                },
                AbRoutingSample {
                    observed_at_ms: 100,
                    route_variant: "treatment".into(),
                    treatment_win: true,
                    control_win: false,
                    lift: 0.9,
                },
            ],
        };

        let ratio = blended_gray_ratio(
            0.2,
            Some(&session_stats),
            Some(&task_stats),
            Some(&tool_stats),
            Some(&server_stats),
        );

        assert!(ratio > 0.2);
    }

    #[test]
    fn ab_routing_stats_decay_old_treatment_history() {
        let old = ExecutionReport {
            task: SwarmTask {
                task_id: "ab-old-treatment".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "old".into(),
                depends_on: Vec::new(),
            },
            output: "failed".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{}".into()),
            outcome_score: -6,
            route_variant: "treatment".into(),
            control_score: 6,
            treatment_score: 1,
            guard_decision: "Allow".into(),
        };
        let fresh = ExecutionReport {
            task: SwarmTask {
                task_id: "ab-fresh-treatment".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "fresh".into(),
                depends_on: Vec::new(),
            },
            output: "completed".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{}".into()),
            outcome_score: 4,
            route_variant: "treatment".into(),
            control_score: 4,
            treatment_score: 9,
            guard_decision: "Allow".into(),
        };

        let stats = update_ab_routing_stats(None, &old, 1_700_000_000_000);
        let updated = update_ab_routing_stats(
            Some(stats),
            &fresh,
            1_700_000_000_000 + AB_STATS_WINDOW_MS - 1,
        );

        assert!(updated.effective_win_rate > 0.5);
        assert!(updated.effective_lift > 0.0);
    }

    #[test]
    fn ab_routing_stats_drop_samples_outside_window() {
        let report = ExecutionReport {
            task: SwarmTask {
                task_id: "ab-window-attempt".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "attempt".into(),
                depends_on: Vec::new(),
            },
            output: "completed".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{}".into()),
            outcome_score: 4,
            route_variant: "treatment".into(),
            control_score: 4,
            treatment_score: 7,
            guard_decision: "Allow".into(),
        };

        let stats = update_ab_routing_stats(None, &report, 1_700_000_000_000);
        let updated = update_ab_routing_stats(
            Some(stats),
            &report,
            1_700_000_000_000 + AB_STATS_WINDOW_MS + 1,
        );

        assert_eq!(updated.samples.len(), 1);
    }
}


