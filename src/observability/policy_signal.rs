use std::collections::BTreeMap;

use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};

use crate::{
    observability::collector::TelemetryCollectorSnapshot,
    orchestration::governance_telemetry_scope::GovernanceTelemetryScope,
    orchestration::{ExecutionReport, current_time_ms},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySignalAggregate {
    pub session_id: String,
    pub generated_at_ms: u64,
    pub latency_p95_ms: u64,
    pub retries_total: u64,
    pub verifier_fail_rate: f32,
    pub breaker_hits: u64,
    pub delegation_loops: u64,
    pub suggested_quota_factor: f32,
    pub suggested_trust_decay_delta: f32,
    pub suggested_approval_threshold: String,
    pub runtime_mode_hint: String,
    pub summary: BTreeMap<String, String>,
}

pub async fn aggregate_and_persist(
    db: &StateStore,
    session_id: &str,
    scope: &GovernanceTelemetryScope,
    verifier_score: f32,
    reports: &[ExecutionReport],
) -> Result<PolicySignalAggregate> {
    let collector = db
        .get_knowledge(&format!("observability:{session_id}:collector:snapshot"))
        .await?
        .and_then(|record| serde_json::from_str::<TelemetryCollectorSnapshot>(&record.value).ok())
        .unwrap_or(TelemetryCollectorSnapshot {
            governance_scope: scope.clone(),
            session_id: session_id.to_string(),
            generated_at_ms: current_time_ms(),
            exec_spans: Vec::new(),
            latency_samples: Vec::new(),
            retries: Vec::new(),
            guard_hits: Vec::new(),
            breaker_trips: Vec::new(),
            token_costs: Vec::new(),
            delegation_traces: Vec::new(),
            unified_trace: Vec::new(),
            branch_collapse: Vec::new(),
            regressions: Vec::new(),
            contradictions: Vec::new(),
            replay_fingerprints: Vec::new(),
            replay_diff_locators: Vec::new(),
            summary: BTreeMap::new(),
        });

    let latency_p95_ms = percentile95(
        collector
            .latency_samples
            .iter()
            .map(|item| item.latency_ms)
            .collect::<Vec<_>>(),
    );
    let retries_total = collector
        .retries
        .iter()
        .map(|item| item.retries as u64)
        .sum::<u64>();
    let verifier_fail_rate = (1.0 - verifier_score.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let breaker_hits = collector.breaker_trips.len() as u64;

    let mut delegation_by_task = std::collections::HashMap::<String, u64>::new();
    for trace in &collector.delegation_traces {
        if trace.delegated {
            *delegation_by_task.entry(trace.task_id.clone()).or_insert(0) += 1;
        }
    }
    let delegation_loops = delegation_by_task
        .values()
        .filter(|count| **count > 1)
        .count() as u64;

    let stress_score = (latency_p95_ms as f32 / 2_000.0).min(2.0)
        + (retries_total as f32 / 5.0).min(2.0)
        + (breaker_hits as f32 / 3.0).min(2.0);
    let suggested_quota_factor = if stress_score >= 3.0 {
        0.85
    } else if stress_score >= 2.0 {
        0.95
    } else {
        1.05
    };
    let suggested_trust_decay_delta = if verifier_fail_rate > 0.35 || delegation_loops > 0 {
        0.12
    } else {
        0.03
    };
    let suggested_approval_threshold = if scope.risk_tier == "high" || verifier_fail_rate > 0.4 {
        "strict".to_string()
    } else if verifier_fail_rate > 0.2 {
        "moderate".to_string()
    } else {
        "relaxed".to_string()
    };
    let runtime_mode_hint = if breaker_hits >= 2 || verifier_fail_rate > 0.45 {
        "degraded".to_string()
    } else if reports.is_empty() {
        "shadow".to_string()
    } else {
        "safe".to_string()
    };

    let mut summary = BTreeMap::new();
    summary.insert("risk_tier".into(), scope.risk_tier.clone());
    summary.insert("privacy_level".into(), scope.privacy_level.clone());
    summary.insert(
        "telemetry_retention_hours".into(),
        scope.retention_hours.to_string(),
    );

    let aggregate = PolicySignalAggregate {
        session_id: session_id.to_string(),
        generated_at_ms: current_time_ms(),
        latency_p95_ms,
        retries_total,
        verifier_fail_rate,
        breaker_hits,
        delegation_loops,
        suggested_quota_factor,
        suggested_trust_decay_delta,
        suggested_approval_threshold,
        runtime_mode_hint,
        summary,
    };

    db.upsert_json_knowledge(
        format!("policy-signals:{session_id}:latest"),
        &aggregate,
        "policy-signal-aggregator",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("policy-signals:{session_id}:{}", aggregate.generated_at_ms),
        &aggregate,
        "policy-signal-aggregator",
    )
    .await?;

    db.upsert_json_knowledge(
        format!("policy:{session_id}:adaptive-update"),
        &serde_json::json!({
            "quota_tuning_factor": aggregate.suggested_quota_factor,
            "risk_policy_update": aggregate.suggested_approval_threshold,
            "source": "policy-signal-aggregator",
        }),
        "policy-engine-feedback",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("capability-admission:{session_id}:feedback"),
        &serde_json::json!({
            "trust_decay_delta": aggregate.suggested_trust_decay_delta,
            "approval_threshold": aggregate.suggested_approval_threshold,
            "source": "policy-signal-aggregator",
        }),
        "trusted-capability-admission-feedback",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("runtime-mode:{session_id}:hint"),
        &serde_json::json!({
            "mode_hint": aggregate.runtime_mode_hint,
            "source": "policy-signal-aggregator",
        }),
        "runtime-mode-feedback",
    )
    .await?;

    Ok(aggregate)
}

fn percentile95(mut values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    let index = ((values.len() as f32) * 0.95).ceil() as usize;
    values[index.saturating_sub(1).min(values.len() - 1)]
}


