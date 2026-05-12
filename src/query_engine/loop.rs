use anyhow::{Result, bail};
use async_trait::async_trait;
use futures::future::join_all;

use crate::providers::{ChatMessage, LlmResponse, ToolCall};

use super::compactor::CompactionBoundary;
use super::context_compiler::{ContextCompiler, TokenBudgetFrame};
use super::continuation::{
    ContinuationCheckpoint, ContinuationProtocol, build_continuation_protocol,
    build_replay_fingerprint,
};
use super::turn_state::{QueryTurnState, TurnTransition};
use crate::runtime::decision_protocol::ExecutionGuardObservation;

#[derive(Debug, Clone)]
pub struct QueryLoopConfig {
    pub max_iterations: usize,
    pub provider_retry_limit: u8,
    pub tool_retry_limit: u8,
    pub preserve_recent_messages: usize,
}

impl Default for QueryLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 8,
            provider_retry_limit: 1,
            tool_retry_limit: 1,
            preserve_recent_messages: 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueryLoopInput {
    pub session_id: String,
    pub trace_id: String,
    pub messages: Vec<ChatMessage>,
    pub token_budget_frame: Option<TokenBudgetFrame>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolExecutionEvent {
    pub tool_name: String,
    pub output: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QueryLoopStreamEvent {
    pub event_type: String,
    pub call_id: String,
    pub sequence: u64,
    pub tool_name: String,
    pub payload: serde_json::Value,
    pub emitted_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginExecutionTrace {
    pub plugin_id: String,
    pub plugin_kind: String,
    pub phase: String,
    pub verdict: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct QueryLoopOutput {
    pub final_text: String,
    pub messages: Vec<ChatMessage>,
    pub transitions: Vec<TurnTransition>,
    pub iteration_count: usize,
    pub tool_call_count: usize,
    pub provider_retry_count: usize,
    pub tool_retry_count: usize,
    pub state: QueryTurnState,
    pub tool_events: Vec<ToolExecutionEvent>,
    pub stream_events: Vec<QueryLoopStreamEvent>,
    pub token_budget_frame: TokenBudgetFrame,
    pub estimated_input_tokens: u32,
    pub compaction_boundary: Option<CompactionBoundary>,
    pub continuation: Option<ContinuationProtocol>,
    pub replay_fingerprint: String,
    pub hardgate_pass_token: String,
    pub constraint_version: String,
    pub hard_constraint_ids: Vec<String>,
    pub proof: super::default_plugins::ProofResult,
    pub plugin_execution_traces: Vec<PluginExecutionTrace>,
    pub guard_observations: Vec<ExecutionGuardObservation>,
}

#[async_trait]
pub trait QueryLoopBackend: Send + Sync {
    async fn provider_step(
        &self,
        session_id: &str,
        messages: &[ChatMessage],
        hardgate_pass_token: &str,
    ) -> Result<LlmResponse>;
    async fn tool_step(
        &self,
        session_id: &str,
        call: &ToolCall,
        hardgate_pass_token: &str,
    ) -> Result<String>;

    fn guard_observations(&self) -> Vec<ExecutionGuardObservation> {
        Vec::new()
    }
}

#[derive(Debug, Clone)]
pub struct QueryLoopEngine {
    config: QueryLoopConfig,
}

impl QueryLoopEngine {
    pub fn new(config: QueryLoopConfig) -> Self {
        Self { config }
    }

    pub async fn run<B: QueryLoopBackend>(
        &self,
        backend: &B,
        input: QueryLoopInput,
    ) -> Result<QueryLoopOutput> {
        if self.config.max_iterations == 0 {
            bail!("query loop max_iterations must be > 0");
        }

        let budget = input.token_budget_frame.unwrap_or_default();
        let compiler = ContextCompiler::new(budget.clone(), self.config.preserve_recent_messages);
        let compiler_plugins = compiler.plugin_manifests();
        let compiled = compiler.compile(&input.messages)?;
        let plugin_execution_traces = compiler_plugins
            .into_iter()
            .map(|manifest| {
                let phase = match manifest.kind {
                    crate::contracts::plugin::PluginKind::ContextConstraint => "hardgate",
                    crate::contracts::plugin::PluginKind::SoftEstimator => "soft_estimate",
                    crate::contracts::plugin::PluginKind::Optimizer => "optimize",
                    crate::contracts::plugin::PluginKind::Repair => "repair",
                    crate::contracts::plugin::PluginKind::Proof => "proof",
                    _ => "other",
                }
                .to_string();
                PluginExecutionTrace {
                    plugin_id: manifest.id,
                    plugin_kind: format!("{:?}", manifest.kind).to_ascii_lowercase(),
                    phase,
                    verdict: "pass".into(),
                    reason: "compiled".into(),
                }
            })
            .collect::<Vec<_>>();

        let mut state = QueryTurnState::Initialized;
        let mut transitions = vec![
            TurnTransition::new(
                QueryTurnState::Initialized,
                QueryTurnState::Initialized,
                "turn initialized",
                0,
            )
            .with_meta(
                "estimated_input_tokens",
                compiled.estimated_tokens.to_string(),
            )
            .with_meta(
                "compaction_applied",
                compiled.compaction_applied.to_string(),
            )
            .with_meta("hardgate_pass_token", compiled.hardgate_pass_token.clone())
            .with_meta("constraint_version", compiled.constraint_version.clone())
            .with_meta("constraint_ids", compiled.hard_constraint_ids.join(",")),
        ];
        if let Some(boundary) = &compiled.boundary {
            transitions.push(
                TurnTransition::new(
                    QueryTurnState::Initialized,
                    QueryTurnState::Initialized,
                    "context compaction boundary applied",
                    0,
                )
                .with_meta("boundary_id", boundary.boundary_id.clone())
                .with_meta(
                    "compressed_messages",
                    boundary.compressed_messages.to_string(),
                )
                .with_meta(
                    "preserved_messages",
                    boundary.preserved_messages.to_string(),
                ),
            );
        }

        let mut messages = compiled.messages;
        let mut provider_retry_count = 0usize;
        let mut tool_retry_count = 0usize;
        let mut tool_call_count = 0usize;
        let mut tool_events = Vec::new();
        let mut stream_events = Vec::new();
        let mut event_sequence: u64 = 0;

        for iteration in 1..=self.config.max_iterations {
            transitions.push(TurnTransition::new(
                state,
                QueryTurnState::ProviderCall,
                "invoke provider",
                iteration,
            ));
            state = QueryTurnState::ProviderCall;

            let (response, provider_retries) = self
                .provider_with_retry(
                    backend,
                    &input.session_id,
                    &messages,
                    &compiled.hardgate_pass_token,
                )
                .await?;
            provider_retry_count += provider_retries;

            if response.tool_calls.is_empty() {
                let final_text = response
                    .content
                    .unwrap_or_else(|| "No response content.".to_string());
                transitions.push(TurnTransition::new(
                    state,
                    QueryTurnState::Completed,
                    "provider produced final text",
                    iteration,
                ));
                state = QueryTurnState::Completed;

                let checkpoint = ContinuationCheckpoint::from_turn(
                    input.session_id.clone().into(),
                    input.trace_id.clone().into(),
                    format!("turn-{iteration}"),
                    messages.len(),
                    compiled.boundary.as_ref().map(|item| item.summary.clone()),
                );
                let continuation = Some(build_continuation_protocol(
                    checkpoint,
                    compiled.boundary.as_ref(),
                    &messages,
                    &final_text,
                ));
                let replay_fingerprint = continuation
                    .as_ref()
                    .map(|item| item.replay_fingerprint.clone())
                    .unwrap_or_else(|| build_replay_fingerprint(&messages, &final_text));

                return Ok(QueryLoopOutput {
                    final_text,
                    messages,
                    transitions,
                    iteration_count: iteration,
                    tool_call_count,
                    provider_retry_count,
                    tool_retry_count,
                    state,
                    tool_events,
                    stream_events,
                    token_budget_frame: budget,
                    estimated_input_tokens: compiled.estimated_tokens,
                    compaction_boundary: compiled.boundary,
                    continuation,
                    replay_fingerprint,
                    hardgate_pass_token: compiled.hardgate_pass_token,
                    constraint_version: compiled.constraint_version,
                    hard_constraint_ids: compiled.hard_constraint_ids,
                    proof: compiled.proof,
                    plugin_execution_traces,
                    guard_observations: backend.guard_observations(),
                });
            }

            if let Some(content) = response.content {
                let tc = if response.tool_calls.is_empty() { None } else { Some(response.tool_calls.clone()) };
                messages.push(ChatMessage { tool_call_id: None, tool_calls: tc,
                    role: "assistant".into(),
                    content,
                });
            }

            transitions.push(TurnTransition::new(
                state,
                QueryTurnState::ToolDispatch,
                "dispatch tool calls",
                iteration,
            ));
            state = QueryTurnState::ToolDispatch;

            let calls = response.tool_calls;
            tool_call_count += calls.len();
            let (results, retries_used, next_sequence, mut iteration_events) = self
                .dispatch_tools_concurrently(
                    backend,
                    &input.session_id,
                    &calls,
                    &compiled.hardgate_pass_token,
                    event_sequence,
                )
                .await?;
            tool_retry_count += retries_used;
            event_sequence = next_sequence;
            stream_events.append(&mut iteration_events);

            for (call, tool_output) in results {
                tool_events.push(ToolExecutionEvent {
                    tool_name: call.name.clone(),
                    output: tool_output.clone(),
                });
                messages.push(ChatMessage { tool_call_id: Some(call.id.clone()), tool_calls: None,
                    role: "tool".into(),
                    content: tool_output,
                });
            }
        }

        transitions.push(TurnTransition::new(
            state,
            QueryTurnState::MaxIterationsReached,
            "max iteration reached",
            self.config.max_iterations,
        ));
        state = QueryTurnState::MaxIterationsReached;

        let final_text = "Agent stopped after reaching the max iteration limit.".to_string();
        let checkpoint = ContinuationCheckpoint::from_turn(
            input.session_id.clone().into(),
            input.trace_id.clone().into(),
            format!("turn-{}", self.config.max_iterations),
            messages.len(),
            compiled.boundary.as_ref().map(|item| item.summary.clone()),
        );
        let continuation = Some(build_continuation_protocol(
            checkpoint,
            compiled.boundary.as_ref(),
            &messages,
            &final_text,
        ));
        let replay_fingerprint = continuation
            .as_ref()
            .map(|item| item.replay_fingerprint.clone())
            .unwrap_or_else(|| build_replay_fingerprint(&messages, &final_text));

        Ok(QueryLoopOutput {
            final_text,
            messages,
            transitions,
            iteration_count: self.config.max_iterations,
            tool_call_count,
            provider_retry_count,
            tool_retry_count,
            state,
            tool_events,
            stream_events,
            token_budget_frame: budget,
            estimated_input_tokens: compiled.estimated_tokens,
            compaction_boundary: compiled.boundary,
            continuation,
            replay_fingerprint,
            hardgate_pass_token: compiled.hardgate_pass_token,
            constraint_version: compiled.constraint_version,
            hard_constraint_ids: compiled.hard_constraint_ids,
            proof: compiled.proof,
            plugin_execution_traces,
            guard_observations: backend.guard_observations(),
        })
    }

    async fn dispatch_tools_concurrently<B: QueryLoopBackend>(
        &self,
        backend: &B,
        session_id: &str,
        calls: &[ToolCall],
        hardgate_pass_token: &str,
        start_sequence: u64,
    ) -> Result<(
        Vec<(ToolCall, String)>,
        usize,
        u64,
        Vec<QueryLoopStreamEvent>,
    )> {
        let mut sequence = start_sequence;
        let mut stream_events = Vec::new();
        for call in calls {
            sequence += 1;
            stream_events.push(QueryLoopStreamEvent {
                event_type: "tool_started".to_string(),
                call_id: call.id.clone(),
                sequence,
                tool_name: call.name.clone(),
                payload: serde_json::json!({
                    "arguments": call.arguments,
                }),
                emitted_at_ms: current_time_ms(),
            });
        }

        let futs = calls
            .iter()
            .enumerate()
            .map(|(index, call)| async move {
                let result = self
                    .tool_with_retry(backend, session_id, call, hardgate_pass_token)
                    .await;
                (index, call.clone(), result)
            })
            .collect::<Vec<_>>();

        let settled = join_all(futs).await;
        let mut results = Vec::new();
        let mut total_retries = 0usize;

        for (_index, call, result) in settled {
            match result {
                Ok((output, retries)) => {
                    total_retries += retries;
                    sequence += 1;
                    stream_events.push(QueryLoopStreamEvent {
                        event_type: "tool_completed".to_string(),
                        call_id: call.id.clone(),
                        sequence,
                        tool_name: call.name.clone(),
                        payload: serde_json::json!({
                            "output": output,
                            "is_error": false,
                        }),
                        emitted_at_ms: current_time_ms(),
                    });
                    results.push((call, output));
                }
                Err(error) => {
                    sequence += 1;
                    stream_events.push(QueryLoopStreamEvent {
                        event_type: "tool_completed".to_string(),
                        call_id: call.id.clone(),
                        sequence,
                        tool_name: call.name,
                        payload: serde_json::json!({
                            "error": error.to_string(),
                            "is_error": true,
                        }),
                        emitted_at_ms: current_time_ms(),
                    });
                    return Err(error);
                }
            }
        }

        Ok((results, total_retries, sequence, stream_events))
    }

    async fn provider_with_retry<B: QueryLoopBackend>(
        &self,
        backend: &B,
        session_id: &str,
        messages: &[ChatMessage],
        hardgate_pass_token: &str,
    ) -> Result<(LlmResponse, usize)> {
        let mut retries_used = 0usize;
        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 0..=self.config.provider_retry_limit {
            match backend
                .provider_step(session_id, messages, hardgate_pass_token)
                .await
            {
                Ok(response) => {
                    return Ok((response, retries_used));
                }
                Err(error) => {
                    if attempt >= self.config.provider_retry_limit {
                        last_error = Some(error);
                        break;
                    }
                    retries_used += 1;
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("provider step failed")))
    }

    async fn tool_with_retry<B: QueryLoopBackend>(
        &self,
        backend: &B,
        session_id: &str,
        call: &ToolCall,
        hardgate_pass_token: &str,
    ) -> Result<(String, usize)> {
        let mut retries_used = 0usize;
        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 0..=self.config.tool_retry_limit {
            match backend
                .tool_step(session_id, call, hardgate_pass_token)
                .await
            {
                Ok(output) => {
                    return Ok((output, retries_used));
                }
                Err(error) => {
                    if attempt >= self.config.tool_retry_limit {
                        last_error = Some(error);
                        break;
                    }
                    retries_used += 1;
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("tool step failed")))
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
