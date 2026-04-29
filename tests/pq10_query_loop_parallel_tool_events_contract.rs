use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use autoloop::providers::{ChatMessage, LlmResponse, ToolCall};
use autoloop::query_engine::{
    QueryLoopBackend, QueryLoopConfig, QueryLoopEngine, QueryLoopInput, TokenBudgetFrame,
    turn_state::QueryTurnState,
};

#[derive(Clone, Default)]
struct ParallelBackend {
    provider_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl QueryLoopBackend for ParallelBackend {
    async fn provider_step(
        &self,
        _session_id: &str,
        _messages: &[ChatMessage],
        _hardgate_pass_token: &str,
    ) -> Result<LlmResponse> {
        let turn = self.provider_calls.fetch_add(1, Ordering::SeqCst);
        if turn == 0 {
            Ok(LlmResponse {
                content: Some("dispatch parallel tools".to_string()),
                tool_calls: vec![
                    ToolCall {
                        id: "call-a".to_string(),
                        name: "mcp::local::fast-a".to_string(),
                        arguments: r#"{"sleep_ms":160}"#.to_string(),
                    },
                    ToolCall {
                        id: "call-b".to_string(),
                        name: "mcp::local::fast-b".to_string(),
                        arguments: r#"{"sleep_ms":160}"#.to_string(),
                    },
                ],
            })
        } else {
            Ok(LlmResponse {
                content: Some("final answer".to_string()),
                tool_calls: Vec::new(),
            })
        }
    }

    async fn tool_step(
        &self,
        _session_id: &str,
        call: &ToolCall,
        _hardgate_pass_token: &str,
    ) -> Result<String> {
        let sleep_ms = if call.arguments.contains("160") { 160 } else { 50 };
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        Ok(format!("ok:{}", call.name))
    }
}

#[tokio::test]
async fn query_loop_parallel_tool_calls_emit_stream_contract_events() {
    let backend = ParallelBackend::default();
    let engine = QueryLoopEngine::new(QueryLoopConfig {
        max_iterations: 4,
        provider_retry_limit: 0,
        tool_retry_limit: 0,
        preserve_recent_messages: 4,
    });

    let input = QueryLoopInput {
        session_id: "pq10-parallel".to_string(),
        trace_id: "trace:pq10-parallel".to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: "parallel test".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "run two tools".to_string(),
            },
        ],
        token_budget_frame: Some(TokenBudgetFrame::bounded(8192, 1024)),
    };

    let started = tokio::time::Instant::now();
    let output = engine.run(&backend, input).await.expect("query loop ok");
    let elapsed = started.elapsed();

    assert_eq!(output.state, QueryTurnState::Completed);
    assert_eq!(output.tool_call_count, 2);
    assert_eq!(output.tool_events.len(), 2);

    let started_events = output
        .stream_events
        .iter()
        .filter(|evt| evt.event_type == "tool_started")
        .count();
    let completed_events = output
        .stream_events
        .iter()
        .filter(|evt| evt.event_type == "tool_completed")
        .count();
    assert_eq!(started_events, 2);
    assert_eq!(completed_events, 2);

    let mut last_seq = 0_u64;
    for evt in &output.stream_events {
        assert!(evt.sequence > last_seq, "event sequence must be strictly increasing");
        assert!(
            evt.call_id == "call-a" || evt.call_id == "call-b",
            "event call_id must come from provider tool-calls"
        );
        last_seq = evt.sequence;
    }

    assert!(
        elapsed < Duration::from_millis(300),
        "tool-call dispatch should be parallel (elapsed={:?})",
        elapsed
    );
}

#[derive(Clone, Default)]
struct ReadWriteVerifyBackend {
    provider_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl QueryLoopBackend for ReadWriteVerifyBackend {
    async fn provider_step(
        &self,
        _session_id: &str,
        _messages: &[ChatMessage],
        _hardgate_pass_token: &str,
    ) -> Result<LlmResponse> {
        let turn = self.provider_calls.fetch_add(1, Ordering::SeqCst);
        match turn {
            // 并行阶段：read + write
            0 => Ok(LlmResponse {
                content: Some("dispatch read/write".to_string()),
                tool_calls: vec![
                    ToolCall {
                        id: "call-read".to_string(),
                        name: "tool::read_file".to_string(),
                        arguments: r#"{"path":"README.md","sleep_ms":120}"#.to_string(),
                    },
                    ToolCall {
                        id: "call-write".to_string(),
                        name: "tool::write_file".to_string(),
                        arguments: r#"{"path":"output.txt","content":"ok","sleep_ms":120}"#
                            .to_string(),
                    },
                ],
            }),
            // 串行阶段：verify（第二轮 provider 返回，保证在 read/write 完成后触发）
            1 => Ok(LlmResponse {
                content: Some("dispatch verify".to_string()),
                tool_calls: vec![ToolCall {
                    id: "call-verify".to_string(),
                    name: "tool::verify_artifact".to_string(),
                    arguments: r#"{"path":"output.txt","sleep_ms":40}"#.to_string(),
                }],
            }),
            _ => Ok(LlmResponse {
                content: Some("read/write/verify complete".to_string()),
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
        let sleep_ms = if call.arguments.contains("\"sleep_ms\":120") {
            120
        } else {
            40
        };
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        Ok(format!("ok:{}", call.name))
    }
}

#[tokio::test]
async fn query_loop_mixed_parallel_and_serial_tool_calls_keep_complete_event_chain() {
    let backend = ReadWriteVerifyBackend::default();
    let engine = QueryLoopEngine::new(QueryLoopConfig {
        max_iterations: 5,
        provider_retry_limit: 0,
        tool_retry_limit: 0,
        preserve_recent_messages: 4,
    });

    let input = QueryLoopInput {
        session_id: "pq10-rw-verify".to_string(),
        trace_id: "trace:pq10-rw-verify".to_string(),
        messages: vec![
            ChatMessage {
                role: "system".to_string(),
                content: "mixed tool execution stack test".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "read write and verify".to_string(),
            },
        ],
        token_budget_frame: Some(TokenBudgetFrame::bounded(8192, 1024)),
    };

    let output = engine.run(&backend, input).await.expect("query loop ok");
    assert_eq!(output.state, QueryTurnState::Completed);
    assert_eq!(output.tool_call_count, 3);
    assert_eq!(output.tool_events.len(), 3);

    let started_events = output
        .stream_events
        .iter()
        .filter(|evt| evt.event_type == "tool_started")
        .count();
    let completed_events = output
        .stream_events
        .iter()
        .filter(|evt| evt.event_type == "tool_completed")
        .count();
    assert_eq!(started_events, 3);
    assert_eq!(completed_events, 3);

    // 每个 call_id 都必须存在 started + completed
    for call_id in ["call-read", "call-write", "call-verify"] {
        let has_started = output
            .stream_events
            .iter()
            .any(|evt| evt.call_id == call_id && evt.event_type == "tool_started");
        let has_completed = output
            .stream_events
            .iter()
            .any(|evt| evt.call_id == call_id && evt.event_type == "tool_completed");
        assert!(has_started, "missing tool_started for {call_id}");
        assert!(has_completed, "missing tool_completed for {call_id}");
    }

    // 全序列严格递增
    let mut last_seq = 0_u64;
    for evt in &output.stream_events {
        assert!(
            evt.sequence > last_seq,
            "event sequence must be strictly increasing"
        );
        last_seq = evt.sequence;
    }

    // verify 必须发生在 read/write 完成之后（串行阶段）
    let rw_completed_max_seq = output
        .stream_events
        .iter()
        .filter(|evt| {
            evt.event_type == "tool_completed"
                && (evt.call_id == "call-read" || evt.call_id == "call-write")
        })
        .map(|evt| evt.sequence)
        .max()
        .expect("rw completed seq");
    let verify_started_seq = output
        .stream_events
        .iter()
        .find(|evt| evt.event_type == "tool_started" && evt.call_id == "call-verify")
        .map(|evt| evt.sequence)
        .expect("verify started seq");
    assert!(
        verify_started_seq > rw_completed_max_seq,
        "verify must start after read/write completed"
    );
}



