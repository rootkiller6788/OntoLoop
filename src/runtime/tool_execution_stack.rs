use anyhow::Result;
use autoloop_state_adapter::StateStore;
use tokio::time::{Duration, sleep, timeout};

use crate::contracts::signal::SignalContext;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolDegradeStrategy {
    Off,
    BestEffortFallback,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolReliabilityPolicy {
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub retry_backoff_ms: u64,
    pub circuit_failure_threshold: u8,
    pub degrade_strategy: ToolDegradeStrategy,
}

impl Default for ToolReliabilityPolicy {
    fn default() -> Self {
        Self {
            timeout_ms: 90_000,
            max_retries: 1,
            retry_backoff_ms: 120,
            circuit_failure_threshold: 3,
            degrade_strategy: ToolDegradeStrategy::Off,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolReliabilityReport {
    pub policy: ToolReliabilityPolicy,
    pub attempts: u8,
    pub retries_used: u8,
    pub timed_out_attempts: u8,
    pub circuit_trip_suggested: bool,
    pub degraded: bool,
    pub final_status: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolExecutionObservation {
    pub layer: String,
    pub status: String,
    pub detail: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolExecutionPipelineResult {
    pub content: String,
    pub observations: Vec<ToolExecutionObservation>,
    pub reliability: ToolReliabilityReport,
    pub persisted_ref: String,
}

pub async fn run_layered_tool_execution<F, Fut>(
    db: &StateStore,
    context: &SignalContext,
    tool_name: &str,
    policy: ToolReliabilityPolicy,
    mut executor: F,
) -> Result<ToolExecutionPipelineResult>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let mut observations = Vec::new();
    observations.push(ToolExecutionObservation {
        layer: "registry".into(),
        status: "ok".into(),
        detail: format!("tool selected: {tool_name}"),
        at_ms: current_time_ms(),
    });
    observations.push(ToolExecutionObservation {
        layer: "orchestrator".into(),
        status: "ok".into(),
        detail: "execution pipeline started".into(),
        at_ms: current_time_ms(),
    });
    observations.push(ToolExecutionObservation {
        layer: "reliability".into(),
        status: "ok".into(),
        detail: format!(
            "policy timeout_ms={} max_retries={} backoff_ms={} circuit_threshold={} degrade={:?}",
            policy.timeout_ms,
            policy.max_retries,
            policy.retry_backoff_ms,
            policy.circuit_failure_threshold,
            policy.degrade_strategy
        ),
        at_ms: current_time_ms(),
    });

    let mut attempts = 0_u8;
    let mut retries_used = 0_u8;
    let mut timed_out_attempts = 0_u8;
    let mut failed_attempts = 0_u8;
    let mut last_error: Option<String> = None;
    let mut content: Option<String> = None;

    while attempts <= policy.max_retries {
        attempts = attempts.saturating_add(1);
        observations.push(ToolExecutionObservation {
            layer: "reliability".into(),
            status: "running".into(),
            detail: format!("attempt {attempts} started"),
            at_ms: current_time_ms(),
        });

        let run_result = if policy.timeout_ms == 0 {
            executor().await.map_err(|error| anyhow::anyhow!(error))
        } else {
            match timeout(Duration::from_millis(policy.timeout_ms), executor()).await {
                Ok(inner) => inner.map_err(|error| anyhow::anyhow!(error)),
                Err(_) => {
                    timed_out_attempts = timed_out_attempts.saturating_add(1);
                    Err(anyhow::anyhow!(
                        "tool execution timeout after {}ms",
                        policy.timeout_ms
                    ))
                }
            }
        };

        match run_result {
            Ok(output) => {
                content = Some(output);
                observations.push(ToolExecutionObservation {
                    layer: "executor".into(),
                    status: "ok".into(),
                    detail: format!("tool execution succeeded on attempt {attempts}"),
                    at_ms: current_time_ms(),
                });
                break;
            }
            Err(error) => {
                failed_attempts = failed_attempts.saturating_add(1);
                let error_text = error.to_string();
                let retryable =
                    is_retryable_error(&error_text) && attempts.saturating_sub(1) < policy.max_retries;
                observations.push(ToolExecutionObservation {
                    layer: "executor".into(),
                    status: "failed".into(),
                    detail: format!("attempt {attempts} failed: {error_text}"),
                    at_ms: current_time_ms(),
                });
                last_error = Some(error_text);
                if retryable {
                    retries_used = retries_used.saturating_add(1);
                    let backoff_ms = policy.retry_backoff_ms.saturating_mul(attempts as u64).max(1);
                    observations.push(ToolExecutionObservation {
                        layer: "reliability".into(),
                        status: "retry".into(),
                        detail: format!("retry scheduled after {backoff_ms}ms"),
                        at_ms: current_time_ms(),
                    });
                    sleep(Duration::from_millis(backoff_ms)).await;
                    continue;
                }
                break;
            }
        }
    }

    let circuit_trip_suggested = failed_attempts >= policy.circuit_failure_threshold.max(1);
    if circuit_trip_suggested {
        observations.push(ToolExecutionObservation {
            layer: "reliability".into(),
            status: "circuit".into(),
            detail: "failure threshold reached; circuit trip suggested".into(),
            at_ms: current_time_ms(),
        });
    }

    let (content, degraded, final_status) = match content {
        Some(value) => (value, false, "ok".to_string()),
        None => match policy.degrade_strategy {
            ToolDegradeStrategy::BestEffortFallback => {
                let reason = last_error
                    .clone()
                    .unwrap_or_else(|| "unknown tool failure".to_string());
                observations.push(ToolExecutionObservation {
                    layer: "reliability".into(),
                    status: "degraded".into(),
                    detail: format!("best-effort fallback activated: {reason}"),
                    at_ms: current_time_ms(),
                });
                (
                    format!(
                        "degraded-tool-fallback: tool={} reason=\"{}\"",
                        tool_name, reason
                    ),
                    true,
                    "degraded".to_string(),
                )
            }
            ToolDegradeStrategy::Off => {
                observations.push(ToolExecutionObservation {
                    layer: "executor".into(),
                    status: "failed".into(),
                    detail: last_error
                        .clone()
                        .unwrap_or_else(|| "tool execution failed".to_string()),
                    at_ms: current_time_ms(),
                });
                return Err(anyhow::anyhow!(
                    "{}",
                    last_error.unwrap_or_else(|| "tool execution failed".to_string())
                ));
            }
        },
    };

    let reliability = ToolReliabilityReport {
        policy: policy.clone(),
        attempts,
        retries_used,
        timed_out_attempts,
        circuit_trip_suggested,
        degraded,
        final_status,
        last_error: last_error.clone(),
    };
    observations.push(ToolExecutionObservation {
        layer: "reliability".into(),
        status: "ok".into(),
        detail: serde_json::json!({
            "attempts": reliability.attempts,
            "retries_used": reliability.retries_used,
            "timed_out_attempts": reliability.timed_out_attempts,
            "circuit_trip_suggested": reliability.circuit_trip_suggested,
            "degraded": reliability.degraded,
            "final_status": reliability.final_status,
        })
        .to_string(),
        at_ms: current_time_ms(),
    });
    observations.push(ToolExecutionObservation {
        layer: "observation".into(),
        status: "ok".into(),
        detail: "execution observations collected".into(),
        at_ms: current_time_ms(),
    });
    observations.push(ToolExecutionObservation {
        layer: "commit".into(),
        status: "ok".into(),
        detail: "side-effect commit prepared".into(),
        at_ms: current_time_ms(),
    });

    let persisted_ref = format!(
        "execution-stack:{}:{}:{}",
        context.session_id,
        context.task_id.as_deref().unwrap_or("none"),
        current_time_ms()
    );
    db.upsert_json_knowledge(
        persisted_ref.clone(),
        &serde_json::json!({
            "session_id": context.session_id,
            "trace_id": context.trace_id,
            "span_id": context.span_id,
            "task_id": context.task_id,
            "capability_id": context.capability_id,
            "tool_name": tool_name,
            "reliability": reliability,
            "observations": observations,
        }),
        "runtime:tool-execution-stack",
    )
    .await?;

    Ok(ToolExecutionPipelineResult {
        content,
        observations,
        reliability,
        persisted_ref,
    })
}

fn is_retryable_error(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("timeout")
        || lowered.contains("temporar")
        || lowered.contains("unavailable")
        || lowered.contains("connection")
        || lowered.contains("reset")
        || lowered.contains("refused")
        || lowered.contains("429")
        || lowered.contains("rate limit")
        || lowered.contains("throttle")
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    fn memory_store() -> StateStore {
        StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        })
    }

    fn signal_context() -> SignalContext {
        SignalContext {
            session_id: "session-tool-stack".into(),
            trace_id: "trace-tool-stack".into(),
            span_id: Some("span-tool-stack".into()),
            task_id: Some("task-tool-stack".into()),
            capability_id: Some("tool:test".into()),
            tenant_id: Some("tenant".into()),
            principal_id: Some("principal".into()),
        }
    }

    #[tokio::test]
    async fn retries_then_succeeds_with_unified_policy() {
        let db = memory_store();
        let ctx = signal_context();
        let mut attempts = 0_u8;
        let policy = ToolReliabilityPolicy {
            timeout_ms: 5_000,
            max_retries: 2,
            retry_backoff_ms: 1,
            circuit_failure_threshold: 3,
            degrade_strategy: ToolDegradeStrategy::Off,
        };

        let result = run_layered_tool_execution(&db, &ctx, "tool::retry-demo", policy, || {
            attempts = attempts.saturating_add(1);
            async move {
                if attempts < 2 {
                    Err(anyhow::anyhow!("temporary unavailable"))
                } else {
                    Ok("ok".to_string())
                }
            }
        })
        .await
        .expect("execution succeeds");

        assert_eq!(result.content, "ok");
        assert_eq!(result.reliability.attempts, 2);
        assert_eq!(result.reliability.retries_used, 1);
        assert_eq!(result.reliability.final_status, "ok");
    }

    #[tokio::test]
    async fn timeout_failure_can_degrade_when_policy_allows() {
        let db = memory_store();
        let ctx = signal_context();
        let policy = ToolReliabilityPolicy {
            timeout_ms: 10,
            max_retries: 0,
            retry_backoff_ms: 1,
            circuit_failure_threshold: 1,
            degrade_strategy: ToolDegradeStrategy::BestEffortFallback,
        };

        let result = run_layered_tool_execution(&db, &ctx, "tool::timeout-demo", policy, || async {
            sleep(Duration::from_millis(40)).await;
            Ok("late".to_string())
        })
        .await
        .expect("degraded fallback should succeed");

        assert!(result.content.contains("degraded-tool-fallback"));
        assert!(result.reliability.degraded);
        assert_eq!(result.reliability.final_status, "degraded");
        assert_eq!(result.reliability.timed_out_attempts, 1);
    }
}
