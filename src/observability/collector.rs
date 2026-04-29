use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};

use crate::{
    contracts::version::CONTRACT_VERSION,
    observability::event_stream::{EventEnvelope, append_event, list_session_events},
    orchestration::ExecutionReport,
    orchestration::governance_telemetry_scope::GovernanceTelemetryScope,
    runtime::execution_fabric::ExecutionFabricReplaySequence,
    runtime::{CircuitPhase, CircuitState},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecSpanRecord {
    pub span_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: Option<String>,
    pub status: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub latency_ms: u64,
    pub pool: Option<String>,
    pub route_variant: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencySample {
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub stage_from: String,
    pub stage_to: String,
    pub from_at_ms: u64,
    pub to_at_ms: u64,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryRecord {
    pub session_id: String,
    pub task_id: String,
    pub trace_id: String,
    pub attempts: u32,
    pub retries: u32,
    pub final_status: String,
    pub last_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardHitRecord {
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: Option<String>,
    pub decision: String,
    pub reason: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakerTripRecord {
    pub session_id: String,
    pub scope_key: String,
    pub phase: String,
    pub failure_count: u32,
    pub threshold: u32,
    pub cooldown_ms: u64,
    pub last_reason: Option<String>,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenCostRecord {
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: Option<String>,
    pub estimated_prompt_tokens: Option<u32>,
    pub token_cost_micros: u64,
    pub tool_cost_micros: u64,
    pub duration_cost_micros: u64,
    pub total_cost_micros: u64,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationTraceRecord {
    pub session_id: String,
    pub task_id: String,
    pub peer: String,
    pub delegated: bool,
    pub source: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedTraceRecord {
    pub event_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub branch_id: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchCollapseRecord {
    pub session_id: String,
    pub branch_id: String,
    pub task_id: String,
    pub candidate_trace_ids: Vec<String>,
    pub selected_trace_id: String,
    pub selected_status: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionCaptureRecord {
    pub session_id: String,
    pub task_id: String,
    pub previous_trace_id: String,
    pub current_trace_id: String,
    pub from_status: String,
    pub to_status: String,
    pub detected_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionCaptureRecord {
    pub session_id: String,
    pub task_id: String,
    pub capability_id: Option<String>,
    pub left_decision: String,
    pub right_decision: String,
    pub left_trace_id: String,
    pub right_trace_id: String,
    pub detected_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayFingerprintRecord {
    pub session_id: String,
    pub task_id: String,
    pub trace_id: String,
    pub deterministic_replay_fingerprint: String,
    pub event_count: usize,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayDiffLocatorRecord {
    pub session_id: String,
    pub task_id: String,
    pub baseline_trace_id: String,
    pub candidate_trace_id: String,
    pub baseline_fingerprint: String,
    pub candidate_fingerprint: String,
    pub mismatch_fields: Vec<String>,
    pub located: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryCollectorSnapshot {
    pub governance_scope: GovernanceTelemetryScope,
    pub session_id: String,
    pub generated_at_ms: u64,
    pub exec_spans: Vec<ExecSpanRecord>,
    pub latency_samples: Vec<LatencySample>,
    pub retries: Vec<RetryRecord>,
    pub guard_hits: Vec<GuardHitRecord>,
    pub breaker_trips: Vec<BreakerTripRecord>,
    pub token_costs: Vec<TokenCostRecord>,
    pub delegation_traces: Vec<DelegationTraceRecord>,
    #[serde(default)]
    pub unified_trace: Vec<UnifiedTraceRecord>,
    #[serde(default)]
    pub branch_collapse: Vec<BranchCollapseRecord>,
    #[serde(default)]
    pub regressions: Vec<RegressionCaptureRecord>,
    #[serde(default)]
    pub contradictions: Vec<ContradictionCaptureRecord>,
    #[serde(default)]
    pub replay_fingerprints: Vec<ReplayFingerprintRecord>,
    #[serde(default)]
    pub replay_diff_locators: Vec<ReplayDiffLocatorRecord>,
    pub summary: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
struct SpanAccumulator {
    trace_id: String,
    task_id: String,
    capability_id: Option<String>,
    status: String,
    start_ms: u64,
    end_ms: u64,
    pool: Option<String>,
    route_variant: Option<String>,
}

#[derive(Debug, Clone)]
struct RetryAccumulator {
    task_id: String,
    trace_id: String,
    attempts: u32,
    final_status: String,
    last_reason: String,
}

pub async fn collect_and_persist(
    db: &StateStore,
    session_id: &str,
    governance_scope: &GovernanceTelemetryScope,
    reports: &[ExecutionReport],
) -> Result<TelemetryCollectorSnapshot> {
    let events = list_session_events(db, session_id).await?;
    let unified_trace = collect_unified_trace(session_id, &events);
    let exec_spans = collect_exec_spans(session_id, &events, reports);
    let latency_samples = collect_latency_samples(db, session_id).await?;
    let retries = collect_retries(session_id, &events);
    let guard_hits = collect_guard_hits(session_id, &events, reports);
    let breaker_trips = collect_breaker_trips(db, session_id).await?;
    let token_costs = collect_token_costs(session_id, &events);
    let delegation_traces = collect_delegation_traces(db, session_id, reports).await?;
    let branch_collapse = collect_branch_collapse(session_id, &exec_spans);
    let regressions = collect_regressions(session_id, &exec_spans);
    let contradictions = collect_contradictions(session_id, &guard_hits);
    let replay_fingerprints = collect_replay_fingerprints(session_id, &events);
    let replay_diff_locators =
        collect_replay_diff_locators(session_id, &events, &replay_fingerprints);

    let mut summary = BTreeMap::new();
    summary.insert("exec_spans".into(), exec_spans.len());
    summary.insert("latency_samples".into(), latency_samples.len());
    summary.insert("retries".into(), retries.len());
    summary.insert("guard_hits".into(), guard_hits.len());
    summary.insert("breaker_trips".into(), breaker_trips.len());
    summary.insert("token_costs".into(), token_costs.len());
    summary.insert("delegation_traces".into(), delegation_traces.len());
    summary.insert("unified_trace".into(), unified_trace.len());
    summary.insert("branch_collapse".into(), branch_collapse.len());
    summary.insert("regressions".into(), regressions.len());
    summary.insert("contradictions".into(), contradictions.len());
    summary.insert("replay_fingerprints".into(), replay_fingerprints.len());
    summary.insert("replay_diff_locators".into(), replay_diff_locators.len());

    let snapshot = TelemetryCollectorSnapshot {
        governance_scope: governance_scope.clone(),
        session_id: session_id.to_string(),
        generated_at_ms: current_time_ms(),
        exec_spans,
        latency_samples,
        retries,
        guard_hits,
        breaker_trips,
        token_costs,
        delegation_traces,
        unified_trace,
        branch_collapse,
        regressions,
        contradictions,
        replay_fingerprints,
        replay_diff_locators,
        summary,
    };

    persist_snapshot(db, &snapshot).await?;
    let _ = append_event(
        db,
        "telemetry.collector.updated",
        format!("trace:{session_id}:collector:{}", snapshot.generated_at_ms),
        session_id.to_string(),
        None,
        Some("observability:collector".into()),
        CONTRACT_VERSION,
        serde_json::json!({
            "summary": snapshot.summary,
            "governance_scope": snapshot.governance_scope,
            "replay_diff_located": snapshot.replay_diff_locators.iter().all(|item| item.located),
        }),
    )
    .await;

    Ok(snapshot)
}

fn collect_exec_spans(
    session_id: &str,
    events: &[EventEnvelope],
    reports: &[ExecutionReport],
) -> Vec<ExecSpanRecord> {
    let mut report_by_task = HashMap::new();
    for report in reports {
        report_by_task.insert(report.task.task_id.clone(), report);
    }

    let mut grouped = HashMap::<String, SpanAccumulator>::new();
    for event in events {
        let task_id = event
            .task_id
            .clone()
            .unwrap_or_else(|| "task:unknown".into());
        let key = format!("{}::{}", event.trace_id, task_id);
        let event_status = status_from_event(event);

        let entry = grouped.entry(key).or_insert_with(|| SpanAccumulator {
            trace_id: event.trace_id.clone(),
            task_id: task_id.clone(),
            capability_id: event.capability_id.clone(),
            status: event_status.to_string(),
            start_ms: event.created_at_ms,
            end_ms: event.created_at_ms,
            pool: report_by_task
                .get(&task_id)
                .map(|report| infer_pool_from_role(&report.task.role)),
            route_variant: report_by_task
                .get(&task_id)
                .map(|report| report.route_variant.clone()),
        });

        if event.created_at_ms < entry.start_ms {
            entry.start_ms = event.created_at_ms;
        }
        if event.created_at_ms > entry.end_ms {
            entry.end_ms = event.created_at_ms;
        }
        if entry.capability_id.is_none() {
            entry.capability_id = event.capability_id.clone();
        }
        entry.status = merge_status(&entry.status, event_status).to_string();
    }

    let mut spans = grouped
        .into_iter()
        .map(|(_, value)| ExecSpanRecord {
            span_id: format!("span:{}:{}:{}", session_id, value.trace_id, value.task_id),
            session_id: session_id.to_string(),
            trace_id: value.trace_id,
            task_id: value.task_id,
            capability_id: value.capability_id,
            status: value.status,
            start_ms: value.start_ms,
            end_ms: value.end_ms,
            latency_ms: value.end_ms.saturating_sub(value.start_ms),
            pool: value.pool,
            route_variant: value.route_variant,
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| span.start_ms);
    spans
}

fn collect_unified_trace(session_id: &str, events: &[EventEnvelope]) -> Vec<UnifiedTraceRecord> {
    let mut records = events
        .iter()
        .map(|event| {
            let task_id = event
                .task_id
                .clone()
                .unwrap_or_else(|| "task:unknown".into());
            UnifiedTraceRecord {
                event_id: event.event_id.clone(),
                session_id: session_id.to_string(),
                trace_id: event.trace_id.clone(),
                task_id: task_id.clone(),
                capability_id: event.capability_id.clone(),
                kind: event.kind.clone(),
                status: status_from_event(event).to_string(),
                branch_id: format!("branch:{session_id}:{task_id}"),
                created_at_ms: event.created_at_ms,
            }
        })
        .collect::<Vec<_>>();
    records.sort_by_key(|item| item.created_at_ms);
    records
}

fn collect_branch_collapse(
    session_id: &str,
    exec_spans: &[ExecSpanRecord],
) -> Vec<BranchCollapseRecord> {
    let mut by_branch = HashMap::<String, Vec<&ExecSpanRecord>>::new();
    for span in exec_spans {
        let branch_id = format!("branch:{session_id}:{}", span.task_id);
        by_branch.entry(branch_id).or_default().push(span);
    }

    let mut records = Vec::new();
    for (branch_id, mut spans) in by_branch {
        spans.sort_by_key(|span| span.start_ms);
        let task_id = spans
            .first()
            .map(|span| span.task_id.clone())
            .unwrap_or_else(|| "task:unknown".into());
        let candidate_trace_ids = spans
            .iter()
            .map(|span| span.trace_id.clone())
            .collect::<Vec<_>>();
        let selected = spans
            .iter()
            .max_by(|left, right| {
                let left_rank = status_rank(&left.status);
                let right_rank = status_rank(&right.status);
                left_rank
                    .cmp(&right_rank)
                    .then(left.end_ms.cmp(&right.end_ms))
            })
            .copied();

        if let Some(selected) = selected {
            let start_ms = spans.first().map(|span| span.start_ms).unwrap_or(selected.start_ms);
            let end_ms = spans.last().map(|span| span.end_ms).unwrap_or(selected.end_ms);
            records.push(BranchCollapseRecord {
                session_id: session_id.to_string(),
                branch_id,
                task_id,
                candidate_trace_ids,
                selected_trace_id: selected.trace_id.clone(),
                selected_status: selected.status.clone(),
                start_ms,
                end_ms,
            });
        }
    }

    records.sort_by_key(|item| item.start_ms);
    records
}

fn collect_regressions(session_id: &str, exec_spans: &[ExecSpanRecord]) -> Vec<RegressionCaptureRecord> {
    let mut by_task = HashMap::<String, Vec<&ExecSpanRecord>>::new();
    for span in exec_spans {
        by_task.entry(span.task_id.clone()).or_default().push(span);
    }

    let mut records = Vec::new();
    for (task_id, mut spans) in by_task {
        spans.sort_by_key(|span| span.end_ms);
        for window in spans.windows(2) {
            if let [left, right] = window {
                if status_rank(&right.status) < status_rank(&left.status) {
                    records.push(RegressionCaptureRecord {
                        session_id: session_id.to_string(),
                        task_id: task_id.clone(),
                        previous_trace_id: left.trace_id.clone(),
                        current_trace_id: right.trace_id.clone(),
                        from_status: left.status.clone(),
                        to_status: right.status.clone(),
                        detected_at_ms: right.end_ms,
                    });
                }
            }
        }
    }

    records.sort_by_key(|item| item.detected_at_ms);
    records
}

fn collect_contradictions(
    session_id: &str,
    guard_hits: &[GuardHitRecord],
) -> Vec<ContradictionCaptureRecord> {
    let mut by_key = HashMap::<String, Vec<&GuardHitRecord>>::new();
    for hit in guard_hits {
        let key = format!(
            "{}::{}",
            hit.task_id,
            hit.capability_id
                .clone()
                .unwrap_or_else(|| "capability:unknown".into())
        );
        by_key.entry(key).or_default().push(hit);
    }

    let mut contradictions = Vec::new();
    for hits in by_key.values_mut() {
        hits.sort_by_key(|hit| hit.created_at_ms);
        for window in hits.windows(2) {
            if let [left, right] = window {
                let left_normalized = normalize_decision(&left.decision);
                let right_normalized = normalize_decision(&right.decision);
                if left_normalized != right_normalized {
                    contradictions.push(ContradictionCaptureRecord {
                        session_id: session_id.to_string(),
                        task_id: right.task_id.clone(),
                        capability_id: right.capability_id.clone(),
                        left_decision: left.decision.clone(),
                        right_decision: right.decision.clone(),
                        left_trace_id: left.trace_id.clone(),
                        right_trace_id: right.trace_id.clone(),
                        detected_at_ms: right.created_at_ms,
                    });
                }
            }
        }
    }

    contradictions.sort_by_key(|item| item.detected_at_ms);
    contradictions
}
async fn collect_latency_samples(db: &StateStore, session_id: &str) -> Result<Vec<LatencySample>> {
    let mut samples = Vec::new();
    let records = db
        .list_knowledge_by_prefix(&format!("execution-fabric-sequence:{session_id}:"))
        .await?;

    for record in records {
        if let Ok(sequence) = serde_json::from_str::<ExecutionFabricReplaySequence>(&record.value) {
            let mut steps = sequence.steps;
            steps.sort_by_key(|step| step.sequence);
            for window in steps.windows(2) {
                if let [from, to] = window {
                    samples.push(LatencySample {
                        session_id: sequence.session_id.clone(),
                        trace_id: sequence.trace_id.clone(),
                        task_id: sequence.task_id.clone(),
                        stage_from: from.stage.clone(),
                        stage_to: to.stage.clone(),
                        from_at_ms: from.at_ms,
                        to_at_ms: to.at_ms,
                        latency_ms: to.at_ms.saturating_sub(from.at_ms),
                    });
                }
            }
        }
    }

    samples.sort_by_key(|item| item.from_at_ms);
    Ok(samples)
}

fn collect_retries(session_id: &str, events: &[EventEnvelope]) -> Vec<RetryRecord> {
    let mut by_task = HashMap::<String, RetryAccumulator>::new();

    for event in events
        .iter()
        .filter(|event| event.kind == "task_runs" || event.kind == "runtime_blocks")
    {
        let task_id = event
            .task_id
            .clone()
            .unwrap_or_else(|| "task:unknown".into());
        let entry = by_task
            .entry(task_id.clone())
            .or_insert_with(|| RetryAccumulator {
                task_id: task_id.clone(),
                trace_id: event.trace_id.clone(),
                attempts: 0,
                final_status: "pending".into(),
                last_reason: String::new(),
            });
        entry.attempts = entry.attempts.saturating_add(1);
        entry.trace_id = event.trace_id.clone();
        entry.final_status = status_from_event(event).to_string();
        entry.last_reason =
            payload_string(&event.payload, "reason").unwrap_or_else(|| event.kind.clone());
    }

    let mut retries = by_task
        .into_values()
        .filter(|item| item.attempts > 1)
        .map(|item| RetryRecord {
            session_id: session_id.to_string(),
            task_id: item.task_id,
            trace_id: item.trace_id,
            attempts: item.attempts,
            retries: item.attempts.saturating_sub(1),
            final_status: item.final_status,
            last_reason: item.last_reason,
        })
        .collect::<Vec<_>>();
    retries.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    retries
}

fn collect_guard_hits(
    session_id: &str,
    events: &[EventEnvelope],
    reports: &[ExecutionReport],
) -> Vec<GuardHitRecord> {
    let mut hits = events
        .iter()
        .filter(|event| event.kind == "task_runs" || event.kind == "runtime_blocks")
        .map(|event| GuardHitRecord {
            session_id: session_id.to_string(),
            trace_id: event.trace_id.clone(),
            task_id: event
                .task_id
                .clone()
                .unwrap_or_else(|| "task:unknown".into()),
            capability_id: event.capability_id.clone(),
            decision: payload_string(&event.payload, "decision")
                .unwrap_or_else(|| status_from_event(event).to_string()),
            reason: payload_string(&event.payload, "reason").unwrap_or_else(|| event.kind.clone()),
            created_at_ms: event.created_at_ms,
        })
        .collect::<Vec<_>>();

    for report in reports {
        let task_id = report.task.task_id.clone();
        let exists = hits.iter().any(|hit| hit.task_id == task_id);
        if !exists {
            hits.push(GuardHitRecord {
                session_id: session_id.to_string(),
                trace_id: format!("trace:{session_id}:{task_id}"),
                task_id,
                capability_id: report.tool_used.clone(),
                decision: report.guard_decision.clone(),
                reason: "execution_report_fallback".into(),
                created_at_ms: current_time_ms(),
            });
        }
    }

    hits.sort_by_key(|item| item.created_at_ms);
    hits
}

async fn collect_breaker_trips(
    db: &StateStore,
    session_id: &str,
) -> Result<Vec<BreakerTripRecord>> {
    let mut records = Vec::new();
    for record in db.list_knowledge_by_prefix("metrics:circuit:").await? {
        if let Ok(state) = serde_json::from_str::<CircuitState>(&record.value) {
            let is_trip = !matches!(state.phase, CircuitPhase::Closed)
                || state.failure_count >= state.threshold.max(1);
            if is_trip {
                records.push(BreakerTripRecord {
                    session_id: session_id.to_string(),
                    scope_key: state.scope_key.clone(),
                    phase: format!("{:?}", state.phase).to_ascii_lowercase(),
                    failure_count: state.failure_count,
                    threshold: state.threshold,
                    cooldown_ms: state.cooldown_ms,
                    last_reason: state.last_reason.clone(),
                    observed_at_ms: state
                        .last_failure_ms
                        .or(state.opened_at_ms)
                        .or(state.last_success_ms)
                        .unwrap_or_else(current_time_ms),
                });
            }
        }
    }
    records.sort_by_key(|item| item.observed_at_ms);
    Ok(records)
}

fn collect_token_costs(session_id: &str, events: &[EventEnvelope]) -> Vec<TokenCostRecord> {
    let mut records = events
        .iter()
        .filter(|event| event.kind == "task_runs" || event.kind == "runtime_blocks")
        .filter_map(|event| {
            let payload = &event.payload;
            let cost = payload.get("cost_breakdown")?;
            Some(TokenCostRecord {
                session_id: session_id.to_string(),
                trace_id: event.trace_id.clone(),
                task_id: event
                    .task_id
                    .clone()
                    .unwrap_or_else(|| "task:unknown".into()),
                capability_id: event.capability_id.clone(),
                estimated_prompt_tokens: payload
                    .get("estimated_prompt_tokens")
                    .and_then(as_u64)
                    .map(|v| v as u32),
                token_cost_micros: cost.get("token_cost_micros").and_then(as_u64).unwrap_or(0),
                tool_cost_micros: cost.get("tool_cost_micros").and_then(as_u64).unwrap_or(0),
                duration_cost_micros: cost
                    .get("duration_cost_micros")
                    .and_then(as_u64)
                    .unwrap_or(0),
                total_cost_micros: cost.get("total_cost_micros").and_then(as_u64).unwrap_or(0),
                created_at_ms: event.created_at_ms,
            })
        })
        .collect::<Vec<_>>();
    records.sort_by_key(|item| item.created_at_ms);
    records
}

fn collect_replay_fingerprints(
    session_id: &str,
    events: &[EventEnvelope],
) -> Vec<ReplayFingerprintRecord> {
    let mut by_trace = HashMap::<String, Vec<&EventEnvelope>>::new();
    for event in events {
        by_trace.entry(event.trace_id.clone()).or_default().push(event);
    }

    let mut records = Vec::new();
    for (trace_id, mut trace_events) in by_trace {
        trace_events.sort_by_key(|event| event.created_at_ms);
        let task_id = trace_events
            .first()
            .and_then(|event| event.task_id.clone())
            .unwrap_or_else(|| "task:unknown".into());
        let fingerprint = deterministic_trace_fingerprint(&trace_events);
        let generated_at_ms = trace_events
            .last()
            .map(|event| event.created_at_ms)
            .unwrap_or_else(current_time_ms);
        records.push(ReplayFingerprintRecord {
            session_id: session_id.to_string(),
            task_id,
            trace_id,
            deterministic_replay_fingerprint: fingerprint,
            event_count: trace_events.len(),
            generated_at_ms,
        });
    }
    records.sort_by_key(|item| item.generated_at_ms);
    records
}

fn collect_replay_diff_locators(
    session_id: &str,
    events: &[EventEnvelope],
    replay_fingerprints: &[ReplayFingerprintRecord],
) -> Vec<ReplayDiffLocatorRecord> {
    let mut by_task = HashMap::<String, Vec<&ReplayFingerprintRecord>>::new();
    for item in replay_fingerprints {
        by_task.entry(item.task_id.clone()).or_default().push(item);
    }

    let mut traces_by_id = HashMap::<String, Vec<&EventEnvelope>>::new();
    for event in events {
        traces_by_id
            .entry(event.trace_id.clone())
            .or_default()
            .push(event);
    }
    for trace_events in traces_by_id.values_mut() {
        trace_events.sort_by_key(|event| event.created_at_ms);
    }

    let mut records = Vec::new();
    for (task_id, mut items) in by_task {
        if items.len() < 2 {
            continue;
        }
        items.sort_by_key(|item| item.generated_at_ms);
        for window in items.windows(2) {
            if let [baseline, candidate] = window {
                if baseline.deterministic_replay_fingerprint
                    == candidate.deterministic_replay_fingerprint
                {
                    continue;
                }
                let mismatch_fields = infer_mismatch_fields(
                    traces_by_id
                        .get(&baseline.trace_id)
                        .map(|entries| entries.as_slice())
                        .unwrap_or(&[]),
                    traces_by_id
                        .get(&candidate.trace_id)
                        .map(|entries| entries.as_slice())
                        .unwrap_or(&[]),
                );
                records.push(ReplayDiffLocatorRecord {
                    session_id: session_id.to_string(),
                    task_id: task_id.clone(),
                    baseline_trace_id: baseline.trace_id.clone(),
                    candidate_trace_id: candidate.trace_id.clone(),
                    baseline_fingerprint: baseline.deterministic_replay_fingerprint.clone(),
                    candidate_fingerprint: candidate.deterministic_replay_fingerprint.clone(),
                    located: !mismatch_fields.is_empty(),
                    mismatch_fields,
                });
            }
        }
    }

    records
}
async fn collect_delegation_traces(
    db: &StateStore,
    session_id: &str,
    reports: &[ExecutionReport],
) -> Result<Vec<DelegationTraceRecord>> {
    let mut traces = Vec::new();

    for record in db
        .list_knowledge_by_prefix(&format!("peer-delegation:{session_id}:"))
        .await?
    {
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&record.value) {
            let task_id = payload
                .get("task_id")
                .and_then(|v| v.as_str())
                .unwrap_or("task:unknown")
                .to_string();
            let peer = payload
                .get("peer")
                .and_then(|v| v.as_str())
                .unwrap_or("peer:unknown")
                .to_string();
            traces.push(DelegationTraceRecord {
                session_id: session_id.to_string(),
                task_id,
                peer,
                delegated: payload
                    .get("delegated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                source: "knowledge-record".into(),
                created_at_ms: payload
                    .get("created_at_ms")
                    .and_then(as_u64)
                    .unwrap_or_else(current_time_ms),
            });
        }
    }

    for report in reports {
        if let Some(peer) = parse_peer_from_output(&report.output) {
            let exists = traces
                .iter()
                .any(|item| item.task_id == report.task.task_id && item.peer == peer);
            if !exists {
                traces.push(DelegationTraceRecord {
                    session_id: session_id.to_string(),
                    task_id: report.task.task_id.clone(),
                    peer,
                    delegated: true,
                    source: "execution-output".into(),
                    created_at_ms: current_time_ms(),
                });
            }
        }
    }

    traces.sort_by_key(|item| item.created_at_ms);
    Ok(traces)
}

async fn persist_snapshot(db: &StateStore, snapshot: &TelemetryCollectorSnapshot) -> Result<()> {
    let session_id = &snapshot.session_id;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:exec-spans"),
        &snapshot.exec_spans,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:latency"),
        &snapshot.latency_samples,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:retries"),
        &snapshot.retries,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:guard-hits"),
        &snapshot.guard_hits,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:breaker-trips"),
        &snapshot.breaker_trips,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:token-cost"),
        &snapshot.token_costs,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:delegation"),
        &snapshot.delegation_traces,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:unified-trace"),
        &snapshot.unified_trace,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:branch-collapse"),
        &snapshot.branch_collapse,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:regressions"),
        &snapshot.regressions,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:contradictions"),
        &snapshot.contradictions,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:replay-fingerprints"),
        &snapshot.replay_fingerprints,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:replay-diff-locators"),
        &snapshot.replay_diff_locators,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:collector:snapshot"),
        snapshot,
        "observability",
    )
    .await?;
    Ok(())
}

fn infer_pool_from_role(role: &str) -> String {
    let normalized = role.to_ascii_lowercase();
    if normalized.contains("execution")
        || normalized.contains("security")
        || normalized.contains("operations")
    {
        "stable".into()
    } else {
        "adaptive".into()
    }
}

fn status_from_event(event: &EventEnvelope) -> &'static str {
    match event.kind.as_str() {
        "policy_reject" | "runtime_blocks" => "blocked",
        "task_runs" => {
            let decision = payload_string(&event.payload, "decision")
                .unwrap_or_else(|| "allow".into())
                .to_ascii_lowercase();
            if decision.contains("block") || decision.contains("require") {
                "blocked"
            } else {
                "succeeded"
            }
        }
        "runtime_mode_dispatch" => "pending",
        "trust_admission" => "ready",
        _ => "running",
    }
}

fn merge_status(current: &str, next: &str) -> &'static str {
    fn rank(value: &str) -> u8 {
        match value {
            "blocked" => 5,
            "failed" => 4,
            "succeeded" => 3,
            "running" => 2,
            "ready" => 1,
            _ => 0,
        }
    }
    if rank(next) >= rank(current) {
        match next {
            "blocked" => "blocked",
            "failed" => "failed",
            "succeeded" => "succeeded",
            "running" => "running",
            "ready" => "ready",
            _ => "pending",
        }
    } else {
        match current {
            "blocked" => "blocked",
            "failed" => "failed",
            "succeeded" => "succeeded",
            "running" => "running",
            "ready" => "ready",
            _ => "pending",
        }
    }
}

fn status_rank(value: &str) -> u8 {
    match value {
        "blocked" => 5,
        "failed" => 4,
        "succeeded" => 3,
        "running" => 2,
        "ready" => 1,
        _ => 0,
    }
}

fn normalize_decision(decision: &str) -> &'static str {
    let normalized = decision.to_ascii_lowercase();
    if normalized.contains("block") || normalized.contains("deny") || normalized.contains("reject") {
        "deny"
    } else {
        "allow"
    }
}

fn infer_mismatch_fields(baseline: &[&EventEnvelope], candidate: &[&EventEnvelope]) -> Vec<String> {
    let baseline_kinds = baseline
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    let candidate_kinds = candidate
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();

    let baseline_capabilities = baseline
        .iter()
        .filter_map(|event| event.capability_id.clone())
        .collect::<Vec<_>>();
    let candidate_capabilities = candidate
        .iter()
        .filter_map(|event| event.capability_id.clone())
        .collect::<Vec<_>>();

    let mut fields = Vec::new();
    if baseline_kinds != candidate_kinds {
        fields.push("event_kind_sequence".to_string());
    }
    if baseline_capabilities != candidate_capabilities {
        fields.push("capability_sequence".to_string());
    }
    if baseline.len() != candidate.len() {
        fields.push("event_count".to_string());
    }
    let baseline_terminal = baseline.last().map(|event| status_from_event(event).to_string());
    let candidate_terminal = candidate.last().map(|event| status_from_event(event).to_string());
    if baseline_terminal != candidate_terminal {
        fields.push("terminal_status".to_string());
    }

    fields
}

fn deterministic_trace_fingerprint(events: &[&EventEnvelope]) -> String {
    let mut parts = Vec::new();
    for event in events {
        let mut payload_pairs = event
            .payload
            .as_object()
            .map(|object| {
                let mut pairs = object
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>();
                pairs.sort();
                pairs
            })
            .unwrap_or_default();
        payload_pairs.sort();
        let payload_text = payload_pairs.join(",");
        parts.push(format!(
            "{}|{}|{}|{}|{}",
            event.kind,
            event.task_id.clone().unwrap_or_else(|| "task:unknown".into()),
            event
                .capability_id
                .clone()
                .unwrap_or_else(|| "capability:unknown".into()),
            event.version,
            payload_text
        ));
    }

    digest_of_parts(&parts)
}

fn digest_of_parts(parts: &[String]) -> String {
    let payload = if parts.is_empty() {
        "<empty>".to_string()
    } else {
        parts.join("::")
    };
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in payload.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}
fn payload_string(payload: &serde_json::Value, field: &str) -> Option<String> {
    payload.get(field).and_then(|value| {
        value
            .as_str()
            .map(str::to_string)
            .or_else(|| value.as_i64().map(|v| v.to_string()))
            .or_else(|| value.as_u64().map(|v| v.to_string()))
    })
}

fn as_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
        .or_else(|| value.as_f64().map(|v| v.max(0.0) as u64))
        .or_else(|| value.as_str().and_then(|v| v.parse::<u64>().ok()))
}

fn parse_peer_from_output(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("[delegation] peer="))
        .map(str::trim)
        .map(str::to_string)
        .filter(|peer| !peer.is_empty())
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

    use crate::{
        observability::event_stream::append_event,
        orchestration::{ExecutionReport, SwarmTask},
    };

    #[tokio::test]
    async fn collector_persists_layered_telemetry_views() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });

        let session = "session-collector";
        let task_id = "task-collector";

        let _ = append_event(
            &db,
            "runtime_mode_dispatch",
            "trace:collector:1",
            session,
            Some(task_id.into()),
            Some("mcp::local-mcp::invoke".into()),
            CONTRACT_VERSION,
            serde_json::json!({
                "mode": "normal"
            }),
        )
        .await;

        let _ = append_event(
            &db,
            "runtime_blocks",
            "trace:collector:1",
            session,
            Some(task_id.into()),
            Some("mcp::local-mcp::invoke".into()),
            CONTRACT_VERSION,
            serde_json::json!({
                "decision": "Blocked",
                "reason": "tool circuit open",
                "cost_breakdown": {
                    "token_cost_micros": 0,
                    "tool_cost_micros": 0,
                    "duration_cost_micros": 0,
                    "total_cost_micros": 0
                }
            }),
        )
        .await;

        let _ = append_event(
            &db,
            "task_runs",
            "trace:collector:1",
            session,
            Some(task_id.into()),
            Some("mcp::local-mcp::invoke".into()),
            CONTRACT_VERSION,
            serde_json::json!({
                "decision": "Allow",
                "reason": "retry passed",
                "estimated_prompt_tokens": 256,
                "cost_breakdown": {
                    "token_cost_micros": 2560,
                    "tool_cost_micros": 500,
                    "duration_cost_micros": 200,
                    "total_cost_micros": 3260
                }
            }),
        )
        .await;

        db.upsert_json_knowledge(
            format!("execution-fabric-sequence:{session}:{task_id}:100"),
            &ExecutionFabricReplaySequence {
                session_id: session.into(),
                task_id: task_id.into(),
                trace_id: "trace:collector:1".into(),
                capability_id: "mcp::local-mcp::invoke".into(),
                generated_at_ms: 100,
                steps: vec![
                    crate::runtime::execution_fabric::ExecutionFabricStep {
                        sequence: 0,
                        stage: "dispatch".into(),
                        state: crate::contracts::flow::FlowNodeState::Pending,
                        reason: "dispatch".into(),
                        side_effect_state: "none".into(),
                        budget_state: "precharge_pending".into(),
                        trigger_state: "fabric.dispatch".into(),
                        metadata: BTreeMap::new(),
                        at_ms: 100,
                    },
                    crate::runtime::execution_fabric::ExecutionFabricStep {
                        sequence: 1,
                        stage: "admission".into(),
                        state: crate::contracts::flow::FlowNodeState::Ready,
                        reason: "admitted".into(),
                        side_effect_state: "none".into(),
                        budget_state: "reserved".into(),
                        trigger_state: "fabric.admission".into(),
                        metadata: BTreeMap::new(),
                        at_ms: 112,
                    },
                ],
            },
            "execution-fabric",
        )
        .await
        .expect("seed execution sequence");

        db.upsert_json_knowledge(
            "metrics:circuit:tool:mcp::local-mcp::invoke",
            &CircuitState {
                scope_key: "metrics:circuit:tool:mcp::local-mcp::invoke".into(),
                failure_count: 3,
                success_count: 0,
                phase: CircuitPhase::Open,
                opened_at_ms: Some(90),
                last_failure_ms: Some(95),
                last_success_ms: None,
                cooldown_ms: 30000,
                threshold: 2,
                last_reason: Some("tool circuit open".into()),
            },
            "runtime-circuit",
        )
        .await
        .expect("seed circuit");

        db.upsert_json_knowledge(
            format!("peer-delegation:{session}:{task_id}:latest"),
            &serde_json::json!({
                "session_id": session,
                "task_id": task_id,
                "peer": "peer-alpha",
                "delegated": true,
                "created_at_ms": 120
            }),
            "orchestration",
        )
        .await
        .expect("seed delegation");

        let reports = vec![ExecutionReport {
            task: SwarmTask {
                task_id: task_id.into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "execute collector telemetry flow".into(),
                depends_on: Vec::new(),
            },
            output: "done\n[delegation] peer=peer-alpha".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{}".into()),
            outcome_score: 3,
            route_variant: "control".into(),
            control_score: 3,
            treatment_score: 2,
            guard_decision: "Allow".into(),
        }];

        let scope = crate::orchestration::governance_telemetry_scope::GovernanceTelemetryScope {
            scope_id: format!("gov-scope:{session}"),
            session_id: session.to_string(),
            tenant_scope: "tenant:test".into(),
            risk_tier: "low".into(),
            privacy_level: "internal".into(),
            approval_required: false,
            retention_hours: 72,
            redaction_fields: vec![],
        };
        let snapshot = collect_and_persist(&db, session, &scope, &reports)
            .await
            .expect("collect telemetry");

        assert!(!snapshot.exec_spans.is_empty());
        assert!(!snapshot.latency_samples.is_empty());
        assert_eq!(snapshot.retries.len(), 1);
        assert!(!snapshot.guard_hits.is_empty());
        assert!(!snapshot.breaker_trips.is_empty());
        assert!(!snapshot.token_costs.is_empty());
        assert!(!snapshot.delegation_traces.is_empty());
        assert!(!snapshot.unified_trace.is_empty());
        assert!(!snapshot.replay_fingerprints.is_empty());

        let persisted = db
            .get_knowledge(&format!("observability:{session}:collector:snapshot"))
            .await
            .expect("snapshot read")
            .expect("snapshot exists");
        assert!(persisted.value.contains("exec_spans"));
    }

    #[test]
    fn deterministic_trace_fingerprint_same_input_same_output() {
        let event = EventEnvelope {
            event_id: "evt:s:1".into(),
            kind: "task_runs".into(),
            trace_id: "trace:s:1".into(),
            session_id: "s".into(),
            task_id: Some("task-a".into()),
            capability_id: Some("tool:a".into()),
            version: "v1".into(),
            payload: serde_json::json!({
                "decision": "Allow",
                "reason": "ok"
            }),
            created_at_ms: 1,
        };

        let first = deterministic_trace_fingerprint(&[&event]);
        let second = deterministic_trace_fingerprint(&[&event]);
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn collector_locates_replay_diff_between_branches() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let session = "session-replay-diff";
        let task_id = "task-replay";

        let _ = append_event(
            &db,
            "task_runs",
            "trace:replay:base",
            session,
            Some(task_id.into()),
            Some("mcp::local-mcp::invoke".into()),
            CONTRACT_VERSION,
            serde_json::json!({
                "decision": "Allow",
                "reason": "baseline"
            }),
        )
        .await;
        let _ = append_event(
            &db,
            "runtime_blocks",
            "trace:replay:candidate",
            session,
            Some(task_id.into()),
            Some("mcp::local-mcp::invoke".into()),
            CONTRACT_VERSION,
            serde_json::json!({
                "decision": "Blocked",
                "reason": "candidate changed"
            }),
        )
        .await;

        let scope = crate::orchestration::governance_telemetry_scope::GovernanceTelemetryScope {
            scope_id: format!("gov-scope:{session}"),
            session_id: session.to_string(),
            tenant_scope: "tenant:test".into(),
            risk_tier: "low".into(),
            privacy_level: "internal".into(),
            approval_required: false,
            retention_hours: 72,
            redaction_fields: vec![],
        };

        let reports = vec![ExecutionReport {
            task: SwarmTask {
                task_id: task_id.into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "detect replay diff".into(),
                depends_on: Vec::new(),
            },
            output: "done".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{}".into()),
            outcome_score: 1,
            route_variant: "control".into(),
            control_score: 1,
            treatment_score: 1,
            guard_decision: "Allow".into(),
        }];

        let snapshot = collect_and_persist(&db, session, &scope, &reports)
            .await
            .expect("collect telemetry");

        assert!(
            snapshot.replay_diff_locators.iter().any(|item| item.located),
            "expected replay diff locator to pinpoint mismatch"
        );
    }
}










