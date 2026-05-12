pub mod workspace_loader;

use anyhow::{Result, bail};
use async_trait::async_trait;
use autoloop_state_adapter::{ScheduleEvent, StateStore};

use crate::{
    config::AgentConfig,
    contracts::{
        artifact_delivery::{
            ArtifactDeliveryContract, ArtifactHashAlgorithm, ArtifactValidationRules,
            ArtifactWriteProof,
        },
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    hooks::HookRegistry,
    memory::MemorySubsystem,
    observability::event_stream::append_event,
    providers::{ChatMessage, LlmResponse, ProviderRegistry, ToolCall},
    query_engine::{
        QueryLoopBackend, QueryLoopConfig, QueryLoopEngine, QueryLoopInput, QueryLoopOutput, RuntimeQueryLoopBackend,
        TokenBudgetFrame, turn_state::QueryTurnState,
    },
    runtime::{
        RuntimeKernel,
        decision_protocol::{
            RuntimeDecisionKind, UnifiedDecisionInput, UnifiedDecisionOutput,
            evaluate_unified_decision, load_thresholds_from_env, parse_decision_hint,
        },
        evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
        trigger_runtime::TriggerRuntimeEngine,
    },
    security::SecurityPolicy,
    session::SessionStore,
    tools::ToolRegistry,
};

#[derive(Clone)]
pub struct AgentRuntime {
    config: AgentConfig,
    providers: ProviderRegistry,
    tools: ToolRegistry,
    sessions: SessionStore,
    memory: MemorySubsystem,
    hooks: HookRegistry,
    security: SecurityPolicy,
    runtime: RuntimeKernel,
    state_store: StateStore,
}

impl AgentRuntime {
    pub fn new(
        config: AgentConfig,
        providers: ProviderRegistry,
        tools: ToolRegistry,
        sessions: SessionStore,
        memory: MemorySubsystem,
        hooks: HookRegistry,
        security: SecurityPolicy,
        runtime: RuntimeKernel,
        state_store: StateStore,
    ) -> Self {
        Self {
            config,
            providers,
            tools,
            sessions,
            memory,
            hooks,
            security,
            runtime,
            state_store,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.config.max_iterations == 0 {
            bail!("agent.max_iterations must be greater than 0");
        }
        if self.config.memory_window == 0 {
            bail!("agent.memory_window must be greater than 0");
        }
        Ok(())
    }

    pub async fn process_message(&self, session_id: &str, content: &str) -> Result<String> {
        let security_report = self.security.inspect_text(content);
        if security_report.blocked {
            let refusal = format!(
                "Request blocked by security policy: {}",
                security_report
                    .findings
                    .into_iter()
                    .map(|finding| finding.detail)
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            self.sessions
                .append_assistant_message(session_id, &refusal)
                .await;
            return Ok(refusal);
        }

        self.sessions.append_user_message(session_id, content).await;
        let history = self.sessions.history(session_id).await;

        let mut messages = Vec::new();
        let memory_context = self
            .memory
            .build_memory_context_with_learning(&self.state_store, session_id, content, &history)
            .await
            .unwrap_or_else(|_| self.memory.build_memory_context_for(content, &history));
        let evolution_summary = self
            .state_store
            .get_knowledge(&format!("memory:{session_id}:self-evolution"))
            .await
            .ok()
            .flatten()
            .map(|record| record.value);
        let research_summary = self
            .state_store
            .get_knowledge(&format!("research:{session_id}:report"))
            .await
            .ok()
            .flatten()
            .map(|record| record.value);
        let capability_hints = self
            .state_store
            .list_knowledge_by_prefix(&format!("memory:{session_id}:evolution-proposal:"))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
            .filter_map(|value| {
                value
                    .get("tool_name")
                    .and_then(|tool_name| tool_name.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        let prompt_overlay = self.providers.derive_prompt_policy(
            content,
            evolution_summary.as_deref(),
            research_summary.as_deref(),
            &capability_hints,
        );
        let adaptive_guidance = if prompt_overlay.directives.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n# Adaptive Guidance\n{}\n\n# Policy Rationale\n{}",
                prompt_overlay
                    .directives
                    .iter()
                    .map(|line| format!("- {line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                prompt_overlay.rationale
            )
        };
        let system_prompt = self.hooks.augment_system_prompt(&format!(
            "{}\n\n# Memory Targets\n{}",
            self.config.system_prompt, memory_context
        ));
        let system_prompt = format!("{system_prompt}{adaptive_guidance}");

        messages.push(ChatMessage { tool_call_id: None, tool_calls: None,
            role: "system".into(),
            content: system_prompt,
        });
        messages.extend(history);

        let execution_identity = self.execution_identity_for_session(session_id).await?;
        let backend = RuntimeQueryLoopBackend::new(
            self.runtime.clone(),
            self.state_store.clone(),
            self.tools.clone(),
            self.providers.clone(),
            self.security.clone(),
            execution_identity,
            self.default_provider_constraints(),
            self.default_constraints(),
            prompt_overlay.preferred_model.clone(),
        );
        let engine = QueryLoopEngine::new(QueryLoopConfig {
            max_iterations: self.config.max_iterations,
            provider_retry_limit: 1,
            tool_retry_limit: 1,
            preserve_recent_messages: 4,
        });

        let trace_id = format!("{}:agent-loop", session_id);
        let decision_thresholds = load_thresholds_from_env();
        let forced_hint = parse_decision_hint(content);
        let primary_messages = messages;
        let shadow_messages = primary_messages.clone();
        let mut budget_compaction_applied = false;

        let mut loop_output = match engine
            .run(
                &backend,
                QueryLoopInput {
                    session_id: session_id.to_string(),
                    trace_id: trace_id.clone(),
                    messages: primary_messages.clone(),
                    token_budget_frame: Some(TokenBudgetFrame::bounded(12_000, 2_000)),
                },
            )
            .await
        {
            Ok(output) => output,
            Err(error) => {
                let error_text = error.to_string();
                if is_budget_exceeded_error(&error_text) {
                    let compacted_messages = build_budget_compacted_messages(&primary_messages);
                    if compacted_messages.len() >= 2 {
                        budget_compaction_applied = true;
                        match engine
                            .run(
                                &backend,
                                QueryLoopInput {
                                    session_id: session_id.to_string(),
                                    trace_id: format!("{trace_id}:budget-replan"),
                                    messages: compacted_messages,
                                    token_budget_frame: Some(TokenBudgetFrame::bounded(12_000, 1_200)),
                                },
                            )
                            .await
                        {
                            Ok(output) => output,
                            Err(second_error) => {
                                let hardgate_passed = !second_error
                                    .to_string()
                                    .contains("hardgate_reject");
                                let decision = evaluate_unified_decision(
                                    UnifiedDecisionInput {
                                        hardgate_passed,
                                        compile_failed: true,
                                        compaction_applied: true,
                                        max_iterations_reached: false,
                                        provider_retry_count: 0,
                                        tool_retry_count: 0,
                                        guard_observations: Vec::new(),
                                        verifier_score: -1.0,
                                        forced_hint: forced_hint.clone(),
                                    },
                                    &decision_thresholds,
                                );
                                let followup = self
                                    .enqueue_decision_followup(session_id, &trace_id, content, &decision)
                                    .await?;
                                self.persist_unified_decision(
                                    session_id,
                                    &trace_id,
                                    content,
                                    &decision,
                                    followup.as_ref(),
                                    None,
                                )
                                .await?;
                                return Err(second_error);
                            }
                        }
                    } else {
                        let hardgate_passed = !error_text.contains("hardgate_reject");
                        let decision = evaluate_unified_decision(
                            UnifiedDecisionInput {
                                hardgate_passed,
                                compile_failed: true,
                                compaction_applied: false,
                                max_iterations_reached: false,
                                provider_retry_count: 0,
                                tool_retry_count: 0,
                                guard_observations: Vec::new(),
                                verifier_score: -1.0,
                                forced_hint: forced_hint.clone(),
                            },
                            &decision_thresholds,
                        );
                        let followup = self
                            .enqueue_decision_followup(session_id, &trace_id, content, &decision)
                            .await?;
                        self.persist_unified_decision(
                            session_id,
                            &trace_id,
                            content,
                            &decision,
                            followup.as_ref(),
                            None,
                        )
                        .await?;
                        return Err(error);
                    }
                } else {
                    let hardgate_passed = !error_text.contains("hardgate_reject");
                    let decision = evaluate_unified_decision(
                        UnifiedDecisionInput {
                            hardgate_passed,
                            compile_failed: true,
                            compaction_applied: false,
                            max_iterations_reached: false,
                            provider_retry_count: 0,
                            tool_retry_count: 0,
                            guard_observations: Vec::new(),
                            verifier_score: -1.0,
                            forced_hint: forced_hint.clone(),
                        },
                        &decision_thresholds,
                    );
                    let followup = self
                        .enqueue_decision_followup(session_id, &trace_id, content, &decision)
                        .await?;
                    self.persist_unified_decision(
                        session_id,
                        &trace_id,
                        content,
                        &decision,
                        followup.as_ref(),
                        None,
                    )
                    .await?;
                    return Err(error);
                }

            }
        };

        if self.query_loop_shadow_enabled() {
            let _ = self
                .run_query_loop_shadow(session_id, &trace_id, shadow_messages, &loop_output)
                .await;
        }

        let mut decision = evaluate_unified_decision(
            UnifiedDecisionInput {
                hardgate_passed: true,
                compile_failed: false,
                compaction_applied: loop_output.compaction_boundary.is_some() || budget_compaction_applied,
                max_iterations_reached: loop_output.state == QueryTurnState::MaxIterationsReached,
                provider_retry_count: loop_output.provider_retry_count,
                tool_retry_count: loop_output.tool_retry_count,
                guard_observations: loop_output.guard_observations.clone(),
                verifier_score: loop_output.proof.objective.weighted_score,
                forced_hint,
            },
            &decision_thresholds,
        );

        let mut artifact_contract = extract_artifact_delivery_contract(content, session_id, &trace_id);
        if let Some(contract) = artifact_contract.as_mut() {
            if contract.requires_artifact {
                let (mut artifact_ok, mut artifact_reason) =
                    artifact_delivery_gate_passed(contract, &loop_output.tool_events);
                if !artifact_ok {
                    if let Some(tool_output) = self
                        .attempt_artifact_repair_write(contract, content, &loop_output)
                        .await
                    {
                        loop_output
                            .tool_events
                            .push(crate::query_engine::r#loop::ToolExecutionEvent {
                                tool_name: "write_file".to_string(),
                                output: tool_output,
                            });
                        (artifact_ok, artifact_reason) =
                            artifact_delivery_gate_passed(contract, &loop_output.tool_events);
                    }
                }
                if !artifact_ok {
                    decision.kind = RuntimeDecisionKind::Reject;
                    decision.forced = true;
                    if !decision
                        .reasons
                        .iter()
                        .any(|reason| reason == "artifact_delivery_gate_failed")
                    {
                        decision.reasons.push("artifact_delivery_gate_failed".into());
                    }
                    decision.reasons.push(artifact_reason.clone());
                }
            }
        }

        let followup = self
            .enqueue_decision_followup(session_id, &trace_id, content, &decision)
            .await?;
        self.persist_unified_decision(
            session_id,
            &trace_id,
            content,
            &decision,
            followup.as_ref(),
            Some(&loop_output),
        )
        .await?;

        if decision
            .reasons
            .iter()
            .any(|reason| reason == "artifact_delivery_gate_failed")
        {
            let rejection = format!(
                "Execution rejected by artifact delivery gate: {}",
                decision.reasons.join(" | ")
            );
            self.sessions
                .append_assistant_message(session_id, &rejection)
                .await;
            return Err(anyhow::anyhow!(rejection));
        }

        let continuation_turn_id;
        let continuation_checkpoint_token;
        let continuation_replay_fingerprint;
        let continuation_created_at_ms;

        if let Some(continuation) = &loop_output.continuation {
            continuation_turn_id = continuation.checkpoint.turn_id.clone();
            continuation_checkpoint_token = continuation.checkpoint.checkpoint_token.clone();
            continuation_replay_fingerprint = continuation.replay_fingerprint.clone();
            continuation_created_at_ms = continuation.checkpoint.created_at_ms;
        } else {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0);
            continuation_turn_id = format!("{session_id}:turn:{now_ms}");
            continuation_checkpoint_token = format!("resume:{session_id}:{now_ms}");
            continuation_replay_fingerprint = format!("replay-fallback:{session_id}:{now_ms}");
            continuation_created_at_ms = now_ms;
        }

        self.sessions
            .bind_continuation(
                session_id,
                &continuation_turn_id,
                &continuation_checkpoint_token,
            )
            .await;
        let _ = self
            .state_store
            .upsert_json_knowledge(
                format!("query:continuation:{session_id}:latest"),
                &serde_json::json!({
                    "turn_id": continuation_turn_id,
                    "checkpoint_token": continuation_checkpoint_token,
                    "replay_fingerprint": continuation_replay_fingerprint,
                    "created_at_ms": continuation_created_at_ms,
                }),
                "query-engine",
            )
            .await;
        let _ = self
            .state_store
            .upsert_json_knowledge(
                format!(
                    "replay:plugin-trace:{session_id}:{}",
                    crate::orchestration::current_time_ms()
                ),
                &serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "hardgate_pass_token": loop_output.hardgate_pass_token.clone(),
                    "constraint_version": loop_output.constraint_version.clone(),
                    "constraint_ids": loop_output.hard_constraint_ids.clone(),
                    "plugin_execution_traces": loop_output.plugin_execution_traces.clone(),
                    "stream_events": loop_output.stream_events.clone(),
                    "guard_observations": loop_output.guard_observations.clone(),
                    "annotation_proof": loop_output.proof.clone(),
                }),
                "query-engine",
            )
            .await;

        for event in &loop_output.tool_events {
            self.sessions
                .append_tool_message(session_id, &event.tool_name, &event.output)
                .await;
        }

        let final_text = if decision.kind == RuntimeDecisionKind::Reject {
            format!(
                "Execution rejected by verification gate: {}",
                decision.reasons.join(" | ")
            )
        } else {
            loop_output.final_text.clone()
        };

        self.sessions
            .append_assistant_message(session_id, &final_text)
            .await;
        let _ = self
            .state_store
            .upsert_agent_state(
                session_id.to_string(),
                content.to_string(),
                Some(final_text.clone()),
            )
            .await;
        let _ = self
            .hooks
            .run_governed_learning_pipeline(
                &self.state_store,
                &self.memory,
                &self.security,
                session_id,
                session_id,
                content,
                &final_text,
            )
            .await;

        Ok(final_text)
    }

    async fn enqueue_decision_followup(
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
            "prompt_excerpt": preview(content, 220),
            "queued_at_ms": crate::orchestration::current_time_ms(),
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

    async fn persist_unified_decision(
        &self,
        session_id: &str,
        trace_id: &str,
        content: &str,
        decision: &UnifiedDecisionOutput,
        followup: Option<&ScheduleEvent>,
        loop_output: Option<&QueryLoopOutput>,
    ) -> Result<()> {
        let now = crate::orchestration::current_time_ms();
        let key = format!("runtime:decision:{session_id}:{now}");
        let record = serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "decision": decision.kind.as_str(),
            "reasons": decision.reasons.clone(),
            "forced": decision.forced,
            "verifier_score": decision.verifier_score,
            "content_excerpt": preview(content, 240),
            "compile": {
                "hardgate_passed": loop_output.map(|item| !item.hardgate_pass_token.trim().is_empty()).unwrap_or(false),
                "constraint_version": loop_output.map(|item| item.constraint_version.clone()),
                "constraint_ids": loop_output.map(|item| item.hard_constraint_ids.clone()).unwrap_or_default(),
                "compaction_boundary": loop_output.and_then(|item| item.compaction_boundary.clone()),
            },
            "execute": {
                "provider_retry_count": loop_output.map(|item| item.provider_retry_count).unwrap_or(0),
                "tool_retry_count": loop_output.map(|item| item.tool_retry_count).unwrap_or(0),
                "guard_observations": loop_output.map(|item| item.guard_observations.clone()).unwrap_or_default(),
                "tool_call_count": loop_output.map(|item| item.tool_call_count).unwrap_or(0),
                "iteration_count": loop_output.map(|item| item.iteration_count).unwrap_or(0),
            },
            "verify": {
                "proof": loop_output.map(|item| item.proof.clone()),
                "state": loop_output.map(|item| format!("{:?}", item.state).to_ascii_lowercase()),
                "replay_fingerprint": loop_output.map(|item| item.replay_fingerprint.clone()),
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
            Some("agent-decision".to_string()),
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

        Ok(())
    }
}

impl AgentRuntime {
    fn query_loop_shadow_enabled(&self) -> bool {
        let raw = std::env::var("AUTOLOOP_QUERY_LOOP_SHADOW_MODE")
            .unwrap_or_else(|_| "enabled".to_string())
            .to_ascii_lowercase();
        !matches!(raw.as_str(), "disabled" | "off" | "false" | "0")
    }

    fn shadow_token_budget_frame(&self) -> TokenBudgetFrame {
        let max_input = std::env::var("AUTOLOOP_QUERY_LOOP_SHADOW_MAX_INPUT_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(2_048)
            .max(512);
        let max_output = std::env::var("AUTOLOOP_QUERY_LOOP_SHADOW_MAX_OUTPUT_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(512)
            .max(128);
        TokenBudgetFrame::bounded(max_input, max_output)
    }

    async fn run_query_loop_shadow(
        &self,
        session_id: &str,
        trace_id: &str,
        messages: Vec<ChatMessage>,
        primary_output: &QueryLoopOutput,
    ) -> Result<()> {
        let shadow_engine = QueryLoopEngine::new(QueryLoopConfig {
            max_iterations: self.config.max_iterations.min(2),
            provider_retry_limit: 0,
            tool_retry_limit: 0,
            preserve_recent_messages: 4,
        });
        let shadow_backend = NoopShadowQueryLoopBackend::new();
        let shadow_trace_id = format!("{trace_id}:shadow");

        match shadow_engine
            .run(
                &shadow_backend,
                QueryLoopInput {
                    session_id: session_id.to_string(),
                    trace_id: shadow_trace_id.clone(),
                    messages,
                    token_budget_frame: Some(self.shadow_token_budget_frame()),
                },
            )
            .await
        {
            Ok(shadow_output) => {
                let now = crate::orchestration::current_time_ms();
                let key = format!("query-loop:shadow:{session_id}:{now}");
                let payload = serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "shadow_trace_id": shadow_trace_id,
                    "mode": "dual_run_shadow",
                    "primary": {
                        "state": format!("{:?}", primary_output.state).to_ascii_lowercase(),
                        "iteration_count": primary_output.iteration_count,
                        "tool_call_count": primary_output.tool_call_count,
                        "provider_retry_count": primary_output.provider_retry_count,
                        "tool_retry_count": primary_output.tool_retry_count,
                        "compaction_applied": primary_output.compaction_boundary.is_some(),
                        "estimated_input_tokens": primary_output.estimated_input_tokens,
                        "replay_fingerprint": primary_output.replay_fingerprint,
                    },
                    "shadow": {
                        "state": format!("{:?}", shadow_output.state).to_ascii_lowercase(),
                        "iteration_count": shadow_output.iteration_count,
                        "tool_call_count": shadow_output.tool_call_count,
                        "provider_retry_count": shadow_output.provider_retry_count,
                        "tool_retry_count": shadow_output.tool_retry_count,
                        "compaction_applied": shadow_output.compaction_boundary.is_some(),
                        "estimated_input_tokens": shadow_output.estimated_input_tokens,
                        "replay_fingerprint": shadow_output.replay_fingerprint,
                    },
                    "captured_at_ms": now,
                    "evidence_ref": key,
                });
                self.state_store
                    .upsert_json_knowledge(key.clone(), &payload, "query-loop-shadow")
                    .await?;

                let _ = append_event(
                    &self.state_store,
                    "query_loop_shadow_dual_run",
                    trace_id.to_string(),
                    session_id.to_string(),
                    Some("agent-shadow".to_string()),
                    Some("query-engine".to_string()),
                    crate::contracts::version::CONTRACT_VERSION,
                    serde_json::json!({
                        "evidence_ref": key,
                        "primary_replay_fingerprint": primary_output.replay_fingerprint,
                        "shadow_replay_fingerprint": shadow_output.replay_fingerprint,
                        "primary_compaction": primary_output.compaction_boundary.is_some(),
                        "shadow_compaction": shadow_output.compaction_boundary.is_some(),
                    }),
                )
                .await;
                Ok(())
            }
            Err(error) => {
                let now = crate::orchestration::current_time_ms();
                self.state_store
                    .upsert_json_knowledge(
                        format!("query-loop:shadow:{session_id}:{now}:error"),
                        &serde_json::json!({
                            "session_id": session_id,
                            "trace_id": trace_id,
                            "mode": "dual_run_shadow",
                            "status": "error",
                            "error": error.to_string(),
                            "captured_at_ms": now,
                        }),
                        "query-loop-shadow",
                    )
                    .await?;
                Ok(())
            }
        }
    }

    async fn execution_identity_for_session(&self, session_id: &str) -> Result<ExecutionIdentity> {
        if let Some(identity) = self.sessions.identity(session_id).await {
            return Ok(ExecutionIdentity {
                tenant_id: identity.tenant_id,
                principal_id: identity.principal_id,
                policy_id: identity.policy_id,
                lease_token: identity.lease_token,
            });
        }

        if let Some(lease) = self.state_store.get_session_lease(session_id).await? {
            return Ok(ExecutionIdentity {
                tenant_id: lease.tenant_id,
                principal_id: lease.principal_id,
                policy_id: lease.policy_id,
                lease_token: lease.lease_token,
            });
        }

        Ok(ExecutionIdentity {
            tenant_id: "tenant:default".into(),
            principal_id: format!("principal:{session_id}"),
            policy_id: "policy:default".into(),
            lease_token: format!("lease:{session_id}:fallback"),
        })
    }

    fn default_constraints(&self) -> ConstraintSet {
        ConstraintSet {
            max_cpu_percent: 80,
            max_memory_mb: 512,
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
            max_memory_mb: 512,
            timeout_ms: 120_000,
            max_retries: 1,
            max_tokens: 12_000,
            io_allow_paths: vec![".".into()],
            io_deny_paths: vec!["/etc".into(), "/root".into()],
            sandbox_profile: "provider".into(),
            requires_human_approval: false,
        }
    }

    async fn attempt_artifact_repair_write(
        &self,
        contract: &mut ArtifactDeliveryContract,
        request: &str,
        loop_output: &QueryLoopOutput,
    ) -> Option<String> {
        let tool_name = if self.tools.has_tool("write_file") {
            "write_file".to_string()
        } else {
            self.tools
                .names()
                .into_iter()
                .find(|name| name.to_ascii_lowercase().contains("write_file"))?
        };
        let mut write_content = extract_explicit_write_content(request)
            .or_else(|| extract_code_fence_content(&loop_output.final_text));
        let target_ext = std::path::Path::new(&contract.target_path)
            .extension()
            .and_then(|item| item.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(target_ext.as_str(), "html" | "htm")
            && write_content
                .as_ref()
                .is_some_and(|content| !looks_like_html_document(content))
        {
            write_content = None;
        }
        if write_content.is_none() {
            write_content = self
                .generate_artifact_body(&contract.session_id, request, &contract.target_path)
                .await;
        }
        if write_content.is_none() {
            write_content = build_local_artifact_template(request, &contract.target_path);
        }
        let write_content = write_content?;
        let args = serde_json::json!({
            "path": contract.target_path.clone(),
            "content": write_content,
            "append": false
        })
        .to_string();
        let result = self
            .tools
            .execute_with_context(&tool_name, &args, None)
            .await
            .ok()?
            .content;
        let value = serde_json::from_str::<serde_json::Value>(&result).ok()?;
        let path = value
            .get("path")
            .and_then(|item| item.as_str())
            .unwrap_or(&contract.target_path)
            .to_string();
        let size_bytes = value
            .get("size_bytes")
            .and_then(|item| item.as_u64())
            .unwrap_or(0);
        let hash = value
            .get("sha256")
            .and_then(|item| item.as_str())
            .map(str::to_string);
        let computed_hash = hash.or_else(|| {
            std::fs::read(&path)
                .ok()
                .map(|bytes| compute_hash_hex(&ArtifactHashAlgorithm::Sha256, &bytes))
        });
        contract.write_proof = Some(ArtifactWriteProof {
            path: path.clone(),
            size_bytes,
            mime: infer_expected_mime_from_request(request, &path),
            hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
            hash: computed_hash.clone(),
            readable: true,
            written_at_ms: current_time_ms(),
        });
        let evidence_ref = value
            .get("evidence_ref")
            .and_then(|item| item.as_str())
            .map(str::to_string);
        let evidence_ref = if evidence_ref.is_some() {
            evidence_ref
        } else {
            EvidenceLedgerWriter::append_stage(
                    &self.state_store,
                    &contract.session_id,
                    &contract.trace_id,
                    EvidenceStage::Execution,
                    serde_json::json!({
                        "stage": "artifact_repair_write",
                        "target_path": path,
                        "size_bytes": size_bytes,
                        "hash": computed_hash,
                        "hash_algorithm": "sha256",
                    }),
                    None,
                )
                .await
                .ok()
        };
        contract.evidence_ref = evidence_ref;
        Some(result)
    }

    async fn generate_artifact_body(
        &self,
        session_id: &str,
        request: &str,
        target_path: &str,
    ) -> Option<String> {
        let extension = std::path::Path::new(target_path)
            .extension()
            .and_then(|item| item.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let format_hint = match extension.as_str() {
            "html" | "htm" => "Return only valid HTML document text with no markdown fences.",
            "json" => "Return only valid JSON text with no markdown fences.",
            _ => "Return only raw file content text with no markdown fences.",
        };
        let prompt = format!(
            "Generate file content for target path `{target_path}`.\n{format_hint}\nOriginal request:\n{request}"
        );
        let messages = vec![
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "system".to_string(),
                content: "You generate file bodies only. No explanations.".to_string(),
            },
            ChatMessage { tool_call_id: None, tool_calls: None,
                role: "user".to_string(),
                content: prompt,
            },
        ];
        let identity = self.execution_identity_for_session(session_id).await.ok()?;
        let envelope = TaskEnvelope {
            session_id: session_id.into(),
            trace_id: format!("trace:{session_id}:artifact-gen:{}", current_time_ms()).into(),
            task_id: "agent-artifact-generate".into(),
            capability_id: "provider:chat".into(),
            identity,
            payload: serde_json::to_value(&messages).ok()?,
            constraints: self.default_provider_constraints(),
            trust_plan: None,
        };
        let runtime_result = self
            .runtime
            .execute_provider(
                &self.state_store,
                &self.tools,
                &self.providers,
                &envelope,
                &messages,
                None,
            )
            .await
            .ok()?;
        let body = runtime_result.content;
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return None;
        }
        let candidate = extract_code_fence_content(trimmed)
            .unwrap_or_else(|| trimmed.to_string())
            .trim()
            .to_string();
        if looks_like_provider_echo(&candidate) {
            return None;
        }
        if matches!(extension.as_str(), "html" | "htm") && !looks_like_html_document(&candidate) {
            return None;
        }
        Some(candidate)
    }
}

fn build_local_artifact_template(request: &str, target_path: &str) -> Option<String> {
    let extension = std::path::Path::new(target_path)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension != "html" && extension != "htm" {
        return None;
    }
    let title = if request.contains("虚拟货币") || request.to_ascii_lowercase().contains("crypto") {
        "OntoLoop Crypto Exchange MVP"
    } else {
        "OntoLoop Generated Page"
    };
    Some(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width,initial-scale=1" />
  <title>{title}</title>
  <style>
    :root {{
      --bg: #070a14;
      --card: #0f1630;
      --line: #1e2b55;
      --text: #d9e8ff;
      --muted: #8aa0d6;
      --neon: #27e3ff;
      --neon2: #7b61ff;
      --ok: #2ef2a1;
      --warn: #ffb347;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      font-family: "Segoe UI", "PingFang SC", sans-serif;
      color: var(--text);
      background: radial-gradient(1200px 600px at 10% -10%, #16295f 0%, transparent 50%),
                  radial-gradient(800px 500px at 110% 20%, #2b1857 0%, transparent 55%),
                  var(--bg);
    }}
    .wrap {{ max-width: 1200px; margin: 0 auto; padding: 20px; }}
    .nav {{
      display: flex; justify-content: space-between; align-items: center;
      padding: 12px 16px; border: 1px solid var(--line); border-radius: 14px;
      background: linear-gradient(180deg, rgba(255,255,255,.04), rgba(255,255,255,.01));
      box-shadow: 0 0 20px rgba(39,227,255,.15);
    }}
    .brand {{ font-weight: 700; letter-spacing: .3px; }}
    .brand b {{ color: var(--neon); }}
    .menu {{ display: flex; gap: 14px; color: var(--muted); font-size: 14px; }}
    .grid {{ margin-top: 16px; display: grid; gap: 16px; grid-template-columns: 1.2fr .8fr; }}
    .card {{
      border: 1px solid var(--line); border-radius: 14px; padding: 14px;
      background: var(--card);
    }}
    .ticker {{ display: grid; grid-template-columns: repeat(3,1fr); gap: 10px; }}
    .pill {{ border: 1px solid #284078; border-radius: 12px; padding: 10px; }}
    .muted {{ color: var(--muted); font-size: 12px; }}
    .price {{ margin-top: 4px; font-size: 18px; font-weight: 700; }}
    .up {{ color: var(--ok); }} .down {{ color: #ff6b81; }}
    .trade {{
      display: grid; gap: 10px;
      grid-template-columns: 1fr 1fr;
    }}
    .trade input, .trade select {{
      width: 100%; border-radius: 10px; border: 1px solid #2a3f72;
      background: #0b1329; color: var(--text); padding: 10px;
    }}
    .trade .full {{ grid-column: 1 / -1; }}
    .btns {{ display: flex; gap: 10px; }}
    button {{
      border: none; border-radius: 10px; padding: 10px 12px; cursor: pointer; color: #02111d;
      font-weight: 700;
    }}
    .buy {{ background: linear-gradient(90deg, var(--ok), #88ffd3); }}
    .sell {{ background: linear-gradient(90deg, var(--warn), #ffd68a); }}
    table {{ width: 100%; border-collapse: collapse; font-size: 13px; }}
    th, td {{ border-bottom: 1px solid #213561; padding: 8px 6px; text-align: left; }}
    th {{ color: var(--muted); font-weight: 600; }}
    @media (max-width: 900px) {{
      .grid {{ grid-template-columns: 1fr; }}
      .ticker {{ grid-template-columns: 1fr; }}
      .trade {{ grid-template-columns: 1fr; }}
    }}
  </style>
</head>
<body>
  <div class="wrap">
    <div class="nav">
      <div class="brand"><b>OntoLoop</b> Exchange</div>
      <div class="menu"><span>Markets</span><span>Trade</span><span>Orders</span><span>Assets</span></div>
    </div>
    <div class="grid">
      <section class="card">
        <h3>行情卡片</h3>
        <div class="ticker">
          <div class="pill"><div class="muted">BTC/USDT</div><div class="price">$68,420 <span class="up">+1.8%</span></div></div>
          <div class="pill"><div class="muted">ETH/USDT</div><div class="price">$3,560 <span class="up">+2.1%</span></div></div>
          <div class="pill"><div class="muted">SOL/USDT</div><div class="price">$182 <span class="down">-0.7%</span></div></div>
        </div>
      </section>
      <section class="card">
        <h3>交易下单面板</h3>
        <div class="trade">
          <select><option>BTC/USDT</option><option>ETH/USDT</option></select>
          <select><option>Limit</option><option>Market</option></select>
          <input placeholder="Price" />
          <input placeholder="Amount" />
          <input class="full" placeholder="Total" />
          <div class="btns full"><button class="buy">Buy</button><button class="sell">Sell</button></div>
        </div>
      </section>
    </div>
    <section class="card" style="margin-top:16px;">
      <h3>订单列表</h3>
      <table>
        <thead><tr><th>Time</th><th>Pair</th><th>Side</th><th>Price</th><th>Amount</th><th>Status</th></tr></thead>
        <tbody>
          <tr><td>13:00:12</td><td>BTC/USDT</td><td class="up">Buy</td><td>68,300</td><td>0.15</td><td>Filled</td></tr>
          <tr><td>13:05:48</td><td>ETH/USDT</td><td class="down">Sell</td><td>3,575</td><td>1.20</td><td>Partially Filled</td></tr>
          <tr><td>13:08:01</td><td>SOL/USDT</td><td class="up">Buy</td><td>181.8</td><td>25</td><td>Open</td></tr>
        </tbody>
      </table>
    </section>
  </div>
</body>
</html>"#
    ))
}

#[derive(Debug, Default, Clone)]
struct NoopShadowQueryLoopBackend;

impl NoopShadowQueryLoopBackend {
    fn new() -> Self {
        Self
    }
}

#[async_trait]
impl QueryLoopBackend for NoopShadowQueryLoopBackend {
    async fn provider_step(
        &self,
        _session_id: &str,
        messages: &[ChatMessage],
        _hardgate_pass_token: &str,
    ) -> Result<LlmResponse> {
        let latest_user = messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.clone())
            .unwrap_or_else(|| "shadow: no user message".to_string());

        Ok(LlmResponse {
            content: Some(format!(
                "shadow-dry-run::{}",
                preview(&latest_user, 120)
            )),
            tool_calls: Vec::<ToolCall>::new(),
        })
    }

    async fn tool_step(
        &self,
        _session_id: &str,
        _call: &ToolCall,
        _hardgate_pass_token: &str,
    ) -> Result<String> {
        bail!("shadow backend does not execute tools")
    }
}

fn preview(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let clipped = input.chars().take(max_chars).collect::<String>();
    format!("{clipped}...")
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn extract_artifact_delivery_contract(
    content: &str,
    session_id: &str,
    trace_id: &str,
) -> Option<ArtifactDeliveryContract> {
    if let Ok(contract) = serde_json::from_str::<ArtifactDeliveryContract>(content) {
        return Some(contract);
    }

    for block in content.split("```") {
        let trimmed = block.trim();
        let candidate = trimmed
            .strip_prefix("json")
            .map(str::trim)
            .unwrap_or(trimmed);
        if let Ok(contract) = serde_json::from_str::<ArtifactDeliveryContract>(candidate) {
            return Some(contract);
        }
    }

    infer_artifact_delivery_contract(content, session_id, trace_id)
}

fn infer_artifact_delivery_contract(
    content: &str,
    session_id: &str,
    trace_id: &str,
) -> Option<ArtifactDeliveryContract> {
    let target_path = extract_candidate_artifact_path(content)?;
    if !looks_like_artifact_task(content) {
        return None;
    }

    let mut validation_rules = ArtifactValidationRules::default();
    validation_rules.expected_mime = infer_expected_mime_from_request(content, &target_path);
    if validation_rules.expected_mime.as_deref() == Some("text/html") {
        validation_rules.min_size_bytes = Some(64);
    }

    Some(ArtifactDeliveryContract {
        api_version: crate::contracts::artifact_delivery::ARTIFACT_DELIVERY_CONTRACT_VERSION
            .to_string(),
        session_id: session_id.to_string(),
        trace_id: trace_id.to_string(),
        requires_artifact: true,
        target_path,
        validation_rules,
        status: None,
        write_proof: None,
        reason: None,
        evidence_ref: None,
        replay_fp: None,
    })
}

fn looks_like_artifact_task(content: &str) -> bool {
    let lowered = content.to_ascii_lowercase();
    // only trigger on explicit artifact delivery markers, not generic file writes
    lowered.contains("artifact_delivery/v1")
        || lowered.contains("target_path")
        || lowered.contains("artifact_delivery")
        || (lowered.contains("```html") && lowered.contains("deploy"))
        || (lowered.contains("build") && lowered.contains("render"))
}

fn extract_candidate_artifact_path(content: &str) -> Option<String> {
    content
        .split(|ch: char| {
            ch.is_whitespace()
                || matches!(ch, ',' | ';' | '{' | '}' | '(' | ')' | '[' | ']' | '"' | '\'')
        })
        .filter_map(normalize_path_token)
        .filter_map(strip_artifact_key_prefix)
        .find(|path| is_supported_artifact_path(path))
}

fn extract_explicit_write_content(content: &str) -> Option<String> {
    let marker = "containing exactly:";
    let lowered = content.to_ascii_lowercase();
    if let Some(idx) = lowered.find(marker) {
        let tail = content[idx + marker.len()..].trim_start();
        if tail.is_empty() {
            return None;
        }

        if let Some(quote) = tail.chars().next().filter(|ch| *ch == '"' || *ch == '\'') {
            let quoted = &tail[quote.len_utf8()..];
            if let Some(end_idx) = quoted.find(quote) {
                let value = quoted[..end_idx].trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }

        let first_line = tail.lines().next().unwrap_or(tail);
        let lowered_line = first_line.to_ascii_lowercase();
        let mut cut = first_line.len();
        for stop in [
            " must use tool",
            " and return ",
            ". this ",
            ". then ",
            ". also ",
            ". return ",
            ". must ",
            ". use ",
        ] {
            if let Some(stop_idx) = lowered_line.find(stop) {
                cut = cut.min(stop_idx);
            }
        }
        let candidate = &first_line[..cut];
        let value = candidate
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .trim_end_matches('.');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

fn extract_code_fence_content(text: &str) -> Option<String> {
    let mut parts = text.split("```");
    let _ = parts.next()?;
    let block = parts.next()?.trim();
    if block.is_empty() {
        return None;
    }
    let body = if let Some((first_line, rest)) = block.split_once('\n') {
        if first_line.len() <= 16
            && first_line
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        {
            rest
        } else {
            block
        }
    } else {
        block
    };
    let value = body.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn looks_like_provider_echo(text: &str) -> bool {
    let lowered = text.trim_start().to_ascii_lowercase();
    lowered.starts_with("[provider:") || lowered.starts_with("[mock-provider:")
}

fn looks_like_html_document(text: &str) -> bool {
    let lowered = text.trim_start().to_ascii_lowercase();
    lowered.starts_with("<!doctype html")
        || lowered.starts_with("<html")
        || lowered.contains("<html")
}

fn normalize_path_token(token: &str) -> Option<String> {
    let mut started = false;
    let mut collected = String::new();
    for ch in token.chars() {
        if !started {
            if is_path_char(ch) {
                started = true;
                collected.push(ch);
            }
            continue;
        }
        if is_path_char(ch) {
            collected.push(ch);
        } else {
            break;
        }
    }

    if collected.is_empty() {
        return None;
    }
    Some(collected.replace('\\', "/"))
}

fn strip_artifact_key_prefix(token: String) -> Option<String> {
    let lowered = token.to_ascii_lowercase();
    let candidate = if let Some(index) = lowered.find("target_path:") {
        token[index + "target_path:".len()..].to_string()
    } else if let Some(index) = lowered.find("target_path=") {
        token[index + "target_path=".len()..].to_string()
    } else if let Some(index) = lowered.find("artifact_path:") {
        token[index + "artifact_path:".len()..].to_string()
    } else if let Some(index) = lowered.find("artifact_path=") {
        token[index + "artifact_path=".len()..].to_string()
    } else {
        token
    };
    let normalized = candidate.trim().trim_matches('.').trim_matches(',').trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn is_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, ':' | '/' | '\\' | '.' | '_' | '-')
}

fn is_supported_artifact_path(path: &str) -> bool {
    let lowered = path.to_ascii_lowercase();
    lowered.contains(":/")
        || lowered.starts_with('/')
        || lowered.starts_with("./")
        || lowered.starts_with("../")
}

fn infer_expected_mime_from_request(content: &str, target_path: &str) -> Option<String> {
    let extension = std::path::Path::new(target_path)
        .extension()
        .and_then(|item| item.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "html" | "htm" => Some("text/html".into()),
        "json" => Some("application/json".into()),
        "md" => Some("text/markdown".into()),
        "txt" => Some("text/plain".into()),
        "csv" => Some("text/csv".into()),
        _ => {
            let lowered = content.to_ascii_lowercase();
            if lowered.contains("html") {
                Some("text/html".into())
            } else if lowered.contains("json") {
                Some("application/json".into())
            } else {
                None
            }
        }
    }
}

fn artifact_delivery_gate_passed(
    contract: &ArtifactDeliveryContract,
    tool_events: &[crate::query_engine::r#loop::ToolExecutionEvent],
) -> (bool, String) {
    let artifact_path = std::path::Path::new(&contract.target_path);
    let file_exists = artifact_path.exists();
    if contract.validation_rules.exists_required && !file_exists {
        return (
            false,
            format!("artifact_file_missing:path={}", contract.target_path),
        );
    }
    let Some(write_proof) = contract.write_proof.as_ref() else {
        return (
            false,
            format!("artifact_write_proof_missing:path={}", contract.target_path),
        );
    };
    if write_proof
        .hash
        .as_deref()
        .map_or(true, |item| item.trim().is_empty())
    {
        return (
            false,
            format!("artifact_write_hash_missing:path={}", contract.target_path),
        );
    }
    if contract
        .evidence_ref
        .as_deref()
        .map_or(true, |item| item.trim().is_empty())
    {
        return (
            false,
            format!(
                "artifact_write_evidence_ref_missing:path={}",
                contract.target_path
            ),
        );
    }

    let write_event_found = tool_events.iter().any(|event| {
        let tool_name = event.tool_name.to_ascii_lowercase();
        let output = event.output.to_ascii_lowercase();
        let path_hit = event.output.contains(&contract.target_path);
        let write_hint = tool_name.contains("write")
            || tool_name.contains("save")
            || output.contains("write")
            || output.contains("saved")
            || output.contains("created");
        path_hit && write_hint
    });

    if !write_event_found {
        return (
            false,
            format!("artifact_write_evidence_missing:path={}", contract.target_path),
        );
    }

    match verify_post_write(contract, artifact_path) {
        Ok(hash) => (true, format!("artifact_delivery_verified:hash={hash}")),
        Err(reason) => (
            false,
            format!(
                "artifact_post_write_verification_failed:path={}:{}",
                contract.target_path, reason
            ),
        ),
    }
}

fn verify_post_write(contract: &ArtifactDeliveryContract, artifact_path: &std::path::Path) -> Result<String, String> {
    if !artifact_path.exists() {
        return Err("missing_file".into());
    }

    let metadata = std::fs::metadata(artifact_path).map_err(|error| format!("metadata:{error}"))?;
    if !metadata.is_file() {
        return Err("not_a_file".into());
    }
    let size_bytes = metadata.len();
    if let Some(min) = contract.validation_rules.min_size_bytes {
        if size_bytes < min {
            return Err(format!("size_too_small:actual={size_bytes}:min={min}"));
        }
    }
    if let Some(max) = contract.validation_rules.max_size_bytes {
        if size_bytes > max {
            return Err(format!("size_too_large:actual={size_bytes}:max={max}"));
        }
    }

    let bytes = std::fs::read(artifact_path).map_err(|error| format!("read:{error}"))?;
    let readable = !bytes.is_empty() || size_bytes == 0;
    if contract.validation_rules.readable_required && !readable {
        return Err("not_readable".into());
    }

    let inferred_mime = infer_mime_from_bytes_and_path(&bytes, artifact_path);
    if let Some(expected_mime) = contract.validation_rules.expected_mime.as_ref() {
        if !mime_matches(expected_mime, &inferred_mime) {
            return Err(format!(
                "mime_mismatch:actual={inferred_mime}:expected={expected_mime}"
            ));
        }
    }

    if let Some(proof) = contract.write_proof.as_ref() {
        if proof.path != contract.target_path {
            return Err(format!(
                "proof_path_mismatch:proof={}:target={}",
                proof.path, contract.target_path
            ));
        }
        if proof.size_bytes != size_bytes {
            return Err(format!(
                "proof_size_mismatch:proof={}:actual={}",
                proof.size_bytes, size_bytes
            ));
        }
        if let Some(proof_mime) = proof.mime.as_ref() {
            if !mime_matches(proof_mime, &inferred_mime) {
                return Err(format!(
                    "proof_mime_mismatch:proof={proof_mime}:actual={inferred_mime}"
                ));
            }
        }
        if proof.readable != readable {
            return Err(format!(
                "proof_readable_mismatch:proof={}:actual={}",
                proof.readable, readable
            ));
        }
    }

    let selected_hash_algorithm = contract
        .write_proof
        .as_ref()
        .and_then(|proof| proof.hash_algorithm.clone())
        .or_else(|| contract.validation_rules.hash_algorithm.clone())
        .unwrap_or(crate::contracts::artifact_delivery::ArtifactHashAlgorithm::Sha256);
    let computed_hash = compute_hash_hex(&selected_hash_algorithm, &bytes);

    if let Some(expected_hash) = contract
        .write_proof
        .as_ref()
        .and_then(|proof| proof.hash.clone())
    {
        if !hash_matches(&expected_hash, &computed_hash) {
            return Err(format!(
                "proof_hash_mismatch:proof={expected_hash}:actual={computed_hash}"
            ));
        }
    }

    Ok(computed_hash)
}

fn infer_mime_from_bytes_and_path(bytes: &[u8], path: &std::path::Path) -> String {
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if lower.trim_start().starts_with("<!doctype html") || lower.trim_start().starts_with("<html") {
        return "text/html".into();
    }
    if lower.trim_start().starts_with('{') || lower.trim_start().starts_with('[') {
        return "application/json".into();
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html".into(),
        "json" => "application/json".into(),
        "md" => "text/markdown".into(),
        "txt" => "text/plain".into(),
        "csv" => "text/csv".into(),
        "js" => "application/javascript".into(),
        "ts" => "text/plain".into(),
        "css" => "text/css".into(),
        _ => "application/octet-stream".into(),
    }
}

fn compute_hash_hex(
    algorithm: &crate::contracts::artifact_delivery::ArtifactHashAlgorithm,
    bytes: &[u8],
) -> String {
    use sha2::{Digest, Sha256, Sha512};
    match algorithm {
        crate::contracts::artifact_delivery::ArtifactHashAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            format!("{:x}", hasher.finalize())
        }
        crate::contracts::artifact_delivery::ArtifactHashAlgorithm::Sha512 => {
            let mut hasher = Sha512::new();
            hasher.update(bytes);
            format!("{:x}", hasher.finalize())
        }
    }
}

fn hash_matches(expected: &str, computed: &str) -> bool {
    let normalized_expected = expected.trim().trim_start_matches("sha256:").trim_start_matches("sha512:");
    normalized_expected.eq_ignore_ascii_case(computed)
}

fn mime_matches(expected: &str, actual: &str) -> bool {
    expected.trim().eq_ignore_ascii_case(actual.trim())
}

fn is_budget_exceeded_error(error_text: &str) -> bool {
    let lowered = error_text.to_ascii_lowercase();
    lowered.contains("token budget exceeded")
        || lowered.contains("provider token budget exceeded")
        || lowered.contains("context length")
}

fn build_budget_compacted_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    if messages.is_empty() {
        return Vec::new();
    }

    let system_message = messages
        .iter()
        .find(|message| message.role == "system")
        .cloned();
    let user_message = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .cloned();
    let assistant_tail = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .cloned();

    let mut compacted = Vec::new();
    if let Some(system) = system_message {
        compacted.push(system);
    }
    if let Some(user) = user_message {
        compacted.push(user);
    }
    if let Some(assistant) = assistant_tail {
        compacted.push(assistant);
    }
    compacted
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use crate::{
        contracts::artifact_delivery::{
            ArtifactHashAlgorithm, ArtifactValidationRules, ArtifactWriteProof,
        },
        query_engine::r#loop::ToolExecutionEvent,
    };

    use super::{
        artifact_delivery_gate_passed, compute_hash_hex, extract_artifact_delivery_contract,
        extract_explicit_write_content, looks_like_html_document, looks_like_provider_echo,
    };

    fn temp_file_path(label: &str) -> PathBuf {
        let unique = format!(
            "autoloop-agent-artifact-{label}-{}.html",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn artifact_contract_parser_accepts_legacy_aliases() {
        let raw = r#"{
            "api_version":"artifact-delivery/v1",
            "session_id":"s1",
            "trace_id":"t1",
            "must_write_artifact":true,
            "artifact_path":"D:/AutoLoop/output/out.html",
            "checks":{"exists_required":true,"readable_required":true}
        }"#;

        let parsed =
            extract_artifact_delivery_contract(raw, "session-test", "trace-test").expect("parse contract");
        assert!(parsed.requires_artifact);
        assert_eq!(parsed.target_path, "D:/AutoLoop/output/out.html");
    }

    #[test]
    fn artifact_contract_parser_infers_contract_from_plain_task() {
        let raw = "create a crypto trading html page and save it to D:/AutoLoop/output/crypto-mvp.html";
        let parsed =
            extract_artifact_delivery_contract(raw, "session-plain", "trace-plain").expect("infer contract");
        assert!(parsed.requires_artifact);
        assert_eq!(parsed.target_path, "D:/AutoLoop/output/crypto-mvp.html");
        assert_eq!(
            parsed.validation_rules.expected_mime.as_deref(),
            Some("text/html")
        );
    }

    #[test]
    fn artifact_contract_parser_trims_trailing_punctuation_from_path() {
        let raw = "clone repo to D:/AutoLoop/output/ruoyi-fastapi-demo，并返回结果。";
        let parsed =
            extract_artifact_delivery_contract(raw, "session-punc", "trace-punc").expect("infer contract");
        assert_eq!(parsed.target_path, "D:/AutoLoop/output/ruoyi-fastapi-demo");
    }

    #[test]
    fn artifact_contract_parser_infers_contract_from_compact_inline_contract() {
        let raw = "artifact contract {api_version:artifact_delivery/v1,requires_artifact:true,target_path:D:/AutoLoop/output/compact-inline.html,validation_rules:{exists_required:true,mime:text/html}}";
        let parsed =
            extract_artifact_delivery_contract(raw, "session-inline", "trace-inline").expect("infer contract");
        assert!(parsed.requires_artifact);
        assert_eq!(parsed.target_path, "D:/AutoLoop/output/compact-inline.html");
        assert_eq!(
            parsed.validation_rules.expected_mime.as_deref(),
            Some("text/html")
        );
    }

    #[test]
    fn explicit_content_parser_stops_before_instruction_tail() {
        let request = "Write file D:/AutoLoop/output/out.txt containing exactly: ONTOLOOP_TOOL_WRITE_OK. This requires artifact delivery via tool call and must not be text-only.";
        let parsed = extract_explicit_write_content(request).expect("extract explicit content");
        assert_eq!(parsed, "ONTOLOOP_TOOL_WRITE_OK");
    }

    #[test]
    fn explicit_content_parser_respects_quoted_payload() {
        let request = "Write file D:/AutoLoop/output/out.txt containing exactly: \"line 1. line 2.\" and return concise note.";
        let parsed = extract_explicit_write_content(request).expect("extract quoted content");
        assert_eq!(parsed, "line 1. line 2.");
    }

    #[test]
    fn artifact_gate_rejects_when_file_missing() {
        let path = temp_file_path("missing");
        let contract = crate::contracts::artifact_delivery::ArtifactDeliveryContract {
            api_version: "artifact-delivery/v1".into(),
            session_id: "s".into(),
            trace_id: "t".into(),
            requires_artifact: true,
            target_path: path.to_string_lossy().to_string(),
            validation_rules: ArtifactValidationRules::default(),
            status: None,
            write_proof: None,
            reason: None,
            evidence_ref: None,
            replay_fp: None,
        };
        let (ok, reason) = artifact_delivery_gate_passed(&contract, &[]);
        assert!(!ok);
        assert!(reason.contains("artifact_file_missing"));
    }

    #[test]
    fn artifact_gate_requires_write_evidence_even_if_file_exists() {
        let path = temp_file_path("no-evidence");
        fs::write(&path, "<html></html>").expect("write file");

        let contract = crate::contracts::artifact_delivery::ArtifactDeliveryContract {
            api_version: "artifact-delivery/v1".into(),
            session_id: "s".into(),
            trace_id: "t".into(),
            requires_artifact: true,
            target_path: path.to_string_lossy().to_string(),
            validation_rules: ArtifactValidationRules::default(),
            status: None,
            write_proof: Some(ArtifactWriteProof {
                path: path.to_string_lossy().to_string(),
                size_bytes: fs::metadata(&path).expect("metadata").len(),
                mime: Some("text/html".into()),
                hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
                hash: Some(
                    compute_hash_hex(
                        &ArtifactHashAlgorithm::Sha256,
                        &fs::read(&path).expect("read"),
                    ),
                ),
                readable: true,
                written_at_ms: 1,
            }),
            reason: None,
            evidence_ref: None,
            replay_fp: None,
        };

        let events = vec![ToolExecutionEvent {
            tool_name: "search.web".into(),
            output: "done".into(),
        }];
        let (ok, reason) = artifact_delivery_gate_passed(&contract, &events);
        assert!(!ok);
        assert!(reason.contains("artifact_write_evidence_ref_missing"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn artifact_gate_rejects_when_write_proof_missing_even_if_write_event_exists() {
        let path = temp_file_path("proof-missing");
        fs::write(&path, "<html>ok</html>").expect("write file");
        let contract = crate::contracts::artifact_delivery::ArtifactDeliveryContract {
            api_version: "artifact-delivery/v1".into(),
            session_id: "s".into(),
            trace_id: "t".into(),
            requires_artifact: true,
            target_path: path.to_string_lossy().to_string(),
            validation_rules: ArtifactValidationRules::default(),
            status: None,
            write_proof: None,
            reason: None,
            evidence_ref: Some("evidence:artifact:proof-missing".into()),
            replay_fp: None,
        };
        let events = vec![ToolExecutionEvent {
            tool_name: "file.write".into(),
            output: format!("saved {}", contract.target_path),
        }];
        let (ok, reason) = artifact_delivery_gate_passed(&contract, &events);
        assert!(!ok);
        assert!(reason.contains("artifact_write_proof_missing"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn artifact_gate_rejects_when_hash_missing() {
        let path = temp_file_path("hash-missing");
        fs::write(&path, "<html>ok</html>").expect("write file");
        let contract = crate::contracts::artifact_delivery::ArtifactDeliveryContract {
            api_version: "artifact-delivery/v1".into(),
            session_id: "s".into(),
            trace_id: "t".into(),
            requires_artifact: true,
            target_path: path.to_string_lossy().to_string(),
            validation_rules: ArtifactValidationRules::default(),
            status: None,
            write_proof: Some(ArtifactWriteProof {
                path: path.to_string_lossy().to_string(),
                size_bytes: fs::metadata(&path).expect("metadata").len(),
                mime: Some("text/html".into()),
                hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
                hash: None,
                readable: true,
                written_at_ms: 1,
            }),
            reason: None,
            evidence_ref: Some("evidence:artifact:hash-missing".into()),
            replay_fp: None,
        };
        let events = vec![ToolExecutionEvent {
            tool_name: "file.write".into(),
            output: format!("saved {}", contract.target_path),
        }];
        let (ok, reason) = artifact_delivery_gate_passed(&contract, &events);
        assert!(!ok);
        assert!(reason.contains("artifact_write_hash_missing"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn artifact_gate_rejects_fake_success_when_proof_hash_mismatch() {
        let path = temp_file_path("hash-mismatch");
        fs::write(&path, "<html><body>ok</body></html>").expect("write file");

        let contract = crate::contracts::artifact_delivery::ArtifactDeliveryContract {
            api_version: "artifact-delivery/v1".into(),
            session_id: "s".into(),
            trace_id: "t".into(),
            requires_artifact: true,
            target_path: path.to_string_lossy().to_string(),
            validation_rules: ArtifactValidationRules {
                expected_mime: Some("text/html".into()),
                ..ArtifactValidationRules::default()
            },
            status: None,
            write_proof: Some(ArtifactWriteProof {
                path: path.to_string_lossy().to_string(),
                size_bytes: fs::metadata(&path).expect("metadata").len(),
                mime: Some("text/html".into()),
                hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
                hash: Some("deadbeef".into()),
                readable: true,
                written_at_ms: 1,
            }),
            reason: None,
            evidence_ref: Some("evidence:artifact:hash-mismatch".into()),
            replay_fp: None,
        };

        let events = vec![ToolExecutionEvent {
            tool_name: "file.write".into(),
            output: format!("saved {}", contract.target_path),
        }];
        let (ok, reason) = artifact_delivery_gate_passed(&contract, &events);
        assert!(!ok);
        assert!(reason.contains("proof_hash_mismatch"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn artifact_gate_rejects_fake_success_when_mime_mismatch() {
        let path = temp_file_path("mime-mismatch");
        fs::write(&path, "{\"ok\":true}").expect("write file");
        let bytes = fs::read(&path).expect("read");

        let contract = crate::contracts::artifact_delivery::ArtifactDeliveryContract {
            api_version: "artifact-delivery/v1".into(),
            session_id: "s".into(),
            trace_id: "t".into(),
            requires_artifact: true,
            target_path: path.to_string_lossy().to_string(),
            validation_rules: ArtifactValidationRules {
                expected_mime: Some("text/html".into()),
                ..ArtifactValidationRules::default()
            },
            status: None,
            write_proof: Some(ArtifactWriteProof {
                path: path.to_string_lossy().to_string(),
                size_bytes: fs::metadata(&path).expect("metadata").len(),
                mime: Some("application/json".into()),
                hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
                hash: Some(compute_hash_hex(&ArtifactHashAlgorithm::Sha256, &bytes)),
                readable: true,
                written_at_ms: 1,
            }),
            reason: None,
            evidence_ref: Some("evidence:artifact:mime-mismatch".into()),
            replay_fp: None,
        };

        let events = vec![ToolExecutionEvent {
            tool_name: "file.write".into(),
            output: format!("created {}", contract.target_path),
        }];
        let (ok, reason) = artifact_delivery_gate_passed(&contract, &events);
        assert!(!ok);
        assert!(reason.contains("mime_mismatch"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn provider_echo_detection_works() {
        assert!(looks_like_provider_echo(
            "[provider:openai-compatible:qwen-plus-latest] generated content"
        ));
        assert!(!looks_like_provider_echo("<!doctype html><html><body>ok</body></html>"));
    }

    #[test]
    fn html_document_detection_works() {
        assert!(looks_like_html_document(
            "<!doctype html><html><head></head><body></body></html>"
        ));
        assert!(!looks_like_html_document(
            "[provider:openai-compatible:qwen-plus-latest] prompt"
        ));
    }
}














