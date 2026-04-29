use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use tokio::sync::Mutex;

use autoloop::providers::{ChatMessage, LlmResponse, ToolCall};
use autoloop::query_engine::{
    QueryLoopBackend, QueryLoopConfig, QueryLoopEngine, QueryLoopInput, TokenBudgetFrame,
    build_replay_fingerprint,
};

#[derive(Clone, Default)]
struct CompactBackend {
    provider_steps: Arc<Mutex<VecDeque<Result<LlmResponse, String>>>>,
}

impl CompactBackend {
    fn with_provider_steps(steps: Vec<Result<LlmResponse, String>>) -> Self {
        Self {
            provider_steps: Arc::new(Mutex::new(steps.into_iter().collect())),
        }
    }
}

#[async_trait]
impl QueryLoopBackend for CompactBackend {
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
        _call: &ToolCall,
        _hardgate_pass_token: &str,
    ) -> Result<String> {
        Ok("noop".into())
    }
}

#[tokio::test]
async fn pq2_compaction_continue_replay_consistency() {
    let backend = CompactBackend::with_provider_steps(vec![Ok(LlmResponse {
        content: Some("final after compaction".into()),
        tool_calls: Vec::new(),
    })]);

    let engine = QueryLoopEngine::new(QueryLoopConfig {
        max_iterations: 2,
        provider_retry_limit: 0,
        tool_retry_limit: 0,
        preserve_recent_messages: 2,
    });

    let long_messages = vec![
        ChatMessage {
            role: "system".into(),
            content: "You are an assistant following strict budgeted context policies".into(),
        },
        ChatMessage {
            role: "user".into(),
            content: "Alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau".into(),
        },
        ChatMessage {
            role: "assistant".into(),
            content: "I acknowledged and produced a long intermediate reasoning artifact for audit and continuity.".into(),
        },
        ChatMessage {
            role: "user".into(),
            content: "Continue with updated constraints and provide compact-ready state transition notes.".into(),
        },
    ];

    let output = engine
        .run(
            &backend,
            QueryLoopInput {
                session_id: "session-pq2".into(),
                trace_id: "trace-pq2".into(),
                token_budget_frame: Some(TokenBudgetFrame {
                    max_input_tokens: 80,
                    max_output_tokens: 32,
                    reserve_tokens: 8,
                    compaction_trigger_tokens: 20,
                }),
                messages: long_messages,
            },
        )
        .await
        .expect("query loop should succeed with compaction");

    assert!(
        output.compaction_boundary.is_some(),
        "expected compaction boundary"
    );
    let boundary = output.compaction_boundary.as_ref().expect("boundary");
    assert!(!boundary.boundary_id.is_empty());
    assert!(boundary.compressed_messages > 0);

    let continuation = output.continuation.as_ref().expect("continuation protocol");
    assert_eq!(
        continuation.boundary_id.as_deref(),
        Some(boundary.boundary_id.as_str())
    );
    assert!(
        continuation
            .checkpoint
            .compacted_summary
            .as_ref()
            .is_some_and(|summary| !summary.is_empty())
    );

    let replay_fp = build_replay_fingerprint(&output.messages, &output.final_text);
    assert_eq!(replay_fp, output.replay_fingerprint);
    assert_eq!(replay_fp, continuation.replay_fingerprint);
    assert!(continuation.is_replay_consistent(&output.messages, &output.final_text));
}



