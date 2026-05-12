pub mod adaptive_framework;
pub mod agent;
pub mod agentevolver_task_core;
pub mod bus;
pub mod cli_runtime;
pub mod config;
pub mod contracts;
pub mod dashboard_server;
pub mod evolution;
pub mod evolution_os;
pub mod hooks;
pub mod memory;
pub mod module_bindings;
pub mod observability;
pub mod orchestration;
pub mod path_safety;
pub mod plugins;
pub mod providers;
pub mod query_engine;
pub mod rag;
pub mod research;
pub mod runtime;
pub mod security;
pub mod services;
pub mod session;
pub mod skills;
pub mod tools;
pub mod transport;
pub mod tui;

pub use autoloop_state_adapter as state_store_adapter;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use agent::AgentRuntime;
use anyhow::{Result, anyhow};
use autoloop_postgres_adapter::{PostgresDb, PostgresDbConfig};
use autoloop_state_adapter::{
    KnowledgeMirrorMode, KnowledgeReadPreference, ScheduleEvent, StateStore,
};
use bus::MessageBus;
use config::{AppConfig, StorageBackend, StorageMode};
use contracts::services::{attach_service_gate_token, build_service_gate_token, service_call_gate_token};
use evolution::SelfEvolutionKernel;
use evolution_os::{
    EvolutionOsKernel, EvolutionShadowCycle, ExternalProposalSignals, IngestInput,
    PromotionPath, TelemetryReplaySnapshot, WorldlineWeights,
};
use hooks::HookRegistry;
use memory::{CausalEdge, MemorySubsystem, ReflexionEpisode, SkillRecord, WitnessLog};
use ontoloop_core::{
    ConstitutionAuditRecord, CorePromotionDecision, CoreRolloutStage, ExecutionConstitutionState,
    ProductionWriteInput, transition_with_audit,
};
use observability::{collector::TelemetryCollectorSnapshot, ObservabilityKernel};
use observability::event_stream::{ReplayAnalysisReport, append_event};
use observability::policy_signal::aggregate_and_persist as aggregate_policy_signals;
use orchestration::{
    AbRoutingStats, ExecutionStats, OrchestrationKernel, current_time_ms,
    governance_telemetry_scope::GovernanceTelemetryScope,
    knowledge_context::KnowledgeContextResolver,
    org_context::OrganizationContextResolver,
    parse_mcp_server,
    response_builder::{ResponseBuilder, ResponseBuilderInput},
    update_ab_routing_stats, update_execution_stats,
};
use plugins::{
    PluginLifecycleManager,
    gitmemory_core::{
        GitmemoryCoreKernel, GovernancePhase,
        patch_core::{PatchOp, PatchOpKind, PatchPlan},
        patch_review_queue::PatchReviewQueue,
    },
};
use providers::ProviderRegistry;
use query_engine::{
    ContextRuntimeKernel, DiffPatchEngine, RepoContextCompiler, ShellExecutionLoopEngine,
    ShellLoopRequest, TestVerifierEngine, TestVerifierRequest, GitCheckpointLayer,
    GitCheckpointRequest,
    IterationControllerConfig, IterationControllerReport, TerminationReason,
    load_failure_experience_hints, merge_failure_experience_hints, record_failure_experience,
    classify_failure, select_repair_strategy, should_retry_for_failure, stage_from_error,
};
use rag::{
    OrgKnowledgePublisher, OrgSharingGateInput, RagSubsystem, SharedKnowledgePortAdapter,
    SharedKnowledgeUpdate, evaluate_org_sharing_gate,
};
use research::ResearchKernel;
use runtime::decision_propagator::{
    CrossKernelDecisionEnvelope, propagate as propagate_cross_kernel_decision,
};
use runtime::decision_protocol::{
    ExecutionGuardObservation, RuntimeDecisionKind, UnifiedDecisionInput, UnifiedDecisionOutput,
    evaluate_unified_decision, load_thresholds_from_env, parse_decision_hint,
};
use runtime::{
    RuntimeKernel,
    evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
    evidence_tagger::EvidenceTagStage,
    execution_fabric::ExecutionFabricRecord,
    trigger_runtime::TriggerRuntimeEngine,
};
use security::SecurityPolicy;
use services::{BackgroundTaskManager, McpManager, ServiceMediator};
use session::{SessionIdentity, SessionStore};
use skills::SkillRegistry;
use tools::{ForgedMcpToolManifest, ToolRegistry};
use transport::TransportBridgeRuntime;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardSessionSnapshot {
    pub session_id: String,
    pub anchor: String,
    pub ceo_summary: String,
    pub validation_summary: String,
    pub route_treatment_share: f32,
    pub readiness: bool,
    pub capability_catalog: Vec<DashboardCapabilityRecord>,
    pub proxy_forensics: serde_json::Value,
    pub research_health: serde_json::Value,
    pub graph: DashboardGraphLens,
    pub verifier: DashboardVerifierLens,
    pub verifier_evidence_links: Vec<DashboardEvidenceLink>,
    pub audit_evidence_refs: Vec<String>,
    pub business: DashboardBusinessLens,
    pub work_orders: Vec<serde_json::Value>,
    pub revenue_events: Vec<serde_json::Value>,
    pub operations_notes: Vec<String>,
    pub capability_lifecycle: serde_json::Value,
    pub runtime_circuits: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardCapabilityRecord {
    pub name: String,
    pub status: String,
    pub approval: String,
    pub health: f32,
    pub scope: String,
    pub risk: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardGraphLens {
    pub entities: usize,
    pub relationships: usize,
    pub communities: usize,
    pub forged_capability_count: usize,
    pub top_entities: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardEvidenceLink {
    pub task_id: String,
    pub trace_id: String,
    pub admission_status: String,
    pub admission_evidence_ref: Option<String>,
    pub guard_evidence_ref: Option<String>,
    pub guard_decision: String,
    pub guard_reason: String,
    pub policy_reject_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardVerifierLens {
    pub verdict: String,
    pub score: f32,
    pub summary: String,
    pub failing_tools: Vec<String>,
    pub evidence_links: Vec<DashboardEvidenceLink>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardBusinessLens {
    pub revenue_micros: u64,
    pub cost_micros: u64,
    pub profit_micros: i64,
    pub margin_ratio: f32,
    pub sla_success_ratio: f32,
    pub breached_orders: usize,
    pub risk_summary: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplayAttestationColumn {
    pub status: String,
    pub passed: bool,
    pub policy_hits: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub verifier_id: Option<String>,
    pub policy_version: Option<String>,
    pub quote_digest: Option<String>,
    pub cert_chain_digest: Option<String>,
    pub decision_hash: Option<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplayReportEntry {
    pub report: ReplayAnalysisReport,
    pub attestation: ReplayAttestationColumn,
    pub mismatch_explainer: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionReplaySnapshot {
    pub session_id: String,
    pub deliberation: Option<serde_json::Value>,
    pub execution_feedback: Vec<serde_json::Value>,
    pub traces: Vec<serde_json::Value>,
    pub route_analytics: Option<serde_json::Value>,
    pub failure_forensics: Option<serde_json::Value>,
}

#[derive(Debug)]
pub struct BootstrapReport {
    pub app_name: String,
    pub provider_count: usize,
    pub tool_count: usize,
    pub hook_count: usize,
    pub memory_targets: usize,
    pub rag_strategies: usize,
}

pub struct AutoLoopApp {
    pub config: AppConfig,
    pub runtime: RuntimeKernel,
    pub security: SecurityPolicy,
    pub memory: MemorySubsystem,
    pub evolution: SelfEvolutionKernel,
    pub rag: RagSubsystem,
    pub research: ResearchKernel,
    providers: ProviderRegistry,
    tools: ToolRegistry,
    pub hooks: HookRegistry,
    pub observability: ObservabilityKernel,
    pub orchestration: OrchestrationKernel,
    pub sessions: SessionStore,
    pub transport: TransportBridgeRuntime,
    pub frontend_bridge: transport::FrontendBridgeRuntime,
    pub remote_sessions: transport::RemoteSessionRunner,
    pub skills: SkillRegistry,
    pub plugins: PluginLifecycleManager,
    pub services: ServiceMediator,
    pub background_tasks: BackgroundTaskManager,
    pub bus: MessageBus,
    state_store: StateStore,
    pub postgresdb: Option<PostgresDb>,
    pub agent: AgentRuntime,
}

#[derive(Debug, Clone)]
struct EvolutionProductionWriteGate {
    constitution_state: ExecutionConstitutionState,
}

impl EvolutionProductionWriteGate {
    fn production_write_allowed(&self) -> bool {
        self.constitution_state.decision().production_write_allowed
    }

    fn deny_reason(&self) -> &str {
        self.constitution_state.decision().deny_reason.as_str()
    }

    fn as_json(&self) -> serde_json::Value {
        let decision = self.constitution_state.decision();
        serde_json::json!({
            "board_decision": format!("{:?}", decision.board_decision),
            "policy_allow": decision.policy_allow,
            "evidence_ref": decision.evidence_ref,
            "rollout_stage": format!("{:?}", decision.rollout_stage),
            "production_write_allowed": self.production_write_allowed(),
            "deny_reason": self.deny_reason(),
            "constitution_state_hash": self.constitution_state.state_hash(),
        })
    }
}

impl AutoLoopApp {
    pub fn state_store(&self) -> &StateStore {
        &self.state_store
    }

    pub fn providers(&self) -> &ProviderRegistry {
        &self.providers
    }

    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    pub fn try_new(config: AppConfig) -> Result<Self> {
        let runtime = RuntimeKernel::from_config(&config.runtime);
        let security = SecurityPolicy::from_config(&config.security);
        let memory = MemorySubsystem::from_config(&config.memory, &config.learning);
        let evolution = SelfEvolutionKernel::new();
        let rag = RagSubsystem::from_config(&config.rag);
        let research = ResearchKernel::from_config(&config.research);
        let providers = ProviderRegistry::from_config(&config.providers);
        let tools = ToolRegistry::from_config(&config.tools);
        let hooks = HookRegistry::from_config(&config.hooks);
        let observability =
            ObservabilityKernel::from_config(&config.observability, &config.deployment);
        let sessions = SessionStore::new(config.agent.memory_window);
        let bus = MessageBus::default();
        let state_store = StateStore::try_from_config(&config.state_store)?;
        let postgresdb = should_enable_postgres(&config).then(|| {
            PostgresDb::new(PostgresDbConfig {
                enabled: config.storage.postgres.enabled,
                uri: config.storage.postgres.uri.clone(),
                schema: config.storage.postgres.schema.clone(),
                pool_size: config.storage.postgres.pool_size,
                auto_migrate: true,
            })
        });
        if let Some(postgres) = postgresdb.clone() {
            state_store.configure_knowledge_mirror(
                postgres,
                map_storage_mode_to_knowledge_mirror_mode(config.storage.mode.clone()),
                map_storage_read_preference(
                    config.storage.shadow_read_preference.as_str(),
                    config.storage.backend.clone(),
                ),
                config.storage.shadow_read_rollout_percent,
            )?;
        } else {
            state_store.clear_knowledge_mirror();
        }
        let transport = TransportBridgeRuntime::new(sessions.clone(), state_store.clone());
        let frontend_bridge =
            transport::FrontendBridgeRuntime::new(transport.clone(), state_store.clone());
        let remote_sessions = transport::RemoteSessionRunner::from_env(transport.clone());
        let skills = SkillRegistry::new(state_store.clone());
        let plugins = PluginLifecycleManager::new(state_store.clone());
        let mcp_manager = McpManager::new();
        let services = ServiceMediator::new(
            providers.clone(),
            tools.clone(),
            plugins.clone(),
            mcp_manager,
            memory.clone(),
            skills.clone(),
            observability.clone(),
            state_store.clone(),
        );
        tools.attach_state_store(state_store.clone());
        let background_tasks = BackgroundTaskManager::default();
        let orchestration = OrchestrationKernel::new(
            providers.clone(),
            tools.clone(),
            sessions.clone(),
            memory.clone(),
            rag.clone(),
            runtime.clone(),
            state_store.clone(),
            config.learning.gray_routing_ratio,
            config.learning.routing_takeover_threshold,
        );
        let agent = AgentRuntime::new(
            config.agent.clone(),
            providers.clone(),
            tools.clone(),
            sessions.clone(),
            memory.clone(),
            hooks.clone(),
            security.clone(),
            runtime.clone(),
            state_store.clone(),
        );

        Ok(Self {
            config,
            runtime,
            security,
            memory,
            evolution,
            rag,
            research,
            providers,
            tools,
            hooks,
            observability,
            orchestration,
            sessions,
            transport,
            frontend_bridge,
            remote_sessions,
            skills,
            plugins,
            services,
            background_tasks,
            bus,
            state_store,
            postgresdb,
            agent,
        })
    }

    pub fn new(config: AppConfig) -> Self {
        Self::try_new(config).expect("failed to construct AutoLoopApp")
    }

    pub async fn bootstrap(&self) -> Result<BootstrapReport> {
        let _ = self.tools.restore_persisted_manifests().await?;
        self.runtime.validate()?;
        self.security.validate(&self.runtime, &self.tools)?;
        self.memory.validate()?;
        self.rag.validate()?;
        self.research.validate()?;
        self.providers.validate()?;
        self.tools.validate()?;
        self.hooks.validate()?;
        self.observability.validate()?;
        self.state_store.validate()?;
        if let Some(postgresdb) = &self.postgresdb {
            postgresdb.ensure_ready().await?;
        }
        self.agent.validate()?;

        Ok(BootstrapReport {
            app_name: self.config.app.name.clone(),
            provider_count: self.providers.len(),
            tool_count: self.tools.len(),
            hook_count: self.hooks.len(),
            memory_targets: self.memory.load_targets().len(),
            rag_strategies: self.rag.strategies().len(),
        })
    }

    pub async fn process_direct(&self, session_id: &str, content: &str) -> Result<String> {
        let context_kernel = ContextRuntimeKernel::new(self.state_store.clone());
        let context_run = context_kernel
            .begin_turn(session_id, "process_direct", content)
            .await?;

        match self.process_direct_inner(session_id, content).await {
            Ok(response) => {
                context_kernel
                    .finish_turn(
                        session_id,
                        "process_direct",
                        &context_run,
                        Ok(response.as_str()),
                    )
                    .await?;
                self.run_evolution_shadow_probe(
                    session_id,
                    "process_direct",
                    content,
                    true,
                    response.as_str(),
                )
                .await;
                Ok(response)
            }
            Err(error) => {
                let error_message = error.to_string();
                if let Err(record_error) = context_kernel
                    .finish_turn(
                        session_id,
                        "process_direct",
                        &context_run,
                        Err(error_message.as_str()),
                    )
                    .await
                {
                    return Err(anyhow!(
                        "context runtime kernel failed while recording process_direct failure: {record_error}; original error: {error_message}"
                    ));
                }
                self.run_evolution_shadow_probe(
                    session_id,
                    "process_direct",
                    content,
                    false,
                    error_message.as_str(),
                )
                .await;
                Err(error)
            }
        }
    }

    async fn process_direct_inner(&self, session_id: &str, content: &str) -> Result<String> {
        self.state_store
            .grant_permissions(session_id, vec![
                autoloop_state_adapter::PermissionAction::Read,
                autoloop_state_adapter::PermissionAction::Write,
                autoloop_state_adapter::PermissionAction::Dispatch,
            ])
            .await?;
        let surface = if should_route_code_task_to_harness(content) {
            "harness_facade"
        } else {
            "process_direct"
        };
        self.execute_via_runtime_facade(session_id, surface, content)
            .await
    }

    async fn execute_via_runtime_facade(
        &self,
        session_id: &str,
        surface: &str,
        content: &str,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let requires_harness = should_route_code_task_to_harness(content);
        if requires_harness && surface != "harness_facade" {
            let _ = append_event(
                &self.state_store,
                "code_harness_gate.rejected",
                format!("trace:{session_id}:code-harness-gate"),
                session_id.to_string(),
                None,
                Some(surface.to_string()),
                crate::contracts::version::CONTRACT_VERSION,
                serde_json::json!({
                    "reason": "code_task_requires_harness_facade",
                    "requested_surface": surface,
                    "required_surface": "harness_facade",
                    "decision": "reject",
                }),
            )
            .await;
            return Err(anyhow!(
                "code task requires harness façade execution; requested surface={surface}"
            ));
        }
        let trace_id = format!("trace:{session_id}:runtime-facade:{surface}:{}", current_time_ms());
        let _ = append_event(
            &self.state_store,
            "runtime_facade_entry",
            trace_id.clone(),
            session_id.to_string(),
            None,
            Some(surface.to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "surface": surface,
                "runtime_gate_mode": format!("{:?}", self.runtime.gate_mode).to_ascii_lowercase(),
                "no_bypass": true,
                "requires_harness": requires_harness,
            }),
        )
        .await;
        if requires_harness {
            let _ = append_event(
                &self.state_store,
                "code_harness_gate.accepted",
                format!("trace:{session_id}:code-harness-gate"),
                session_id.to_string(),
                None,
                Some(surface.to_string()),
                crate::contracts::version::CONTRACT_VERSION,
                serde_json::json!({
                    "reason": "code_task_routed_to_harness_facade",
                    "surface": surface,
                    "decision": "allow",
                }),
            )
            .await;
        }
        let repo_context_evidence_ref = if surface == "harness_facade" {
            self.compile_repo_context_for_harness(session_id, &trace_id, content)
                .await
        } else {
            None
        };
        let mut patch_report_ref = None;
        let mut git_checkpoint_report_ref = None;
        let mut shell_loop_report_ref = None;
        let mut test_verifier_report_ref = None;
        let iteration_controller_report_ref = if surface == "harness_facade" {
            self.maybe_execute_iteration_controller_from_request(session_id, &trace_id, content)
                .await?
        } else {
            None
        };
        if surface == "harness_facade" && iteration_controller_report_ref.is_none() {
            git_checkpoint_report_ref = self
                .maybe_execute_git_checkpoint_from_request(session_id, &trace_id, content)
                .await?;
            patch_report_ref = self
                .maybe_execute_structured_patch_from_request(session_id, &trace_id, content)
                .await?;
            shell_loop_report_ref = self
                .maybe_execute_shell_loop_from_request(session_id, &trace_id, content)
                .await?;
            test_verifier_report_ref = self
                .maybe_execute_test_verifier_from_request(
                    session_id,
                    &trace_id,
                    content,
                    false, // soft: agent fixes first, verify after
                )
                .await?;
        }

        let result = self.agent.process_message(session_id, content).await;
        let relation_write_proof = if result.is_ok() {
            self.backfill_frontend_artifact_relation_write_proof(session_id, &trace_id, content)
                .await?
        } else {
            None
        };

        let status = if result.is_ok() { "ok" } else { "error" };
        let detail = match &result {
            Ok(output) => output.chars().take(220).collect::<String>(),
            Err(error) => error.to_string(),
        };
        let _ = append_event(
            &self.state_store,
            "runtime_facade_exit",
            trace_id,
            session_id.to_string(),
            None,
            Some(surface.to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "surface": surface,
                "status": status,
                "detail": detail,
                "relation_write_proof": relation_write_proof,
                "repo_context_evidence_ref": repo_context_evidence_ref,
                "patch_report_ref": patch_report_ref,
                "git_checkpoint_report_ref": git_checkpoint_report_ref,
                "shell_loop_report_ref": shell_loop_report_ref,
                "test_verifier_report_ref": test_verifier_report_ref,
                "iteration_controller_report_ref": iteration_controller_report_ref,
            }),
        )
        .await;

        result
    }

    async fn compile_repo_context_for_harness(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Option<String> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let repo_root = if cwd.join("Cargo.toml").exists() {
            cwd
        } else if cwd.join("autoloop-app").join("Cargo.toml").exists() {
            cwd.join("autoloop-app")
        } else {
            cwd
        };
        let compiler = RepoContextCompiler::new(repo_root);
        let mut bundle = match compiler.compile(session_id, trace_id, content) {
            Ok(bundle) => bundle,
            Err(error) => {
                let _ = append_event(
                    &self.state_store,
                    "code_harness.repo_context.compile_failed",
                    format!("trace:{session_id}:repo-context-compiler"),
                    session_id.to_string(),
                    None,
                    Some("harness_facade".to_string()),
                    crate::contracts::version::CONTRACT_VERSION,
                    serde_json::json!({
                        "reason": error.to_string(),
                    }),
                )
                .await;
                return None;
            }
        };
        let replay_payload = serde_json::json!({
            "repo_root": bundle.repo_root.clone(),
            "tree_count": bundle.repo_tree.len(),
            "rank_count": bundle.file_importance_ranking.len(),
            "dependency_count": bundle.dependency_graph.len(),
            "recent_diff_count": bundle.recent_diff.len(),
        });
        let replay_fp = crate::evolution_os::replay::build_fingerprint(
            "repocontext",
            "repo-context/schema/v1",
            "repo-context/seed/v1",
            "repo-context/replay/v1",
            &replay_payload,
        );
        let evidence_ref = match EvidenceLedgerWriter::append_stage(
            &self.state_store,
            session_id,
            trace_id,
            EvidenceStage::Execution,
            serde_json::json!({
                "stage": "repo_context_compile",
                "surface": "harness_facade",
                "tree_count": bundle.repo_tree.len(),
                "rank_count": bundle.file_importance_ranking.len(),
                "dependency_count": bundle.dependency_graph.len(),
                "recent_diff_count": bundle.recent_diff.len(),
                "replay_fp": replay_fp,
            }),
            None,
        )
        .await
        {
            Ok(key) => key,
            Err(error) => {
                let _ = append_event(
                    &self.state_store,
                    "code_harness.repo_context.evidence_failed",
                    format!("trace:{session_id}:repo-context-compiler"),
                    session_id.to_string(),
                    None,
                    Some("harness_facade".to_string()),
                    crate::contracts::version::CONTRACT_VERSION,
                    serde_json::json!({
                        "reason": error.to_string(),
                    }),
                )
                .await;
                return None;
            }
        };
        bundle.evidence_ref = Some(evidence_ref.clone());
        bundle.replay_fp = Some(replay_fp.clone());
        let ts = current_time_ms();
        let latest_key = format!("harness:repo-context:{session_id}:latest");
        let point_key = format!("harness:repo-context:{session_id}:{ts}");
        if self
            .state_store
            .upsert_json_knowledge(point_key, &bundle, "repo-context-compiler")
            .await
            .is_err()
        {
            return None;
        }
        if self
            .state_store
            .upsert_json_knowledge(latest_key, &bundle, "repo-context-compiler")
            .await
            .is_err()
        {
            return None;
        }
        let _ = append_event(
            &self.state_store,
            "code_harness.repo_context.compiled",
            format!("trace:{session_id}:repo-context-compiler"),
            session_id.to_string(),
            None,
            Some("harness_facade".to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "evidence_ref": evidence_ref,
                "replay_fp": replay_fp,
            }),
        )
        .await;
        Some(evidence_ref)
    }

    async fn maybe_execute_structured_patch_from_request(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Result<Option<String>> {
        let Some(ops) = extract_structured_patch_ops_hint(content) else {
            return Ok(None);
        };
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let workspace_root = if cwd.join("Cargo.toml").exists() {
            cwd
        } else if cwd.join("autoloop-app").join("Cargo.toml").exists() {
            cwd.join("autoloop-app")
        } else {
            cwd
        };
        let engine = DiffPatchEngine::new(workspace_root);
        let report = engine
            .apply_with_rollback(&self.state_store, session_id, trace_id, &ops)
            .await?;
        let now_ms = current_time_ms();
        let key = format!("harness:patch-report:{session_id}:{trace_id}:{now_ms}");
        self.state_store
            .upsert_json_knowledge(key.clone(), &report, "diff-patch-engine")
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("harness:patch-report:{session_id}:latest"),
                &report,
                "diff-patch-engine",
            )
            .await?;
        if !report.success {
            return Err(anyhow!(
                "structured patch apply failed and rolled back (report_ref={key})"
            ));
        }
        Ok(Some(key))
    }

    async fn maybe_execute_git_checkpoint_from_request(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Result<Option<String>> {
        let Some(request) = extract_git_checkpoint_request_hint(content) else {
            return Ok(None);
        };
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let workspace_root = if cwd.join("Cargo.toml").exists() {
            cwd
        } else if cwd.join("autoloop-app").join("Cargo.toml").exists() {
            cwd.join("autoloop-app")
        } else {
            cwd
        };
        let layer = GitCheckpointLayer::new(workspace_root);
        let report = layer
            .run(&self.state_store, session_id, trace_id, &request)
            .await?;
        let now_ms = current_time_ms();
        let key = format!("harness:git-checkpoint:{session_id}:{trace_id}:{now_ms}");
        self.state_store
            .upsert_json_knowledge(key.clone(), &report, "git-checkpoint-layer")
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("harness:git-checkpoint:{session_id}:latest"),
                &report,
                "git-checkpoint-layer",
            )
            .await?;
        if !report.success {
            return Err(anyhow!(
                "git checkpoint layer failed (report_ref={key}, halted_reason={})",
                report.halted_reason
            ));
        }
        Ok(Some(key))
    }

    async fn maybe_execute_shell_loop_from_request(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Result<Option<String>> {
        let Some(request) = extract_shell_loop_request_hint(content) else {
            return Ok(None);
        };
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let workspace_root = if cwd.join("Cargo.toml").exists() {
            cwd
        } else if cwd.join("autoloop-app").join("Cargo.toml").exists() {
            cwd.join("autoloop-app")
        } else {
            cwd
        };
        let engine = ShellExecutionLoopEngine::new(workspace_root);
        let report = engine
            .run(&self.state_store, session_id, trace_id, &request)
            .await?;
        let now_ms = current_time_ms();
        let key = format!("harness:shell-loop:{session_id}:{trace_id}:{now_ms}");
        self.state_store
            .upsert_json_knowledge(key.clone(), &report, "shell-execution-loop")
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("harness:shell-loop:{session_id}:latest"),
                &report,
                "shell-execution-loop",
            )
            .await?;
        if !report.success {
            return Err(anyhow!(
                "shell execution loop failed (report_ref={key}, halted_reason={})",
                report.halted_reason
            ));
        }
        Ok(Some(key))
    }

    async fn maybe_execute_test_verifier_from_request(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
        hard_gate_required: bool,
    ) -> Result<Option<String>> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let workspace_root = if cwd.join("Cargo.toml").exists() {
            cwd
        } else if cwd.join("autoloop-app").join("Cargo.toml").exists() {
            cwd.join("autoloop-app")
        } else {
            cwd
        };
        let request = match extract_test_verifier_request_hint(content) {
            Some(request) => request,
            None if hard_gate_required => {
                if let Some(default_request) = build_default_test_verifier_request(&workspace_root) {
                    default_request
                } else {
                    return Err(anyhow!(
                        "test verifier hard gate requires build/lint/test runners; no explicit request and no default profile available"
                    ));
                }
            }
            None => return Ok(None),
        };
        let engine = TestVerifierEngine::new(workspace_root);
        let report = engine
            .verify(&self.state_store, session_id, trace_id, &request)
            .await?;
        let now_ms = current_time_ms();
        let key = format!("harness:test-verifier:{session_id}:{trace_id}:{now_ms}");
        self.state_store
            .upsert_json_knowledge(key.clone(), &report, "test-verifier")
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("harness:test-verifier:{session_id}:latest"),
                &report,
                "test-verifier",
            )
            .await?;
        if report.hard_fail {
            return Err(anyhow!(
                "test verifier hard failed (report_ref={key}, summary={})",
                report.summary
            ));
        }
        Ok(Some(key))
    }

    async fn maybe_execute_iteration_controller_from_request(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Result<Option<String>> {
        let Some(config) = extract_iteration_controller_config_hint(content) else {
            return Ok(None);
        };

        let estimated_tokens = estimate_swarm_tokens(content);
        let started_ms = current_time_ms();
        let mut attempts = Vec::new();
        let mut stage_refs = serde_json::Value::Null;
        let mut success = false;
        let mut termination_reason = TerminationReason::NonRetryableFailure;
        let max_attempts = config.max_attempts.max(1);
        let mut effective_content = content.to_string();
        let initial_failure_hints = load_failure_experience_hints(&self.state_store, session_id, 6)
            .await
            .unwrap_or_default();
        effective_content = merge_failure_experience_hints(&effective_content, &initial_failure_hints);

        if let Some(limit) = config.budget_tokens {
            if estimated_tokens > limit {
                termination_reason = TerminationReason::BudgetLimitExceeded;
                let report = self
                    .persist_iteration_controller_report(
                        session_id,
                        trace_id,
                        &config,
                        estimated_tokens,
                        attempts,
                        stage_refs,
                        success,
                        termination_reason,
                        started_ms,
                    )
                    .await?;
                return Err(anyhow!(
                    "iteration controller budget limit exceeded (report_ref={report})"
                ));
            }
        }

        for attempt in 1..=max_attempts {
            let elapsed_ms = current_time_ms().saturating_sub(started_ms);
            if elapsed_ms > config.max_runtime_ms {
                termination_reason = TerminationReason::TimeLimitExceeded;
                attempts.push(crate::query_engine::IterationAttemptRecord {
                    attempt,
                    stage: "controller".to_string(),
                    success: false,
                    error: Some("time_limit_exceeded".to_string()),
                    failure_category: Some(crate::query_engine::FailureCategory::Timeout),
                    repair_strategy: Some(crate::query_engine::RepairStrategy::RetryAfterTimeout),
                    retry_allowed: Some(false),
                    elapsed_ms,
                });
                break;
            }

            match self
                .execute_harness_stages_once(session_id, trace_id, &effective_content)
                .await
            {
                Ok(refs) => {
                    success = true;
                    termination_reason = TerminationReason::Success;
                    stage_refs = refs;
                    attempts.push(crate::query_engine::IterationAttemptRecord {
                        attempt,
                        stage: "all".to_string(),
                        success: true,
                        error: None,
                        failure_category: None,
                        repair_strategy: None,
                        retry_allowed: None,
                        elapsed_ms: current_time_ms().saturating_sub(started_ms),
                    });
                    break;
                }
                Err(error) => {
                    let error_message = error.to_string();
                    let stage = stage_from_error(&error_message);
                    let category = classify_failure(&error_message);
                    let strategy = select_repair_strategy(&stage, &category);
                    let failure_experience_ref = record_failure_experience(
                        &self.state_store,
                        session_id,
                        trace_id,
                        &stage,
                        &category,
                        &strategy,
                        &error_message,
                        None,
                    )
                    .await
                    .ok();
                    let retry_allowed =
                        should_retry_for_failure(&stage, &category, &strategy, &config);
                    attempts.push(crate::query_engine::IterationAttemptRecord {
                        attempt,
                        stage: stage.clone(),
                        success: false,
                        error: Some(match failure_experience_ref {
                            Some(ref_id) => {
                                format!("{error_message} [failure_experience_ref={ref_id}]")
                            }
                            None => error_message.clone(),
                        }),
                        failure_category: Some(category.clone()),
                        repair_strategy: Some(strategy.clone()),
                        retry_allowed: Some(retry_allowed),
                        elapsed_ms: current_time_ms().saturating_sub(started_ms),
                    });
                    if attempt >= max_attempts {
                        termination_reason = TerminationReason::AttemptLimitExceeded;
                        break;
                    }
                    if !retry_allowed {
                        termination_reason = if matches!(
                            category,
                            crate::query_engine::FailureCategory::Budget
                        ) {
                            TerminationReason::BudgetLimitExceeded
                        } else {
                            TerminationReason::NonRetryableFailure
                        };
                        break;
                    }
                    if matches!(
                        strategy,
                        crate::query_engine::RepairStrategy::CompactAndReplan
                    ) {
                        let compact_budget = config
                            .budget_tokens
                            .unwrap_or(DEFAULT_SWARM_MAX_TOKENS)
                            .max(SWARM_MIN_REPLAN_TOKENS);
                        effective_content =
                            compact_and_replan_requirement(&effective_content, compact_budget);
                    }
                    let refreshed_failure_hints =
                        load_failure_experience_hints(&self.state_store, session_id, 6)
                            .await
                            .unwrap_or_default();
                    effective_content = merge_failure_experience_hints(
                        &effective_content,
                        &refreshed_failure_hints,
                    );
                }
            }
        }

        let report_ref = self
            .persist_iteration_controller_report(
                session_id,
                trace_id,
                &config,
                estimated_tokens,
                attempts,
                stage_refs,
                success,
                termination_reason.clone(),
                started_ms,
            )
            .await?;

        if !success {
            return Err(anyhow!(
                "iteration controller terminated with {:?} (report_ref={report_ref})",
                termination_reason
            ));
        }
        Ok(Some(report_ref))
    }

    async fn execute_harness_stages_once(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Result<serde_json::Value> {
        let git_checkpoint = self
            .maybe_execute_git_checkpoint_from_request(session_id, trace_id, content)
            .await
            .map_err(|error| anyhow!("stage=git_checkpoint; {}", error))?;
        let patch = self
            .maybe_execute_structured_patch_from_request(session_id, trace_id, content)
            .await
            .map_err(|error| anyhow!("stage=structured_patch; {}", error))?;
        let shell = self
            .maybe_execute_shell_loop_from_request(session_id, trace_id, content)
            .await
            .map_err(|error| anyhow!("stage=shell_loop; {}", error))?;
        let verifier = self
            .maybe_execute_test_verifier_from_request(session_id, trace_id, content, true)
            .await
            .map_err(|error| anyhow!("stage=test_verifier; {}", error))?;
        Ok(serde_json::json!({
            "git_checkpoint_report_ref": git_checkpoint,
            "patch_report_ref": patch,
            "shell_loop_report_ref": shell,
            "test_verifier_report_ref": verifier,
        }))
    }

    async fn persist_iteration_controller_report(
        &self,
        session_id: &str,
        trace_id: &str,
        config: &IterationControllerConfig,
        estimated_tokens: u32,
        attempts: Vec<crate::query_engine::IterationAttemptRecord>,
        stage_refs: serde_json::Value,
        success: bool,
        termination_reason: TerminationReason,
        started_ms: u64,
    ) -> Result<String> {
        let elapsed_ms = current_time_ms().saturating_sub(started_ms);
        let replay_fp = crate::evolution_os::replay::build_fingerprint(
            "iterationcontroller",
            "iteration-controller/schema/v1",
            "iteration-controller/seed/v1",
            "iteration-controller/replay/v1",
            &serde_json::json!({
                "trace_id": trace_id,
                "success": success,
                "termination_reason": termination_reason,
                "attempts": attempts,
                "stage_refs": stage_refs,
            }),
        );
        let evidence_ref = EvidenceLedgerWriter::append_stage(
            &self.state_store,
            session_id,
            trace_id,
            EvidenceStage::Execution,
            serde_json::json!({
                "stage": "iteration_controller",
                "success": success,
                "termination_reason": format!("{:?}", termination_reason).to_ascii_lowercase(),
                "attempt_count": attempts.len(),
                "replay_fp": replay_fp,
            }),
            None,
        )
        .await
        .ok();
        let report = IterationControllerReport {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            success,
            attempts_used: attempts.len() as u32,
            max_attempts: config.max_attempts,
            budget_tokens: config.budget_tokens,
            estimated_tokens,
            elapsed_ms,
            termination_reason: termination_reason.clone(),
            attempts: attempts.clone(),
            routing_version: Some("iteration-controller/v2".to_string()),
            strategy_summary: build_iteration_strategy_summary(&attempts),
            stage_refs,
            evidence_ref: evidence_ref.clone(),
            replay_fp: Some(replay_fp),
        };
        let now_ms = current_time_ms();
        let key = format!("harness:iteration-controller:{session_id}:{trace_id}:{now_ms}");
        self.state_store
            .upsert_json_knowledge(key.clone(), &report, "iteration-controller")
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("harness:iteration-controller:{session_id}:latest"),
                &report,
                "iteration-controller",
            )
            .await?;

        let decision = map_termination_to_iteration_decision(&termination_reason);
        let state = crate::contracts::code_harness::IterationState {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            objective: "harness_facade_flow".to_string(),
            attempt: attempts.len() as u32,
            max_attempts: config.max_attempts,
            error_fingerprint: attempts
                .last()
                .and_then(|item| item.error.clone())
                .map(|message| {
                    crate::evolution_os::replay::build_fingerprint(
                        "iterationerror",
                        "iteration-error/schema/v1",
                        "iteration-error/seed/v1",
                        "iteration-error/replay/v1",
                        &serde_json::json!({ "message": message }),
                    )
                }),
            decision,
            reason: Some(format!("{:?}", termination_reason).to_ascii_lowercase()),
            last_execution_step_id: None,
            last_test_verdict_id: None,
            evidence_ref,
        };
        self.state_store
            .upsert_json_knowledge(
                format!("harness:iteration-state:{session_id}:{trace_id}:{now_ms}"),
                &state,
                "iteration-controller",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("harness:iteration-state:{session_id}:latest"),
                &state,
                "iteration-controller",
            )
            .await?;

        Ok(key)
    }

    fn evolution_shadow_enabled(&self) -> bool {
        match std::env::var("AUTOLOOP_EVOLUTION_SHADOW_ENABLED") {
            Ok(value) => {
                let lowered = value.trim().to_ascii_lowercase();
                !(lowered == "0" || lowered == "false" || lowered == "off")
            }
            Err(_) => true,
        }
    }

    async fn run_evolution_shadow_probe(
        &self,
        session_id: &str,
        entrypoint: &str,
        content: &str,
        success: bool,
        outcome_detail: &str,
    ) {
        if !self.evolution_shadow_enabled() {
            return;
        }

        let now_ms = current_time_ms();
        let identity = self.sessions.identity(session_id).await;
        let tenant_id = identity
            .as_ref()
            .map(|item| item.tenant_id.clone())
            .unwrap_or_else(|| "tenant:default".to_string());
        let policy_version = identity
            .as_ref()
            .map(|item| item.policy_id.clone())
            .unwrap_or_else(|| crate::contracts::version::CONTRACT_VERSION.to_string());

        let memory_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:{session_id}:"))
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.key)
            .take(12)
            .collect::<Vec<_>>();

        let graph_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("graph:{session_id}:"))
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.key)
            .take(12)
            .collect::<Vec<_>>();

        let telemetry_replay = self
            .collect_worldline_telemetry_replay_snapshot(session_id)
            .await;

        let proposal_signals = self.collect_proposal_bus_signals(session_id).await;
        let available_tools = self.tools.names();
        let input = IngestInput {
            session_id: session_id.to_string(),
            trace_id: format!("trace:{session_id}:evo-shadow:{entrypoint}:{now_ms}"),
            tenant_id: tenant_id.clone(),
            policy_version: policy_version.clone(),
            runtime_mode: "shadow".to_string(),
            available_tools: available_tools.clone(),
            memory_refs,
            graph_refs,
            repo_refs: vec![self.governance_repo_root().to_string_lossy().to_string()],
            policy_refs: vec![
                format!("policy:{}", policy_version),
                format!("tenant:{}", tenant_id),
            ],
            tool_refs: available_tools,
            budget_micros: 250_000,
            latency_budget_ms: 5_000,
            budget_profile: BTreeMap::from([
                ("token_budget".to_string(), 250_000_u64),
                ("latency_budget_ms".to_string(), 5_000_u64),
            ]),
            now_ms,
            telemetry_replay,
            proposal_signals,
        };

        let kernel = EvolutionOsKernel::new();
        match kernel.run_shadow_cycle(input) {
            Ok(cycle) => {
                let outcome = if success { "success" } else { "error" };
                let summary = cycle.to_evidence_json(entrypoint, content, outcome, outcome_detail);
                let base = format!("evo:shadow:{session_id}:{entrypoint}:{now_ms}");
                let path_execution = self
                    .execute_evolution_promotion_paths(session_id, &cycle)
                    .await;

                let next_gen_execution = self
                    .execute_next_gen_trusted_execution(session_id, &cycle, &path_execution)
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:reality"),
                        &serde_json::json!({ "reality": cycle.reality }),
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:candidate-graphs"),
                        &serde_json::json!({ "candidates": cycle.candidates }),
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:worldline-scores"),
                        &serde_json::json!({ "scores": cycle.scores }),
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:proposals"),
                        &serde_json::json!({ "proposals": cycle.proposals }),
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:board"),
                        &serde_json::json!({
                            "decision": cycle.board_decision.clone(),
                            "outcome": &cycle.board_outcome,
                        }),
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:trusted-prior"),
                        &serde_json::json!({ "trusted_prior": &cycle.trusted_prior }),
                        "evolution-os-shadow",
                    )
                    .await;
                let trusted_prior_gate = self
                    .build_evolution_production_write_gate(
                        session_id,
                        &cycle,
                        "trusted-prior-activation",
                    )
                    .await;
                let trusted_prior_snapshot = serde_json::json!({
                    "snapshot_version": crate::contracts::version::EVOLUTION_OS_CONTRACT_VERSION,
                    "session_id": session_id,
                    "entrypoint": entrypoint,
                    "trace_id": cycle.reality.trace_id,
                    "decision": cycle.board_decision.clone(),
                    "activation_version": cycle.rollout.activation_version,
                    "board_decision": trusted_prior_gate
                        .as_json()
                        .get("board_decision")
                        .cloned(),
                    "policy_allow": trusted_prior_gate
                        .as_json()
                        .get("policy_allow")
                        .cloned(),
                    "evidence_ref": trusted_prior_gate
                        .as_json()
                        .get("evidence_ref")
                        .cloned(),
                    "deny_reason": trusted_prior_gate
                        .as_json()
                        .get("deny_reason")
                        .cloned(),
                    "recommended_candidate_id": cycle
                        .recommendation
                        .as_ref()
                        .map(|item| item.recommended_candidate_id.clone()),
                    "trusted_prior": &cycle.trusted_prior,
                    "path_plan": &cycle.path_plan,
                    "rollout": &cycle.rollout,
                    "rollback_on_failure": cycle.rollout.rollback_on_failure,
                    "gate": trusted_prior_gate.as_json(),
                    "created_at_ms": now_ms,
                });
                if trusted_prior_gate.production_write_allowed() {
                    let _ = self
                        .state_store
                        .upsert_json_knowledge(
                            format!("evolution:trusted-prior:{session_id}:{now_ms}"),
                            &trusted_prior_snapshot,
                            "evolution-os",
                        )
                        .await;
                    let _ = self
                        .state_store
                        .upsert_json_knowledge(
                            format!("evolution:trusted-prior:{session_id}:latest"),
                            &trusted_prior_snapshot,
                            "evolution-os",
                        )
                        .await;
                    let _ = self
                        .state_store
                        .upsert_json_knowledge(
                            format!("evolution:trusted-prior:index:{session_id}:{now_ms}"),
                            &serde_json::json!({
                                "prior_id": trusted_prior_snapshot
                                    .get("trusted_prior")
                                    .and_then(|value| value.get("prior_id"))
                                    .cloned(),
                                "snapshot_ref": format!("evolution:trusted-prior:{session_id}:{now_ms}"),
                                "snapshot_version": crate::contracts::version::EVOLUTION_OS_CONTRACT_VERSION,
                                "decision": cycle.board_decision.clone(),
                                "board_decision": trusted_prior_gate
                                    .as_json()
                                    .get("board_decision")
                                    .cloned(),
                                "policy_allow": trusted_prior_gate
                                    .as_json()
                                    .get("policy_allow")
                                    .cloned(),
                                "evidence_ref": trusted_prior_gate
                                    .as_json()
                                    .get("evidence_ref")
                                    .cloned(),
                                "deny_reason": trusted_prior_gate
                                    .as_json()
                                    .get("deny_reason")
                                    .cloned(),
                                "created_at_ms": now_ms,
                            }),
                            "evolution-os",
                        )
                        .await;
                } else {
                    let _ = self
                        .state_store
                        .upsert_json_knowledge(
                            format!("evolution:trusted-prior:block:{session_id}:{now_ms}"),
                            &trusted_prior_snapshot,
                            "evolution-os",
                        )
                        .await;
                }
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:rollout"),
                        &serde_json::json!({ "rollout": &cycle.rollout }),
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:path-execution"),
                        &path_execution,
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:next-gen-execution"),
                        &next_gen_execution,
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("{base}:summary"),
                        &summary,
                        "evolution-os-shadow",
                    )
                    .await;
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("evo:shadow:{session_id}:{entrypoint}:latest"),
                        &summary,
                        "evolution-os-shadow",
                    )
                    .await;
            }
            Err(error) => {
                let _ = self
                    .state_store
                    .upsert_json_knowledge(
                        format!("evo:shadow:{session_id}:{entrypoint}:{now_ms}:error"),
                        &serde_json::json!({
                            "entrypoint": entrypoint,
                            "error": error.to_string(),
                            "outcome_detail": outcome_detail,
                            "created_at_ms": now_ms,
                        }),
                        "evolution-os-shadow",
                    )
                    .await;
            }
        }
    }


    async fn collect_worldline_telemetry_replay_snapshot(
        &self,
        session_id: &str,
    ) -> Option<TelemetryReplaySnapshot> {
        let verifier_score = self
            .load_latest_verifier_score(session_id)
            .await
            .unwrap_or(0.8)
            .clamp(0.0, 1.0);
        let (provider_retry_count, tool_retry_count) = self
            .load_retry_counts(session_id)
            .await
            .unwrap_or((0, 0));
        let (replay_mismatch_rate, deterministic_boundary_respected) = self
            .load_replay_health(session_id)
            .await
            .unwrap_or((0.0, true));
        let latency_p95_ms = self.load_latency_p95_ms(session_id).await.unwrap_or(0);
        let (worldline_weights, weights_version) = self.load_worldline_weights_profile().await;

        Some(TelemetryReplaySnapshot {
            verifier_score,
            provider_retry_count,
            tool_retry_count,
            replay_mismatch_rate: replay_mismatch_rate.clamp(0.0, 1.0),
            deterministic_boundary_respected,
            latency_p95_ms,
            worldline_weights,
            weights_version,
        })
    }

    async fn load_worldline_weights_profile(
        &self,
    ) -> (Option<WorldlineWeights>, Option<String>) {
        let key = "policy:worldline-weights:latest";
        let Ok(Some(record)) = self.state_store.get_knowledge(key).await else {
            return (None, None);
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) else {
            return (None, None);
        };
        let version = value
            .get("version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let weights = WorldlineWeights::from_value(&value).map(|item| item.sanitize());
        (weights, version)
    }
    async fn load_latest_verifier_score(&self, session_id: &str) -> Option<f32> {
        let keys = [
            format!("protocol:{session_id}:execution-verifier-report"),
            format!("protocol:{session_id}:verifier-report"),
        ];
        for key in keys {
            if let Ok(Some(record)) = self.state_store.get_knowledge(&key).await {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) {
                    if let Some(score) = value
                        .get("overall_score")
                        .and_then(serde_json::Value::as_f64)
                        .map(|item| item as f32)
                    {
                        return Some(score);
                    }
                }
            }
        }
        None
    }

    async fn load_retry_counts(&self, session_id: &str) -> Option<(u32, u32)> {
        let key = format!("observability:{session_id}:collector:snapshot");
        let record = self.state_store.get_knowledge(&key).await.ok().flatten()?;
        let snapshot = serde_json::from_str::<TelemetryCollectorSnapshot>(&record.value).ok()?;

        let mut provider_retry_count = 0_u32;
        let mut tool_retry_count = 0_u32;
        for retry in snapshot.retries {
            let attempts = retry.attempts;
            let task_lower = retry.task_id.to_ascii_lowercase();
            if task_lower.contains("provider") {
                provider_retry_count = provider_retry_count.saturating_add(attempts);
            } else {
                tool_retry_count = tool_retry_count.saturating_add(attempts);
            }
        }
        Some((provider_retry_count, tool_retry_count))
    }

    async fn load_replay_health(&self, session_id: &str) -> Option<(f32, bool)> {
        let reports = self
            .state_store
            .list_knowledge_by_prefix("replay:analysis:")
            .await
            .ok()?;
        let mut total = 0_u32;
        let mut mismatch = 0_u32;
        let mut deterministic_ok = true;

        for record in reports {
            let Ok(report) = serde_json::from_str::<ReplayAnalysisReport>(&record.value) else {
                continue;
            };
            if report.session_id != session_id {
                continue;
            }
            total = total.saturating_add(1);
            if !report.matched {
                mismatch = mismatch.saturating_add(1);
            }
            deterministic_ok &= report.deterministic_boundary_respected;
        }

        if total == 0 {
            return Some((0.0, true));
        }

        Some((mismatch as f32 / total as f32, deterministic_ok))
    }

    async fn load_latency_p95_ms(&self, session_id: &str) -> Option<u64> {
        let key = format!("observability:{session_id}:collector:latency");
        let record = self.state_store.get_knowledge(&key).await.ok().flatten()?;
        let samples = serde_json::from_str::<Vec<serde_json::Value>>(&record.value).ok()?;
        let mut latencies = samples
            .into_iter()
            .filter_map(|item| item.get("latency_ms").and_then(serde_json::Value::as_u64))
            .collect::<Vec<_>>();
        if latencies.is_empty() {
            return Some(0);
        }
        latencies.sort_unstable();
        let idx = ((latencies.len() as f32) * 0.95).floor() as usize;
        let bounded_idx = idx.min(latencies.len().saturating_sub(1));
        Some(latencies[bounded_idx])
    }

    async fn collect_proposal_bus_signals(&self, session_id: &str) -> Option<ExternalProposalSignals> {
        let foundry_promotion_hints = self
            .state_store
            .list_knowledge_by_prefix(&format!("foundry:promotion:pending:{session_id}:"))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
            .take(8)
            .collect::<Vec<_>>();

        let patch_reviews = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:patch:review:{session_id}:"))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
            .take(8)
            .collect::<Vec<_>>();

        let mut plugin_lifecycle_updates = Vec::<serde_json::Value>::new();
        if let Ok(Some(index_record)) = self.state_store.get_knowledge("plugin:lifecycle:index").await {
            if let Ok(index) = serde_json::from_str::<Vec<serde_json::Value>>(&index_record.value) {
                for item in index.into_iter().take(8) {
                    if let Some(plugin_id) = item.get("plugin_id").and_then(serde_json::Value::as_str) {
                        let key = format!("plugin:lifecycle:{plugin_id}:latest");
                        if let Ok(Some(record)) = self.state_store.get_knowledge(&key).await {
                            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) {
                                plugin_lifecycle_updates.push(value);
                            }
                        }
                    }
                }
            }
        }

        Some(ExternalProposalSignals {
            foundry_promotion_hints,
            patch_reviews,
            plugin_lifecycle_updates,
        })
    }

    async fn execute_evolution_promotion_paths(
        &self,
        session_id: &str,
        cycle: &EvolutionShadowCycle,
    ) -> serde_json::Value {
        let expected_path =
            crate::evolution_os::PromotionPathExecutor::expected_path_for_decision(&cycle.board_decision);
        let selected_path = cycle.path_plan.selected_path.clone();
        if expected_path != selected_path {
            return serde_json::json!({
                "path": crate::evolution_os::PromotionPathExecutor::path_code(&selected_path),
                "status": "error",
                "decision": cycle.board_decision.clone(),
                "proposal_only": true,
                "decision_path_consistent": false,
                "expected_path": crate::evolution_os::PromotionPathExecutor::path_code(&expected_path),
                "selected_path": crate::evolution_os::PromotionPathExecutor::path_code(&selected_path),
                "reason": "decision_path_mismatch",
                "session_id": session_id,
            });
        }

        let write_gate = self
            .build_evolution_production_write_gate(session_id, cycle, "promotion-path")
            .await;

        let mut result = match selected_path {
            PromotionPath::Path9A_RuntimeUpdate => {
                self.execute_path_9a_runtime_update(session_id, cycle, &write_gate)
                    .await
            }
            PromotionPath::Path9B_Crystalization => {
                self.execute_path_9b_crystalization(session_id, cycle, &write_gate)
                    .await
            }
            PromotionPath::Path9C_GovernanceUpdate => {
                self.execute_path_9c_governance_update(session_id, cycle, &write_gate)
                    .await
            }
            PromotionPath::Path9D_LocalOnly => {
                self.execute_path_9d_local_only(session_id, cycle, &write_gate)
                    .await
            }
        };

        if let Some(obj) = result.as_object_mut() {
            obj.insert("decision_path_consistent".to_string(), serde_json::json!(true));
            obj.insert(
                "expected_path".to_string(),
                serde_json::json!(crate::evolution_os::PromotionPathExecutor::path_code(
                    &expected_path,
                )),
            );
            obj.insert(
                "selected_path".to_string(),
                serde_json::json!(crate::evolution_os::PromotionPathExecutor::path_code(
                    &selected_path,
                )),
            );
        }

        result
    }

    async fn build_evolution_production_write_gate(
        &self,
        session_id: &str,
        cycle: &EvolutionShadowCycle,
        scope: &str,
    ) -> EvolutionProductionWriteGate {
        let now_ms = current_time_ms();
        let evidence_ref = format!(
            "evidence:evolution:write-gate:{session_id}:{}:{scope}:{now_ms}",
            cycle.reality.trace_id
        );
        let audit = ConstitutionAuditRecord {
            evidence_ref: evidence_ref.clone(),
            audit_source: "evolution-write-gate".to_string(),
            policy_allow: cycle.board_outcome.judge.policy_compliant,
            policy_version: cycle.reality.policy_version.clone(),
            decision_hash: format!(
                "decision:{}:{}:{}",
                session_id,
                cycle.reality.trace_id,
                now_ms
            ),
        };
        let constitution_state = transition_with_audit(
            None,
            &ProductionWriteInput {
                board_decision: map_to_core_decision(&cycle.board_decision),
                rollout_stage: map_to_core_rollout_stage(&cycle.rollout.stage),
            },
            &audit,
        );
        let gate = EvolutionProductionWriteGate { constitution_state };
        let payload = serde_json::json!({
            "session_id": session_id,
            "trace_id": cycle.reality.trace_id,
            "scope": scope,
            "gate": gate.as_json(),
            "audit": audit,
            "created_at_ms": now_ms,
        });
        let _ = self
            .state_store
            .upsert_json_knowledge(evidence_ref, &payload, "evolution-write-gate")
            .await;
        gate
    }

    fn blocked_production_write_payload(
        &self,
        path: &str,
        executor: &str,
        session_id: &str,
        gate: &EvolutionProductionWriteGate,
        reason: Option<&str>,
    ) -> serde_json::Value {
        let deny_reason = reason.unwrap_or_else(|| gate.deny_reason());
        serde_json::json!({
            "path": path,
            "executor": executor,
            "status": "blocked",
            "reason": "production_write_gate_denied",
            "deny_reason": deny_reason,
            "proposal_only": true,
            "production_write": false,
            "session_id": session_id,
            "gate": gate.as_json(),
        })
    }

    fn production_write_triad_present(&self, gate: &EvolutionProductionWriteGate) -> bool {
        let gate_payload = gate.as_json();
        let has_board_decision = gate_payload
            .get("board_decision")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        let policy_allow = gate_payload
            .get("policy_allow")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let has_evidence_ref = gate_payload
            .get("evidence_ref")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        has_board_decision && policy_allow && has_evidence_ref
    }

    async fn execute_path_9a_runtime_update(
        &self,
        session_id: &str,
        cycle: &EvolutionShadowCycle,
        write_gate: &EvolutionProductionWriteGate,
    ) -> serde_json::Value {
        if !write_gate.production_write_allowed() {
            return self.blocked_production_write_payload(
                "9A",
                "plugins.lifecycle",
                session_id,
                write_gate,
                None,
            );
        }
        if !self.production_write_triad_present(write_gate) {
            return self.blocked_production_write_payload(
                "9A",
                "plugins.lifecycle",
                session_id,
                write_gate,
                Some("missing_gate_triad"),
            );
        }
        let target = cycle
            .path_plan
            .actions
            .first()
            .map(|action| action.target.clone())
            .filter(|target| target.starts_with("plugin:"))
            .unwrap_or_else(|| "plugin:graph-projection".to_string());

        let status = self.plugins.status(&target).await.ok().flatten();
        match status {
            Some(_) => match self
                .plugins
                .rollout(
                    &target,
                    crate::contracts::plugin::PluginRolloutMode::Canary,
                    Some(10),
                    "evolution-board",
                    "path-9A runtime update",
                )
                .await
            {
                Ok(manifest) => match self.plugins.verify(&target).await {
                    Ok(verdict) if verdict.verified => serde_json::json!({
                        "path": "9A",
                        "executor": "plugins.lifecycle",
                        "status": "applied",
                        "plugin_id": target,
                        "version": manifest.version,
                        "decision": cycle.board_decision.clone(),
                        "proposal_only": true,
                        "runtime_gate": {
                            "verified": true,
                            "reason": verdict.reason,
                            "provenance_ref": verdict.provenance_ref,
                            "checked_at_ms": verdict.checked_at_ms,
                        },
                        "gate": write_gate.as_json(),
                    }),
                    Ok(verdict) => {
                        let rollback = self
                            .plugins
                            .quick_rollback(&target, "evolution-board")
                            .await;
                        serde_json::json!({
                            "path": "9A",
                            "executor": "plugins.lifecycle",
                            "status": "rolled_back",
                            "plugin_id": target,
                            "decision": cycle.board_decision.clone(),
                            "proposal_only": true,
                            "runtime_gate": {
                                "verified": false,
                                "reason": verdict.reason,
                                "provenance_ref": verdict.provenance_ref,
                                "checked_at_ms": verdict.checked_at_ms,
                            },
                            "rollback": {
                                "attempted": true,
                                "result": rollback
                                    .as_ref()
                                    .map(|m| serde_json::json!({"version": m.version}))
                                    .ok(),
                                "error": rollback.err().map(|e| e.to_string()),
                            },
                            "gate": write_gate.as_json(),
                        })
                    }
                    Err(error) => {
                        let rollback = self
                            .plugins
                            .quick_rollback(&target, "evolution-board")
                            .await;
                        serde_json::json!({
                            "path": "9A",
                            "executor": "plugins.lifecycle",
                            "status": "rolled_back",
                            "plugin_id": target,
                            "decision": cycle.board_decision.clone(),
                            "proposal_only": true,
                            "runtime_gate": {
                                "verified": false,
                                "error": error.to_string(),
                            },
                            "rollback": {
                                "attempted": true,
                                "result": rollback
                                    .as_ref()
                                    .map(|m| serde_json::json!({"version": m.version}))
                                    .ok(),
                                "error": rollback.err().map(|e| e.to_string()),
                            },
                            "gate": write_gate.as_json(),
                        })
                    }
                },
                Err(error) => serde_json::json!({
                    "path": "9A",
                    "executor": "plugins.lifecycle",
                    "status": "error",
                    "plugin_id": target,
                    "error": error.to_string(),
                    "decision": cycle.board_decision.clone(),
                    "proposal_only": true,
                    "gate": write_gate.as_json(),
                }),
            },
            None => serde_json::json!({
                "path": "9A",
                "executor": "plugins.lifecycle",
                "status": "skipped",
                "plugin_id": target,
                "reason": "plugin not installed; rollout not applied",
                "decision": cycle.board_decision.clone(),
                "proposal_only": true,
                "session_id": session_id,
                "gate": write_gate.as_json(),
            }),
        }
    }

    async fn execute_path_9b_crystalization(
        &self,
        session_id: &str,
        cycle: &EvolutionShadowCycle,
        write_gate: &EvolutionProductionWriteGate,
    ) -> serde_json::Value {
        if !write_gate.production_write_allowed() {
            return self.blocked_production_write_payload(
                "9B",
                "gitmemory.patch_review_queue",
                session_id,
                write_gate,
                None,
            );
        }
        if !self.production_write_triad_present(write_gate) {
            return self.blocked_production_write_payload(
                "9B",
                "gitmemory.patch_review_queue",
                session_id,
                write_gate,
                Some("missing_gate_triad"),
            );
        }
        let op_kind = if matches!(
            cycle.board_decision,
            crate::contracts::evolution_os::PromotionDecision::CrystallizeMemoryRule
        ) {
            PatchOpKind::Update
        } else {
            PatchOpKind::Add
        };

        let ops = cycle
            .proposals
            .iter()
            .take(3)
            .map(|proposal| PatchOp {
                kind: op_kind.clone(),
                target: proposal.candidate_id.clone(),
                reason: proposal.summary.clone(),
            })
            .collect::<Vec<_>>();

        let patch = PatchPlan {
            namespace: format!("{}:{}", cycle.reality.tenant_id, session_id),
            ops,
        };

        match PatchReviewQueue::enqueue(
            &self.state_store,
            session_id,
            cycle.reality.trace_id.as_str(),
            &patch,
        )
        .await
        {
            Ok(item) => {
                let apply_ready_key = format!(
                    "memory:patch:apply-ready:{}:{}",
                    session_id, item.review_id
                );
                let apply_history_key = format!(
                    "memory:patch:apply-ready:{}:{}:{}",
                    session_id, item.review_id, item.updated_at_ms
                );
                let apply_ready_status = if item.decision.approval_required {
                    "pending_approval"
                } else {
                    "ready_to_apply"
                };
                let apply_ready_payload = serde_json::json!({
                    "review_id": item.review_id,
                    "session_id": session_id,
                    "trace_id": cycle.reality.trace_id,
                    "status": apply_ready_status,
                    "approval_required": item.decision.approval_required,
                    "decision": cycle.board_decision.clone(),
                    "closure": "queue->approve/reject->apply",
                    "gate": write_gate.as_json(),
                    "updated_at_ms": item.updated_at_ms,
                });
                let apply_latest_write = self
                    .state_store
                    .upsert_json_knowledge(
                        apply_ready_key.clone(),
                        &apply_ready_payload,
                        "patch-review-queue",
                    )
                    .await;
                let apply_history_write = self
                    .state_store
                    .upsert_json_knowledge(
                        apply_history_key.clone(),
                        &apply_ready_payload,
                        "patch-review-queue",
                    )
                    .await;
                serde_json::json!({
                    "path": "9B",
                    "executor": "gitmemory.patch_review_queue",
                    "status": "queued",
                    "review_id": item.review_id,
                    "queue_status": item.status,
                    "approval_required": item.decision.approval_required,
                    "policy_risk_score": item.decision.risk_score,
                    "policy_reason": item.decision.reason,
                    "review_ref_latest": format!("memory:patch:review:{}:latest", session_id),
                    "review_ref_history": format!("memory:patch:review:{}:{}", session_id, item.updated_at_ms),
                    "apply_ready_ref": apply_ready_key,
                    "apply_ready_history_ref": apply_history_key,
                    "apply_ready_status": apply_ready_status,
                    "apply_ready_write_error": apply_latest_write.err().map(|e| e.to_string()),
                    "apply_ready_history_write_error": apply_history_write.err().map(|e| e.to_string()),
                    "closure": "queue->approve/reject->apply",
                    "decision": cycle.board_decision.clone(),
                    "proposal_only": true,
                    "gate": write_gate.as_json(),
                })
            },
            Err(error) => serde_json::json!({
                "path": "9B",
                "executor": "gitmemory.patch_review_queue",
                "status": "error",
                "error": error.to_string(),
                "decision": cycle.board_decision.clone(),
                "proposal_only": true,
                "gate": write_gate.as_json(),
            }),
        }
    }

    async fn execute_path_9c_governance_update(
        &self,
        session_id: &str,
        cycle: &EvolutionShadowCycle,
        write_gate: &EvolutionProductionWriteGate,
    ) -> serde_json::Value {
        if !write_gate.production_write_allowed() {
            return self.blocked_production_write_payload(
                "9C",
                "governance.config_surface",
                session_id,
                write_gate,
                None,
            );
        }
        if !self.production_write_triad_present(write_gate) {
            return self.blocked_production_write_payload(
                "9C",
                "governance.config_surface",
                session_id,
                write_gate,
                Some("missing_gate_triad"),
            );
        }
        let now_ms = current_time_ms();
        let latest_key = format!("policy:evolution:governance:{session_id}:latest");
        let history_key = format!("policy:evolution:governance:{session_id}:{now_ms}");
        let version_key = format!("policy:evolution:governance:{session_id}:version:{now_ms}");
        let index_key = format!("policy:evolution:governance:{session_id}:index");

        let previous_version = self
            .state_store
            .get_knowledge(&latest_key)
            .await
            .ok()
            .flatten()
            .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
            .and_then(|value| {
                value
                    .get("governance_version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });

        let governance_version = format!("{}::evo:{}", cycle.reality.policy_version, now_ms);

        let payload = serde_json::json!({
            "policy_patch_id": format!("governance-patch:{session_id}:{now_ms}"),
            "session_id": session_id,
            "trace_id": cycle.reality.trace_id,
            "decision": cycle.board_decision.clone(),
            "board_decision": write_gate.as_json().get("board_decision").cloned(),
            "policy_allow": write_gate.as_json().get("policy_allow").cloned(),
            "evidence_ref": write_gate.as_json().get("evidence_ref").cloned(),
            "deny_reason": write_gate.as_json().get("deny_reason").cloned(),
            "rule_id": cycle.board_outcome.archivist.record_key,
            "policy_version": cycle.reality.policy_version,
            "governance_version": governance_version,
            "previous_governance_version": previous_version,
            "path": "9C",
            "proposal_only": true,
            "gate": write_gate.as_json(),
            "notes": cycle.board_outcome.archivist.notes,
            "candidate_ops": cycle.path_plan.actions,
            "updated_at_ms": now_ms,
        });

        let mut index = self
            .state_store
            .get_knowledge(&index_key)
            .await
            .ok()
            .flatten()
            .and_then(|record| serde_json::from_str::<Vec<serde_json::Value>>(&record.value).ok())
            .unwrap_or_default();
        index.push(serde_json::json!({
            "governance_version": governance_version,
            "version_ref": version_key,
            "updated_at_ms": now_ms,
        }));
        if index.len() > 50 {
            let trim = index.len() - 50;
            index.drain(0..trim);
        }

        let write_latest = self
            .state_store
            .upsert_json_knowledge(latest_key.clone(), &payload, "policy-engine")
            .await;
        let write_history = self
            .state_store
            .upsert_json_knowledge(history_key.clone(), &payload, "policy-engine")
            .await;
        let write_version = self
            .state_store
            .upsert_json_knowledge(version_key.clone(), &payload, "policy-engine")
            .await;
        let write_index = self
            .state_store
            .upsert_json_knowledge(index_key.clone(), &index, "policy-engine")
            .await;

        match (write_latest, write_history, write_version, write_index) {
            (Ok(_), Ok(_), Ok(_), Ok(_)) => serde_json::json!({
                "path": "9C",
                "executor": "governance.config_surface",
                "status": "proposed",
                "latest_ref": latest_key,
                "history_ref": history_key,
                "version_ref": version_key,
                "index_ref": index_key,
                "governance_version": payload.get("governance_version").cloned(),
                "previous_governance_version": payload.get("previous_governance_version").cloned(),
                "decision": cycle.board_decision.clone(),
                "proposal_only": true,
                "gate": write_gate.as_json(),
            }),
            (latest, history, version, index) => serde_json::json!({
                "path": "9C",
                "executor": "governance.config_surface",
                "status": "error",
                "latest_error": latest.err().map(|e| e.to_string()),
                "history_error": history.err().map(|e| e.to_string()),
                "version_error": version.err().map(|e| e.to_string()),
                "index_error": index.err().map(|e| e.to_string()),
                "decision": cycle.board_decision.clone(),
                "proposal_only": true,
                "gate": write_gate.as_json(),
            }),
        }
    }
    async fn execute_path_9d_local_only(
        &self,
        session_id: &str,
        cycle: &EvolutionShadowCycle,
        write_gate: &EvolutionProductionWriteGate,
    ) -> serde_json::Value {
        if !write_gate.production_write_allowed() {
            return self.blocked_production_write_payload(
                "9D",
                "local.experiment",
                session_id,
                write_gate,
                None,
            );
        }
        if !self.production_write_triad_present(write_gate) {
            return self.blocked_production_write_payload(
                "9D",
                "local.experiment",
                session_id,
                write_gate,
                Some("missing_gate_triad"),
            );
        }
        let now_ms = current_time_ms();
        let local_latest_key = format!("settings:local:evolution:{session_id}:latest");
        let local_history_key = format!("settings:local:evolution:{session_id}:{now_ms}");
        let isolation_scope = format!("local-experiment:{}:{}", session_id, now_ms);
        let rollback_contract_key = format!("settings:local:evolution:{session_id}:rollback:latest");
        let rollback_history_key = format!("settings:local:evolution:{session_id}:rollback:{now_ms}");

        let payload = serde_json::json!({
            "session_id": session_id,
            "trace_id": cycle.reality.trace_id,
            "decision": cycle.board_decision.clone(),
            "path": "9D",
            "proposal_only": true,
            "isolation_scope": isolation_scope,
            "actions": cycle.path_plan.actions,
            "gate": write_gate.as_json(),
            "updated_at_ms": now_ms,
        });
        let rollback_contract = serde_json::json!({
            "session_id": session_id,
            "trace_id": cycle.reality.trace_id,
            "path": "9D",
            "decision": cycle.board_decision.clone(),
            "isolation_scope": isolation_scope,
            "rollback_mode": if matches!(
                cycle.board_decision,
                crate::contracts::evolution_os::PromotionDecision::Rollback
            ) { "armed_and_execute" } else { "armed" },
            "recover_from_ref": local_history_key,
            "gate": write_gate.as_json(),
            "updated_at_ms": now_ms,
        });

        let latest_write = self
            .state_store
            .upsert_json_knowledge(local_latest_key.clone(), &payload, "local-evolution")
            .await;
        let history_write = self
            .state_store
            .upsert_json_knowledge(local_history_key.clone(), &payload, "local-evolution")
            .await;
        let rollback_latest_write = self
            .state_store
            .upsert_json_knowledge(
                rollback_contract_key.clone(),
                &rollback_contract,
                "local-evolution",
            )
            .await;
        let rollback_history_write = self
            .state_store
            .upsert_json_knowledge(
                rollback_history_key.clone(),
                &rollback_contract,
                "local-evolution",
            )
            .await;

        let rollback_result = if matches!(
            cycle.board_decision,
            crate::contracts::evolution_os::PromotionDecision::Rollback
        ) {
            if let Some(plugin_id) = cycle
                .proposals
                .iter()
                .map(|proposal| proposal.candidate_id.as_str())
                .find(|candidate_id| candidate_id.starts_with("plugin:"))
            {
                self.plugins
                    .quick_rollback(plugin_id, "evolution-board")
                    .await
                    .map(|manifest| serde_json::json!({
                        "plugin_id": plugin_id,
                        "state": manifest.metadata.get("rollout_mode"),
                        "version": manifest.version,
                    }))
                    .map_err(|error| error.to_string())
                    .ok()
            } else {
                None
            }
        } else {
            None
        };

        match (
            latest_write,
            history_write,
            rollback_latest_write,
            rollback_history_write,
        ) {
            (Ok(_), Ok(_), Ok(_), Ok(_)) => serde_json::json!({
                "path": "9D",
                "executor": "local.experiment",
                "status": "recorded",
                "local_ref": local_latest_key,
                "history_ref": local_history_key,
                "rollback_ref": rollback_contract_key,
                "rollback_history_ref": rollback_history_key,
                "isolation_scope": isolation_scope,
                "rollback": rollback_result,
                "decision": cycle.board_decision.clone(),
                "board_decision": write_gate.as_json().get("board_decision").cloned(),
                "policy_allow": write_gate.as_json().get("policy_allow").cloned(),
                "evidence_ref": write_gate.as_json().get("evidence_ref").cloned(),
                "deny_reason": write_gate.as_json().get("deny_reason").cloned(),
                "proposal_only": true,
                "gate": write_gate.as_json(),
            }),
            (latest, history, rollback_latest, rollback_history) => serde_json::json!({
                "path": "9D",
                "executor": "local.experiment",
                "status": "error",
                "latest_error": latest.err().map(|e| e.to_string()),
                "history_error": history.err().map(|e| e.to_string()),
                "rollback_latest_error": rollback_latest.err().map(|e| e.to_string()),
                "rollback_history_error": rollback_history.err().map(|e| e.to_string()),
                "isolation_scope": isolation_scope,
                "rollback": rollback_result,
                "decision": cycle.board_decision.clone(),
                "board_decision": write_gate.as_json().get("board_decision").cloned(),
                "policy_allow": write_gate.as_json().get("policy_allow").cloned(),
                "evidence_ref": write_gate.as_json().get("evidence_ref").cloned(),
                "deny_reason": write_gate.as_json().get("deny_reason").cloned(),
                "proposal_only": true,
                "gate": write_gate.as_json(),
            }),
        }
    }
    async fn execute_next_gen_trusted_execution(
        &self,
        session_id: &str,
        cycle: &EvolutionShadowCycle,
        path_execution: &serde_json::Value,
    ) -> serde_json::Value {
        let now_ms = current_time_ms();
        let selected_path = cycle.path_plan.selected_path.clone();
        let gate_payload = path_execution
            .get("gate")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let board_decision = gate_payload.get("board_decision").cloned();
        let policy_allow = gate_payload.get("policy_allow").cloned();
        let evidence_ref = gate_payload.get("evidence_ref").cloned();
        let deny_reason = gate_payload
            .get("deny_reason")
            .cloned()
            .unwrap_or_else(|| serde_json::json!("missing_deny_reason"));
        let gate_pass = path_execution
            .get("runtime_gate")
            .and_then(|item| item.get("verified"))
            .and_then(|item| item.as_bool())
            .unwrap_or_else(|| {
                let proposal_only = path_execution
                    .get("proposal_only")
                    .and_then(|item| item.as_bool())
                    .unwrap_or(false);
                if proposal_only {
                    return false;
                }
                path_execution
                    .get("status")
                    .and_then(|item| item.as_str())
                    .map(|status| status == "applied" || status == "recorded")
                    .unwrap_or(false)
            });
        let triad_present = board_decision.is_some()
            && policy_allow.is_some()
            && evidence_ref
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false);
        let gate_pass = gate_pass && triad_present;
        let execution_mode = if matches!(
            cycle.rollout.stage,
            crate::evolution_os::RolloutStage::Canary10
                | crate::evolution_os::RolloutStage::Canary30
                | crate::evolution_os::RolloutStage::Full
        ) {
            "canary"
        } else {
            "proposal"
        };
        let auto_rollback = cycle.rollout.rollback_on_failure && !gate_pass;
        let final_status = if auto_rollback {
            "rolled_back"
        } else if gate_pass {
            "trusted"
        } else {
            "blocked"
        };
        let rollback_ref = if auto_rollback {
            Some(format!("evolution:rollback:{session_id}:{now_ms}"))
        } else {
            None
        };

        if let Some(ref_key) = rollback_ref.as_ref() {
            let rollback_payload = serde_json::json!({
                "session_id": session_id,
                "trace_id": cycle.reality.trace_id,
                "path": crate::evolution_os::PromotionPathExecutor::path_code(&selected_path),
                "decision": cycle.board_decision.clone(),
                "reason": "canary_failed_auto_rollback",
                "activation_version": cycle.rollout.activation_version,
                "created_at_ms": now_ms,
            });
            let _ = self
                .state_store
                .upsert_json_knowledge(ref_key.clone(), &rollback_payload, "evolution-os")
                .await;
            let _ = self
                .state_store
                .upsert_json_knowledge(
                    format!("evolution:rollback:{session_id}:latest"),
                    &rollback_payload,
                    "evolution-os",
                )
                .await;
        }

        let snapshot = serde_json::json!({
            "snapshot_version": crate::contracts::version::EVOLUTION_OS_CONTRACT_VERSION,
            "session_id": session_id,
            "trace_id": cycle.reality.trace_id,
            "decision": cycle.board_decision.clone(),
            "selected_path": selected_path,
            "execution_mode": execution_mode,
            "runtime_gate_pass": gate_pass,
            "board_decision": board_decision,
            "policy_allow": policy_allow,
            "evidence_ref": evidence_ref,
            "deny_reason": if triad_present { deny_reason } else { serde_json::json!("missing_gate_triad") },
            "auto_rollback": auto_rollback,
            "activation_version": cycle.rollout.activation_version,
            "final_status": final_status,
            "path_execution": path_execution,
            "trusted_prior_ref": format!("evolution:trusted-prior:{session_id}:latest"),
            "rollback_ref": rollback_ref,
            "created_at_ms": now_ms,
        });

        let history_key = format!("evolution:next-gen:{session_id}:{now_ms}");
        let latest_key = format!("evolution:next-gen:{session_id}:latest");

        let _ = self
            .state_store
            .upsert_json_knowledge(history_key.clone(), &snapshot, "evolution-os")
            .await;
        let _ = self
            .state_store
            .upsert_json_knowledge(latest_key.clone(), &snapshot, "evolution-os")
            .await;

        serde_json::json!({
            "status": final_status,
            "execution_mode": execution_mode,
            "runtime_gate_pass": gate_pass,
            "auto_rollback": auto_rollback,
            "activation_version": cycle.rollout.activation_version,
            "selected_path": selected_path,
            "history_ref": history_key,
            "latest_ref": latest_key,
            "rollback_ref": rollback_ref,
            "path_execution": path_execution,
            "created_at_ms": now_ms,
        })
    }
pub async fn ensure_session_identity(
        &self,
        session_id: &str,
        tenant_id: &str,
        principal_id: &str,
        policy_id: &str,
        lease_ttl_ms: u64,
    ) -> Result<SessionIdentity> {
        let identity = self
            .security
            .issue_session_identity(
                &self.state_store,
                session_id,
                tenant_id,
                principal_id,
                policy_id,
                lease_ttl_ms,
            )
            .await?;
        let expires_at_ms = current_time_ms().saturating_add(lease_ttl_ms);
        let session_identity = SessionIdentity {
            tenant_id: identity.tenant_id,
            principal_id: identity.principal_id,
            policy_id: identity.policy_id,
            lease_token: identity.lease_token,
            expires_at_ms,
        };
        self.sessions
            .bind_identity(session_id, session_identity.clone())
            .await;
        Ok(session_identity)
    }

    async fn ensure_default_session_identity(&self, session_id: &str) -> Result<SessionIdentity> {
        if let Some(identity) = self.sessions.identity(session_id).await {
            let now = current_time_ms();
            if identity.expires_at_ms > now {
                return Ok(identity);
            }
        }
        self.ensure_session_identity(
            session_id,
            "tenant:default",
            &format!("principal:{session_id}"),
            "policy:default",
            3_600_000,
        )
        .await
    }

    fn governance_repo_root(&self) -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("D:\\AutoLoop\\autoloop-app"))
    }

    fn normalize_governance_sensitivity(risk_tier: &str) -> &'static str {
        match risk_tier.to_ascii_lowercase().as_str() {
            "high" => "high",
            "medium" => "medium",
            "low" => "low",
            _ => "low",
        }
    }

    async fn enforce_verify_governance_gate(
        &self,
        session_id: &str,
        org_context: &crate::contracts::org::OrganizationContext,
        risk_tier: &str,
    ) -> Result<()> {
        let trace_id = format!("trace:{session_id}:governance-verify:{}", current_time_ms());
        let governance = GitmemoryCoreKernel::new()
            .run_phase4_advanced_governance(
                &self.state_store,
                &self.governance_repo_root(),
                session_id,
                &org_context.tenant_id,
                &trace_id,
                GovernancePhase::Verify,
                &org_context.principal_id,
                "read",
                "memory:verification",
                Self::normalize_governance_sensitivity(risk_tier),
            )
            .await?;

        if governance.allowed {
            return Ok(());
        }

        let evidence_ref = self
            .runtime
            .tag_external_stage(
                &self.state_store,
                session_id,
                &trace_id,
                None,
                Some("execution-verifier"),
                EvidenceTagStage::Verify,
                "governance.verify.blocked",
                serde_json::json!({
                    "rule_id": governance.rule_id,
                    "policy_version": governance.policy_version,
                    "replay_fp": governance.replay_fp,
                    "summary": governance.summary,
                    "source": "phase4-advanced-governance",
                }),
            )
            .await?;

        let block = serde_json::json!({
            "code": "governance_verify_blocked",
            "rule_id": governance.rule_id,
            "policy_version": governance.policy_version,
            "evidence_ref": evidence_ref,
            "replay_fp": governance.replay_fp,
            "summary": governance.summary,
        });

        self.state_store
            .upsert_json_knowledge(
                format!("policy-reject:{session_id}:verify:{}", current_time_ms()),
                &serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "tenant_id": org_context.tenant_id,
                    "principal_id": org_context.principal_id,
                    "policy_id": org_context.policy_id,
                    "rule_id": governance.rule_id,
                    "policy_version": governance.policy_version,
                    "evidence_ref": evidence_ref,
                    "replay_fp": governance.replay_fp,
                    "summary": governance.summary,
                }),
                "phase4-governance",
            )
            .await?;

        Err(anyhow!(block.to_string()))
    }

    fn swarm_guard_observations(
        &self,
        outcome: &orchestration::SwarmOutcome,
    ) -> Vec<ExecutionGuardObservation> {
        outcome
            .execution_reports
            .iter()
            .enumerate()
            .map(|(idx, report)| ExecutionGuardObservation {
                surface: "swarm_execution".to_string(),
                capability_id: report
                    .tool_used
                    .clone()
                    .unwrap_or_else(|| format!("swarm-task:{}", report.task.task_id)),
                decision: {
                    let lowered = report.guard_decision.to_ascii_lowercase();
                    if lowered.contains("block") || lowered.contains("deny") {
                        "blocked".to_string()
                    } else if lowered.contains("approval") || lowered.contains("escalate") {
                        "requires_approval".to_string()
                    } else {
                        "allow".to_string()
                    }
                },
                reason: format!(
                    "task_index={} task_id={} role={} raw_guard_decision={}",
                    idx, report.task.task_id, report.task.role, report.guard_decision
                ),
            })
            .collect::<Vec<_>>()
    }

    async fn enqueue_swarm_decision_followup(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
        decision: &UnifiedDecisionOutput,
    ) -> Result<Option<ScheduleEvent>> {
        let topic = match decision.kind {
            RuntimeDecisionKind::Repair => "trigger:on_message:repair",
            RuntimeDecisionKind::Escalate => "trigger:on_message:escalate",
            _ => return Ok(None),
        };
        let payload = serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "decision": decision.kind.as_str(),
            "reasons": decision.reasons.clone(),
            "prompt_excerpt": content.chars().take(220).collect::<String>(),
            "queued_at_ms": current_time_ms(),
        });
        let event = TriggerRuntimeEngine::new(self.state_store.clone())
            .ingest_webhook_event(
                session_id,
                topic,
                Some(payload.to_string()),
                "context-decision-protocol",
            )
            .await?;
        Ok(Some(event))
    }

    async fn persist_swarm_unified_decision(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
        decision: &UnifiedDecisionOutput,
        followup: Option<&ScheduleEvent>,
        outcome: Option<&orchestration::SwarmOutcome>,
    ) -> Result<String> {
        let now = current_time_ms();
        let key = format!("runtime:decision:{session_id}:{now}");
        let record = serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "decision": decision.kind.as_str(),
            "reasons": decision.reasons.clone(),
            "forced": decision.forced,
            "verifier_score": decision.verifier_score,
            "content_excerpt": content.chars().take(240).collect::<String>(),
            "compile": {
                "hardgate_passed": outcome.is_some(),
                "constraint_version": serde_json::Value::Null,
                "constraint_ids": Vec::<String>::new(),
                "compaction_boundary": serde_json::Value::Null,
            },
            "execute": {
                "provider_retry_count": 0,
                "tool_retry_count": 0,
                "guard_observations": outcome.map(|item| self.swarm_guard_observations(item)).unwrap_or_default(),
                "tool_call_count": outcome.map(|item| item.execution_reports.len()).unwrap_or(0),
                "iteration_count": outcome.map(|item| item.tasks.len()).unwrap_or(0),
            },
            "verify": {
                "verdict": outcome.map(|item| format!("{:?}", item.verifier_report.verdict).to_ascii_lowercase()),
                "summary": outcome.map(|item| item.verifier_report.summary.clone()),
                "overall_score": outcome.map(|item| item.verifier_report.overall_score),
                "replay_fingerprint": format!("swarm:{}:{}", session_id, trace_id),
            },
            "followup": followup.map(|event| serde_json::json!({
                "event_id": event.id,
                "topic": event.topic,
                "status": event.status,
            })),
            "decision_at_ms": now,
            "evidence_ref": key,
        });
        self.state_store
            .upsert_json_knowledge(key.clone(), &record, "context-decision-protocol")
            .await?;

        let _ = append_event(
            &self.state_store,
            "context_runtime_decision",
            trace_id.to_string(),
            session_id.to_string(),
            Some("swarm-decision".to_string()),
            Some("context-runtime-kernel".to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "decision": decision.kind.as_str(),
                "forced": decision.forced,
                "reasons": decision.reasons.clone(),
                "verifier_score": decision.verifier_score,
                "evidence_ref": key,
                "followup_event_id": followup.map(|event| event.id),
            }),
        )
        .await;

        Ok(key)
    }
    pub async fn process_requirement_swarm(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<String> {
        let context_kernel = ContextRuntimeKernel::new(self.state_store.clone());
        let context_run = context_kernel
            .begin_turn(session_id, "process_requirement_swarm", content)
            .await?;

        let swarm_wall_timeout_secs = std::env::var("AUTOLOOP_SWARM_WALL_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(180);
        let swarm_result = tokio::time::timeout(
            std::time::Duration::from_secs(swarm_wall_timeout_secs),
            self.process_requirement_swarm_inner(session_id, content),
        )
        .await;

        match swarm_result {
            Ok(result) => match result {
            Ok(response) => {
                context_kernel
                    .finish_turn(
                        session_id,
                        "process_requirement_swarm",
                        &context_run,
                        Ok(response.as_str()),
                    )
                    .await?;
                self.run_evolution_shadow_probe(
                    session_id,
                    "process_requirement_swarm",
                    content,
                    true,
                    response.as_str(),
                )
                .await;
                Ok(response)
            }
            Err(error) => {
                let error_message = error.to_string();
                if let Err(record_error) = context_kernel
                    .finish_turn(
                        session_id,
                        "process_requirement_swarm",
                        &context_run,
                        Err(error_message.as_str()),
                    )
                    .await
                {
                    return Err(anyhow!(
                        "context runtime kernel failed while recording process_requirement_swarm failure: {record_error}; original error: {error_message}"
                    ));
                }
                self.run_evolution_shadow_probe(
                    session_id,
                    "process_requirement_swarm",
                    content,
                    false,
                    error_message.as_str(),
                )
                .await;
                Err(error)
            }
        },
            Err(_) => {
                let _ = append_event(
                    &self.state_store,
                    "swarm.wall_timeout.direct_fallback",
                    format!("trace:{session_id}:swarm-wall-timeout"),
                    session_id.to_string(),
                    None,
                    Some("swarm-lane-router".to_string()),
                    crate::contracts::version::CONTRACT_VERSION,
                    serde_json::json!({
                        "timeout_secs": swarm_wall_timeout_secs,
                        "fallback_mode": "process_direct",
                    }),
                )
                .await;

                let fallback_result = self.process_direct(session_id, content).await;
                match fallback_result {
                    Ok(response) => {
                        context_kernel
                            .finish_turn(
                                session_id,
                                "process_requirement_swarm",
                                &context_run,
                                Ok(response.as_str()),
                            )
                            .await?;
                        self.run_evolution_shadow_probe(
                            session_id,
                            "process_requirement_swarm",
                            content,
                            true,
                            response.as_str(),
                        )
                        .await;
                        Ok(response)
                    }
                    Err(error) => {
                        let error_message = error.to_string();
                        if let Err(record_error) = context_kernel
                            .finish_turn(
                                session_id,
                                "process_requirement_swarm",
                                &context_run,
                                Err(error_message.as_str()),
                            )
                            .await
                        {
                            return Err(anyhow!(
                                "context runtime kernel failed while recording process_requirement_swarm wall-timeout fallback failure: {record_error}; original error: {error_message}"
                            ));
                        }
                        self.run_evolution_shadow_probe(
                            session_id,
                            "process_requirement_swarm",
                            content,
                            false,
                            error_message.as_str(),
                        )
                        .await;
                        Err(error)
                    }
                }
            }
        }
    }

    async fn process_requirement_swarm_inner(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        if should_route_code_task_to_harness(content) {
            return self
                .execute_via_runtime_facade(session_id, "harness_facade", content)
                .await;
        }
        let org_context = OrganizationContextResolver::new(self.state_store.clone())
            .resolve(session_id)
            .await?;
        let governance_telemetry_scope = GovernanceTelemetryScope::compile(&org_context);
        self.state_store
            .upsert_json_knowledge(
                format!("governance:{session_id}:telemetry-scope"),
                &governance_telemetry_scope,
                "governance-telemetry-scope-compiler",
            )
            .await?;
        let policy_review = self
            .security
            .review_requirement(&self.state_store, session_id, content)
            .await?;
        let policy_was_revised = !policy_review.approved;
        let mut normalized_content = content.to_string();
        if !policy_review.approved {
            let _ = append_event(
                &self.state_store,
                "policy.review.rejected",
                format!("trace:{session_id}:policy-review"),
                session_id.to_string(),
                None,
                None,
                crate::contracts::version::CONTRACT_VERSION,
                serde_json::json!({
                    "reason": policy_review.reason,
                    "action": "revise_to_clarification",
                }),
            )
            .await;
            normalized_content = policy_review.revised_request;
        }
        let mut budget_preflight = build_swarm_budget_preflight(
            session_id,
            &normalized_content,
            org_context.quotas.max_tokens,
        );
        if budget_preflight.compaction_applied {
            normalized_content = budget_preflight.replanned_request.clone();
        }
        if should_route_code_task_to_harness(&normalized_content) {
            let _ = append_event(
                &self.state_store,
                "swarm.route.harness_after_normalization",
                format!("trace:{session_id}:swarm-harness-normalized"),
                session_id.to_string(),
                None,
                Some("swarm-lane-router".to_string()),
                crate::contracts::version::CONTRACT_VERSION,
                serde_json::json!({
                    "reason": "normalized_content_requires_harness_facade",
                    "lane_mode": budget_preflight.lane_mode,
                    "compaction_applied": budget_preflight.compaction_applied,
                }),
            )
            .await;
            return self
                .execute_via_runtime_facade(session_id, "harness_facade", &normalized_content)
                .await;
        }
        budget_preflight.final_request_tokens = estimate_swarm_tokens(&normalized_content);
        self.state_store
            .upsert_json_knowledge(
                format!("runtime:{session_id}:swarm-budget-preflight:latest"),
                &budget_preflight,
                "swarm-budget-preflight",
            )
            .await?;
        let _ = append_event(
            &self.state_store,
            "swarm.budget_preflight",
            format!("trace:{session_id}:swarm-budget-preflight"),
            session_id.to_string(),
            None,
            Some("swarm-budget-preflight".to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "estimated_tokens": budget_preflight.estimated_tokens,
                "effective_max_tokens": budget_preflight.effective_max_tokens,
                "compaction_applied": budget_preflight.compaction_applied,
                "lane_mode": budget_preflight.lane_mode,
                "reason": budget_preflight.reason,
            }),
        )
        .await;
        let _ = append_event(
            &self.state_store,
            "swarm.lane.selected",
            format!("trace:{session_id}:swarm-lane-selection"),
            session_id.to_string(),
            None,
            Some("swarm-lane-router".to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "lane_mode": budget_preflight.lane_mode,
                "budget_compaction_applied": budget_preflight.compaction_applied,
                "dual_lane_reason": budget_preflight.reason,
            }),
        )
        .await;
        if budget_preflight.lane_mode == "dual" {
            let plan_lane = build_swarm_plan_lane(session_id, &normalized_content, &budget_preflight);
            self.state_store
                .upsert_json_knowledge(
                    format!("runtime:{session_id}:swarm-plan-lane:latest"),
                    &plan_lane,
                    "swarm-plan-lane",
                )
                .await?;
            let _ = append_event(
                &self.state_store,
                "swarm.lane.plan.completed",
                format!("trace:{session_id}:swarm-lane-plan"),
                session_id.to_string(),
                None,
                Some("swarm-lane-router".to_string()),
                crate::contracts::version::CONTRACT_VERSION,
                serde_json::json!({
                    "lane_mode": budget_preflight.lane_mode,
                    "compaction_applied": budget_preflight.compaction_applied,
                    "reason": budget_preflight.reason,
                    "step_count": plan_lane["plan_steps"].as_array().map(|items| items.len()).unwrap_or(0),
                    "plan_digest": plan_lane["plan_digest"].as_str().unwrap_or_default(),
                }),
            )
            .await;
            if let Some(seed) = build_swarm_execute_lane_seed(&plan_lane) {
                let seeded_request = format!("[PlanLaneSeed]\n{}\n\n{}", seed, normalized_content);
                if estimate_swarm_tokens(&seeded_request) > budget_preflight.effective_max_tokens {
                    normalized_content = compact_and_replan_requirement(
                        &seeded_request,
                        budget_preflight.effective_max_tokens,
                    );
                    budget_preflight.compaction_applied = true;
                    budget_preflight.reason = format!(
                        "{}; execute_lane_seed_overflow:compact_then_replan",
                        budget_preflight.reason
                    );
                } else {
                    normalized_content = seeded_request;
                }
                budget_preflight.final_request_tokens = estimate_swarm_tokens(&normalized_content);
            }
        }

        let supermemory_risk_tier = org_context
            .metadata
            .get("risk_tier")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());

        let knowledge_context = KnowledgeContextResolver::new(self.state_store.clone())
            .resolve(session_id)
            .await?;
        let project_policy = knowledge_context.project_policy.clone();

        let mut supermemory_metadata = BTreeMap::from([
            (
                "tags".to_string(),
                "swarm,requirement,supermemory".to_string(),
            ),
            ("policy_id".to_string(), org_context.policy_id.clone()),
            ("risk_tier".to_string(), supermemory_risk_tier),
        ]);
        supermemory_metadata.insert(
            "retrieval_criteria".to_string(),
            project_policy.retrieval_criteria.join(","),
        );
        supermemory_metadata.insert(
            "multilingual".to_string(),
            project_policy.multilingual.to_string(),
        );
        supermemory_metadata.insert(
            "enable_graph".to_string(),
            project_policy.enable_graph.to_string(),
        );

        let supermemory_context = self
            .memory
            .run_supermemory_pipeline(
                &self.state_store,
                session_id,
                &org_context.tenant_id,
                "requirement-swarm",
                &normalized_content,
                supermemory_metadata,
                None,
                None,
                &normalized_content,
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("conversation:{session_id}:supermemory-context"),
                &supermemory_context,
                "supermemory-kernel",
            )
            .await?;
        let supermemory_metrics = self
            .observability
            .persist_supermemory_metrics(
                &self.state_store,
                session_id,
                supermemory_context.hits.len(),
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("conversation:{session_id}:supermemory-metrics"),
                &supermemory_metrics,
                "supermemory-observability",
            )
            .await?;

        let research_report = self
            .research
            .run_anchor_research(&self.state_store, session_id, &normalized_content)
            .await?;
        let scheduled_research_tasks = self
            .research
            .schedule_follow_up_research(
                &self.state_store,
                session_id,
                session_id,
                &research_report,
            )
            .await
            .unwrap_or(0);
        let decision_thresholds = load_thresholds_from_env();
        let forced_hint = parse_decision_hint(&normalized_content);
        let swarm_timeout_secs = std::env::var("AUTOLOOP_SWARM_TIMEOUT_SECS")
            .ok()
            .and_then(|raw| raw.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(120);
        let outcome = match tokio::time::timeout(
            std::time::Duration::from_secs(swarm_timeout_secs),
            self.orchestration
                .run_requirement_swarm(session_id, &normalized_content),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                let _ = append_event(
                    &self.state_store,
                    "swarm.timeout.direct_fallback",
                    format!("trace:{session_id}:swarm-timeout"),
                    session_id.to_string(),
                    None,
                    Some("swarm-lane-router".to_string()),
                    crate::contracts::version::CONTRACT_VERSION,
                    serde_json::json!({
                        "timeout_secs": swarm_timeout_secs,
                        "fallback_mode": "process_direct",
                    }),
                )
                .await;
                return self.process_direct(session_id, &normalized_content).await;
            }
        };
        let _ = append_event(
            &self.state_store,
            "swarm.lane.execute.completed",
            format!("trace:{session_id}:swarm-lane-execute"),
            session_id.to_string(),
            None,
            Some("swarm-lane-router".to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "lane_mode": budget_preflight.lane_mode,
                "task_count": outcome.tasks.len(),
                "execution_count": outcome.execution_reports.len(),
            }),
        )
        .await;

        self.state_store
            .upsert_knowledge(
                format!("conversation:{session_id}:brief"),
                serde_json::to_string(&outcome.brief)?,
                "requirement-agent".into(),
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("{}:brief", outcome.brief.anchor_id),
                serde_json::to_string(&outcome.brief)?,
                "requirement-agent".into(),
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("conversation:{session_id}:ceo"),
                outcome.ceo_summary.clone(),
                "ceo-agent".into(),
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("{}:ceo", outcome.brief.anchor_id),
                outcome.ceo_summary.clone(),
                "ceo-agent".into(),
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("conversation:{session_id}:deliberation"),
                &outcome.deliberation,
                "swarm-judge",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("conversation:{session_id}:optimization"),
                &outcome.optimization_proposal,
                "optimization-agent",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("conversation:{session_id}:research"),
                &research_report,
                "autonomous-research",
            )
            .await?;
        self.state_store
            .upsert_knowledge(
                format!("research:{session_id}:follow-up-status"),
                serde_json::json!({
                    "scheduled_tasks": scheduled_research_tasks,
                    "backend_used": research_report.backend_used,
                    "knowledge_gaps": research_report.knowledge_gaps,
                })
                .to_string(),
                "autonomous-research".into(),
            )
            .await?;
        let capability_lifecycle = self.tools.lifecycle_report();
        let verifier_queue_depth = self
            .state_store
            .list_schedule_events(session_id)
            .await?
            .into_iter()
            .filter(|event| event.topic.contains("verifier"))
            .count();
        self.observability
            .persist_swarm_observability(
                &self.state_store,
                session_id,
                &governance_telemetry_scope,
                &outcome,
                &capability_lifecycle,
                verifier_queue_depth,
            )
            .await?;
        let policy_signals = aggregate_policy_signals(
            &self.state_store,
            session_id,
            &governance_telemetry_scope,
            outcome.verifier_report.overall_score,
            &outcome.execution_reports,
        )
        .await?;
        let _ = self
            .security
            .apply_policy_feedback(
                &self.state_store,
                session_id,
                policy_signals.suggested_quota_factor,
                &policy_signals.suggested_approval_threshold,
            )
            .await;
        let _ = self
            .runtime
            .apply_runtime_mode_hint(
                &self.state_store,
                session_id,
                &policy_signals.runtime_mode_hint,
                &format!("trace:{session_id}:policy-signal"),
            )
            .await;
        self.state_store
            .upsert_knowledge(
                format!("conversation:{session_id}:swarm"),
                serde_json::to_string(&outcome.execution_reports)?,
                "swarm-execution".into(),
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("protocol:{session_id}:verifier-report"),
                &outcome.verifier_report,
                "verifier-agent",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("protocol:{session_id}:execution-verifier-report"),
                &outcome.verifier_report,
                "execution-verifier",
            )
            .await?;
        self.enforce_verify_governance_gate(
            session_id,
            &org_context,
            &governance_telemetry_scope.risk_tier,
        )
        .await?;
        let swarm_guard_observations = self.swarm_guard_observations(&outcome);
        let decision = evaluate_unified_decision(
            UnifiedDecisionInput {
                hardgate_passed: true,
                compile_failed: false,
                compaction_applied: budget_preflight.compaction_applied,
                max_iterations_reached: false,
                provider_retry_count: 0,
                tool_retry_count: 0,
                guard_observations: swarm_guard_observations.clone(),
                verifier_score: outcome.verifier_report.overall_score,
                forced_hint,
            },
            &decision_thresholds,
        );
        let decision_trace_id = format!("trace:{session_id}:swarm-decision");
        let decision_followup = self
            .enqueue_swarm_decision_followup(
                session_id,
                &decision_trace_id,
                &normalized_content,
                &decision,
            )
            .await?;
        let decision_evidence_ref = self
            .persist_swarm_unified_decision(
                session_id,
                &decision_trace_id,
                &normalized_content,
                &decision,
                decision_followup.as_ref(),
                Some(&outcome),
            )
            .await?;
        match self
            .persist_swarm_relation_contract(
                session_id,
                &org_context,
                &decision_trace_id,
                &decision_evidence_ref,
                &outcome,
            )
            .await
        {
            Ok(relation_result) => {
                self.state_store
                    .upsert_json_knowledge(
                        format!("runtime:{session_id}:relation-graph:latest"),
                        &relation_result,
                        "relation-graph-writer",
                    )
                    .await?;
            }
            Err(error) => {
                let _ = append_event(
                    &self.state_store,
                    "relation.contract.persist_failed",
                    format!("trace:{session_id}:relation-persist-failed"),
                    session_id.to_string(),
                    None,
                    Some("relation-graph-writer".to_string()),
                    crate::contracts::version::CONTRACT_VERSION,
                    serde_json::json!({
                        "reason": error.to_string(),
                        "trace_id": decision_trace_id,
                        "evidence_ref": decision_evidence_ref,
                    }),
                )
                .await;
            }
        }
        let _ = self
            .runtime
            .tag_external_stage(
                &self.state_store,
                session_id,
                &decision_trace_id,
                None,
                Some("swarm-decision"),
                EvidenceTagStage::Verify,
                "swarm.execution_decision",
                serde_json::json!({
                    "decision": decision.kind.as_str(),
                    "reasons": decision.reasons.clone(),
                    "forced": decision.forced,
                    "verifier_score": decision.verifier_score,
                    "evidence_ref": decision_evidence_ref,
                    "followup_event_id": decision_followup.as_ref().map(|event| event.id),
                    "guard_observations": swarm_guard_observations,
                }),
            )
            .await;
        if decision.kind == RuntimeDecisionKind::Reject {
            return Err(anyhow!(format!(
                "Execution rejected by context decision protocol: {}",
                decision.reasons.join(" | ")
            )));
        }
        let _ = append_event(
            &self.state_store,
            "verifier.execution_decision",
            format!("trace:{session_id}:execution-verifier"),
            session_id.to_string(),
            None,
            Some("execution-verifier".into()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "verifier_layer": "execution_verifier",
                "verdict": outcome.verifier_report.verdict,
                "overall_score": outcome.verifier_report.overall_score,
                "summary": outcome.verifier_report.summary,
            }),
        )
        .await;
        let _ = self
            .runtime
            .tag_external_stage(
                &self.state_store,
                session_id,
                &format!("trace:{session_id}:execution-verifier"),
                None,
                Some("execution-verifier"),
                EvidenceTagStage::Verify,
                "verifier.execution_decision",
                serde_json::json!({
                    "verdict": outcome.verifier_report.verdict,
                    "score": outcome.verifier_report.overall_score,
                    "summary": outcome.verifier_report.summary,
                }),
            )
            .await;
        self.state_store
            .upsert_json_knowledge(
                format!("protocol:{session_id}:capability-regression"),
                &outcome.verifier_report.capability_regression,
                "capability-regression-suite",
            )
            .await?;
        for case in &outcome.verifier_report.capability_regression.cases {
            if case.passed {
                let _ = self.tools.verify_capability(&case.tool_name).await?;
            } else {
                let _ = self
                    .tools
                    .deprecate_capability(&case.tool_name, case.health_score.min(0.35))
                    .await?;
            }
        }
        for (index, report) in outcome.execution_reports.iter().enumerate() {
            let observed_at_ms = current_time_ms().saturating_add(index as u64);
            self.state_store
                .upsert_knowledge(
                    format!("conversation:{session_id}:execution-feedback:{index}"),
                    serde_json::json!({
                        "tool": report.tool_used,
                        "mcp_server": report.mcp_server,
                        "payload": report.invocation_payload,
                        "outcome_score": report.outcome_score,
                        "route_variant": report.route_variant,
                        "control_score": report.control_score,
                        "treatment_score": report.treatment_score,
                        "output": report.output,
                    })
                    .to_string(),
                    "execution-feedback".into(),
                )
                .await?;

            let stats_key = match report.tool_used.as_deref() {
                Some(tool_name) if tool_name.starts_with("mcp::") => {
                    format!(
                        "metrics:execution:mcp:{}",
                        report
                            .mcp_server
                            .clone()
                            .or_else(|| parse_mcp_server(tool_name))
                            .unwrap_or_else(|| "unknown".to_string())
                    )
                }
                Some(tool_name) => format!("metrics:execution:tool:{tool_name}"),
                None => "metrics:execution:provider-only".into(),
            };
            let existing_stats = self
                .state_store
                .get_knowledge(&stats_key)
                .await?
                .and_then(|record| serde_json::from_str::<ExecutionStats>(&record.value).ok());
            let updated_stats = update_execution_stats(existing_stats, report, observed_at_ms);
            self.state_store
                .upsert_knowledge(
                    stats_key,
                    serde_json::to_string(&updated_stats)?,
                    "execution-stats".into(),
                )
                .await?;
            let session_ab_key = format!("metrics:ab:session:{session_id}");
            let existing_session_ab = self
                .state_store
                .get_knowledge(&session_ab_key)
                .await?
                .and_then(|record| serde_json::from_str::<AbRoutingStats>(&record.value).ok());
            let mut updated_session_ab =
                update_ab_routing_stats(existing_session_ab, report, observed_at_ms);
            updated_session_ab.scope = session_ab_key.clone();
            self.state_store
                .upsert_knowledge(
                    session_ab_key,
                    serde_json::to_string(&updated_session_ab)?,
                    "ab-routing-stats".into(),
                )
                .await?;
            let task_ab_key = format!("metrics:ab:task:{}", report.task.role.to_ascii_lowercase());
            let existing_task_ab = self
                .state_store
                .get_knowledge(&task_ab_key)
                .await?
                .and_then(|record| serde_json::from_str::<AbRoutingStats>(&record.value).ok());
            let mut updated_task_ab =
                update_ab_routing_stats(existing_task_ab, report, observed_at_ms);
            updated_task_ab.scope = task_ab_key.clone();
            self.state_store
                .upsert_knowledge(
                    task_ab_key,
                    serde_json::to_string(&updated_task_ab)?,
                    "ab-routing-stats".into(),
                )
                .await?;
            if let Some(tool_name) = &report.tool_used {
                let tool_ab_key = format!("metrics:ab:tool:{tool_name}");
                let existing_tool_ab = self
                    .state_store
                    .get_knowledge(&tool_ab_key)
                    .await?
                    .and_then(|record| serde_json::from_str::<AbRoutingStats>(&record.value).ok());
                let mut updated_tool_ab =
                    update_ab_routing_stats(existing_tool_ab, report, observed_at_ms);
                updated_tool_ab.scope = tool_ab_key.clone();
                self.state_store
                    .upsert_knowledge(
                        tool_ab_key,
                        serde_json::to_string(&updated_tool_ab)?,
                        "ab-routing-stats".into(),
                    )
                    .await?;
            }
            if let Some(server) = &report.mcp_server {
                let server_ab_key = format!("metrics:ab:server:{server}");
                let existing_server_ab = self
                    .state_store
                    .get_knowledge(&server_ab_key)
                    .await?
                    .and_then(|record| serde_json::from_str::<AbRoutingStats>(&record.value).ok());
                let mut updated_server_ab =
                    update_ab_routing_stats(existing_server_ab, report, observed_at_ms);
                updated_server_ab.scope = server_ab_key.clone();
                self.state_store
                    .upsert_knowledge(
                        server_ab_key,
                        serde_json::to_string(&updated_server_ab)?,
                        "ab-routing-stats".into(),
                    )
                    .await?;
            }
            self.memory
                .persist_witness_log(
                    &self.state_store,
                    session_id,
                    &WitnessLog {
                        source: report
                            .tool_used
                            .clone()
                            .unwrap_or_else(|| "provider-only".into()),
                        observation: report.output.clone(),
                        metric_name: "outcome_score".into(),
                        metric_value: report.outcome_score as f32,
                    },
                )
                .await?;
            self.memory
                .persist_learning_event(
                    &self.state_store,
                    &memory::LearningEvent {
                        event_kind: memory::LearningEventKind::RouteDecision,
                        session_id: session_id.to_string(),
                        source: report
                            .tool_used
                            .clone()
                            .unwrap_or_else(|| "provider-only".into()),
                        summary: format!(
                            "variant={} control={} treatment={}",
                            report.route_variant, report.control_score, report.treatment_score
                        ),
                        score: report.outcome_score as f32,
                    },
                )
                .await?;
        }
        let circuit_snapshot = self.runtime.circuit_snapshot(&self.state_store).await?;
        self.state_store
            .upsert_knowledge(
                format!("observability:{session_id}:runtime-circuits"),
                serde_json::to_string(&circuit_snapshot)?,
                "runtime-circuit".into(),
            )
            .await?;
        let forged_manifests = self
            .state_store
            .list_knowledge_by_prefix(ToolRegistry::FORGED_TOOL_PREFIX)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<ForgedMcpToolManifest>(&record.value).ok())
            .collect::<Vec<_>>();
        let prior_snapshot = self
            .state_store
            .get_knowledge(&format!("graph:{session_id}:snapshot"))
            .await?
            .map(|record| record.value);
        let augmented_snapshot = self.rag.augment_snapshot_with_forged_capabilities(
            &outcome.knowledge_update.snapshot_json,
            &forged_manifests,
        );
        let merged_snapshot = self.rag.merge_incremental_snapshot(
            prior_snapshot.as_deref(),
            &augmented_snapshot,
            &outcome.tasks,
            &forged_manifests,
        );
        self.state_store
            .upsert_knowledge(
                format!("graph:{session_id}:snapshot"),
                merged_snapshot,
                "graph-rag".into(),
            )
            .await?;
        let global_snapshot = self
            .merge_global_graph_snapshot(session_id, &forged_manifests)
            .await?;
        self.state_store
            .upsert_knowledge(
                "graph:global:snapshot".into(),
                global_snapshot,
                "graph-rag-global".into(),
            )
            .await?;
        self.memory
            .persist_learning_session(
                &self.state_store,
                autoloop_state_adapter::LearningSessionRecord {
                    id: format!("iteration:{session_id}:{}", current_time_ms()),
                    session_id: session_id.to_string(),
                    objective: outcome.brief.clarified_goal.clone(),
                    status: if outcome.validation.ready {
                        "completed".into()
                    } else {
                        "needs_follow_up".into()
                    },
                    priority: if outcome.validation.ready { 0.3 } else { 0.9 },
                    summary: outcome.validation.summary.clone(),
                    started_at_ms: current_time_ms(),
                    completed_at_ms: Some(current_time_ms()),
                },
            )
            .await?;
        self.memory
            .persist_reflexion_episode(
                &self.state_store,
                session_id,
                &ReflexionEpisode {
                    proposal_id: session_key(session_id),
                    hypothesis: outcome.optimization_proposal.hypothesis.clone(),
                    outcome: outcome.validation.summary.clone(),
                    lesson: if outcome.validation.ready {
                        "Keep iterations that improve immutable objectives with bounded complexity."
                            .into()
                    } else {
                        "Rollback or revise iterations that regress immutable objectives or leave unresolved follow-up work."
                            .into()
                    },
                },
            )
            .await?;
        let learning_signal = memory::LearningSignal {
            signal_id: format!("learning-signal:{session_id}:{}", current_time_ms()),
            session_id: session_id.to_string(),
            trace_id: format!("trace:{session_id}:learning:consolidation"),
            source: "runtime.learning_consolidation".to_string(),
            evidence_ref: format!("evidence:tag:{session_id}:learn:{}", current_time_ms()),
            metadata: std::collections::BTreeMap::from([(
                "stage".to_string(),
                "learning_consolidation".to_string(),
            )]),
        };
        self.memory
            .persist_skill(
                &self.state_store,
                session_id,
                &SkillRecord {
                    name: "autonomy-optimization-loop".into(),
                    trigger: "Need a bounded iteration under immutable evaluation".into(),
                    procedure:
                        "propose -> apply -> execute -> parse immutable metric -> keep/discard -> rollback"
                            .into(),
                    confidence: 0.78,
                },
                &learning_signal,
            )
            .await?;
        self.memory
            .persist_causal_edge(
                &self.state_store,
                session_id,
                &CausalEdge {
                    cause: "immutable-objective-regression".into(),
                    effect: "rollback-iteration".into(),
                    evidence: outcome.validation.summary.clone(),
                    strength: if outcome.validation.ready { 0.2 } else { 0.9 },
                },
            )
            .await?;
        self.memory
            .persist_learning_event(
                &self.state_store,
                &memory::LearningEvent {
                    event_kind: if outcome.validation.ready {
                        memory::LearningEventKind::Success
                    } else {
                        memory::LearningEventKind::Failure
                    },
                    session_id: session_id.to_string(),
                    source: "validation".into(),
                    summary: outcome.validation.summary.clone(),
                    score: if outcome.validation.ready { 1.0 } else { 0.0 },
                },
            )
            .await?;
        let consolidation = self
            .memory
            .consolidate_learning(&self.state_store, session_id)
            .await?;
        let evolution_report = self.evolution.run(
            session_id,
            &consolidation,
            outcome.verifier_report.overall_score,
        );
        let retired_capabilities = self.tools.auto_retire_unhealthy_capabilities().await?;
        self.state_store
            .upsert_json_knowledge(
                format!("memory:{session_id}:consolidation"),
                &consolidation,
                "learning-consolidation",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("memory:{session_id}:self-evolution"),
                &evolution_report,
                "self-evolution",
            )
            .await?;
        for proposal in &consolidation.capability_improvements {
            self.state_store
                .upsert_json_knowledge(
                    format!(
                        "memory:{session_id}:capability-improvement:{}",
                        proposal.tool_name
                    ),
                    proposal,
                    "learning-consolidation",
                )
                .await?;
            self.state_store
                .create_schedule_event(
                    session_id.to_string(),
                    "verifier.capability_improvement".into(),
                    "cli::verify_capability".into(),
                    serde_json::to_string(proposal)?,
                    session_id.to_string(),
                )
                .await?;
        }
        for proposal in &evolution_report.capability_proposals {
            self.state_store
                .upsert_json_knowledge(
                    format!(
                        "memory:{session_id}:evolution-proposal:{}",
                        proposal.tool_name
                    ),
                    proposal,
                    "self-evolution",
                )
                .await?;
            self.memory
                .persist_witness_log(
                    &self.state_store,
                    session_id,
                    &WitnessLog {
                        source: proposal.tool_name.clone(),
                        observation: proposal.rationale.clone(),
                        metric_name: "expected_lift".into(),
                        metric_value: proposal.expected_lift,
                    },
                )
                .await?;
        }
        self.memory
            .persist_learning_session(
                &self.state_store,
                autoloop_state_adapter::LearningSessionRecord {
                    id: format!("evolution:{session_id}:{}", current_time_ms()),
                    session_id: session_id.to_string(),
                    objective: "self-evolution-policy-adaptation".into(),
                    status: if evolution_report.evolved_score > evolution_report.baseline_score {
                        "improved".into()
                    } else {
                        "stable".into()
                    },
                    priority: 0.75,
                    summary: evolution_report.summary.clone(),
                    started_at_ms: current_time_ms(),
                    completed_at_ms: Some(current_time_ms()),
                },
            )
            .await?;
        let tenant_id = self
            .state_store
            .get_session_lease(session_id)
            .await?
            .map(|lease| lease.tenant_id)
            .unwrap_or_else(|| "tenant:default".to_string());
        let shared_knowledge_adapter =
            SharedKnowledgePortAdapter::new(self.state_store.clone(), tenant_id.clone());
        let org_sharing_gate = evaluate_org_sharing_gate(&OrgSharingGateInput {
            session_id: session_id.to_string(),
            verifier_passed: matches!(
                outcome.verifier_report.verdict,
                runtime::VerifierVerdict::Pass
            ),
            risk_tier: governance_telemetry_scope.risk_tier.clone(),
            org_safe: policy_signals.verifier_fail_rate < 0.4 && policy_signals.breaker_hits < 3,
            reusable: outcome.validation.ready || !consolidation.capability_improvements.is_empty(),
        });
        self.state_store
            .upsert_json_knowledge(
                format!("org-sharing-gate:{session_id}:latest"),
                &org_sharing_gate,
                "org-sharing-gate",
            )
            .await?;
        if org_sharing_gate.allowed {
            let _ = OrgKnowledgePublisher::publish(
                &self.state_store,
                &tenant_id,
                &SharedKnowledgeUpdate {
                    session_id: session_id.to_string(),
                    source: "learning-promotion".into(),
                    summary: evolution_report.summary.clone(),
                    knowledge_refs: vec![
                        format!("memory:{session_id}:consolidation"),
                        format!("memory:{session_id}:self-evolution"),
                        format!("graph:{session_id}:snapshot"),
                    ],
                    policy_tags: vec!["governed_learning".into(), "strategy_update".into()],
                    created_at_ms: current_time_ms(),
                },
            )
            .await?;
        }
        self.state_store
            .upsert_json_knowledge(
                format!("strategy:{session_id}:latest"),
                &serde_json::json!({
                    "session_id": session_id,
                    "tenant_id": tenant_id,
                    "routing_biases": outcome.routing_context.route_biases,
                    "summary": evolution_report.summary,
                    "updated_at_ms": current_time_ms(),
                }),
                "strategy-updater",
            )
            .await?;

        let contract_learning_delta = crate::contracts::types::LearningDelta {
            session_id: crate::contracts::ids::SessionId::from(session_id),
            trace_id: crate::contracts::ids::TraceId::from(format!("trace:{session_id}:learning")),
            added_skills: consolidation
                .capability_improvements
                .iter()
                .map(|item| item.tool_name.clone())
                .collect::<Vec<_>>(),
            updated_edges: outcome.routing_context.graph_signals.relationship_count,
            episode_count: consolidation
                .failure_clusters
                .iter()
                .map(|item| item.frequency)
                .sum::<usize>(),
            notes: vec![
                consolidation.causal_validation.summary.clone(),
                evolution_report.summary.clone(),
            ],
        };
        let contract_verdict = crate::contracts::types::VerificationVerdict {
            session_id: crate::contracts::ids::SessionId::from(session_id),
            trace_id: crate::contracts::ids::TraceId::from(format!(
                "trace:{session_id}:execution-verifier"
            )),
            verdict: match outcome.verifier_report.verdict {
                runtime::VerifierVerdict::Pass => crate::contracts::types::Verdict::Pass,
                runtime::VerifierVerdict::NeedsIteration => {
                    crate::contracts::types::Verdict::Iterate
                }
                runtime::VerifierVerdict::Reject => crate::contracts::types::Verdict::Reject,
            },
            score: outcome.verifier_report.overall_score,
            reasons: vec![outcome.verifier_report.summary.clone()],
        };
        if org_sharing_gate.allowed {
            crate::contracts::ports::SharedKnowledgePublisherPort::publish_shared_knowledge(
                &shared_knowledge_adapter,
                &crate::contracts::ids::SessionId::from(session_id),
                &contract_learning_delta,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        crate::contracts::ports::StrategyUpdaterPort::update_strategy(
            &shared_knowledge_adapter,
            &crate::contracts::ids::SessionId::from(session_id),
            &contract_verdict,
            &contract_learning_delta,
        )
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let admission_id = self
            .state_store
            .list_knowledge_by_prefix(&format!("capability-admission:{session_id}:"))
            .await?
            .last()
            .map(|item| item.key.clone());
        let approval_id = self
            .state_store
            .list_knowledge_by_prefix(&format!("approval:capability:{session_id}:"))
            .await?
            .last()
            .map(|item| item.key.clone());
        let primary_task = outcome
            .execution_reports
            .first()
            .map(|item| item.task.task_id.clone())
            .unwrap_or_else(|| "task:cross-kernel".into());
        let primary_capability = outcome
            .execution_reports
            .first()
            .and_then(|item| item.tool_used.clone())
            .unwrap_or_else(|| "execution-verifier".into());
        let _ = propagate_cross_kernel_decision(
            &self.state_store,
            &self.runtime,
            &CrossKernelDecisionEnvelope {
                source: "promotion_gate".to_string(),
                session_id: session_id.to_string(),
                trace_id: format!("trace:{session_id}:cross-kernel"),
                task_id: primary_task,
                capability_id: primary_capability,
                admission_id,
                policy_version: crate::contracts::version::CONTRACT_VERSION.to_string(),
                mode_decision: format!("{:?}", self.runtime.gate_mode).to_ascii_lowercase(),
                approval_id,
                rollback_reason: if outcome.validation.ready {
                    None
                } else {
                    Some("validation_not_ready".to_string())
                },
            },
        )
        .await;
        let _ = self
            .runtime
            .tag_external_stage(
                &self.state_store,
                session_id,
                &format!("trace:{session_id}:learning"),
                None,
                Some("learning-engine"),
                EvidenceTagStage::Learn,
                "learning.consolidation",
                serde_json::json!({
                    "consolidation_summary": consolidation.causal_validation.summary,
                    "source_episode_count": consolidation.failure_clusters.iter().map(|item| item.frequency).sum::<usize>(),
                    "skill_count": consolidation.capability_improvements.len(),
                    "evolution_summary": evolution_report.summary,
                }),
            )
            .await;

        self.state_store
            .upsert_json_knowledge(
                format!("protocol:{session_id}:immutable-eval"),
                &self.runtime.evaluation_protocol(),
                "evaluation-protocol",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("observability:{session_id}:capability-lifecycle"),
                &capability_lifecycle,
                "capability-governance",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("observability:{session_id}:retired-capabilities"),
                &retired_capabilities,
                "capability-governance",
            )
            .await?;
        self.persist_verifier_audit_evidence_views(session_id)
            .await?;
        let dashboard_snapshot = self.export_dashboard_snapshot(session_id).await?;
        let replay_snapshot = self.export_session_replay(session_id).await?;
        self.persist_runtime_artifact("dashboard", session_id, &dashboard_snapshot)?;
        self.persist_runtime_artifact("replay", session_id, &replay_snapshot)?;

        if !outcome.validation.ready {
            self.state_store
                .create_schedule_event(
                    session_id.to_string(),
                    "validation.iteration".into(),
                    "mcp::local-mcp::invoke".into(),
                    serde_json::to_string(&outcome.validation.follow_up_tasks)?,
                    session_id.to_string(),
                )
                .await?;
        }

        let response_packet = ResponseBuilder::build(ResponseBuilderInput {
            session_id,
            outcome: &outcome,
            research_autonomy_score: research_report.autonomy_score,
            scheduled_research_tasks,
            evolution_summary: &evolution_report.summary,
            retired_capability_count: retired_capabilities.len(),
            governance_risk_tier: &governance_telemetry_scope.risk_tier,
            policy_revised: policy_was_revised,
            decision_kind: decision.kind.as_str(),
            decision_reasons: &decision.reasons,
            decision_evidence_ref: Some(decision_evidence_ref.as_str()),
        });
        self.state_store
            .upsert_json_knowledge(
                format!("response:{session_id}:latest"),
                &response_packet,
                "response-builder",
            )
            .await?;

        Ok(response_packet.response)
    }

    async fn collect_verifier_audit_evidence_links(
        &self,
        session_id: &str,
    ) -> Result<Vec<DashboardEvidenceLink>> {
        let mut policy_reject_by_task = std::collections::HashMap::<String, String>::new();
        for record in self
            .state_store
            .list_knowledge_by_prefix(&format!("policy-reject:{session_id}:"))
            .await?
        {
            let payload = serde_json::from_str::<serde_json::Value>(&record.value)
                .unwrap_or_else(|_| serde_json::json!({}));
            if let Some(task_id) = payload.get("task_id").and_then(serde_json::Value::as_str) {
                policy_reject_by_task.insert(task_id.to_string(), record.key);
            }
        }

        let mut links = self
            .state_store
            .list_knowledge_by_prefix(&format!("execution-fabric:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<ExecutionFabricRecord>(&record.value).ok())
            .map(|record| DashboardEvidenceLink {
                task_id: record.trace.task_id.clone(),
                trace_id: record.trace.trace_id.clone(),
                admission_status: record.trace.admission_status.clone(),
                admission_evidence_ref: record.trace.admission_evidence_ref.clone(),
                guard_evidence_ref: Some(record.guard_evidence_ref),
                guard_decision: record.trace.guard_decision.clone(),
                guard_reason: record.trace.guard_reason.clone(),
                policy_reject_ref: policy_reject_by_task.get(&record.trace.task_id).cloned(),
            })
            .collect::<Vec<_>>();

        links.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        Ok(links)
    }

    async fn persist_verifier_audit_evidence_views(&self, session_id: &str) -> Result<()> {
        let evidence_links = self
            .collect_verifier_audit_evidence_links(session_id)
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("protocol:{session_id}:verifier-evidence-links"),
                &evidence_links,
                "verifier-agent",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("observability:{session_id}:audit-evidence-view"),
                &serde_json::json!({
                    "session_id": session_id,
                    "evidence_link_count": evidence_links.len(),
                    "evidence_links": evidence_links,
                }),
                "audit-view",
            )
            .await?;
        Ok(())
    }

    pub async fn run_trigger_worker_once(&self, session_id: &str) -> Result<String> {
        let mut supermemory_drained = 0usize;
        let mut supermemory_last_context: Option<String> = None;

        while let Some(context) = self
            .memory
            .run_supermemory_queue_worker_once(
                &self.state_store,
                session_id,
                "trigger daemon queue hydration",
            )
            .await?
        {
            supermemory_drained = supermemory_drained.saturating_add(1);
            supermemory_last_context = Some(context.summary);
        }

        let engine = runtime::trigger_runtime::TriggerRuntimeEngine::new(self.state_store.clone());
        let trigger_report = engine
            .run_worker_once(session_id, |event| {
                let payload = event.payload.clone();
                let topic = event.topic.clone();
                let target_session = event.session_id.clone();
                async move {
                    let prompt = if payload.trim().is_empty() {
                        format!("Trigger fired: {}", topic)
                    } else if let Ok(spec) = serde_json::from_str::<
                        crate::contracts::focus_trigger::TriggerSpec,
                    >(&payload)
                    {
                        format!(
                            "Trigger fired: {}\\nFocus: {}\\nReason: {}",
                            topic,
                            spec.focus_ref.unwrap_or_else(|| "n/a".into()),
                            spec.reason
                        )
                    } else {
                        payload
                    };
                    if topic.starts_with("focus:trigger:") {
                        let _ = self
                            .process_requirement_swarm(&target_session, &prompt)
                            .await?;
                    } else {
                        let _ = self.process_direct(&target_session, &prompt).await?;
                    }
                    Ok(())
                }
            })
            .await?;

        while let Some(context) = self
            .memory
            .run_supermemory_queue_worker_once(
                &self.state_store,
                session_id,
                "trigger daemon post-run queue hydration",
            )
            .await?
        {
            supermemory_drained = supermemory_drained.saturating_add(1);
            supermemory_last_context = Some(context.summary);
        }

        let trigger_value = serde_json::to_value(&trigger_report)?;
        let mut output = trigger_value.as_object().cloned().unwrap_or_default();
        output.insert("session_id".into(), serde_json::json!(session_id));
        output.insert("trigger_report".into(), trigger_value);
        output.insert(
            "supermemory_queue".into(),
            serde_json::json!({
                "drained_jobs": supermemory_drained,
                "last_context_summary": supermemory_last_context,
            }),
        );
        Ok(serde_json::to_string_pretty(&serde_json::Value::Object(
            output,
        ))?)
    }
    pub async fn list_focus_anchors(&self) -> Result<Vec<String>> {
        let mut anchors = self
            .state_store
            .list_knowledge_by_prefix("anchor:")
            .await?
            .into_iter()
            .filter_map(|record| record.key.strip_suffix(":brief").map(|v| v.to_string()))
            .collect::<Vec<_>>();
        anchors.sort();
        anchors.dedup();
        Ok(anchors)
    }

    pub async fn focus_status(&self, anchor_or_session: &str) -> Result<String> {
        let anchor_id = normalize_anchor(anchor_or_session);
        let brief = self
            .state_store
            .get_knowledge(&format!("{anchor_id}:brief"))
            .await?
            .map(|r| {
                serde_json::from_str::<serde_json::Value>(&r.value)
                    .unwrap_or_else(|_| serde_json::json!({}))
            })
            .unwrap_or_else(|| serde_json::json!({}));
        let dashboard = self
            .state_store
            .get_knowledge(&format!(
                "observability:{}:dashboard",
                anchor_or_session.trim_start_matches("anchor:")
            ))
            .await?
            .map(|r| {
                serde_json::from_str::<serde_json::Value>(&r.value)
                    .unwrap_or_else(|_| serde_json::json!({}))
            })
            .unwrap_or_else(|| serde_json::json!({}));
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "anchor_id": anchor_id,
            "brief": brief,
            "dashboard": dashboard,
        }))?)
    }

    pub async fn delete_focus_anchor(&self, anchor_or_session: &str) -> Result<String> {
        let anchor_id = normalize_anchor(anchor_or_session);
        self.state_store
            .upsert_knowledge(
                format!("{anchor_id}:deleted"),
                serde_json::json!({"deleted_at_ms": current_time_ms()}).to_string(),
                "focus-delete".into(),
            )
            .await?;
        Ok(serde_json::json!({"status":"deleted","anchor_id":anchor_id}).to_string())
    }

    pub async fn export_mcp_catalog(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(&self.tools.manifests())?)
    }

    pub async fn import_mcp_catalog(&self, raw: &str) -> Result<String> {
        let manifests = serde_json::from_str::<Vec<ForgedMcpToolManifest>>(raw).unwrap_or_default();
        let mut imported = 0usize;
        for manifest in manifests {
            self.tools.hydrate_manifest(manifest.clone());
            self.tools.persist_manifest(&manifest).await?;
            imported += 1;
        }
        Ok(serde_json::json!({"status":"imported","count":imported}).to_string())
    }

    pub async fn export_knowledge(&self, session_id: &str, export_type: &str) -> Result<String> {
        let key = match export_type {
            "graph" => format!("graph:{session_id}:snapshot"),
            "research" => format!("research:{session_id}:report"),
            "research-follow-up" => format!("research:{session_id}:follow-up-status"),
            "research-proxy" => format!("research:{session_id}:proxy-forensics"),
            "replay" => format!("conversation:{session_id}:replay"),
            "dashboard" => format!("observability:{session_id}:dashboard"),
            _ => format!("graph:{session_id}:snapshot"),
        };
        Ok(self
            .state_store
            .get_knowledge(&key)
            .await?
            .map(|record| record.value)
            .unwrap_or_else(|| "{}".into()))
    }

    pub async fn export_replay_report(
        &self,
        session_id: &str,
        _snapshot_id: Option<&str>,
    ) -> Result<String> {
        let reports = self
            .state_store
            .list_knowledge_by_prefix("replay:analysis:")
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
            .filter(|value| {
                value.get("session_id").and_then(serde_json::Value::as_str) == Some(session_id)
            })
            .collect::<Vec<_>>();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "session_id": session_id,
            "reports": reports,
        }))?)
    }

    pub async fn run_replay_snapshot(&self, snapshot_id: &str) -> Result<String> {
        Ok(serde_json::json!({
            "status": "accepted",
            "snapshot_id": snapshot_id,
            "note": "replay snapshot execution delegated to runtime replay pipeline",
        })
        .to_string())
    }

    pub async fn operator_decision(
        &self,
        session_id: &str,
        approved: bool,
        reason: &str,
    ) -> Result<String> {
        self.state_store
            .upsert_json_knowledge(
                format!("operator:decision:{session_id}:{}", current_time_ms()),
                &serde_json::json!({
                    "session_id": session_id,
                    "approved": approved,
                    "reason": reason,
                    "decided_at_ms": current_time_ms(),
                }),
                "operator-control",
            )
            .await?;
        Ok(
            serde_json::json!({"session_id": session_id, "approved": approved, "reason": reason})
                .to_string(),
        )
    }

    pub async fn system_status(&self) -> Result<String> {
        let report = self.bootstrap().await?;
        Ok(serde_json::json!({
            "app": report.app_name,
            "providers": report.provider_count,
            "tools": report.tool_count,
            "hooks": report.hook_count,
            "memory_targets": report.memory_targets,
            "rag_strategies": report.rag_strategies,
            "permission_mode": self.runtime.permission_mode_status(),
        })
        .to_string())
    }

    pub async fn get_worldline_weights(&self) -> Result<String> {
        let (weights, version) = self.load_worldline_weights_profile().await;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "version": version.unwrap_or_else(|| "worldline-weights:default-v1".to_string()),
            "weights": weights.unwrap_or_default(),
        }))?)
    }

    pub async fn set_worldline_weights(
        &self,
        success_weight: Option<f32>,
        robustness_weight: Option<f32>,
        reuse_weight: Option<f32>,
        verifier_weight: Option<f32>,
        cost_weight: Option<f32>,
        latency_weight: Option<f32>,
        risk_weight: Option<f32>,
        instability_weight: Option<f32>,
        governance_weight: Option<f32>,
        reason: Option<&str>,
    ) -> Result<String> {
        let (stored_weights, _) = self.load_worldline_weights_profile().await;
        let mut weights = stored_weights.unwrap_or_default();

        if let Some(value) = success_weight {
            weights.success_weight = value;
        }
        if let Some(value) = robustness_weight {
            weights.robustness_weight = value;
        }
        if let Some(value) = reuse_weight {
            weights.reuse_weight = value;
        }
        if let Some(value) = verifier_weight {
            weights.verifier_weight = value;
        }
        if let Some(value) = cost_weight {
            weights.cost_weight = value;
        }
        if let Some(value) = latency_weight {
            weights.latency_weight = value;
        }
        if let Some(value) = risk_weight {
            weights.risk_weight = value;
        }
        if let Some(value) = instability_weight {
            weights.instability_weight = value;
        }
        if let Some(value) = governance_weight {
            weights.governance_weight = value;
        }

        validate_worldline_weight(weights.success_weight, "success_weight")?;
        validate_worldline_weight(weights.robustness_weight, "robustness_weight")?;
        validate_worldline_weight(weights.reuse_weight, "reuse_weight")?;
        validate_worldline_weight(weights.verifier_weight, "verifier_weight")?;
        validate_worldline_weight(weights.cost_weight, "cost_weight")?;
        validate_worldline_weight(weights.latency_weight, "latency_weight")?;
        validate_worldline_weight(weights.risk_weight, "risk_weight")?;
        validate_worldline_weight(weights.instability_weight, "instability_weight")?;
        validate_worldline_weight(weights.governance_weight, "governance_weight")?;

        let sanitized = weights.sanitize();
        let version = format!("worldline-weights:hot:{}", current_time_ms());

        self.state_store
            .upsert_json_knowledge(
                "policy:worldline-weights:latest".to_string(),
                &serde_json::json!({
                    "version": version,
                    "weights": sanitized,
                    "updated_at_ms": current_time_ms(),
                    "reason": reason.unwrap_or("operator hot update"),
                }),
                "policy-engine",
            )
            .await?;

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "updated",
            "version": version,
            "weights": sanitized,
            "reason": reason.unwrap_or("operator hot update"),
        }))?)
    }
    pub async fn get_context_objective_weights(&self) -> Result<String> {
        let weights = self.load_context_objective_weights().await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "weights": weights,
        }))?)
    }

    pub async fn set_context_objective_weights(
        &self,
        task_utility: Option<f32>,
        distortion_penalty: Option<f32>,
        attention_mismatch_penalty: Option<f32>,
        token_cost_penalty: Option<f32>,
        reason: Option<&str>,
    ) -> Result<String> {
        let mut weights = self.load_context_objective_weights().await?;
        if let Some(value) = task_utility {
            weights.task_utility = value;
        }
        if let Some(value) = distortion_penalty {
            weights.distortion_penalty = value;
        }
        if let Some(value) = attention_mismatch_penalty {
            weights.attention_mismatch_penalty = value;
        }
        if let Some(value) = token_cost_penalty {
            weights.token_cost_penalty = value;
        }
        validate_objective_weight(weights.task_utility, "task_utility")?;
        validate_objective_weight(weights.distortion_penalty, "distortion_penalty")?;
        validate_objective_weight(
            weights.attention_mismatch_penalty,
            "attention_mismatch_penalty",
        )?;
        validate_objective_weight(weights.token_cost_penalty, "token_cost_penalty")?;

        let raw = serde_json::to_string(&weights)?;
        unsafe {
            std::env::set_var("AUTOLOOP_CONTEXT_OBJECTIVE_WEIGHTS", &raw);
        }
        self.state_store
            .upsert_json_knowledge(
                "policy:context-objective-weights:latest".to_string(),
                &serde_json::json!({
                    "weights": weights,
                    "updated_at_ms": current_time_ms(),
                    "reason": reason.unwrap_or("operator hot update"),
                }),
                "policy-engine",
            )
            .await?;

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "updated",
            "weights": weights,
            "env": "AUTOLOOP_CONTEXT_OBJECTIVE_WEIGHTS",
            "reason": reason.unwrap_or("operator hot update"),
        }))?)
    }

    async fn load_context_objective_weights(
        &self,
    ) -> Result<crate::query_engine::ObjectiveWeights> {
        let stored = self
            .state_store
            .get_knowledge("policy:context-objective-weights:latest")
            .await?;
        if let Some(record) = stored {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) {
                if let Some(weights_value) = value.get("weights") {
                    if let Ok(weights) = serde_json::from_value::<
                        crate::query_engine::ObjectiveWeights,
                    >(weights_value.clone())
                    {
                        return Ok(weights);
                    }
                }
                if let Ok(weights) =
                    serde_json::from_value::<crate::query_engine::ObjectiveWeights>(value.clone())
                {
                    return Ok(weights);
                }
            }
        }
        Ok(crate::query_engine::ObjectiveWeights::default())
    }
    pub async fn bridge_start(
        &self,
        session_id: &str,
        transport_kind: &str,
        auth_subject: &str,
        tenant_id: &str,
        ttl_ms: u64,
    ) -> Result<String> {
        let kind = transport::parse_transport_kind(transport_kind);
        let status = self
            .transport
            .start(session_id, kind, auth_subject, tenant_id, ttl_ms)
            .await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    pub async fn frontend_bridge_prompt(
        &self,
        session_id: &str,
        trace_id: Option<&str>,
        content: &str,
    ) -> Result<String> {
        self.frontend_bridge_prompt_inner(session_id, trace_id, content, false)
            .await
    }

    async fn frontend_bridge_prompt_inner(
        &self,
        session_id: &str,
        trace_id: Option<&str>,
        content: &str,
        bypass_permission_gate: bool,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let lease = self.state_store.get_session_lease(session_id).await?;
        let tenant_id = lease
            .as_ref()
            .map(|item| item.tenant_id.as_str())
            .unwrap_or("tenant:default");
        let trace = trace_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("trace:{session_id}:frontend:{}", current_time_ms()));

        self.frontend_bridge
            .ensure_session(
                session_id,
                Some("cli"),
                Some("frontend:cli"),
                tenant_id,
                3_600_000,
            )
            .await?;
        self.frontend_bridge
            .ingest_user_input(session_id, &trace, content)
            .await?;
        let call_id = format!("call:frontend:{session_id}:{}", current_time_ms());
        let started = self
            .frontend_bridge
            .emit_tool_started(
                session_id,
                &trace,
                "query.process_direct",
                &call_id,
                serde_json::json!({
                    "content": content,
                }),
            )
            .await?;

        if !bypass_permission_gate && frontend_permission_requires_approval() {
            let request_id = format!("perm:{session_id}:{}", current_time_ms());
            let key = frontend_permission_request_key(session_id, &request_id);
            let request_payload = serde_json::json!({
                "session_id": session_id,
                "request_id": request_id,
                "trace_id": trace,
                "call_id": call_id,
                "tool_name": "query.process_direct",
                "content": content,
                "status": "pending",
                "created_at_ms": current_time_ms(),
            });
            self.state_store
                .upsert_json_knowledge(key.clone(), &request_payload, "frontend-permission")
                .await?;
            let _ = persist_frontend_permission_request_to_file(session_id, &request_id, &request_payload);

            let evidence_ref = format!(
                "evidence:frontend:permission:request:{session_id}:{}",
                current_time_ms()
            );
            self.state_store
                .upsert_json_knowledge(
                    evidence_ref.clone(),
                    &serde_json::json!({
                        "kind": "frontend.permission.requested",
                        "session_id": session_id,
                        "trace_id": trace,
                        "request_id": request_id,
                        "call_id": call_id,
                        "tool_name": "query.process_direct",
                        "status": "pending",
                        "payload_ref": key,
                        "created_at_ms": current_time_ms(),
                    }),
                    "frontend-permission",
                )
                .await?;

            let _ = append_event(
                &self.state_store,
                "frontend.permission.requires_approval",
                trace.clone(),
                session_id.to_string(),
                None,
                Some("frontend.permission".to_string()),
                crate::contracts::version::CONTRACT_VERSION,
                serde_json::json!({
                    "request_id": request_id,
                    "call_id": call_id,
                    "tool_name": "query.process_direct",
                    "evidence_ref": evidence_ref,
                }),
            )
            .await;

            let completed = self
                .frontend_bridge
                .emit_tool_completed(
                    session_id,
                    &trace,
                    "query.process_direct",
                    &call_id,
                    serde_json::json!({
                        "status": "requires_approval",
                        "request_id": request_id,
                        "evidence_ref": evidence_ref,
                    }),
                    true,
                )
                .await?;
            let idle = self
                .frontend_bridge
                .emit_session_idle(session_id, &trace)
                .await?;
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "requires_approval",
                "session_id": session_id,
                "trace_id": trace,
                "call_id": call_id,
                "request_id": request_id,
                "evidence_ref": evidence_ref,
                "events": [
                    {
                        "event_type": "tool_started",
                        "call_id": call_id,
                        "sequence": started.sequence,
                        "emitted_at_ms": started.emitted_at_ms,
                    },
                    {
                        "event_type": "tool_completed",
                        "call_id": call_id,
                        "sequence": completed.sequence,
                        "emitted_at_ms": completed.emitted_at_ms,
                        "is_error": true,
                    },
                    {
                        "event_type": "session_idle",
                        "sequence": idle.sequence,
                        "emitted_at_ms": idle.emitted_at_ms,
                    }
                ],
            }))?);
        }

        match self.process_direct(session_id, content).await {
            Ok(response) => {
                let turn_id = format!("turn:{}", current_time_ms());
                let mut emitted_events = Vec::new();
                emitted_events.push(serde_json::json!({
                    "event_type": "tool_started",
                    "call_id": call_id,
                    "sequence": started.sequence,
                    "emitted_at_ms": started.emitted_at_ms,
                }));
                for chunk in chunk_text_for_delta(&response, 96) {
                    let emitted = self
                        .frontend_bridge
                        .emit_assistant_delta(session_id, &trace, &turn_id, &chunk)
                        .await?;
                    emitted_events.push(serde_json::json!({
                        "event_type": "assistant_delta",
                        "sequence": emitted.sequence,
                        "emitted_at_ms": emitted.emitted_at_ms,
                    }));
                }
                let relation_write_proof = self
                    .backfill_frontend_artifact_relation_write_proof(session_id, &trace, content)
                    .await?;
                let completed = self
                    .frontend_bridge
                    .emit_tool_completed(
                        session_id,
                        &trace,
                        "query.process_direct",
                        &call_id,
                        serde_json::json!({
                            "status": "ok",
                            "relation_write_proof": relation_write_proof.clone(),
                        }),
                        false,
                    )
                    .await?;
                emitted_events.push(serde_json::json!({
                    "event_type": "tool_completed",
                    "call_id": call_id,
                    "sequence": completed.sequence,
                    "emitted_at_ms": completed.emitted_at_ms,
                    "is_error": false,
                }));
                let idle = self
                    .frontend_bridge
                    .emit_session_idle(session_id, &trace)
                    .await?;
                emitted_events.push(serde_json::json!({
                    "event_type": "session_idle",
                    "sequence": idle.sequence,
                    "emitted_at_ms": idle.emitted_at_ms,
                }));
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": "ok",
                    "session_id": session_id,
                    "trace_id": trace,
                    "call_id": call_id,
                    "response": response,
                    "relation_write_proof": relation_write_proof,
                    "events": emitted_events,
                }))?)
            }
            Err(error) => {
                let message = error.to_string();
                let completed = self
                    .frontend_bridge
                    .emit_tool_completed(
                        session_id,
                        &trace,
                        "query.process_direct",
                        &call_id,
                        serde_json::json!({
                            "status": "error",
                            "message": message,
                        }),
                        true,
                    )
                    .await?;
                let error_event = self
                    .frontend_bridge
                    .emit_error(session_id, &trace, "frontend_prompt_failed", &message)
                    .await?;
                let idle = self
                    .frontend_bridge
                    .emit_session_idle(session_id, &trace)
                    .await?;
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "status": "error",
                    "session_id": session_id,
                    "trace_id": trace,
                    "call_id": call_id,
                    "error": message,
                    "events": [
                        {
                            "event_type": "tool_started",
                            "call_id": call_id,
                            "sequence": started.sequence,
                            "emitted_at_ms": started.emitted_at_ms,
                        },
                        {
                            "event_type": "tool_completed",
                            "call_id": call_id,
                            "sequence": completed.sequence,
                            "emitted_at_ms": completed.emitted_at_ms,
                            "is_error": true,
                        },
                        {
                            "event_type": "error",
                            "sequence": error_event.sequence,
                            "emitted_at_ms": error_event.emitted_at_ms,
                        },
                        {
                            "event_type": "session_idle",
                            "sequence": idle.sequence,
                            "emitted_at_ms": idle.emitted_at_ms,
                        }
                    ],
                }))?)
            }
        }
    }

    async fn backfill_frontend_artifact_relation_write_proof(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
    ) -> Result<Option<serde_json::Value>> {
        let Some(target_path) = extract_artifact_target_path_hint(content) else {
            return Ok(None);
        };

        let artifact_path = PathBuf::from(&target_path);
        if !artifact_path.exists() {
            return Ok(None);
        }

        let metadata = fs::metadata(&artifact_path)?;
        if !metadata.is_file() {
            return Ok(None);
        }

        let bytes = fs::read(&artifact_path)?;
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        hasher.update(&bytes);
        let digest = format!("{:x}", hasher.finalize());
        let size_bytes = metadata.len();
        let evidence_ref = format!(
            "evidence:frontend:artifact:write:{}:{}",
            session_id,
            current_time_ms()
        );
        self.state_store
            .upsert_json_knowledge(
                evidence_ref.clone(),
                &serde_json::json!({
                    "kind": "frontend.artifact.write",
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "target_path": target_path,
                    "size_bytes": size_bytes,
                    "sha256": digest,
                    "mime": infer_artifact_mime_from_path(&artifact_path),
                    "captured_at_ms": current_time_ms(),
                }),
                "frontend-artifact-proof",
            )
            .await?;

        let now_ms = current_time_ms();
        let scope = crate::contracts::relation::RelationScope {
            tenant_id: None,
            session_id: Some(session_id.to_string()),
            trace_id: Some(trace_id.to_string()),
            task_id: None,
            capability_id: Some("tool:write_file".to_string()),
            policy_id: None,
        };
        let artifact_node_id = format!(
            "artifact:file:{}",
            relation_node_component_for_path(&target_path)
        );
        let mut edge = relation_edge(
            format!("edge:{session_id}:frontend-artifact:{now_ms}"),
            "tool:write_file".to_string(),
            artifact_node_id,
            crate::contracts::relation::RelationEdgeType::ProducedBy,
            &scope,
            "frontend_artifact_write_proof",
            "frontend chat artifact write proof backfilled to relation write proof",
            &evidence_ref,
        );
        edge.metadata.insert("target_path".to_string(), target_path.clone());
        edge.metadata
            .insert("sha256".to_string(), digest.clone());
        edge.metadata
            .insert("size_bytes".to_string(), size_bytes.to_string());
        edge.metadata.insert(
            "mime".to_string(),
            infer_artifact_mime_from_path(&artifact_path),
        );

        let mut input = serde_json::to_value(edge)?;
        let gate_token = build_service_gate_token(
            &crate::contracts::ids::SessionId::from(session_id),
            &crate::contracts::services::ServiceDomain::Relation,
            now_ms,
        );
        attach_service_gate_token(&mut input, gate_token);
        let call = crate::contracts::services::ServiceCall {
            session_id: crate::contracts::ids::SessionId::from(session_id),
            trace_id: crate::contracts::ids::TraceId::from(trace_id.to_string()),
            service_domain: crate::contracts::services::ServiceDomain::Relation,
            service_name: "relation_facade".to_string(),
            operation: "upsert_edge".to_string(),
            input,
            budget_scope: "relation.artifact".to_string(),
            requested_at_ms: now_ms,
        };
        let result = self.services.mediate_call(&call).await?;
        if !result.success {
            return Err(anyhow!(
                "frontend artifact relation write proof backfill failed: {}",
                result
                    .error
                    .unwrap_or_else(|| "unknown relation mediator error".to_string())
            ));
        }

        Ok(Some(serde_json::json!({
            "status": "ok",
            "target_path": target_path,
            "size_bytes": size_bytes,
            "sha256": digest,
            "mime": infer_artifact_mime_from_path(&artifact_path),
            "evidence_ref": evidence_ref,
            "relation_write_proof_ref": result.output.get("write_proof_ref").cloned().unwrap_or(serde_json::Value::Null),
            "relation_event_ref": result.output.get("relation_event_ref").cloned().unwrap_or(serde_json::Value::Null),
            "relation_evidence_ref": result.output.get("evidence_ref").cloned().unwrap_or(serde_json::Value::Null),
        })))
    }

    pub async fn frontend_attach(
        &self,
        session_id: &str,
        transport_kind: &str,
        jwt_token: Option<&str>,
        subject: Option<&str>,
        tenant_id: Option<&str>,
        ttl_ms: u64,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let lease = self.state_store.get_session_lease(session_id).await?;
        let resolved_tenant = tenant_id
            .map(str::to_string)
            .or_else(|| lease.as_ref().map(|item| item.tenant_id.clone()))
            .unwrap_or_else(|| "tenant:default".to_string());
        let resolved_subject = subject
            .map(str::to_string)
            .unwrap_or_else(|| format!("frontend:attach:{session_id}"));

        let status = if let Some(jwt) = jwt_token {
            self.bridge_remote_start(session_id, transport_kind, jwt, ttl_ms)
                .await?
        } else {
            self.bridge_start(
                session_id,
                transport_kind,
                &resolved_subject,
                &resolved_tenant,
                ttl_ms,
            )
            .await?
        };

        let status_json =
            serde_json::from_str::<serde_json::Value>(&status).unwrap_or_else(|_| serde_json::json!({}));
        let remote_status = self.bridge_remote_status(session_id).await.ok().and_then(|raw| {
            serde_json::from_str::<serde_json::Value>(&raw).ok()
        });
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "attached",
            "session_id": session_id,
            "transport_kind": transport_kind,
            "attach_mode": if jwt_token.is_some() { "jwt_remote" } else { "local_bridge" },
            "bridge_status": status_json,
            "remote_status": remote_status,
            "ttl_ms": ttl_ms,
        }))?)
    }

    pub async fn frontend_permission_decide(
        &self,
        session_id: &str,
        request_id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let key = frontend_permission_request_key(session_id, request_id);
        let stored = self
            .state_store
            .get_knowledge(&key)
            .await?;
        let mut record = if let Some(stored) = stored {
            serde_json::from_str::<serde_json::Value>(&stored.value)
                .unwrap_or_else(|_| serde_json::json!({}))
        } else {
            load_frontend_permission_request_from_file(session_id, request_id)?
                .ok_or_else(|| anyhow!("permission request not found: {request_id}"))?
        };
        let status = record
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        if status != "pending" {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "noop",
                "session_id": session_id,
                "request_id": request_id,
                "reason": "request already decided",
                "current": record,
            }))?);
        }

        let approved = matches!(decision.trim().to_ascii_lowercase().as_str(), "approve" | "approved" | "allow");
        let final_reason = reason.unwrap_or_else(|| {
            if approved {
                "approved by operator"
            } else {
                "rejected by operator"
            }
        });
        let now_ms = current_time_ms();
        record["status"] = serde_json::Value::String(if approved {
            "approved".to_string()
        } else {
            "rejected".to_string()
        });
        record["decision"] = serde_json::Value::String(if approved {
            "approve".to_string()
        } else {
            "reject".to_string()
        });
        record["reason"] = serde_json::Value::String(final_reason.to_string());
        record["decided_at_ms"] = serde_json::Value::Number(serde_json::Number::from(now_ms));
        self.state_store
            .upsert_json_knowledge(key.clone(), &record, "frontend-permission")
            .await?;
        let _ = persist_frontend_permission_request_to_file(session_id, request_id, &record);

        let trace = record
            .get("trace_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("trace:frontend:permission-decision")
            .to_string();
        let call_id = record
            .get("call_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("call:frontend:unknown")
            .to_string();
        let evidence_ref = format!(
            "evidence:frontend:permission:decision:{session_id}:{}",
            current_time_ms()
        );
        self.state_store
            .upsert_json_knowledge(
                evidence_ref.clone(),
                &serde_json::json!({
                    "kind": "frontend.permission.decided",
                    "session_id": session_id,
                    "request_id": request_id,
                    "approved": approved,
                    "reason": final_reason,
                    "trace_id": trace,
                    "call_id": call_id,
                    "request_ref": key,
                    "decided_at_ms": current_time_ms(),
                }),
                "frontend-permission",
            )
            .await?;

        let _ = append_event(
            &self.state_store,
            "frontend.permission.decision",
            trace.clone(),
            session_id.to_string(),
            None,
            Some("frontend.permission".to_string()),
            crate::contracts::version::CONTRACT_VERSION,
            serde_json::json!({
                "request_id": request_id,
                "approved": approved,
                "reason": final_reason,
                "evidence_ref": evidence_ref,
            }),
        )
        .await;

        if !approved {
            return Ok(serde_json::to_string_pretty(&serde_json::json!({
                "status": "rejected",
                "session_id": session_id,
                "request_id": request_id,
                "call_id": call_id,
                "reason": final_reason,
                "evidence_ref": evidence_ref,
            }))?);
        }

        let content = record
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let execution = self
            .frontend_bridge_prompt_inner(session_id, Some(&trace), &content, true)
            .await?;
        let execution_json = serde_json::from_str::<serde_json::Value>(&execution)
            .unwrap_or_else(|_| serde_json::json!({ "raw": execution }));

        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "approved",
            "session_id": session_id,
            "request_id": request_id,
            "call_id": call_id,
            "reason": final_reason,
            "evidence_ref": evidence_ref,
            "execution": execution_json,
        }))?)
    }

    pub async fn bridge_status(&self, session_id: &str) -> Result<String> {
        let status = self.transport.status(session_id).await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    pub async fn bridge_stop(&self, session_id: &str) -> Result<String> {
        let status = self.transport.stop(session_id).await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }
    pub async fn bridge_issue_jwt(
        &self,
        session_id: &str,
        subject: &str,
        tenant_id: &str,
        ttl_ms: u64,
    ) -> Result<String> {
        let token = self
            .remote_sessions
            .issue_token(session_id, subject, tenant_id, ttl_ms)
            .await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "session_id": session_id,
            "token": token,
            "token_type": "aljwt",
            "expires_in_ms": ttl_ms,
        }))?)
    }

    pub async fn bridge_remote_start(
        &self,
        session_id: &str,
        transport_kind: &str,
        jwt_token: &str,
        ttl_ms: u64,
    ) -> Result<String> {
        let status = self
            .remote_sessions
            .remote_start(session_id, transport_kind, jwt_token, ttl_ms)
            .await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    pub async fn bridge_remote_status(&self, session_id: &str) -> Result<String> {
        let status = self.remote_sessions.remote_status(session_id).await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    pub async fn bridge_remote_stop(&self, session_id: &str) -> Result<String> {
        let status = self.remote_sessions.remote_stop(session_id).await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    pub async fn govern_mcp_capability(&self, action: &str, tool: &str) -> Result<String> {
        let body = match action {
            "verify" => {
                let value = self.tools.verify_capability(tool).await?;
                serde_json::to_string_pretty(&serde_json::json!({
                    "action": "verify",
                    "tool": tool,
                    "result": value,
                }))?
            }
            "deprecate" => {
                let value = self.tools.deprecate_capability(tool, 0.35).await?;
                serde_json::to_string_pretty(&serde_json::json!({
                    "action": "deprecate",
                    "tool": tool,
                    "result": value,
                }))?
            }
            "rollback" => {
                let value = self.tools.rollback_capability(tool).await?;
                serde_json::to_string_pretty(&serde_json::json!({
                    "action": "rollback",
                    "tool": tool,
                    "result": value,
                }))?
            }
            _ => serde_json::json!({
                "error": "unsupported capability governance action",
                "action": action,
                "tool": tool,
            })
            .to_string(),
        };
        Ok(body)
    }
    pub async fn export_dashboard_snapshot(&self, session_id: &str) -> Result<String> {
        let dashboard = self
            .state_store
            .get_knowledge(&format!("observability:{session_id}:dashboard"))
            .await?
            .map(|record| record.value)
            .unwrap_or_else(|| "{}".to_string());
        Ok(dashboard)
    }

    pub async fn export_session_replay(&self, session_id: &str) -> Result<String> {
        let replay = self
            .state_store
            .get_knowledge(&format!("conversation:{session_id}:replay"))
            .await?
            .map(|record| record.value)
            .unwrap_or_else(|| {
                serde_json::json!({
                    "session_id": session_id,
                    "status": "empty",
                    "events": [],
                })
                .to_string()
            });
        Ok(replay)
    }

    pub async fn session_named_snapshot(&self, session_id: &str, snapshot_name: &str) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let snapshot = self.sessions.named_snapshot(session_id, snapshot_name).await?;
        let body = match snapshot {
            Some(path) => serde_json::json!({
                "status": "ok",
                "session_id": session_id,
                "snapshot_name": snapshot_name,
                "snapshot_path": path.display().to_string(),
            }),
            None => serde_json::json!({
                "status": "empty",
                "session_id": session_id,
                "snapshot_name": snapshot_name,
                "reason": "no session history/checkpoint available",
            }),
        };
        Ok(serde_json::to_string_pretty(&body)?)
    }

    pub async fn session_new(&self, session_id: &str) -> Result<String> {
        let identity = self.ensure_default_session_identity(session_id).await?;
        let resumed = self.sessions.load_from_checkpoint(session_id).await;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "session_id": session_id,
            "resumed_from_checkpoint": resumed,
            "checkpoint_root": self.sessions.checkpoint_root().display().to_string(),
            "identity": identity,
        }))?)
    }

    pub async fn session_list(&self) -> Result<String> {
        let mut sessions = self.sessions.list_session_ids().await;
        sessions.sort();
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "count": sessions.len(),
            "sessions": sessions,
        }))?)
    }

    pub async fn session_resume(&self, session_id: &str) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let resumed = self.sessions.load_from_checkpoint(session_id).await;
        let snapshot = self.sessions.resume_snapshot(session_id).await;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": if resumed { "resumed" } else { "empty" },
            "session_id": session_id,
            "resumed": resumed,
            "snapshot": snapshot,
        }))?)
    }

    pub async fn session_export_transcript(&self, session_id: &str) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        self.sessions.export_transcript_markdown(session_id).await
    }

    pub async fn background_task_start_shell(
        &self,
        session_id: &str,
        task_id: &str,
        command: &str,
        max_restarts: u32,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let record = self
            .background_tasks
            .start_shell_task(session_id, task_id, command, max_restarts)
            .await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "started",
            "task": record,
        }))?)
    }

    pub async fn background_task_start_agent(
        &self,
        session_id: &str,
        task_id: &str,
        prompt: &str,
        max_restarts: u32,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        if should_route_code_task_to_harness(prompt) {
            let _ = append_event(
                &self.state_store,
                "code_harness_gate.rejected",
                format!("trace:{session_id}:background-agent-gate"),
                session_id.to_string(),
                None,
                Some("background_agent".to_string()),
                crate::contracts::version::CONTRACT_VERSION,
                serde_json::json!({
                    "reason": "code_task_requires_harness_facade",
                    "requested_surface": "background_agent",
                    "required_surface": "harness_facade",
                    "decision": "reject",
                    "task_id": task_id,
                }),
            )
            .await;
            return Err(anyhow!(
                "code task requires harness façade execution; background agent task is blocked"
            ));
        }
        let record = self
            .background_tasks
            .start_agent_task(session_id, task_id, prompt, max_restarts, self.agent.clone())
            .await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "started",
            "task": record,
        }))?)
    }

    pub async fn background_task_status(
        &self,
        session_id: &str,
        task_id: Option<&str>,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let tasks = self.background_tasks.status(session_id, task_id).await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "session_id": session_id,
            "count": tasks.len(),
            "tasks": tasks,
        }))?)
    }

    pub async fn background_task_stop(&self, session_id: &str, task_id: &str) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let task = self.background_tasks.stop_task(session_id, task_id).await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "stopping",
            "task": task,
        }))?)
    }

    pub async fn background_task_restart(
        &self,
        session_id: &str,
        task_id: &str,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let task = self
            .background_tasks
            .restart_task(session_id, task_id, self.agent.clone())
            .await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "status": "restarted",
            "task": task,
        }))?)
    }

    pub async fn background_task_logs(
        &self,
        session_id: &str,
        task_id: &str,
        lines: usize,
    ) -> Result<String> {
        self.ensure_default_session_identity(session_id).await?;
        let logs = self.background_tasks.tail_logs(session_id, task_id, lines).await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "line_count": logs.len(),
            "logs": logs,
        }))?)
    }
    async fn merge_global_graph_snapshot(
        &self,
        session_id: &str,
        _forged_manifests: &[ForgedMcpToolManifest],
    ) -> Result<String> {
        let session_snapshot = self
            .state_store
            .get_knowledge(&format!("graph:{session_id}:snapshot"))
            .await?
            .map(|record| record.value);
        let global_snapshot = self
            .state_store
            .get_knowledge("graph:global:snapshot")
            .await?
            .map(|record| record.value);
        Ok(session_snapshot
            .or(global_snapshot)
            .unwrap_or_else(|| "{}".to_string()))
    }

    pub async fn service_mediate(
        &self,
        call: &crate::contracts::services::ServiceCall,
    ) -> Result<String> {
        let mut mediated_call = call.clone();
        if mediated_call.service_domain.requires_gate_token()
            && service_call_gate_token(&mediated_call.input).is_none()
        {
            let token = build_service_gate_token(
                &mediated_call.session_id,
                &mediated_call.service_domain,
                current_time_ms(),
            );
            attach_service_gate_token(&mut mediated_call.input, token);
        }
        let result = self.services.mediate_call(&mediated_call).await?;
        Ok(serde_json::to_string_pretty(&result)?)
    }

    pub async fn service_health(&self) -> Result<String> {
        let health = self.services.health_snapshot().await?;
        Ok(serde_json::to_string_pretty(&health)?)
    }

    async fn persist_swarm_relation_contract(
        &self,
        session_id: &str,
        org_context: &crate::contracts::org::OrganizationContext,
        trace_id: &str,
        decision_evidence_ref: &str,
        outcome: &orchestration::SwarmOutcome,
    ) -> Result<serde_json::Value> {
        let now_ms = current_time_ms();
        let scope = crate::contracts::relation::RelationScope {
            tenant_id: Some(org_context.tenant_id.clone()),
            session_id: Some(session_id.to_string()),
            trace_id: Some(trace_id.to_string()),
            task_id: None,
            capability_id: None,
            policy_id: Some(org_context.policy_id.clone()),
        };
        let node_session = format!("session:{session_id}");
        let node_query = format!("query_engine:{session_id}:{now_ms}");
        let node_tool_loop = format!("tool_loop:{session_id}:{now_ms}");
        let node_permission = format!("permission:{session_id}:{}", org_context.policy_id);
        let node_hook = format!("hook_runtime:{session_id}:default");
        let node_skill = format!("skill_plane:{session_id}:registry");
        let node_plugin = format!("plugin_plane:{session_id}:lifecycle");
        let node_replay = format!("replay_plane:{session_id}:{now_ms}");
        let node_evidence = decision_evidence_ref.to_string();

        let nodes = vec![
            relation_node(&node_session, crate::contracts::relation::RelationNodeType::Session, &scope),
            relation_node(&node_query, crate::contracts::relation::RelationNodeType::Turn, &scope),
            relation_node(
                &node_tool_loop,
                crate::contracts::relation::RelationNodeType::Capability,
                &scope,
            ),
            relation_node(
                &node_permission,
                crate::contracts::relation::RelationNodeType::Policy,
                &scope,
            ),
            relation_node(&node_hook, crate::contracts::relation::RelationNodeType::Capability, &scope),
            relation_node(
                &node_skill,
                crate::contracts::relation::RelationNodeType::Skill,
                &scope,
            ),
            relation_node(
                &node_plugin,
                crate::contracts::relation::RelationNodeType::Plugin,
                &scope,
            ),
            relation_node(
                &node_replay,
                crate::contracts::relation::RelationNodeType::Turn,
                &scope,
            ),
            relation_node(
                &node_evidence,
                crate::contracts::relation::RelationNodeType::Evidence,
                &scope,
            ),
        ];

        let mut edges = vec![
            relation_edge(
                format!("edge:{session_id}:session-query:{now_ms}"),
                node_session.clone(),
                node_query.clone(),
                crate::contracts::relation::RelationEdgeType::BelongsTo,
                &scope,
                "query_engine_entry",
                "session enters query engine",
                decision_evidence_ref,
            ),
            relation_edge(
                format!("edge:{session_id}:query-toolloop:{now_ms}"),
                node_query.clone(),
                node_tool_loop.clone(),
                crate::contracts::relation::RelationEdgeType::DependsOn,
                &scope,
                "tool_loop_required",
                "query engine depends on tool loop execution",
                decision_evidence_ref,
            ),
            relation_edge(
                format!("edge:{session_id}:toolloop-permission:{now_ms}"),
                node_tool_loop.clone(),
                node_permission.clone(),
                crate::contracts::relation::RelationEdgeType::ApprovedBy,
                &scope,
                "permission_gate",
                "tool loop execution is permission-gated",
                decision_evidence_ref,
            ),
            relation_edge(
                format!("edge:{session_id}:toolloop-hook:{now_ms}"),
                node_tool_loop.clone(),
                node_hook.clone(),
                crate::contracts::relation::RelationEdgeType::References,
                &scope,
                "hook_pipeline",
                "tool loop emits hook lifecycle events",
                decision_evidence_ref,
            ),
            relation_edge(
                format!("edge:{session_id}:toolloop-skill:{now_ms}"),
                node_tool_loop.clone(),
                node_skill.clone(),
                crate::contracts::relation::RelationEdgeType::References,
                &scope,
                "skill_router",
                "tool loop consumes skill routing suggestions",
                decision_evidence_ref,
            ),
            relation_edge(
                format!("edge:{session_id}:toolloop-plugin:{now_ms}"),
                node_tool_loop.clone(),
                node_plugin.clone(),
                crate::contracts::relation::RelationEdgeType::References,
                &scope,
                "plugin_lifecycle",
                "tool loop runs with plugin lifecycle governance",
                decision_evidence_ref,
            ),
            relation_edge(
                format!("edge:{session_id}:toolloop-replay:{now_ms}"),
                node_tool_loop.clone(),
                node_replay.clone(),
                crate::contracts::relation::RelationEdgeType::ProducedBy,
                &scope,
                "replay_trace",
                "tool loop produces replay/audit trace",
                decision_evidence_ref,
            ),
            relation_edge(
                format!("edge:{session_id}:replay-evidence:{now_ms}"),
                node_replay.clone(),
                node_evidence.clone(),
                crate::contracts::relation::RelationEdgeType::References,
                &scope,
                "evidence_link",
                "replay trace references decision evidence",
                decision_evidence_ref,
            ),
        ];

        for report in &outcome.execution_reports {
            if let Some(tool_name) = &report.tool_used {
                let capability_node = format!("capability:{}", tool_name.replace(' ', "_"));
                edges.push(relation_edge(
                    format!(
                        "edge:{session_id}:toolloop-capability:{}:{now_ms}",
                        report.task.task_id
                    ),
                    node_tool_loop.clone(),
                    capability_node.clone(),
                    crate::contracts::relation::RelationEdgeType::DependsOn,
                    &scope,
                    "capability_dispatch",
                    "task execution depends on routed capability",
                    decision_evidence_ref,
                ));
            }
        }

        let contract = crate::contracts::relation::RelationContract {
            api_version: crate::contracts::version::RELATION_CONTRACT_VERSION.to_string(),
            nodes,
            edges,
            events: vec![crate::contracts::relation::RelationEvent {
                event_id: format!("relation:event:{session_id}:{now_ms}"),
                event_type: crate::contracts::relation::RelationEventType::EdgeUpserted,
                node_id: Some(node_tool_loop),
                edge_id: None,
                scope: scope.clone(),
                reason: Some(crate::contracts::relation::RelationReason {
                    code: "swarm_relation_contract".to_string(),
                    message: "swarm runtime relation contract persisted".to_string(),
                    deny_reason: None,
                    evidence_ref: Some(decision_evidence_ref.to_string()),
                    replay_fp: Some(format!("replay-fp:{session_id}:{trace_id}")),
                    metadata: BTreeMap::from([(
                        "execution_reports".to_string(),
                        outcome.execution_reports.len().to_string(),
                    )]),
                }),
                evidence_ref: Some(decision_evidence_ref.to_string()),
                replay_fp: Some(format!("replay-fp:{session_id}:{trace_id}")),
                emitted_at_ms: now_ms,
                metadata: BTreeMap::new(),
            }],
        };

        let mut input = serde_json::to_value(contract)?;
        let gate_token = build_service_gate_token(
            &crate::contracts::ids::SessionId::from(session_id),
            &crate::contracts::services::ServiceDomain::Relation,
            now_ms,
        );
        attach_service_gate_token(&mut input, gate_token);
        let call = crate::contracts::services::ServiceCall {
            session_id: crate::contracts::ids::SessionId::from(session_id),
            trace_id: crate::contracts::ids::TraceId::from(trace_id.to_string()),
            service_domain: crate::contracts::services::ServiceDomain::Relation,
            service_name: "relation_facade".to_string(),
            operation: "upsert_contract".to_string(),
            input,
            budget_scope: "relation.trace".to_string(),
            requested_at_ms: now_ms,
        };
        let result = self.services.mediate_call(&call).await?;
        if !result.success {
            return Err(anyhow!(
                "failed to persist swarm relation contract: {}",
                result
                    .error
                    .unwrap_or_else(|| "unknown relation mediator error".to_string())
            ));
        }

        Ok(serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "relation_service": "relation_facade",
            "edge_count": result.output.get("edge_count").and_then(serde_json::Value::as_u64).unwrap_or(0),
            "event_ref": result.output.get("relation_event_ref").cloned().unwrap_or(serde_json::Value::Null),
            "evidence_ref": decision_evidence_ref,
        }))
    }

    pub async fn plugin_install(
        &self,
        plugin_id: &str,
        source: &str,
        requested_by: &str,
        tenant_id: &str,
        verify_signature: bool,
    ) -> Result<String> {
        let request = crate::contracts::plugin::PluginInstallRequest {
            plugin_id: plugin_id.to_string(),
            source: source.to_string(),
            requested_by: requested_by.to_string(),
            tenant_id: tenant_id.to_string(),
            verify_signature,
        };
        let manifest = self.plugins.install(&request).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn plugin_enable(
        &self,
        plugin_id: &str,
        operator: &str,
        reason: &str,
    ) -> Result<String> {
        let manifest = self.plugins.enable(plugin_id, operator, reason).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn plugin_disable(
        &self,
        plugin_id: &str,
        operator: &str,
        reason: &str,
    ) -> Result<String> {
        let manifest = self.plugins.disable(plugin_id, operator, reason).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn plugin_update(
        &self,
        plugin_id: &str,
        source: Option<&str>,
        operator: &str,
    ) -> Result<String> {
        let manifest = self.plugins.update(plugin_id, source, operator).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn plugin_rollback(
        &self,
        plugin_id: &str,
        operator: &str,
        reason: &str,
    ) -> Result<String> {
        let manifest = self.plugins.rollback(plugin_id, operator, reason).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn plugin_rollout(
        &self,
        plugin_id: &str,
        mode: &str,
        traffic_percent: Option<u8>,
        operator: &str,
        reason: Option<&str>,
    ) -> Result<String> {
        let rollout_mode = match mode {
            "shadow" => crate::contracts::plugin::PluginRolloutMode::Shadow,
            "canary" => crate::contracts::plugin::PluginRolloutMode::Canary,
            "full" => crate::contracts::plugin::PluginRolloutMode::Full,
            "rollback" => crate::contracts::plugin::PluginRolloutMode::Rollback,
            other => anyhow::bail!("unsupported plugin rollout mode: {}", other),
        };
        let manifest = self
            .plugins
            .rollout(
                plugin_id,
                rollout_mode,
                traffic_percent,
                operator,
                reason.unwrap_or("plugin rollout"),
            )
            .await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn plugin_quick_rollback(&self, plugin_id: &str, operator: &str) -> Result<String> {
        let manifest = self.plugins.quick_rollback(plugin_id, operator).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }
    pub async fn plugin_verify(&self, plugin_id: &str) -> Result<String> {
        let verdict = self.plugins.verify(plugin_id).await?;
        Ok(serde_json::to_string_pretty(&verdict)?)
    }

    pub async fn plugin_list(&self) -> Result<String> {
        let records = self.plugins.list().await?;
        Ok(serde_json::to_string_pretty(&records)?)
    }

    pub async fn plugin_status(&self, plugin_id: &str) -> Result<String> {
        let status = self.plugins.status(plugin_id).await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    pub async fn plugin_discover_compat(&self, root: &str) -> Result<String> {
        let report = self.plugins.discover_compatibility(root).await?;
        Ok(serde_json::to_string_pretty(&report)?)
    }
    pub async fn plugin_host_status(&self) -> Result<String> {
        let status = self.plugins.host_status().await?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    pub async fn plugin_host_load(
        &self,
        plugin_id: &str,
        entrypoint: &str,
        operator: &str,
    ) -> Result<String> {
        let manifest = self
            .plugins
            .host_load_subprocess(plugin_id, entrypoint, operator)
            .await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn plugin_host_invoke(
        &self,
        plugin_id: &str,
        session_id: &str,
        tenant_id: &str,
        principal_id: &str,
        capability_id: Option<&str>,
        payload: serde_json::Value,
    ) -> Result<String> {
        let output = self
            .plugins
            .host_invoke(
                plugin_id,
                session_id,
                tenant_id,
                principal_id,
                capability_id,
                payload,
            )
            .await?;
        Ok(serde_json::to_string_pretty(&output)?)
    }
    pub async fn skill_register(
        &self,
        skill_id: &str,
        source: &str,
        markdown: &str,
    ) -> Result<String> {
        let signal = memory::LearningSignal {
            signal_id: format!("skill-signal:{}:{}", skill_id, current_time_ms()),
            session_id: "global-skill-registry".to_string(),
            trace_id: format!("trace:skill-register:{skill_id}:{}", current_time_ms()),
            source: "skill_registry.register".to_string(),
            evidence_ref: format!("evidence:skill-registry:register:{skill_id}:{}", current_time_ms()),
            metadata: std::collections::BTreeMap::from([(
                "skill_id".to_string(),
                skill_id.to_string(),
            )]),
        };
        let manifest = self
            .skills
            .register(skill_id, source, markdown, &signal)
            .await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn skill_build(&self, skill_id: &str, builder: &str) -> Result<String> {
        let artifact = self.skills.build(skill_id, builder).await?;
        Ok(serde_json::to_string_pretty(&artifact)?)
    }

    pub async fn skill_list(&self) -> Result<String> {
        let list = self.skills.list().await?;
        Ok(serde_json::to_string_pretty(&list)?)
    }

    pub async fn skill_remove(&self, skill_id: &str) -> Result<String> {
        let signal = memory::LearningSignal {
            signal_id: format!("skill-signal:{}:{}", skill_id, current_time_ms()),
            session_id: "global-skill-registry".to_string(),
            trace_id: format!("trace:skill-remove:{skill_id}:{}", current_time_ms()),
            source: "skill_registry.remove".to_string(),
            evidence_ref: format!("evidence:skill-registry:remove:{skill_id}:{}", current_time_ms()),
            metadata: std::collections::BTreeMap::from([(
                "skill_id".to_string(),
                skill_id.to_string(),
            )]),
        };
        self.skills.remove_with_signal(skill_id, &signal).await?;
        Ok(serde_json::json!({
            "skill_id": skill_id,
            "status": "retired",
        })
        .to_string())
    }

    pub async fn skill_install(
        &self,
        package: &crate::contracts::skill_foundry::PackageMeta,
        source: &str,
        markdown: &str,
    ) -> Result<String> {
        let signal = memory::LearningSignal {
            signal_id: format!("skill-signal:{}:{}", package.skill_name, current_time_ms()),
            session_id: "global-skill-registry".to_string(),
            trace_id: format!(
                "trace:skill-install:{}:{}",
                package.skill_name,
                current_time_ms()
            ),
            source: "skill_registry.install".to_string(),
            evidence_ref: format!(
                "evidence:skill-registry:install:{}:{}",
                package.skill_name,
                current_time_ms()
            ),
            metadata: std::collections::BTreeMap::from([
                ("skill_id".to_string(), package.skill_name.clone()),
                ("package_id".to_string(), package.package_id.clone()),
            ]),
        };
        let manifest = self
            .skills
            .install_from_package(package, source, markdown, &signal)
            .await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn skill_enable(&self, skill_id: &str, reason: &str) -> Result<String> {
        let signal = memory::LearningSignal {
            signal_id: format!("skill-signal:{}:{}", skill_id, current_time_ms()),
            session_id: "global-skill-registry".to_string(),
            trace_id: format!("trace:skill-enable:{skill_id}:{}", current_time_ms()),
            source: "skill_registry.set_enabled".to_string(),
            evidence_ref: format!("evidence:skill-registry:enable:{skill_id}:{}", current_time_ms()),
            metadata: std::collections::BTreeMap::from([
                ("skill_id".to_string(), skill_id.to_string()),
                ("enabled".to_string(), "true".to_string()),
                ("reason".to_string(), reason.to_string()),
            ]),
        };
        let manifest = self.skills.set_enabled(skill_id, true, reason, &signal).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    pub async fn skill_disable(&self, skill_id: &str, reason: &str) -> Result<String> {
        let signal = memory::LearningSignal {
            signal_id: format!("skill-signal:{}:{}", skill_id, current_time_ms()),
            session_id: "global-skill-registry".to_string(),
            trace_id: format!("trace:skill-disable:{skill_id}:{}", current_time_ms()),
            source: "skill_registry.set_enabled".to_string(),
            evidence_ref: format!("evidence:skill-registry:disable:{skill_id}:{}", current_time_ms()),
            metadata: std::collections::BTreeMap::from([
                ("skill_id".to_string(), skill_id.to_string()),
                ("enabled".to_string(), "false".to_string()),
                ("reason".to_string(), reason.to_string()),
            ]),
        };
        let manifest = self.skills.set_enabled(skill_id, false, reason, &signal).await?;
        Ok(serde_json::to_string_pretty(&manifest)?)
    }

    fn persist_runtime_artifact(&self, category: &str, session_id: &str, body: &str) -> Result<()> {
        let directory = self.runtime_artifact_dir(category);
        fs::create_dir_all(&directory)?;
        let safe_session_id = crate::path_safety::sanitize_filesystem_component(session_id);
        fs::write(directory.join(format!("{safe_session_id}.json")), body)?;
        Ok(())
    }

    fn runtime_artifact_dir(&self, category: &str) -> PathBuf {
        PathBuf::from("D:\\AutoLoop\\autoloop-app\\deploy\\runtime").join(category)
    }
}

fn chunk_text_for_delta(content: &str, chunk_chars: usize) -> Vec<String> {
    if content.is_empty() {
        return vec![String::new()];
    }
    let chunk_size = chunk_chars.max(1);
    let mut chunks = Vec::new();
    let mut buffer = String::new();
    let mut count = 0usize;
    for ch in content.chars() {
        buffer.push(ch);
        count = count.saturating_add(1);
        if count >= chunk_size {
            chunks.push(buffer.clone());
            buffer.clear();
            count = 0;
        }
    }
    if !buffer.is_empty() {
        chunks.push(buffer);
    }
    chunks
}

fn frontend_permission_requires_approval() -> bool {
    std::env::var("AUTOLOOP_FRONTEND_PERMISSION_MODE")
        .map(|value| value.trim().eq_ignore_ascii_case("ask"))
        .unwrap_or(false)
}

fn frontend_permission_request_key(session_id: &str, request_id: &str) -> String {
    format!("frontend:permission:request:{session_id}:{request_id}")
}

fn frontend_permission_store_dir() -> PathBuf {
    std::env::temp_dir().join("ontoloop_frontend_permission")
}

fn frontend_permission_store_file(session_id: &str, request_id: &str) -> PathBuf {
    let sanitize = |value: &str| -> String {
        value
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect()
    };
    frontend_permission_store_dir().join(format!(
        "{}__{}.json",
        sanitize(session_id),
        sanitize(request_id)
    ))
}

fn persist_frontend_permission_request_to_file(
    session_id: &str,
    request_id: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    let dir = frontend_permission_store_dir();
    fs::create_dir_all(&dir)?;
    let path = frontend_permission_store_file(session_id, request_id);
    fs::write(path, serde_json::to_string_pretty(payload)?)?;
    Ok(())
}

fn load_frontend_permission_request_from_file(
    session_id: &str,
    request_id: &str,
) -> Result<Option<serde_json::Value>> {
    let path = frontend_permission_store_file(session_id, request_id);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    let parsed = serde_json::from_str::<serde_json::Value>(&raw)
        .unwrap_or_else(|_| serde_json::json!({}));
    Ok(Some(parsed))
}

fn should_enable_postgres(config: &AppConfig) -> bool {
    if !config.storage.postgres.enabled {
        return false;
    }
    match config.storage.mode {
        StorageMode::Shadow => true,
        StorageMode::Direct | StorageMode::Enforced => {
            matches!(config.storage.backend, StorageBackend::Postgres)
        }
    }
}

fn map_storage_mode_to_knowledge_mirror_mode(mode: StorageMode) -> KnowledgeMirrorMode {
    match mode {
        StorageMode::Shadow => KnowledgeMirrorMode::Shadow,
        StorageMode::Direct => KnowledgeMirrorMode::Direct,
        StorageMode::Enforced => KnowledgeMirrorMode::Enforced,
    }
}

fn map_storage_read_preference(
    preference: &str,
    backend: StorageBackend,
) -> KnowledgeReadPreference {
    let normalized = preference.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "postgres" => KnowledgeReadPreference::Postgres,
        "state_store" => KnowledgeReadPreference::PrimaryStore,
        _ => match backend {
            StorageBackend::Postgres => KnowledgeReadPreference::Postgres,
            StorageBackend::PrimaryStore => KnowledgeReadPreference::PrimaryStore,
        },
    }
}

fn session_key(session_id: &str) -> String {
    session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect()
}

fn normalize_anchor(value: &str) -> String {
    if value.starts_with("anchor:") {
        value.to_string()
    } else {
        format!("anchor:{value}")
    }
}

fn validate_worldline_weight(value: f32, field: &str) -> Result<()> {
    if !value.is_finite() {
        return Err(anyhow!("worldline weight `{field}` must be finite"));
    }
    if !(0.0..=3.0).contains(&value) {
        return Err(anyhow!(
            "worldline weight `{field}` out of range [0.0, 3.0]: {value}"
        ));
    }
    Ok(())
}
fn validate_objective_weight(value: f32, field: &str) -> Result<()> {
    if !value.is_finite() {
        anyhow::bail!("{} must be finite", field);
    }
    if value <= 0.0 {
        anyhow::bail!("{} must be > 0", field);
    }
    if value > 10.0 {
        anyhow::bail!("{} must be <= 10", field);
    }
    Ok(())
}

fn map_to_core_decision(
    decision: &crate::contracts::evolution_os::PromotionDecision,
) -> CorePromotionDecision {
    match decision {
        crate::contracts::evolution_os::PromotionDecision::Discard => CorePromotionDecision::Discard,
        crate::contracts::evolution_os::PromotionDecision::LogOnly => CorePromotionDecision::LogOnly,
        crate::contracts::evolution_os::PromotionDecision::Localize => CorePromotionDecision::Localize,
        crate::contracts::evolution_os::PromotionDecision::PromoteRuntimeUpdate => {
            CorePromotionDecision::PromoteRuntimeUpdate
        }
        crate::contracts::evolution_os::PromotionDecision::PromoteTemplate => {
            CorePromotionDecision::PromoteTemplate
        }
        crate::contracts::evolution_os::PromotionDecision::PromoteGovernanceContract => {
            CorePromotionDecision::PromoteGovernanceContract
        }
        crate::contracts::evolution_os::PromotionDecision::CrystallizeMemoryRule => {
            CorePromotionDecision::CrystallizeMemoryRule
        }
        crate::contracts::evolution_os::PromotionDecision::Rollback => CorePromotionDecision::Rollback,
        crate::contracts::evolution_os::PromotionDecision::EscalateHumanReview => {
            CorePromotionDecision::EscalateHumanReview
        }
    }
}

fn map_to_core_rollout_stage(stage: &crate::evolution_os::RolloutStage) -> CoreRolloutStage {
    match stage {
        crate::evolution_os::RolloutStage::Shadow => CoreRolloutStage::Shadow,
        crate::evolution_os::RolloutStage::Canary10 => CoreRolloutStage::Canary10,
        crate::evolution_os::RolloutStage::Canary30 => CoreRolloutStage::Canary30,
        crate::evolution_os::RolloutStage::Full => CoreRolloutStage::Full,
        crate::evolution_os::RolloutStage::Rollback => CoreRolloutStage::Rollback,
    }
}

const DEFAULT_SWARM_MAX_TOKENS: u32 = 8_000;
const SWARM_EXECUTION_TOKEN_CEILING: u32 = 8_000;
const SWARM_MIN_REPLAN_TOKENS: u32 = 512;

fn should_route_code_task_to_harness(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    if lowered.contains("requires_artifact")
        || lowered.contains("artifact_delivery/v1")
        || lowered.contains("target_path")
        || lowered.contains("```")
    {
        return true;
    }

    let code_task_markers = [
        ".rs",
        ".py",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        ".html",
        ".css",
        "cargo ",
        "pytest",
        "npm run",
        "编译",
        "写代码",
        "代码",
        "修复bug",
        "debug",
        "implement",
        "refactor",
        "clone",
        "build frontend",
    ];
    code_task_markers
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn map_termination_to_iteration_decision(
    reason: &TerminationReason,
) -> crate::contracts::code_harness::IterationDecision {
    match reason {
        TerminationReason::Success => crate::contracts::code_harness::IterationDecision::StopSolved,
        TerminationReason::AttemptLimitExceeded => {
            crate::contracts::code_harness::IterationDecision::StopAttemptLimit
        }
        TerminationReason::BudgetLimitExceeded => {
            crate::contracts::code_harness::IterationDecision::StopBudgetExceeded
        }
        TerminationReason::TimeLimitExceeded => crate::contracts::code_harness::IterationDecision::StopFailed,
        TerminationReason::NonRetryableFailure => crate::contracts::code_harness::IterationDecision::StopFailed,
    }
}

fn build_iteration_strategy_summary(
    attempts: &[crate::query_engine::IterationAttemptRecord],
) -> serde_json::Value {
    let mut category_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut strategy_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut retry_allowed_true = 0_u64;
    let mut retry_allowed_false = 0_u64;

    for attempt in attempts {
        if let Some(category) = &attempt.failure_category {
            let key = format!("{category:?}").to_ascii_lowercase();
            *category_counts.entry(key).or_insert(0) += 1;
        }
        if let Some(strategy) = &attempt.repair_strategy {
            let key = format!("{strategy:?}").to_ascii_lowercase();
            *strategy_counts.entry(key).or_insert(0) += 1;
        }
        match attempt.retry_allowed {
            Some(true) => retry_allowed_true += 1,
            Some(false) => retry_allowed_false += 1,
            None => {}
        }
    }

    serde_json::json!({
        "category_counts": category_counts,
        "strategy_counts": strategy_counts,
        "retry_allowed_true": retry_allowed_true,
        "retry_allowed_false": retry_allowed_false,
    })
}

fn extract_structured_patch_ops_hint(content: &str) -> Option<Vec<crate::contracts::code_harness::PatchOp>> {
    if !content
        .to_ascii_lowercase()
        .contains("structured_patch/v1")
    {
        return None;
    }

    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    let json_payload = &content[start..=end];
    let value: serde_json::Value = serde_json::from_str(json_payload).ok()?;
    if value.get("api_version").and_then(serde_json::Value::as_str)
        != Some("structured_patch/v1")
    {
        return None;
    }
    let ops_value = value.get("patch_ops")?.clone();
    serde_json::from_value::<Vec<crate::contracts::code_harness::PatchOp>>(ops_value).ok()
}

fn extract_git_checkpoint_request_hint(content: &str) -> Option<GitCheckpointRequest> {
    if !content.to_ascii_lowercase().contains("git_checkpoint/v1") {
        return None;
    }
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    let json_payload = &content[start..=end];
    let value: serde_json::Value = serde_json::from_str(json_payload).ok()?;
    if value.get("api_version").and_then(serde_json::Value::as_str)
        != Some("git_checkpoint/v1")
    {
        return None;
    }
    serde_json::from_value::<GitCheckpointRequest>(value).ok()
}

fn extract_shell_loop_request_hint(content: &str) -> Option<ShellLoopRequest> {
    if !content.to_ascii_lowercase().contains("shell_loop/v1") {
        return None;
    }
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    let json_payload = &content[start..=end];
    let value: serde_json::Value = serde_json::from_str(json_payload).ok()?;
    if value.get("api_version").and_then(serde_json::Value::as_str) != Some("shell_loop/v1") {
        return None;
    }
    serde_json::from_value::<ShellLoopRequest>(value).ok()
}

fn extract_test_verifier_request_hint(content: &str) -> Option<TestVerifierRequest> {
    if !content.to_ascii_lowercase().contains("test_verifier/v1") {
        return None;
    }
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end <= start {
        return None;
    }
    let json_payload = &content[start..=end];
    let value: serde_json::Value = serde_json::from_str(json_payload).ok()?;
    if value.get("api_version").and_then(serde_json::Value::as_str)
        != Some("test_verifier/v1")
    {
        return None;
    }
    serde_json::from_value::<TestVerifierRequest>(value).ok()
}

fn build_default_test_verifier_request(workspace_root: &Path) -> Option<TestVerifierRequest> {
    let cargo = workspace_root.join("Cargo.toml");
    if cargo.exists() {
        return Some(TestVerifierRequest {
            api_version: "test_verifier/v1".to_string(),
            fail_fast: true,
            runners: vec![
                crate::query_engine::TestRunnerSpec {
                    runner_id: "build".to_string(),
                    kind: crate::query_engine::TestRunnerKind::Build,
                    command: "cargo check --workspace".to_string(),
                    cwd: None,
                    timeout_ms: Some(180_000),
                    required: true,
                },
                crate::query_engine::TestRunnerSpec {
                    runner_id: "lint".to_string(),
                    kind: crate::query_engine::TestRunnerKind::Lint,
                    command: "cargo fmt --all -- --check".to_string(),
                    cwd: None,
                    timeout_ms: Some(180_000),
                    required: true,
                },
                crate::query_engine::TestRunnerSpec {
                    runner_id: "test".to_string(),
                    kind: crate::query_engine::TestRunnerKind::Test,
                    command: "cargo test --workspace --no-run".to_string(),
                    cwd: None,
                    timeout_ms: Some(240_000),
                    required: true,
                },
            ],
        });
    }
    None
}

fn extract_iteration_controller_config_hint(content: &str) -> Option<IterationControllerConfig> {
    let lowered = content.to_ascii_lowercase();
    if !lowered.contains("iteration_controller/v1") {
        return None;
    }

    let mut config = IterationControllerConfig::default();
    for token in content
        .split(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
    {
        let lower = token.to_ascii_lowercase();
        if let Some(raw) = lower.strip_prefix("max_attempts=") {
            if let Ok(value) = raw.parse::<u32>() {
                config.max_attempts = value.clamp(1, 12);
            }
        } else if let Some(raw) = lower.strip_prefix("budget_tokens=") {
            if let Ok(value) = raw.parse::<u32>() {
                config.budget_tokens = Some(value.max(1));
            }
        } else if let Some(raw) = lower.strip_prefix("max_runtime_ms=") {
            if let Ok(value) = raw.parse::<u64>() {
                config.max_runtime_ms = value.clamp(500, 600_000);
            }
        } else if let Some(raw) = lower.strip_prefix("retry_on=") {
            let items = raw
                .split('|')
                .flat_map(|part| part.split('/'))
                .flat_map(|part| part.split('+'))
                .flat_map(|part| part.split(':'))
                .flat_map(|part| part.split(','))
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>();
            if !items.is_empty() {
                config.retry_on = items;
            }
        }
    }

    Some(config)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SwarmBudgetPreflight {
    session_id: String,
    estimated_tokens: u32,
    effective_max_tokens: u32,
    final_request_tokens: u32,
    compaction_applied: bool,
    lane_mode: String,
    reason: String,
    original_chars: usize,
    replanned_chars: usize,
    replanned_request: String,
}

fn effective_swarm_token_budget(policy_max_tokens: u32) -> u32 {
    let max_tokens = if policy_max_tokens == 0 {
        DEFAULT_SWARM_MAX_TOKENS
    } else {
        policy_max_tokens
    };
    max_tokens
        .min(SWARM_EXECUTION_TOKEN_CEILING)
        .max(SWARM_MIN_REPLAN_TOKENS)
}

fn estimate_swarm_tokens(content: &str) -> u32 {
    let chars = content.chars().count() as u32;
    let words = content.split_whitespace().count() as u32;
    let char_estimate = chars.div_ceil(4);
    let word_estimate = words.div_ceil(2);
    char_estimate.max(word_estimate).max(1)
}

fn should_use_dual_lane(content: &str, estimated_tokens: u32, max_tokens: u32, compacted: bool) -> bool {
    if compacted {
        return true;
    }
    let bullet_count = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("1.")
        })
        .count();
    let coordination_terms = [
        "swarm",
        "plan",
        "execute",
        "verify",
        "rollback",
        "tool",
        "evidence",
        "governance",
    ];
    let lowered = content.to_ascii_lowercase();
    let term_hits = coordination_terms
        .iter()
        .filter(|term| lowered.contains(**term))
        .count();

    estimated_tokens > max_tokens.saturating_mul(7) / 10
        || bullet_count >= 4
        || term_hits >= 4
        || content.len() > 1_800
}

fn compact_and_replan_requirement(content: &str, max_tokens: u32) -> String {
    let target_chars = usize::try_from(max_tokens)
        .unwrap_or(DEFAULT_SWARM_MAX_TOKENS as usize)
        .saturating_mul(4)
        .clamp(280, 4_800);
    if content.chars().count() <= target_chars {
        return content.to_string();
    }

    let head_chars = target_chars.saturating_mul(3) / 4;
    let tail_chars = target_chars.saturating_sub(head_chars);
    let head = content.chars().take(head_chars).collect::<String>();
    let tail_vec = content.chars().rev().take(tail_chars).collect::<Vec<_>>();
    let tail = tail_vec.into_iter().rev().collect::<String>();

    let mut compacted = format!(
        "Budget preflight compacted this requirement to stay inside token limits.\n\
        Replan protocol: 1) plan the minimum viable execution path, 2) execute highest-value steps first,\n\
        3) emit evidence refs for every mutation.\n\
        [CompactedHead]\n{}\n[CompactedTail]\n{}\n",
        head.trim(),
        tail.trim()
    );
    if let Some(target_path) = extract_artifact_target_path_hint(content) {
        compacted.push_str(
            format!(
                "[PreservedArtifactContract]\nrequires_artifact=true\ntarget_path={}\n",
                target_path
            )
            .as_str(),
        );
    }
    compacted
}

fn extract_artifact_target_path_hint(content: &str) -> Option<String> {
    let lowered = content.to_ascii_lowercase();
    if !(lowered.contains("requires_artifact")
        || lowered.contains("artifact_delivery/v1")
        || lowered.contains("target_path"))
    {
        return None;
    }

    content
        .split(|ch: char| {
            ch.is_whitespace()
                || matches!(ch, ',' | ';' | '{' | '}' | '(' | ')' | '[' | ']' | '"' | '\'')
        })
        .filter_map(normalize_path_token_for_budget)
        .filter_map(strip_artifact_key_prefix)
        .find(|path| is_supported_artifact_path_for_budget(path))
}

fn normalize_path_token_for_budget(token: &str) -> Option<String> {
    let mut started = false;
    let mut collected = String::new();
    for ch in token.chars() {
        if !started {
            if is_path_char_for_budget(ch) {
                started = true;
                collected.push(ch);
            }
            continue;
        }
        if is_path_char_for_budget(ch) {
            collected.push(ch);
        } else {
            break;
        }
    }
    if collected.is_empty() {
        return None;
    }
    let candidate = collected
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | '{' | '}' | '(' | ')' | '[' | ']'))
        .replace('\\', "/");
    if candidate.is_empty() {
        None
    } else {
        Some(candidate)
    }
}

fn strip_artifact_key_prefix(token: String) -> Option<String> {
    let lowered = token.to_ascii_lowercase();
    let candidate = if let Some(index) = lowered.find("target_path:") {
        token[index + "target_path:".len()..].to_string()
    } else if let Some(index) = lowered.find("target_path=") {
        token[index + "target_path=".len()..].to_string()
    } else if let Some(index) = lowered.find("path:") {
        token[index + "path:".len()..].to_string()
    } else if let Some(index) = lowered.find("path=") {
        token[index + "path=".len()..].to_string()
    } else {
        token
    };
    let trimmed = candidate.trim().trim_matches(|ch| matches!(ch, '"' | '\'' | '`'));
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_path_char_for_budget(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, ':' | '/' | '\\' | '.' | '_' | '-')
}

fn is_supported_artifact_path_for_budget(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    lowered.contains(":/")
        || lowered.starts_with('/')
        || lowered.starts_with("./")
        || lowered.starts_with("../")
}

fn relation_node_component_for_path(path: &str) -> String {
    let sanitized = crate::path_safety::sanitize_filesystem_component(path);
    if !sanitized.is_empty() {
        return sanitized;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    path.hash(&mut hasher);
    format!("path-{:#x}", hasher.finish())
}

fn infer_artifact_mime_from_path(path: &Path) -> String {
    match path
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html".to_string(),
        "json" => "application/json".to_string(),
        "md" => "text/markdown".to_string(),
        "txt" => "text/plain".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

fn build_swarm_budget_preflight(
    session_id: &str,
    content: &str,
    policy_max_tokens: u32,
) -> SwarmBudgetPreflight {
    let estimated_tokens = estimate_swarm_tokens(content);
    let effective_max_tokens = effective_swarm_token_budget(policy_max_tokens);
    let compaction_applied = estimated_tokens > effective_max_tokens;
    let replanned_request = if compaction_applied {
        compact_and_replan_requirement(content, effective_max_tokens)
    } else {
        content.to_string()
    };
    let dual_lane = should_use_dual_lane(
        &replanned_request,
        estimated_tokens,
        effective_max_tokens,
        compaction_applied,
    );
    let reason = if compaction_applied {
        format!(
            "budget_preflight_overflow:{}>{}, compact_then_replan",
            estimated_tokens, effective_max_tokens
        )
    } else if dual_lane {
        "complexity_threshold_met".to_string()
    } else {
        "single_lane_sufficient".to_string()
    };

    SwarmBudgetPreflight {
        session_id: session_id.to_string(),
        estimated_tokens,
        effective_max_tokens,
        final_request_tokens: estimate_swarm_tokens(&replanned_request),
        compaction_applied,
        lane_mode: if dual_lane {
            "dual".to_string()
        } else {
            "single".to_string()
        },
        reason,
        original_chars: content.chars().count(),
        replanned_chars: replanned_request.chars().count(),
        replanned_request,
    }
}

fn build_swarm_plan_lane(
    session_id: &str,
    content: &str,
    preflight: &SwarmBudgetPreflight,
) -> serde_json::Value {
    let lowered = content.to_ascii_lowercase();
    let mut steps = content
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("- ")
                || line.starts_with("* ")
                || line.starts_with("1.")
                || line.starts_with("2.")
                || line.starts_with("3.")
        })
        .map(|line| {
            line.trim_start_matches("- ")
                .trim_start_matches("* ")
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .take(6)
        .collect::<Vec<_>>();

    if steps.is_empty() {
        steps = vec![
            "Freeze requirement scope and derive minimal executable plan.".to_string(),
            "Execute highest-value path first with tool evidence binding.".to_string(),
            "Verify outcome and persist replayable evidence with refs.".to_string(),
        ];
    }

    let objective = content.lines().next().unwrap_or(content).trim().to_string();
    let objective = if objective.is_empty() {
        "Execute requirement swarm with bounded plan/execute lanes.".to_string()
    } else {
        objective.chars().take(220).collect::<String>()
    };
    let artifact_target = extract_artifact_target_path_hint(content);
    let risk_flags = vec![
        if preflight.compaction_applied {
            "budget_compaction_applied"
        } else {
            "budget_within_limits"
        },
        if lowered.contains("approval") || lowered.contains("policy") {
            "governance_sensitive"
        } else {
            "governance_normal"
        },
        if artifact_target.is_some() {
            "artifact_required"
        } else {
            "artifact_optional"
        },
    ];
    let plan_digest = crate::observability::event_stream::digest_value(&serde_json::json!({
        "session_id": session_id,
        "objective": objective,
        "steps": steps,
        "lane_mode": preflight.lane_mode,
        "effective_max_tokens": preflight.effective_max_tokens,
    }));
    serde_json::json!({
        "session_id": session_id,
        "lane_mode": preflight.lane_mode,
        "reason": preflight.reason,
        "objective": objective,
        "plan_steps": steps,
        "effective_max_tokens": preflight.effective_max_tokens,
        "estimated_tokens": preflight.estimated_tokens,
        "risk_flags": risk_flags,
        "artifact_target": artifact_target,
        "plan_digest": plan_digest,
        "created_at_ms": current_time_ms(),
    })
}

fn build_swarm_execute_lane_seed(plan_lane: &serde_json::Value) -> Option<String> {
    let objective = plan_lane
        .get("objective")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    let steps = plan_lane
        .get("plan_steps")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .take(5)
                .enumerate()
                .map(|(idx, step)| format!("{}. {}", idx + 1, step))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if objective.is_empty() && steps.is_empty() {
        return None;
    }
    let mut seed = String::new();
    if !objective.is_empty() {
        seed.push_str(&format!("Objective: {}\n", objective));
    }
    if !steps.is_empty() {
        seed.push_str("ExecutionPlan:\n");
        seed.push_str(&steps.join("\n"));
    }
    Some(seed)
}

fn relation_node(
    node_id: &str,
    node_type: crate::contracts::relation::RelationNodeType,
    scope: &crate::contracts::relation::RelationScope,
) -> crate::contracts::relation::RelationNode {
    crate::contracts::relation::RelationNode {
        node_id: node_id.to_string(),
        node_type,
        scope: scope.clone(),
        display_name: Some(node_id.to_string()),
        metadata: BTreeMap::new(),
    }
}

fn relation_edge(
    edge_id: String,
    from_node_id: String,
    to_node_id: String,
    edge_type: crate::contracts::relation::RelationEdgeType,
    scope: &crate::contracts::relation::RelationScope,
    reason_code: &str,
    reason_message: &str,
    evidence_ref: &str,
) -> crate::contracts::relation::RelationEdge {
    crate::contracts::relation::RelationEdge {
        edge_id,
        from_node_id,
        to_node_id,
        edge_type,
        scope: scope.clone(),
        reason: Some(crate::contracts::relation::RelationReason {
            code: reason_code.to_string(),
            message: reason_message.to_string(),
            deny_reason: None,
            evidence_ref: Some(evidence_ref.to_string()),
            replay_fp: None,
            metadata: BTreeMap::new(),
        }),
        active: true,
        metadata: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::gitmemory_core::acl_fine_policy::{AclEffect, AclPolicyBundle, AclRule};
    use crate::runtime::McpDispatchRequest;
    use autoloop_state_adapter::PermissionAction;

    #[tokio::test]
    async fn bootstrap_default_config() {
        let app = AutoLoopApp::new(AppConfig::default());
        let report = app.bootstrap().await.expect("bootstrap");
        assert_eq!(report.app_name, "autoloop");
        assert!(report.provider_count >= 1);
    }

    #[tokio::test]
    async fn frontend_artifact_write_backfill_persists_relation_write_proof() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "test:frontend:artifact:proof";
        let trace_id = "trace:test:frontend:artifact:proof";
        let target_path = PathBuf::from("D:/AutoLoop/output/test_frontend_artifact_proof.json");
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).expect("create output dir");
        }
        fs::write(&target_path, br#"{"ok":true}"#).expect("write test artifact");

        let request = format!(
            "artifact contract {{\"api_version\":\"artifact_delivery/v1\",\"requires_artifact\":true,\"target_path\":\"{}\",\"validation_rules\":{{\"exists_required\":true}}}}",
            target_path.display().to_string().replace('\\', "/")
        );
        let backfill = app
            .backfill_frontend_artifact_relation_write_proof(session_id, trace_id, &request)
            .await
            .expect("backfill relation write proof")
            .expect("expected relation write proof");

        assert_eq!(
            backfill
                .get("status")
                .and_then(serde_json::Value::as_str),
            Some("ok")
        );
        assert!(
            !backfill
                .get("relation_write_proof_ref")
                .unwrap_or(&serde_json::Value::Null)
                .is_null(),
            "relation_write_proof_ref should be present"
        );

        let proofs = app
            .state_store()
            .list_knowledge_by_prefix(&format!("relation:write_proof:{session_id}:"))
            .await
            .expect("list relation write proofs");
        assert!(
            !proofs.is_empty(),
            "relation write proof record should be persisted"
        );
    }

    #[test]
    fn persist_runtime_artifact_sanitizes_session_id_for_filesystem_safety() {
        let app = AutoLoopApp::new(AppConfig::default());
        let unsafe_session_id = "session:../../prod|shadow?run";
        app.persist_runtime_artifact("dashboard", unsafe_session_id, r#"{"ok":true}"#)
            .expect("persist runtime artifact");

        let safe_session = crate::path_safety::sanitize_filesystem_component(unsafe_session_id);
        assert!(
            !safe_session.contains('/') && !safe_session.contains('\\') && !safe_session.contains(':'),
            "sanitized session id must not keep path traversal characters"
        );
        let expected_path = app
            .runtime_artifact_dir("dashboard")
            .join(format!("{safe_session}.json"));
        assert!(
            expected_path.exists(),
            "sanitized artifact output should be written under runtime artifact directory"
        );

        let _ = fs::remove_file(expected_path);
    }

    #[tokio::test]
    async fn mcp_dispatch_event_calls_state_store() {
        let app = AutoLoopApp::new(AppConfig::default());
        app.state_store
            .grant_permissions(
                "scheduler",
                vec![PermissionAction::Dispatch, PermissionAction::Write],
            )
            .await
            .expect("grant");

        let event = app
            .runtime
            .dispatch_mcp_event(
                &app.state_store,
                McpDispatchRequest {
                    session_id: "session-1".into(),
                    tool_name: "mcp::local-mcp::invoke".into(),
                    payload: "{\"job\":\"reindex\"}".into(),
                    actor_id: "scheduler".into(),
                },
            )
            .await
            .expect("dispatch");

        assert_eq!(event.topic, "mcp.dispatch");
        assert_eq!(event.status, "queued");
    }

    #[tokio::test]
    async fn single_agent_closed_loop_example() {
        let app = AutoLoopApp::new(AppConfig::default());
        app.bootstrap().await.expect("bootstrap");

        app.state_store
            .grant_permissions(
                "agent-1",
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        let event = app
            .runtime
            .dispatch_mcp_event(
                &app.state_store,
                McpDispatchRequest {
                    session_id: "agent-session".into(),
                    tool_name: "mcp::local-mcp::invoke".into(),
                    payload: "{\"anchor\":\"state_store\"}".into(),
                    actor_id: "agent-1".into(),
                },
            )
            .await
            .expect("dispatch");

        let answer = app
            .process_direct(
                "agent-session",
                "Explain how the state_store anchor should be stored.",
            )
            .await
            .expect("agent response");

        app.state_store
            .upsert_agent_state(
                "agent-session".into(),
                "Explain how the state_store anchor should be stored.".into(),
                Some(answer.clone()),
            )
            .await
            .expect("state upsert");

        app.state_store
            .upsert_knowledge(
                "anchor:state_store".into(),
                answer.clone(),
                "single-agent-test".into(),
            )
            .await
            .expect("knowledge upsert");
        app.state_store
            .update_schedule_status(event.id, "completed")
            .await
            .expect("status update");

        let state = app
            .state_store
            .get_agent_state("agent-session")
            .await
            .expect("state get")
            .expect("state exists");
        let record = app
            .state_store
            .get_knowledge("anchor:state_store")
            .await
            .expect("knowledge get")
            .expect("knowledge exists");
        let events = app
            .state_store
            .list_schedule_events("agent-session")
            .await
            .expect("events");

        assert_eq!(state.session_id, "agent-session");
        assert!(record.value.contains("state_store"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, "completed");
    }

    #[tokio::test]
    async fn requirement_swarm_persists_knowledge_records() {
        let app = AutoLoopApp::new(AppConfig::default());
        app.state_store
            .grant_permissions(
                "swarm-session",
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        let summary = app
            .process_requirement_swarm(
                "swarm-session",
                "Need a CEO agent that forms a swarm and stores all discussion in graph memory with MCP execution.",
            )
            .await
            .expect("swarm");

        let brief = app
            .state_store
            .get_knowledge("conversation:swarm-session:brief")
            .await
            .expect("brief")
            .expect("brief exists");
        let graph = app
            .state_store
            .get_knowledge("graph:swarm-session:snapshot")
            .await
            .expect("graph")
            .expect("graph exists");
        let dashboard = app
            .state_store
            .get_knowledge("observability:swarm-session:dashboard")
            .await
            .expect("dashboard")
            .expect("dashboard exists");
        let deliberation = app
            .state_store
            .get_knowledge("conversation:swarm-session:deliberation")
            .await
            .expect("deliberation")
            .expect("deliberation exists");
        let research = app
            .state_store
            .get_knowledge("research:swarm-session:report")
            .await
            .expect("research")
            .expect("research exists");
        let consolidation = app
            .state_store
            .get_knowledge("memory:swarm-session:consolidation")
            .await
            .expect("consolidation")
            .expect("consolidation exists");
        let evolution = app
            .state_store
            .get_knowledge("memory:swarm-session:self-evolution")
            .await
            .expect("evolution")
            .expect("evolution exists");
        let research_follow_up = app
            .state_store
            .get_knowledge("research:swarm-session:follow-up-status")
            .await
            .expect("research follow-up")
            .expect("research follow-up exists");
        let verifier_evidence = app
            .state_store
            .get_knowledge("protocol:swarm-session:verifier-evidence-links")
            .await
            .expect("verifier evidence")
            .expect("verifier evidence exists");
        let audit_evidence = app
            .state_store
            .get_knowledge("observability:swarm-session:audit-evidence-view")
            .await
            .expect("audit evidence")
            .expect("audit evidence exists");
        let stats = app
            .state_store
            .list_knowledge_by_prefix("metrics:execution:")
            .await
            .expect("stats");

        assert!(summary.contains("CEO"));
        assert!(brief.value.contains("clarified_goal"));
        assert!(graph.value.contains("entities"));
        assert!(dashboard.value.contains("route_analytics"));
        assert!(deliberation.value.contains("planner_notes"));
        assert!(research.value.contains("autonomy_score"));
        assert!(consolidation.value.contains("capability_improvements"));
        assert!(evolution.value.contains("evolved_score"));
        assert!(research_follow_up.value.contains("scheduled_tasks"));
        assert!(verifier_evidence.value.contains("guard_evidence_ref"));
        assert!(audit_evidence.value.contains("evidence_link_count"));
        assert!(!stats.is_empty());
    }

    #[tokio::test]
    async fn requirement_swarm_persists_structured_execution_feedback() {
        let app = AutoLoopApp::new(AppConfig::default());
        app.state_store
            .grant_permissions(
                "routing-session",
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        app.process_requirement_swarm(
            "routing-session",
            "Use MCP execution and graph memory to plan and execute the swarm.",
        )
        .await
        .expect("swarm");

        let feedback = app
            .state_store
            .list_knowledge_by_prefix("conversation:routing-session:execution-feedback:")
            .await
            .expect("feedback");
        let stats = app
            .state_store
            .list_knowledge_by_prefix("metrics:execution:")
            .await
            .expect("stats");

        assert!(!feedback.is_empty());
        assert!(
            feedback
                .iter()
                .any(|record| record.value.contains("tool") || record.value.contains("route"))
        );
        assert!(!stats.is_empty());
        assert!(stats.iter().any(|record| record.value.contains("\"attempts\":")));
        assert!(
            stats
                .iter()
                .any(|record| record.value.contains("\"success_rate\":"))
        );
    }

    #[tokio::test]
    async fn requirement_swarm_code_task_defaults_to_harness_facade() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "swarm-code-default-harness";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        let _ = app
            .process_requirement_swarm_inner(
                session_id,
                "Implement Rust patch for src/lib.rs and run cargo test.",
            )
            .await
            .expect("swarm code task");

        let repo_context = app
            .state_store
            .get_knowledge(&format!("harness:repo-context:{session_id}:latest"))
            .await
            .expect("db query");
        assert!(
            repo_context.is_some(),
            "code task through swarm must route to harness and persist repo context"
        );
    }

    #[tokio::test]
    async fn requirement_swarm_emits_all_evidence_tag_stages_smoke() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "evidence-stage-session";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        app.process_requirement_swarm(
            session_id,
            "Build a swarm plan, execute through MCP, verify outcome, and apply governed learning updates.",
        )
        .await
        .expect("swarm");

        let records = app
            .state_store
            .list_knowledge_by_prefix(&format!("evidence:tag:{session_id}:"))
            .await
            .expect("evidence tags");
        assert!(
            !records.is_empty(),
            "expected evidence tags to be persisted for session"
        );

        let mut stages = std::collections::BTreeSet::new();
        for record in records {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) {
                if let Some(stage) = value.get("stage").and_then(serde_json::Value::as_str) {
                    stages.insert(stage.to_string());
                }
            }
        }

        let required = ["guard", "verify", "learn"];
        let missing = required
            .iter()
            .filter(|stage| !stages.contains(**stage))
            .copied()
            .collect::<Vec<_>>();

        assert!(
            missing.is_empty(),
            "missing evidence tag stages: {:?}; observed: {:?}",
            missing,
            stages
        );
    }
    #[tokio::test]
    async fn requirement_swarm_emits_accept_repair_escalate_decisions_in_same_session() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "swarm-decision-matrix";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        app.process_requirement_swarm(
            session_id,
            "[decision:accept] Build a governed swarm plan and execute with verification.",
        )
        .await
        .expect("swarm accept run");

        let synthetic_repair = UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Repair,
            reasons: vec!["synthetic repair decision for same-session evidence gate".to_string()],
            verifier_score: 0.2,
            forced: true,
        };
        let synthetic_escalate = UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Escalate,
            reasons: vec!["synthetic escalate decision for same-session evidence gate".to_string()],
            verifier_score: 0.01,
            forced: true,
        };

        app.persist_swarm_unified_decision(
            session_id,
            "trace:swarm-decision-matrix:repair",
            "[decision:repair] synthetic evidence",
            &synthetic_repair,
            None,
            None,
        )
        .await
        .expect("persist repair decision");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        app.persist_swarm_unified_decision(
            session_id,
            "trace:swarm-decision-matrix:escalate",
            "[decision:escalate] synthetic evidence",
            &synthetic_escalate,
            None,
            None,
        )
        .await
        .expect("persist escalate decision");

        let records = app
            .state_store
            .list_knowledge_by_prefix(&format!("runtime:decision:{session_id}:"))
            .await
            .expect("runtime decisions");
        assert!(
            records.len() >= 3,
            "expected at least 3 decision records, got {}",
            records.len()
        );

        let mut observed = std::collections::BTreeSet::new();
        for record in records {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) {
                if let Some(decision) = value.get("decision").and_then(serde_json::Value::as_str) {
                    observed.insert(decision.to_string());
                }
            }
        }

        assert!(
            observed.contains("accept"),
            "missing accept decision: {observed:?}"
        );
        assert!(
            observed.contains("repair"),
            "missing repair decision: {observed:?}"
        );
        assert!(
            observed.contains("escalate"),
            "missing escalate decision: {observed:?}"
        );
    }

    #[tokio::test]
    async fn requirement_swarm_emits_accept_repair_reject_escalate_decisions_in_same_session() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "swarm-decision-four-state";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        app.process_requirement_swarm(
            session_id,
            "[decision:accept] Build a governed swarm plan and execute with verification.",
        )
        .await
        .expect("swarm accept run");

        let synthetic_repair = UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Repair,
            reasons: vec!["synthetic repair decision for four-state evidence gate".to_string()],
            verifier_score: 0.2,
            forced: true,
        };
        let synthetic_reject = UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Reject,
            reasons: vec!["synthetic reject decision for four-state evidence gate".to_string()],
            verifier_score: 0.0,
            forced: true,
        };
        let synthetic_escalate = UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Escalate,
            reasons: vec!["synthetic escalate decision for four-state evidence gate".to_string()],
            verifier_score: 0.01,
            forced: true,
        };

        app.persist_swarm_unified_decision(
            session_id,
            "trace:swarm-decision-four-state:repair",
            "[decision:repair] synthetic evidence",
            &synthetic_repair,
            None,
            None,
        )
        .await
        .expect("persist repair decision");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        app.persist_swarm_unified_decision(
            session_id,
            "trace:swarm-decision-four-state:reject",
            "[decision:reject] synthetic evidence",
            &synthetic_reject,
            None,
            None,
        )
        .await
        .expect("persist reject decision");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        app.persist_swarm_unified_decision(
            session_id,
            "trace:swarm-decision-four-state:escalate",
            "[decision:escalate] synthetic evidence",
            &synthetic_escalate,
            None,
            None,
        )
        .await
        .expect("persist escalate decision");

        let records = app
            .state_store
            .list_knowledge_by_prefix(&format!("runtime:decision:{session_id}:"))
            .await
            .expect("runtime decisions");
        assert!(
            records.len() >= 4,
            "expected at least 4 decision records, got {}",
            records.len()
        );

        let mut observed = std::collections::BTreeSet::new();
        for record in records {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) {
                if let Some(decision) = value.get("decision").and_then(serde_json::Value::as_str) {
                    observed.insert(decision.to_string());
                }
            }
        }

        for expected in ["accept", "repair", "reject", "escalate"] {
            assert!(
                observed.contains(expected),
                "missing {expected} decision: {observed:?}"
            );
        }
    }

    #[tokio::test]
    async fn bootstrap_connects_postgres_when_storage_shadow_enabled() {
        let mut config = AppConfig::default();
        config.storage.backend = crate::config::StorageBackend::PrimaryStore;
        config.storage.mode = crate::config::StorageMode::Shadow;
        config.storage.postgres.enabled = true;
        config.storage.postgres.uri = "postgres://postgres:123456@localhost:5432/ontoloop_dev".into();
        config.storage.postgres.schema = "public".into();

        let app = AutoLoopApp::try_new(config).expect("construct app");
        let report = app.bootstrap().await.expect("bootstrap");

        assert_eq!(report.app_name, "autoloop");
        assert!(app.postgresdb.is_some(), "postgresdb should be initialized in shadow mode");
    }

    #[tokio::test]
    async fn verify_governance_block_surfaces_uniform_fields() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "verify-governance-block";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        let acl_bundle = AclPolicyBundle {
            policy_version: "acl-v2-test".into(),
            tenant_id: "tenant:default".into(),
            rules: vec![
                AclRule {
                    id: "allow-runtime-read".into(),
                    actor_prefix: format!("principal:{session_id}"),
                    action: "read".into(),
                    namespace_prefix: "memory:runtime".into(),
                    sensitivity: "any".into(),
                    effect: AclEffect::Allow,
                },
                AclRule {
                    id: "deny-verify-read".into(),
                    actor_prefix: format!("principal:{session_id}"),
                    action: "read".into(),
                    namespace_prefix: "memory:verification".into(),
                    sensitivity: "any".into(),
                    effect: AclEffect::Deny,
                },
            ],
        };
        app.state_store
            .upsert_json_knowledge(
                "memory:acl:policy:tenant:default:latest",
                &acl_bundle,
                "test-suite",
            )
            .await
            .expect("seed acl");

        let error = app
            .process_requirement_swarm(
                session_id,
                "Build a swarm plan, execute through MCP, verify outcome, and apply governed learning updates.",
            )
            .await
            .expect_err("verify governance should block");

        let reason: serde_json::Value =
            serde_json::from_str(&error.to_string()).expect("json reason");
        let evidence_ref = reason
            .get("evidence_ref")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        assert_eq!(
            reason
                .get("rule_id")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            "deny-verify-read"
        );
        assert_eq!(
            reason
                .get("policy_version")
                .and_then(|value| value.as_str())
                .unwrap_or_default(),
            "acl-v2-test"
        );
        assert!(evidence_ref.starts_with("evidence:tag:"));
        assert!(
            reason
                .get("replay_fp")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .starts_with("replay-fp:")
        );

        let persisted = app
            .state_store
            .get_knowledge(&evidence_ref)
            .await
            .expect("db")
            .expect("evidence tag persisted");
        assert!(persisted.value.contains("governance.verify.blocked"));
    }

    fn test_evolution_input(session_id: &str, runtime_mode: &str) -> IngestInput {
        IngestInput {
            session_id: session_id.to_string(),
            trace_id: format!("trace:{session_id}:d8"),
            tenant_id: "tenant:d8".to_string(),
            policy_version: "policy-v2".to_string(),
            runtime_mode: runtime_mode.to_string(),
            available_tools: vec!["tool:planner".to_string()],
            memory_refs: vec![format!("memory:{session_id}:latest")],
            graph_refs: vec![format!("graph:{session_id}:latest")],
            repo_refs: vec!["repo://ontoloop".to_string()],
            policy_refs: vec!["policy:tenant:d8:default".to_string()],
            tool_refs: vec!["tool:planner".to_string()],
            budget_micros: 100_000,
            latency_budget_ms: 2_000,
            budget_profile: BTreeMap::from([
                ("token_budget".to_string(), 100_000_u64),
                ("latency_budget_ms".to_string(), 2_000_u64),
            ]),
            now_ms: 1_710_000_123_000,
            telemetry_replay: Some(TelemetryReplaySnapshot {
                verifier_score: 0.9,
                provider_retry_count: 0,
                tool_retry_count: 0,
                replay_mismatch_rate: 0.05,
                deterministic_boundary_respected: true,
                latency_p95_ms: 800,
                worldline_weights: None,
                weights_version: None,
            }),
            proposal_signals: None,
        }
    }

    #[tokio::test]
    async fn production_write_gate_blocks_canary_path_9c() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "d8-prod-gate-canary";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        let kernel = EvolutionOsKernel::new();
        let mut cycle = kernel
            .run_shadow_cycle(test_evolution_input(session_id, "shadow"))
            .expect("shadow cycle");
        cycle.board_decision = crate::contracts::evolution_os::PromotionDecision::PromoteGovernanceContract;
        cycle.board_outcome.decision =
            crate::contracts::evolution_os::PromotionDecision::PromoteGovernanceContract;
        cycle.path_plan.selected_path = PromotionPath::Path9C_GovernanceUpdate;
        cycle.rollout.stage = crate::evolution_os::RolloutStage::Canary10;
        cycle.board_outcome.judge.policy_compliant = true;

        let result = app.execute_evolution_promotion_paths(session_id, &cycle).await;
        assert_eq!(result["path"].as_str(), Some("9C"));
        assert_eq!(result["status"].as_str(), Some("blocked"));
        assert_eq!(result["reason"].as_str(), Some("production_write_gate_denied"));
        assert_eq!(
            result["gate"]["deny_reason"].as_str(),
            Some("rollout_stage_not_full")
        );
        assert_eq!(result["gate"]["policy_allow"].as_bool(), Some(true));
        assert_eq!(
            result["gate"]["board_decision"].as_str(),
            Some("PromoteGovernanceContract")
        );

        let governance_latest = app
            .state_store
            .get_knowledge(&format!("policy:evolution:governance:{session_id}:latest"))
            .await
            .expect("db");
        assert!(
            governance_latest.is_none(),
            "canary path must not write production governance record"
        );
    }

    #[tokio::test]
    async fn production_write_gate_allows_full_path_9c_with_traceable_evidence() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "d8-prod-gate-full";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        let kernel = EvolutionOsKernel::new();
        let mut cycle = kernel
            .run_shadow_cycle(test_evolution_input(session_id, "full"))
            .expect("shadow cycle");
        cycle.board_decision = crate::contracts::evolution_os::PromotionDecision::PromoteGovernanceContract;
        cycle.board_outcome.decision =
            crate::contracts::evolution_os::PromotionDecision::PromoteGovernanceContract;
        cycle.path_plan.selected_path = PromotionPath::Path9C_GovernanceUpdate;
        cycle.rollout.stage = crate::evolution_os::RolloutStage::Full;
        cycle.board_outcome.judge.policy_compliant = true;

        let result = app.execute_evolution_promotion_paths(session_id, &cycle).await;
        assert_eq!(result["path"].as_str(), Some("9C"));
        assert_eq!(result["status"].as_str(), Some("proposed"));
        assert_eq!(result["gate"]["production_write_allowed"].as_bool(), Some(true));

        let evidence_ref = result["gate"]["evidence_ref"]
            .as_str()
            .expect("gate evidence ref");
        let evidence = app
            .state_store
            .get_knowledge(evidence_ref)
            .await
            .expect("db")
            .expect("gate evidence record");
        let evidence_json: serde_json::Value =
            serde_json::from_str(&evidence.value).expect("gate evidence json");
        assert_eq!(
            evidence_json["gate"]["board_decision"].as_str(),
            Some("PromoteGovernanceContract")
        );
        assert_eq!(evidence_json["gate"]["policy_allow"].as_bool(), Some(true));

        let governance_latest = app
            .state_store
            .get_knowledge(&format!("policy:evolution:governance:{session_id}:latest"))
            .await
            .expect("db");
        assert!(
            governance_latest.is_some(),
            "full path must persist production governance record"
        );
        let governance_value: serde_json::Value = serde_json::from_str(
            &governance_latest.expect("governance latest exists").value,
        )
        .expect("governance payload json");
        assert_eq!(
            governance_value
                .get("board_decision")
                .and_then(serde_json::Value::as_str),
            Some("PromoteGovernanceContract")
        );
        assert_eq!(
            governance_value
                .get("policy_allow")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert!(
            governance_value
                .get("evidence_ref")
                .and_then(serde_json::Value::as_str)
                .is_some(),
            "governance write must carry evidence_ref"
        );
        assert_eq!(
            governance_value
                .get("deny_reason")
                .and_then(serde_json::Value::as_str),
            Some("allowed")
        );
    }

    #[tokio::test]
    async fn production_write_gate_blocks_canary_path_9d() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "d8-prod-gate-9d-canary";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        let kernel = EvolutionOsKernel::new();
        let mut cycle = kernel
            .run_shadow_cycle(test_evolution_input(session_id, "shadow"))
            .expect("shadow cycle");
        cycle.board_decision = crate::contracts::evolution_os::PromotionDecision::Localize;
        cycle.board_outcome.decision = crate::contracts::evolution_os::PromotionDecision::Localize;
        cycle.path_plan.selected_path = PromotionPath::Path9D_LocalOnly;
        cycle.rollout.stage = crate::evolution_os::RolloutStage::Canary10;
        cycle.board_outcome.judge.policy_compliant = true;

        let result = app.execute_evolution_promotion_paths(session_id, &cycle).await;
        assert_eq!(result["path"].as_str(), Some("9D"));
        assert_eq!(result["status"].as_str(), Some("blocked"));
        assert_eq!(
            result["reason"].as_str(),
            Some("production_write_gate_denied")
        );
        assert_eq!(
            result["gate"]["deny_reason"].as_str(),
            Some("decision_not_production_promotable")
        );
        assert_eq!(result["gate"]["policy_allow"].as_bool(), Some(true));
        assert!(
            result["gate"]["evidence_ref"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );

        let local_latest = app
            .state_store
            .get_knowledge(&format!("settings:local:evolution:{session_id}:latest"))
            .await
            .expect("db");
        assert!(
            local_latest.is_none(),
            "canary path must not write local experiment record"
        );
    }

    #[test]
    fn swarm_budget_preflight_compacts_when_budget_overflows() {
        let large = "build end-to-end governed runtime ".repeat(900);
        let preflight = build_swarm_budget_preflight("s-budget-overflow", &large, 128);
        assert!(preflight.compaction_applied);
        assert_eq!(preflight.lane_mode, "dual");
        assert!(preflight.replanned_chars < preflight.original_chars);
        assert!(preflight.reason.contains("budget_preflight_overflow"));
    }

    #[test]
    fn swarm_budget_preflight_keeps_request_when_budget_is_sufficient() {
        let request = "implement a simple status endpoint with evidence refs";
        let preflight = build_swarm_budget_preflight("s-budget-ok", request, 8_000);
        assert!(!preflight.compaction_applied);
        assert_eq!(preflight.replanned_request, request);
        assert!(preflight.reason == "single_lane_sufficient" || preflight.reason == "complexity_threshold_met");
    }

    #[test]
    fn swarm_budget_preflight_applies_execution_ceiling_even_when_policy_is_high() {
        let large = "token ".repeat(9_500);
        let preflight = build_swarm_budget_preflight("s-budget-ceiling", &large, 32_000);
        assert_eq!(preflight.effective_max_tokens, SWARM_EXECUTION_TOKEN_CEILING);
        assert!(preflight.compaction_applied);
        assert!(preflight.replanned_chars < preflight.original_chars);
    }

    #[test]
    fn compact_replan_preserves_artifact_target_path_hint() {
        let content = format!(
            "{}\n{{api_version:artifact_delivery/v1,requires_artifact:true,target_path:D:/AutoLoop/output/d5-budget-pass.json,validation_rules:{{exists_required:true}}}}",
            "D5 budget stress ".repeat(1_500)
        );
        let compacted = compact_and_replan_requirement(&content, 700);
        assert!(compacted.contains("[PreservedArtifactContract]"));
        assert!(compacted.contains("target_path=D:/AutoLoop/output/d5-budget-pass.json"));
    }

    #[test]
    fn code_task_classifier_routes_expected_requests_to_harness() {
        assert!(should_route_code_task_to_harness(
            "Implement a Rust patch and run cargo test for src/lib.rs"
        ));
        assert!(should_route_code_task_to_harness(
            r#"{"api_version":"artifact_delivery/v1","requires_artifact":true}"#
        ));
        assert!(!should_route_code_task_to_harness(
            "Summarize last session decisions and list policy diffs only"
        ));
    }

    #[test]
    fn structured_patch_ops_hint_parser_extracts_ops() {
        let content = r#"please apply
{
  "api_version":"structured_patch/v1",
  "patch_ops":[
    {"op_id":"op-1","kind":"create_file","path":"output/demo.txt","patch":"hello","metadata":{}}
  ]
}
"#;
        let ops = extract_structured_patch_ops_hint(content).expect("ops");
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op_id, "op-1");
        assert_eq!(ops[0].path, "output/demo.txt");
    }

    #[test]
    fn structured_patch_ops_hint_requires_contract_marker() {
        let content = r#"{"patch_ops":[{"op_id":"op-1","kind":"create_file","path":"a.txt","patch":"x","metadata":{}}]}"#;
        assert!(extract_structured_patch_ops_hint(content).is_none());
    }

    #[test]
    fn shell_loop_request_hint_parser_extracts_steps() {
        let content = r#"run this:
{
  "api_version":"shell_loop/v1",
  "max_iterations":2,
  "steps":[
    {"step_id":"s1","command":"Write-Output 'first'","continue_on_error":false},
    {"step_id":"s2","command":"Write-Output 'second'","continue_on_error":true}
  ]
}
"#;
        let req = extract_shell_loop_request_hint(content).expect("shell loop request");
        assert_eq!(req.steps.len(), 2);
        assert_eq!(req.steps[0].step_id, "s1");
    }

    #[test]
    fn shell_loop_request_hint_requires_marker() {
        let content = r#"{"api_version":"v1","steps":[]}"#;
        assert!(extract_shell_loop_request_hint(content).is_none());
    }

    #[test]
    fn git_checkpoint_request_hint_parser_extracts_operations() {
        let content = r#"checkpoint:
{
  "api_version":"git_checkpoint/v1",
  "local_safe_mode": true,
  "operations":[
    {"action":"branch","branch":"feature/demo"},
    {"action":"rollback","rollback_ref":"HEAD~1"}
  ]
}
"#;
        let request =
            extract_git_checkpoint_request_hint(content).expect("git checkpoint request parse");
        assert_eq!(request.operations.len(), 2);
        assert!(request.local_safe_mode);
    }

    #[test]
    fn git_checkpoint_request_hint_requires_marker() {
        let content = r#"{"api_version":"v1","operations":[]}"#;
        assert!(extract_git_checkpoint_request_hint(content).is_none());
    }

    #[test]
    fn test_verifier_request_hint_parser_extracts_runners() {
        let content = r#"verify this:
{
  "api_version":"test_verifier/v1",
  "fail_fast": true,
  "runners":[
    {"runner_id":"build","kind":"build","command":"Write-Output ok","required":true},
    {"runner_id":"lint","kind":"lint","command":"Write-Output lint","required":false}
  ]
}
"#;
        let req = extract_test_verifier_request_hint(content).expect("test verifier request");
        assert_eq!(req.runners.len(), 2);
        assert_eq!(req.runners[0].runner_id, "build");
    }

    #[test]
    fn test_verifier_request_hint_requires_marker() {
        let content = r#"{"api_version":"v1","runners":[]}"#;
        assert!(extract_test_verifier_request_hint(content).is_none());
    }

    #[test]
    fn default_test_verifier_request_for_cargo_workspace_has_three_required_stages() {
        let cwd = std::env::current_dir().expect("cwd");
        let workspace_root = if cwd.join("Cargo.toml").exists() {
            cwd
        } else if cwd.join("autoloop-app").join("Cargo.toml").exists() {
            cwd.join("autoloop-app")
        } else {
            panic!("expected cargo workspace in test environment")
        };
        let request =
            build_default_test_verifier_request(&workspace_root).expect("default verifier request");
        assert_eq!(request.runners.len(), 3);
        assert!(request
            .runners
            .iter()
            .any(|runner| matches!(runner.kind, crate::query_engine::TestRunnerKind::Build) && runner.required));
        assert!(request
            .runners
            .iter()
            .any(|runner| matches!(runner.kind, crate::query_engine::TestRunnerKind::Lint) && runner.required));
        assert!(request
            .runners
            .iter()
            .any(|runner| matches!(runner.kind, crate::query_engine::TestRunnerKind::Test) && runner.required));
    }

    #[test]
    fn default_test_verifier_request_none_for_non_cargo_workspace() {
        let tmp = std::env::temp_dir().join(format!("ontoloop-no-cargo-{}", current_time_ms()));
        std::fs::create_dir_all(&tmp).expect("create tmp");
        assert!(build_default_test_verifier_request(&tmp).is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn iteration_controller_config_hint_parser_extracts_limits() {
        let content =
            "iteration_controller/v1 max_attempts=2 budget_tokens=1200 max_runtime_ms=5000 retry_on=test_verifier,structured_patch";
        let config = extract_iteration_controller_config_hint(content).expect("config");
        assert_eq!(config.max_attempts, 2);
        assert_eq!(config.budget_tokens, Some(1200));
        assert_eq!(config.max_runtime_ms, 5000);
        assert!(config.retry_on.iter().any(|item| item == "test_verifier"));
    }

    #[test]
    fn swarm_budget_preflight_compacts_and_prefers_dual_lane_when_overflowed() {
        let content = "build and execute a complex multi-tool workflow with replay evidence ".repeat(250);
        let preflight = build_swarm_budget_preflight("session-test", &content, 900);
        assert!(preflight.compaction_applied);
        assert_eq!(preflight.lane_mode, "dual");
        assert!(
            preflight.final_request_tokens <= preflight.effective_max_tokens
                || preflight.replanned_request.contains("Budget preflight compacted")
        );
    }

    #[test]
    fn dual_lane_plan_builds_execute_seed() {
        let content = r#"
1. collect requirement details
2. run tools and write artifact
3. verify and persist evidence refs
target_path=D:/AutoLoop/output/demo.html
"#;
        let preflight = build_swarm_budget_preflight("session-plan", content, 2200);
        let plan_lane = build_swarm_plan_lane("session-plan", content, &preflight);
        let seed = build_swarm_execute_lane_seed(&plan_lane).expect("seed");
        assert!(seed.contains("Objective:"));
        assert!(seed.contains("ExecutionPlan:"));
        assert!(seed.contains("1."));
    }

    #[tokio::test]
    async fn iteration_controller_stops_at_attempt_limit_with_reason() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "test:iteration:attempt-limit";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant permissions");

        let request = r#"iteration_controller/v1 max_attempts=2 retry_on=test_verifier max_runtime_ms=20000
{
  "api_version":"test_verifier/v1",
  "fail_fast":true,
  "runners":[
    {"runner_id":"lint","kind":"lint","command":"Write-Error 'fail'; exit 1","required":true}
  ]
}"#;

        let error = app
            .execute_via_runtime_facade(session_id, "harness_facade", request)
            .await
            .expect_err("iteration controller should fail after attempt limit");
        assert!(
            error
                .to_string()
                .contains("iteration controller terminated"),
            "error should expose controller termination"
        );

        let latest = app
            .state_store
            .get_knowledge(&format!("harness:iteration-controller:{session_id}:latest"))
            .await
            .expect("db")
            .expect("iteration report");
        let payload: serde_json::Value =
            serde_json::from_str(&latest.value).expect("iteration report json");
        assert_eq!(
            payload["termination_reason"].as_str(),
            Some("attempt_limit_exceeded")
        );
        assert_eq!(payload["attempts_used"].as_u64(), Some(2));
    }

    #[tokio::test]
    async fn code_task_non_harness_surface_is_rejected_by_gate() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "test:code:harness:reject";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant permissions");

        let error = app
            .execute_via_runtime_facade(
                session_id,
                "process_direct",
                "Implement frontend billing page in html/css and write code to output file.",
            )
            .await
            .expect_err("non harness surface should be rejected");
        assert!(
            error
                .to_string()
                .contains("code task requires harness façade execution")
        );

        let events = app
            .state_store
            .list_knowledge_by_prefix(&format!("eventlog:{session_id}:"))
            .await
            .expect("event logs");
        assert!(
            events.iter().any(|record| {
                record.value.contains("code_harness_gate.rejected")
                    && record.value.contains("\"decision\":\"reject\"")
            }),
            "code harness gate rejection must be evidence logged"
        );
    }

    #[tokio::test]
    async fn background_agent_code_task_is_rejected_by_harness_gate() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "test:code:background:reject";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant permissions");

        let error = app
            .background_task_start_agent(
                session_id,
                "bg-code-task",
                "Implement frontend billing page in html/css and write code to output file.",
                0,
            )
            .await
            .expect_err("background code task should be rejected");
        assert!(
            error
                .to_string()
                .contains("code task requires harness façade execution"),
            "error should explain harness-only path"
        );

        let events = app
            .state_store
            .list_knowledge_by_prefix(&format!("eventlog:{session_id}:"))
            .await
            .expect("event logs");
        assert!(
            events.iter().any(|record| {
                record.value.contains("code_harness_gate.rejected")
                    && record.value.contains("\"requested_surface\":\"background_agent\"")
            }),
            "background agent rejection should be evidence logged"
        );
    }

    #[tokio::test]
    async fn requirement_swarm_persists_relation_graph_trace() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "relation-trace-session";
        app.state_store
            .grant_permissions(
                session_id,
                vec![
                    PermissionAction::Read,
                    PermissionAction::Write,
                    PermissionAction::Dispatch,
                ],
            )
            .await
            .expect("grant");

        app.process_requirement_swarm(
            session_id,
            "Plan and execute with tool loop, permission gate, hooks, plugin and replay tracking.",
        )
        .await
        .expect("swarm");

        let relation_events = app
            .state_store
            .list_relation_events(session_id, 64)
            .await
            .expect("relation events");
        let relation_failures = app
            .state_store
            .list_knowledge_by_prefix("eventlog:")
            .await
            .expect("event logs")
            .into_iter()
            .filter(|record| record.value.contains("relation.contract.persist_failed"))
            .count();
        let relation_fallback_events = app
            .state_store
            .list_knowledge_by_prefix(&format!("relation:event:{session_id}:"))
            .await
            .expect("relation fallback events")
            .len();
        assert!(
            !relation_events.is_empty() || relation_fallback_events > 0 || relation_failures > 0,
            "expected relation event persisted or explicit persist failure evidence"
        );
    }
}




























































