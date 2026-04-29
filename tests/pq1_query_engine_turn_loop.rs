use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::Mutex;

use autoloop::providers::{ChatMessage, LlmResponse, ToolCall};
use autoloop::query_engine::turn_state::QueryTurnState;
use autoloop::query_engine::{QueryLoopBackend, QueryLoopConfig, QueryLoopEngine, QueryLoopInput};

#[derive(Clone, Default)]
struct ScriptedBackend {
    provider_steps: Arc<Mutex<VecDeque<Result<LlmResponse, String>>>>,
    tool_steps: Arc<Mutex<HashMap<String, VecDeque<Result<String, String>>>>>,
}

impl ScriptedBackend {
    fn with_provider_steps(steps: Vec<Result<LlmResponse, String>>) -> Self {
        Self {
            provider_steps: Arc::new(Mutex::new(steps.into_iter().collect())),
            tool_steps: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn add_tool_steps(&self, tool_name: &str, steps: Vec<Result<String, String>>) {
        self.tool_steps
            .lock()
            .await
            .insert(tool_name.to_string(), steps.into_iter().collect());
    }
}

#[async_trait]
impl QueryLoopBackend for ScriptedBackend {
    async fn provider_step(
        &self,
        _session_id: &str,
        _messages: &[ChatMessage],
        _hardgate_pass_token: &str,
    ) -> Result<LlmResponse> {
        let mut guard = self.provider_steps.lock().await;
        match guard.pop_front() {
            Some(Ok(response)) => Ok(response),
            Some(Err(error)) => Err(anyhow!(error)),
            None => Ok(LlmResponse {
                content: Some("fallback".into()),
                tool_calls: Vec::new(),
            }),
        }
    }

    async fn tool_step(
        &self,
        _session_id: &str,
        call: &ToolCall,
        _hardgate_pass_token: &str,
    ) -> Result<String> {
        let mut guard = self.tool_steps.lock().await;
        if let Some(queue) = guard.get_mut(&call.name) {
            match queue.pop_front() {
                Some(Ok(output)) => Ok(output),
                Some(Err(error)) => Err(anyhow!(error)),
                None => Ok("tool-fallback".into()),
            }
        } else {
            Ok("tool-default".into())
        }
    }
}

#[tokio::test]
async fn pq1_tool_call_loop_and_turn_transition() {
    let backend = ScriptedBackend::with_provider_steps(vec![
        Ok(LlmResponse {
            content: Some("need tool".into()),
            tool_calls: vec![ToolCall {
                id: "call-1".into(),
                name: "tool.calc".into(),
                arguments: "{\"expr\":\"6*7\"}".into(),
            }],
        }),
        Ok(LlmResponse {
            content: Some("final answer".into()),
            tool_calls: Vec::new(),
        }),
    ]);
    backend
        .add_tool_steps("tool.calc", vec![Ok("42".into())])
        .await;

    let engine = QueryLoopEngine::new(QueryLoopConfig {
        max_iterations: 4,
        provider_retry_limit: 0,
        tool_retry_limit: 0,
        preserve_recent_messages: 4,
    });

    let output = engine
        .run(
            &backend,
            QueryLoopInput {
                session_id: "session-pq1".into(),
                trace_id: "trace-pq1".into(),
                token_budget_frame: None,
                messages: vec![ChatMessage {
                    role: "user".into(),
                    content: "solve".into(),
                }],
            },
        )
        .await
        .expect("loop should succeed");

    assert_eq!(output.final_text, "final answer");
    assert_eq!(output.tool_call_count, 1);
    assert_eq!(output.iteration_count, 2);
    assert_eq!(output.state, QueryTurnState::Completed);
    assert!(
        output
            .transitions
            .iter()
            .any(|t| t.to == QueryTurnState::ToolDispatch)
    );
    assert!(
        output
            .transitions
            .iter()
            .any(|t| t.to == QueryTurnState::Completed)
    );
}

#[tokio::test]
async fn pq1_retry_and_recovery_path() {
    let backend = ScriptedBackend::with_provider_steps(vec![
        Err("transient provider error".into()),
        Ok(LlmResponse {
            content: Some("retry recovered".into()),
            tool_calls: Vec::new(),
        }),
    ]);

    let engine = QueryLoopEngine::new(QueryLoopConfig {
        max_iterations: 3,
        provider_retry_limit: 1,
        tool_retry_limit: 0,
        preserve_recent_messages: 4,
    });

    let output = engine
        .run(
            &backend,
            QueryLoopInput {
                session_id: "session-pq1-retry".into(),
                trace_id: "trace-pq1-retry".into(),
                token_budget_frame: None,
                messages: vec![ChatMessage {
                    role: "user".into(),
                    content: "retry me".into(),
                }],
            },
        )
        .await
        .expect("retry path should succeed");

    assert_eq!(output.final_text, "retry recovered");
    assert_eq!(output.provider_retry_count, 1);
    assert_eq!(output.state, QueryTurnState::Completed);
}

#[tokio::test]
async fn pq1_max_iteration_turn_transition() {
    let backend = ScriptedBackend::with_provider_steps(vec![Ok(LlmResponse {
        content: Some("still working".into()),
        tool_calls: vec![ToolCall {
            id: "call-max".into(),
            name: "tool.loop".into(),
            arguments: "{}".into(),
        }],
    })]);
    backend
        .add_tool_steps("tool.loop", vec![Ok("tool result".into())])
        .await;

    let engine = QueryLoopEngine::new(QueryLoopConfig {
        max_iterations: 1,
        provider_retry_limit: 0,
        tool_retry_limit: 0,
        preserve_recent_messages: 4,
    });

    let output = engine
        .run(
            &backend,
            QueryLoopInput {
                session_id: "session-pq1-max".into(),
                trace_id: "trace-pq1-max".into(),
                token_budget_frame: None,
                messages: vec![ChatMessage {
                    role: "user".into(),
                    content: "loop".into(),
                }],
            },
        )
        .await
        .expect("engine should stop safely at max iterations");

    assert_eq!(
        output.final_text,
        "Agent stopped after reaching the max iteration limit."
    );
    assert_eq!(output.state, QueryTurnState::MaxIterationsReached);
    assert!(
        output
            .transitions
            .iter()
            .any(|t| t.to == QueryTurnState::MaxIterationsReached)
    );
}



