use anyhow::Result;
use autoloop_state_adapter::{SessionLease, ShadowReadDiffReport, StateStore};
use serde::{Deserialize, Serialize};

use crate::{
    contracts::{
        context::{ProjectMemoryPolicy, UnifiedQueryView},
        errors::ContractError,
        ids::SessionId,
        ports::QueryPlanePort,
        version::CONTRACT_VERSION,
    },
    plugins::gitmemory_core::semantic_lint::build_semantic_lint_report,
    observability::event_stream::{
        ReplayAnalysisReport, append_event, list_replay_snapshots, list_session_events,
    },
    orchestration::current_time_ms,
};
use crate::security::policy_host::summarize_decision_log_artifacts;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayMismatchExplainRecord {
    pub snapshot_id: String,
    pub trace_id: String,
    pub category: String,
    pub summary: String,
    pub deterministic_boundary_respected: bool,
    pub deviations: Vec<String>,
    pub causing_plugins: Vec<String>,
    pub created_at_ms: u64,
}

#[derive(Clone)]
pub struct UnifiedQueryPlaneAdapter {
    db: StateStore,
}

impl UnifiedQueryPlaneAdapter {
    pub fn new(db: StateStore) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl QueryPlanePort for UnifiedQueryPlaneAdapter {
    async fn query_unified(
        &self,
        session_id: &SessionId,
        trace_id: Option<&str>,
    ) -> Result<UnifiedQueryView, ContractError> {
        build_unified_query_view(&self.db, session_id.as_ref(), trace_id)
            .await
            .map_err(|error| ContractError::Storage(error.to_string()))
    }
}

pub async fn persist_unified_query_view(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<UnifiedQueryView> {
    let view = build_unified_query_view(db, session_id, trace_id).await?;
    db.upsert_json_knowledge(
        format!("observability:{session_id}:query-plane"),
        &view,
        "observability",
    )
    .await?;
    db.upsert_json_knowledge(
        format!(
            "observability:{session_id}:query-plane:{}",
            view.generated_at_ms
        ),
        &view,
        "observability",
    )
    .await?;

    let replay_mismatch_count = view
        .replay
        .get("mismatch_explain")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    let shadow_storage_mismatch_count = view
        .metrics
        .get("storage_shadow_diff")
        .and_then(|value| value.get("mismatch_count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let _ = append_event(
        db,
        "query_plane.updated",
        format!("trace:{session_id}:query-plane:{}", view.generated_at_ms),
        session_id.to_string(),
        None,
        Some("observability:query-plane".into()),
        CONTRACT_VERSION,
        serde_json::json!({
            "trace_filter": trace_id,
            "replay_mismatch_count": replay_mismatch_count,
            "storage_shadow_mismatch_count": shadow_storage_mismatch_count,
        }),
    )
    .await;

    Ok(view)
}

pub async fn build_unified_query_view(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<UnifiedQueryView> {
    let lease = db.get_session_lease(session_id).await?;
    let route_policy = resolve_query_route_policy(db, lease.as_ref()).await?;
    let metrics = apply_policy_to_metrics(
        collect_metrics(db, session_id, trace_id).await?,
        &route_policy,
    );
    let traces = apply_policy_to_array(
        collect_traces(db, session_id, trace_id).await?,
        &route_policy,
        trace_id,
        Some("traces"),
    );
    let logs = apply_policy_to_array(
        collect_logs(db, session_id).await?,
        &route_policy,
        trace_id,
        Some("logs"),
    );
    let events = apply_policy_to_array(
        collect_events(db, session_id, trace_id).await?,
        &route_policy,
        trace_id,
        Some("events"),
    );
    let ledger = apply_policy_to_ledger(
        collect_ledger(db, session_id, trace_id, lease.as_ref()).await?,
        &route_policy,
        trace_id,
    );
    let graph = apply_policy_to_graph(
        collect_graph(db, session_id, lease.as_ref()).await?,
        &route_policy,
    );
    let replay = apply_policy_to_replay(
        collect_replay(db, session_id, trace_id).await?,
        &route_policy,
        trace_id,
    );

    Ok(UnifiedQueryView {
        session_id: session_id.to_string(),
        trace_id: trace_id.map(str::to_string),
        metrics,
        traces,
        logs,
        events,
        ledger,
        graph,
        replay,
        generated_at_ms: current_time_ms(),
    })
}

async fn collect_metrics(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let route_analytics =
        get_value(db, &format!("observability:{session_id}:route-analytics")).await?;
    let failure_forensics =
        get_value(db, &format!("observability:{session_id}:failure-forensics")).await?;
    let system_telemetry =
        get_value(db, &format!("observability:{session_id}:system-telemetry")).await?;
    let collector = get_value(
        db,
        &format!("observability:{session_id}:telemetry-collector"),
    )
    .await?;
    let cost_report = get_value(db, &format!("observability:{session_id}:cost-report")).await?;
    let resilience = get_value(db, &format!("observability:{session_id}:resilience")).await?;
    let objective_weights = get_value(db, "policy:context-objective-weights:latest").await?;
    let objective_breakdown = collect_context_objective_breakdown(db, session_id).await?;
    let context_compiler_whitebox =
        collect_context_compiler_whitebox(db, session_id, trace_id).await?;
    let storage_shadow_diff = collect_storage_shadow_diff(db, session_id, trace_id).await?;
    let foundry_feedback = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("foundry:feedback:{session_id}:"))
            .await?,
        256,
    );
    let foundry_feedback_summary = summarize_foundry_feedback(&foundry_feedback);

    let execution_metrics =
        collect_records_as_values(db.list_knowledge_by_prefix("metrics:execution:").await?, 64);
    let ab_metrics =
        collect_records_as_values(db.list_knowledge_by_prefix("metrics:ab:").await?, 64);
    let circuit_metrics =
        collect_records_as_values(db.list_knowledge_by_prefix("metrics:circuit:").await?, 64);

    Ok(serde_json::json!({
        "route_analytics": route_analytics,
        "failure_forensics": failure_forensics,
        "system_telemetry": system_telemetry,
        "collector": collector,
        "cost_report": cost_report,
        "resilience": resilience,
        "context_objective_weights": objective_weights,
        "context_objective_breakdown": objective_breakdown,
        "context_compiler_whitebox": context_compiler_whitebox,
        "storage_shadow_diff": storage_shadow_diff,
        "foundry_feedback_summary": foundry_feedback_summary,
        "execution_metrics": execution_metrics,
        "ab_metrics": ab_metrics,
        "circuit_metrics": circuit_metrics,
    }))
}

async fn collect_storage_shadow_diff(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut reports = Vec::<ShadowReadDiffReport>::new();
    let key_targets = vec![
        format!("observability:{session_id}:query-plane"),
        format!("memory:{session_id}:consolidation"),
        format!("graph:{session_id}:snapshot"),
    ];
    for key in key_targets {
        if let Some(report) = db.compare_shadow_get_knowledge(&key).await? {
            reports.push(report);
        }
    }

    let prefix_targets = vec![
        format!("memory:{session_id}:"),
        format!("replay:analysis:{session_id}:"),
        format!("foundry:feedback:{session_id}:"),
    ];
    for prefix in prefix_targets {
        if let Some(report) = db.compare_shadow_list_knowledge_by_prefix(&prefix).await? {
            reports.push(report);
        }
    }

    if let Some(report) = db.compare_shadow_schedule_events(session_id).await? {
        reports.push(report);
    }
    if let Some(report) = db.compare_shadow_agent_state(session_id).await? {
        reports.push(report);
    }
    if let Some(report) = db.compare_shadow_session_lease(session_id).await? {
        reports.push(report);
    }
    if let Some(lease) = db.get_session_lease(session_id).await? {
        if let Some(report) = db
            .compare_shadow_cost_attribution_by_session(&lease.tenant_id, session_id)
            .await?
        {
            reports.push(report);
        }
    }

    let mut class_counts = std::collections::BTreeMap::<String, u64>::new();
    let mut domain_counts = std::collections::BTreeMap::<String, u64>::new();
    for report in &reports {
        *domain_counts.entry(report.domain.clone()).or_insert(0) += 1;
        for class in &report.diff_classes {
            *class_counts.entry(class.clone()).or_insert(0) += 1;
        }
    }

    let mismatch_count = reports.iter().filter(|report| report.mismatch).count();
    let has_probe = !reports.is_empty();
    Ok(serde_json::json!({
        "enabled": has_probe,
        "trace_filter": trace_id,
        "mismatch_count": mismatch_count,
        "diff_class_counts": class_counts,
        "domain_counts": domain_counts,
        "reports": reports,
    }))
}

async fn collect_context_objective_breakdown(
    db: &StateStore,
    session_id: &str,
) -> Result<serde_json::Value> {
    let mut traces = db
        .list_knowledge_by_prefix(&format!("replay:plugin-trace:{session_id}:"))
        .await?
        .into_iter()
        .filter_map(record_to_json)
        .collect::<Vec<_>>();
    traces.sort_by(|left, right| {
        extract_ts(left)
            .unwrap_or(0)
            .cmp(&extract_ts(right).unwrap_or(0))
    });

    let latest = traces.last().cloned();
    let objective = latest
        .as_ref()
        .and_then(|trace| trace.get("value"))
        .and_then(|value| value.get("annotation_proof"))
        .and_then(|proof| proof.get("objective"))
        .cloned();
    let decision_summary = latest
        .as_ref()
        .and_then(|trace| trace.get("value"))
        .and_then(|value| value.get("annotation_proof"))
        .and_then(|proof| proof.get("annotation"))
        .and_then(|ann| ann.get("decision_summary"))
        .cloned();

    Ok(serde_json::json!({
        "latest_objective": objective,
        "latest_decision_summary": decision_summary,
        "trace_samples": traces.into_iter().rev().take(5).collect::<Vec<_>>(),
    }))
}

async fn collect_context_compiler_whitebox(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut traces = db
        .list_knowledge_by_prefix(&format!("replay:plugin-trace:{session_id}:"))
        .await?
        .into_iter()
        .filter_map(record_to_json)
        .collect::<Vec<_>>();
    if let Some(trace_id) = trace_id {
        traces.retain(|item| trace_matches(item, trace_id));
    }
    traces.sort_by_key(|item| extract_ts(item).unwrap_or(0));

    let mut samples = traces
        .iter()
        .rev()
        .take(8)
        .map(extract_context_compiler_whitebox)
        .collect::<Vec<_>>();
    samples.reverse();
    let latest = samples
        .last()
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    Ok(serde_json::json!({
        "latest": latest,
        "samples": samples,
        "total_trace_records": traces.len(),
    }))
}

fn extract_context_compiler_whitebox(trace: &serde_json::Value) -> serde_json::Value {
    let value = trace.get("value").unwrap_or(trace);
    let annotation = value
        .get("annotation_proof")
        .and_then(|proof| proof.get("annotation"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    let risk_metadata = annotation.get("risk_metadata").cloned().unwrap_or_else(|| {
        serde_json::json!({
            "risk_labels": annotation
                .get("risk_labels")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        })
    });

    serde_json::json!({
        "trace_id": value
            .get("trace_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        "hardgate_pass_token": value
            .get("hardgate_pass_token")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        "constraint_version": value
            .get("constraint_version")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        "constraint_ids": value
            .get("constraint_ids")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
        "prompt_pack": annotation
            .get("prompt_pack")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "dropped_mapping": annotation
            .get("dropped_mapping")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
        "source_mapping": annotation
            .get("source_mapping")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
        "risk_metadata": risk_metadata,
        "decision_summary": annotation
            .get("decision_summary")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "replay_fingerprint": annotation
            .get("replay_fingerprint")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        "generated_at_ms": extract_ts(trace),
    })
}

async fn collect_traces(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut traces = db
        .list_knowledge_by_prefix(&format!("observability:{session_id}:trace:"))
        .await?
        .into_iter()
        .filter_map(record_to_json)
        .collect::<Vec<_>>();
    traces.extend(
        db.list_knowledge_by_prefix(&format!("execution-fabric:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(record_to_json),
    );
    if let Some(trace_id) = trace_id {
        traces.retain(|value| trace_matches(value, trace_id));
    }
    Ok(serde_json::Value::Array(traces))
}

async fn collect_logs(db: &StateStore, session_id: &str) -> Result<serde_json::Value> {
    let mut logs = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("conversation:{session_id}:execution-feedback:"))
            .await?,
        128,
    );
    logs.extend(collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("runtime:failover:{session_id}:"))
            .await?,
        128,
    ));
    logs.extend(collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("policy:{session_id}:decision:"))
            .await?,
        64,
    ));
    logs.extend(collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("context-kernel:shadow:{session_id}:"))
            .await?,
        256,
    ));
    logs.extend(collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("foundry:feedback:{session_id}:"))
            .await?,
        256,
    ));
    Ok(serde_json::Value::Array(logs))
}

async fn collect_events(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut events = serde_json::to_value(list_session_events(db, session_id).await?)?
        .as_array()
        .cloned()
        .unwrap_or_default();

    let cli_events = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("observability:cli-event:{session_id}:"))
            .await?,
        512,
    );
    events.extend(cli_events);

    if let Some(trace_filter) = trace_id {
        events.retain(|event| trace_matches(event, trace_filter));
    }

    Ok(serde_json::Value::Array(events))
}

async fn collect_ledger(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
    lease: Option<&SessionLease>,
) -> Result<serde_json::Value> {
    let stage = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:stage:{session_id}:"))
            .await?,
        256,
    );
    let step = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:step:{session_id}:"))
            .await?,
        256,
    );
    let budget = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:budget:{session_id}:"))
            .await?,
        256,
    );
    let approval = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:approval:{session_id}:"))
            .await?,
        128,
    );
    let replay = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:replay:{session_id}:"))
            .await?,
        128,
    );
    let tags = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:tag:{session_id}:"))
            .await?,
        256,
    );
    let memory_history = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("memory:supermemory:history:{session_id}:"))
            .await?,
        256,
    );
    let foundry_feedback_evidence = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:foundry:{session_id}:"))
            .await?,
        256,
    );
    let op_log_events = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:op-log:{session_id}:"))
            .await?,
        512,
    );
    let relation_evidence_refs = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("relation:evidence:{session_id}:"))
            .await?,
        256,
    );
    let relation_write_proofs = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("relation:write_proof:{session_id}:"))
            .await?,
        256,
    );
    let decision_log_summary = summarize_decision_log_artifacts(&stage);

    let memory_subledger = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:memory:{session_id}:"))
            .await?,
        256,
    );
    let learning_signal_rejections = summarize_learning_signal_rejections(&memory_subledger);

    let (spend_ledger, cost_attribution) = if let Some(lease) = lease {
        let account_id = format!(
            "{}:{}:{}",
            lease.tenant_id, lease.principal_id, lease.policy_id
        );
        let mut spend_ledger = db.list_spend_ledger(&lease.tenant_id, &account_id).await?;
        if let Some(trace_id) = trace_id {
            spend_ledger.retain(|record| record.trace_id == trace_id);
        }
        let mut attribution = db
            .list_cost_attribution_by_session(&lease.tenant_id, session_id)
            .await?;
        if let Some(trace_id) = trace_id {
            attribution.retain(|record| record.trace_id == trace_id);
        }
        (spend_ledger, attribution)
    } else {
        (Vec::new(), Vec::new())
    };

    Ok(serde_json::json!({
        "stage_chain": stage,
        "step_chain": step,
        "foundry_feedback_evidence": foundry_feedback_evidence,
        "op_log_events": op_log_events,
        "decision_log_summary": decision_log_summary,
        "budget_ledger": budget,
        "approval_records": approval,
        "replay_fingerprints": replay,
        "evidence_tags": tags,
        "memory_history": memory_history,
        "memory_subledger": memory_subledger,
        "learning_signal_rejections": learning_signal_rejections,
        "relation_evidence_refs": relation_evidence_refs,
        "relation_write_proofs": relation_write_proofs,
        "spend_ledger": spend_ledger,
        "cost_attribution": cost_attribution,
    }))
}

fn summarize_foundry_feedback(records: &[serde_json::Value]) -> serde_json::Value {
    let mut hit = 0_u64;
    let mut miss = 0_u64;
    let mut failed_trigger = 0_u64;
    let mut missing_script = 0_u64;
    let mut bad_json = 0_u64;
    for record in records {
        let kind = record
            .get("value")
            .and_then(|value| value.get("kind"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        match kind {
            "hit" => hit += 1,
            "miss" => miss += 1,
            "failed_trigger" => failed_trigger += 1,
            "missing_script" => missing_script += 1,
            "bad_json" => bad_json += 1,
            _ => {}
        }
    }
    serde_json::json!({
        "total": records.len(),
        "hit": hit,
        "miss": miss,
        "failed_trigger": failed_trigger,
        "missing_script": missing_script,
        "bad_json": bad_json,
    })
}

fn summarize_learning_signal_rejections(records: &[serde_json::Value]) -> serde_json::Value {
    let mut items = Vec::<serde_json::Value>::new();
    for record in records {
        let value = record.get("value").unwrap_or(record);
        let Some(reason) = value.get("reason").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !reason.starts_with("learning_signal.") {
            continue;
        }
        items.push(serde_json::json!({
            "reject_id": value.get("reject_id").cloned().unwrap_or(serde_json::Value::Null),
            "trace_id": value.get("trace_id").cloned().unwrap_or(serde_json::Value::Null),
            "target": value.get("target").cloned().unwrap_or(serde_json::Value::Null),
            "reason": reason,
            "source": value.get("source").cloned().unwrap_or(serde_json::Value::Null),
            "evidence_ref": value.get("evidence_ref").cloned().unwrap_or(serde_json::Value::Null),
            "created_at_ms": value
                .get("created_at_ms")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        }));
    }
    items.sort_by_key(|item| {
        item.get("created_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    let latest_reason = items
        .last()
        .and_then(|item| item.get("reason"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    serde_json::json!({
        "count": items.len(),
        "latest_reason": latest_reason,
        "items": items.into_iter().rev().take(16).collect::<Vec<_>>(),
    })
}

async fn collect_graph(
    db: &StateStore,
    session_id: &str,
    lease: Option<&SessionLease>,
) -> Result<serde_json::Value> {
    let session_graph = get_value(db, &format!("graph:{session_id}:snapshot")).await?;
    let global_graph = get_value(db, "graph:global:snapshot").await?;
    let graph_health = collect_graph_health(db, session_id).await?;
    let recall_route_fallback = collect_recall_route_fallback(db, session_id).await?;
    let relation_graph = collect_relation_graph(db, session_id).await?;
    let org_knowledge = if let Some(lease) = lease {
        collect_records_as_values(
            db.list_knowledge_by_prefix(&format!("org-knowledge:{}:", lease.tenant_id))
                .await?,
            128,
        )
    } else {
        Vec::new()
    };
    Ok(serde_json::json!({
        "session_graph_snapshot": session_graph,
        "global_graph_snapshot": global_graph,
        "graph_health": graph_health,
        "recall_route_fallback": recall_route_fallback,
        "relation_graph": relation_graph,
        "org_knowledge_updates": org_knowledge,
    }))
}

async fn collect_relation_graph(db: &StateStore, session_id: &str) -> Result<serde_json::Value> {
    let edges = db
        .list_relation_edges_current(session_id, 512)
        .await?
        .into_iter()
        .map(|item| serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({})))
        .collect::<Vec<_>>();
    let events = db
        .list_relation_events(session_id, 512)
        .await?
        .into_iter()
        .map(|item| serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({})))
        .collect::<Vec<_>>();
    let hot_index = db
        .list_relation_hot_index(session_id, 256)
        .await?
        .into_iter()
        .map(|item| serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({})))
        .collect::<Vec<_>>();

    let evidence_refs = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("relation:evidence:{session_id}:"))
            .await?,
        256,
    );
    let write_proofs = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("relation:write_proof:{session_id}:"))
            .await?,
        256,
    );

    Ok(serde_json::json!({
        "edges_current": edges,
        "events_append_only": events,
        "hot_index": hot_index,
        "evidence_refs": evidence_refs,
        "write_proofs": write_proofs,
    }))
}

async fn collect_graph_health(db: &StateStore, session_id: &str) -> Result<serde_json::Value> {
    let latest = get_value(db, &format!("memory:graph:health:{session_id}:latest")).await?;
    let latest_ref = latest
        .as_ref()
        .and_then(|value| value.get("ref"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let summary = if let Some(reference) = latest_ref.as_deref() {
        get_value(db, reference).await?
    } else {
        None
    };
    let mut refs = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("memory:graph:health:{session_id}:"))
            .await?,
        64,
    );
    refs.retain(|item| {
        item.get("key")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.ends_with(":latest"))
    });
    Ok(serde_json::json!({
        "latest": latest,
        "latest_ref": latest_ref,
        "summary": summary,
        "refs": refs,
    }))
}


async fn collect_recall_route_fallback(
    db: &StateStore,
    session_id: &str,
) -> Result<serde_json::Value> {
    let mut records = db
        .list_knowledge_by_prefix(&format!("memory:episode:{session_id}:"))
        .await?;
    records.sort_by(|left, right| left.key.cmp(&right.key));

    let mut samples = Vec::<serde_json::Value>::new();
    for record in records {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) else {
            continue;
        };
        let is_recall = value
            .get("stage")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|stage| stage == "recall");
        if !is_recall {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let query_route_fallback = payload
            .get("recall")
            .and_then(|recall| recall.get("query_route_fallback"))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let cjk_lexical_hits = payload
            .get("recall")
            .and_then(|recall| recall.get("cjk_lexical_hits"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        let seed_hits = payload
            .get("recall")
            .and_then(|recall| recall.get("seed_hits"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));

        samples.push(serde_json::json!({
            "key": record.key,
            "trace_id": value.get("trace_id").cloned().unwrap_or(serde_json::Value::Null),
            "created_at_ms": value.get("created_at_ms").cloned().unwrap_or(serde_json::Value::Null),
            "query_route_fallback": query_route_fallback,
            "cjk_lexical_hits": cjk_lexical_hits,
            "seed_hits": seed_hits,
        }));
    }
    samples.sort_by_key(|item| {
        item.get("created_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    let latest = samples.last().cloned().unwrap_or_else(|| serde_json::json!({}));

    Ok(serde_json::json!({
        "latest": latest,
        "samples": samples.into_iter().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>(),
    }))
}
async fn collect_replay(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut snapshots = list_replay_snapshots(db, session_id).await?;
    if let Some(trace_id) = trace_id {
        snapshots.retain(|snapshot| snapshot.trace_id == trace_id);
    }

    let mut analyses = db
        .list_knowledge_by_prefix("replay:analysis:")
        .await?
        .into_iter()
        .filter_map(|record| serde_json::from_str::<ReplayAnalysisReport>(&record.value).ok())
        .filter(|report| report.session_id == session_id)
        .collect::<Vec<_>>();
    if let Some(trace_id) = trace_id {
        analyses.retain(|report| report.trace_id == trace_id);
    }
    analyses.sort_by_key(|report| report.created_at_ms);

    let mut plugin_traces = db
        .list_knowledge_by_prefix(&format!("replay:plugin-trace:{session_id}:"))
        .await?
        .into_iter()
        .filter_map(record_to_json)
        .collect::<Vec<_>>();
    if let Some(trace_id) = trace_id {
        plugin_traces.retain(|trace| trace_matches(trace, trace_id));
    }

    let mismatch_explain = analyses
        .iter()
        .filter(|report| !report.matched || !report.deterministic_boundary_respected)
        .map(|report| build_mismatch_explain(report, &plugin_traces))
        .collect::<Vec<_>>();
    let evolution_explain = collect_evolution_replay_explain(db, session_id, trace_id).await?;
    let signal_pipeline_explain = collect_signal_pipeline_explain(db, session_id, trace_id).await?;
    let graph_health = collect_graph_health(db, session_id).await?;
    let recall_route_fallback = collect_recall_route_fallback(db, session_id).await?;
    let semantic_lint_report = build_semantic_lint_report(
        session_id,
        trace_id,
        &analyses,
        &plugin_traces,
        &graph_health,
    );
    let harness_whitebox = collect_harness_execution_whitebox(db, session_id, trace_id).await?;

    let mut context_samples = plugin_traces
        .iter()
        .map(extract_context_compiler_whitebox)
        .collect::<Vec<_>>();
    context_samples.sort_by_key(|item| {
        item.get("generated_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    let context_latest = context_samples
        .last()
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    Ok(serde_json::json!({
        "snapshots": snapshots,
        "analysis_reports": analyses,
        "plugin_execution_traces": plugin_traces,
        "mismatch_explain": mismatch_explain,
        "evolution_explain": evolution_explain,
        "signal_pipeline_explain": signal_pipeline_explain,
        "graph_health": graph_health,
        "recall_route_fallback": recall_route_fallback,
        "semantic_lint_report": semantic_lint_report,
        "harness_whitebox": harness_whitebox,
        "context_compiler_whitebox": {
            "latest": context_latest,
            "samples": context_samples,
        },
        "latest_snapshot_id": snapshots.last().map(|item| item.snapshot_id.clone()),
        "latest_analysis_at_ms": analyses.last().map(|item| item.created_at_ms),
    }))
}

async fn collect_harness_execution_whitebox(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut patch_reports = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("harness:patch-report:{session_id}:"))
            .await?,
        512,
    );
    let mut shell_reports = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("harness:shell-loop:{session_id}:"))
            .await?,
        512,
    );
    let mut test_reports = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("harness:test-verifier:{session_id}:"))
            .await?,
        512,
    );
    let mut git_reports = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("harness:git-checkpoint:{session_id}:"))
            .await?,
        512,
    );

    patch_reports.retain(|item| {
        item.get("key")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.ends_with(":latest"))
    });
    shell_reports.retain(|item| {
        item.get("key")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.ends_with(":latest"))
    });
    test_reports.retain(|item| {
        item.get("key")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.ends_with(":latest"))
    });
    git_reports.retain(|item| {
        item.get("key")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.ends_with(":latest"))
    });

    if let Some(trace_filter) = trace_id {
        patch_reports.retain(|item| trace_matches(item, trace_filter));
        shell_reports.retain(|item| trace_matches(item, trace_filter));
        test_reports.retain(|item| trace_matches(item, trace_filter));
        git_reports.retain(|item| trace_matches(item, trace_filter));
    }

    patch_reports.sort_by_key(|item| extract_ts(item).unwrap_or(0));
    shell_reports.sort_by_key(|item| extract_ts(item).unwrap_or(0));
    test_reports.sort_by_key(|item| extract_ts(item).unwrap_or(0));
    git_reports.sort_by_key(|item| extract_ts(item).unwrap_or(0));

    let mut stage_evidence = collect_records_as_values(
        db.list_knowledge_by_prefix(&format!("evidence:stage:{session_id}:"))
            .await?,
        512,
    );
    stage_evidence.retain(|item| {
        let stage = item
            .get("value")
            .and_then(|value| value.get("stage"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        matches!(
            stage,
            "diff_patch_engine"
                | "diff_patch_engine_revert"
                | "shell_execution_loop"
                | "test_verifier"
                | "git_checkpoint_layer"
        )
    });
    if let Some(trace_filter) = trace_id {
        stage_evidence.retain(|item| trace_matches(item, trace_filter));
    }
    stage_evidence.sort_by_key(|item| extract_ts(item).unwrap_or(0));

    let mut trace_ids = std::collections::BTreeSet::<String>::new();
    for set in [
        patch_reports.as_slice(),
        shell_reports.as_slice(),
        test_reports.as_slice(),
        git_reports.as_slice(),
        stage_evidence.as_slice(),
    ] {
        for item in set {
            if let Some(trace) = item
                .get("value")
                .and_then(|value| value.get("trace_id"))
                .and_then(serde_json::Value::as_str)
            {
                trace_ids.insert(trace.to_string());
            }
        }
    }

    let mut rounds = Vec::<serde_json::Value>::new();
    for trace in trace_ids {
        let patch = patch_reports
            .iter()
            .rev()
            .find(|item| trace_matches(item, trace.as_str()))
            .cloned();
        let shell = shell_reports
            .iter()
            .rev()
            .find(|item| trace_matches(item, trace.as_str()))
            .cloned();
        let tests = test_reports
            .iter()
            .rev()
            .find(|item| trace_matches(item, trace.as_str()))
            .cloned();
        let git = git_reports
            .iter()
            .rev()
            .find(|item| trace_matches(item, trace.as_str()))
            .cloned();

        let evidence_records = stage_evidence
            .iter()
            .filter(|item| trace_matches(item, trace.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let evidence_refs = evidence_records
            .iter()
            .filter_map(|item| {
                item.get("key")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        let mut rollback_evidence = Vec::<serde_json::Value>::new();
        if let Some(patch_record) = patch.as_ref() {
            if patch_record
                .get("value")
                .and_then(|value| value.get("rollback_performed"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                rollback_evidence.push(serde_json::json!({
                    "kind": "patch_rollback",
                    "ref": patch_record.get("key").cloned().unwrap_or(serde_json::Value::Null),
                    "message": patch_record
                        .get("value")
                        .and_then(|value| value.get("message"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                }));
            }
        }
        if let Some(git_record) = git.as_ref() {
            let has_git_rollback = git_record
                .get("value")
                .and_then(|value| value.get("steps"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|steps| {
                    steps.iter().any(|step| {
                        step.get("action")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|action| action == "rollback")
                    })
                });
            if has_git_rollback {
                rollback_evidence.push(serde_json::json!({
                    "kind": "git_rollback",
                    "ref": git_record.get("key").cloned().unwrap_or(serde_json::Value::Null),
                    "halted_reason": git_record
                        .get("value")
                        .and_then(|value| value.get("halted_reason"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                }));
            }
        }
        let latest_ts = [patch.as_ref(), shell.as_ref(), tests.as_ref(), git.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(extract_ts)
            .max()
            .unwrap_or(0);

        rounds.push(serde_json::json!({
            "trace_id": trace,
            "generated_at_ms": latest_ts,
            "patch": patch,
            "commands": shell,
            "tests": tests,
            "git_checkpoint": git,
            "rollback_evidence": rollback_evidence,
            "evidence_refs": evidence_refs,
            "evidence_stage_records": evidence_records,
        }));
    }
    rounds.sort_by_key(|item| {
        item.get("generated_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });

    let latest = rounds.last().cloned().unwrap_or_else(|| serde_json::json!({}));
    let round_count = rounds.len();

    Ok(serde_json::json!({
        "latest": latest,
        "rounds": rounds,
        "round_count": round_count,
        "totals": {
            "patch_reports": patch_reports.len(),
            "command_reports": shell_reports.len(),
            "test_reports": test_reports.len(),
            "git_checkpoint_reports": git_reports.len(),
            "stage_evidence_records": stage_evidence.len(),
        }
    }))
}

async fn collect_evolution_replay_explain(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    #[derive(Default)]
    struct EvolutionRun {
        summary: Option<serde_json::Value>,
        board: Option<serde_json::Value>,
        candidate_graphs: Option<serde_json::Value>,
        worldline_scores: Option<serde_json::Value>,
        path_execution: Option<serde_json::Value>,
        next_gen_execution: Option<serde_json::Value>,
    }

    let records = db
        .list_knowledge_by_prefix(&format!("evo:shadow:{session_id}:"))
        .await?;
    let mut runs = std::collections::BTreeMap::<String, EvolutionRun>::new();

    for record in records {
        let Some(value) = serde_json::from_str::<serde_json::Value>(&record.value).ok() else {
            continue;
        };
        if record.key.ends_with(":latest") {
            continue;
        }
        let mut parts = record.key.rsplitn(2, ':');
        let Some(kind) = parts.next() else {
            continue;
        };
        let Some(run_key) = parts.next() else {
            continue;
        };
        let run = runs.entry(run_key.to_string()).or_default();
        match kind {
            "summary" => run.summary = Some(value),
            "board" => run.board = Some(value),
            "candidate-graphs" => run.candidate_graphs = Some(value),
            "worldline-scores" => run.worldline_scores = Some(value),
            "path-execution" => run.path_execution = Some(value),
            "next-gen-execution" => run.next_gen_execution = Some(value),
            _ => {}
        }
    }

    let mut explain = Vec::<serde_json::Value>::new();
    for (run_key, run) in runs {
        let summary = run.summary.unwrap_or_else(|| serde_json::json!({}));
        let board = run.board.unwrap_or_else(|| serde_json::json!({}));
        let path_execution = run.path_execution.unwrap_or_else(|| serde_json::json!({}));
        let next_gen_execution = run.next_gen_execution.unwrap_or_else(|| serde_json::json!({}));
        let candidate_graphs = run.candidate_graphs.unwrap_or_else(|| serde_json::json!({}));
        let worldline_scores = run.worldline_scores.unwrap_or_else(|| serde_json::json!({}));

        let trace_from_summary = summary
            .get("reality")
            .and_then(|item| item.get("trace_id"))
            .and_then(serde_json::Value::as_str);
        if let Some(trace_filter) = trace_id {
            let matches = trace_from_summary
                .map(|item| item == trace_filter)
                .unwrap_or_else(|| run_key.contains(trace_filter));
            if !matches {
                continue;
            }
        }

        let board_decision = summary
            .get("board_decision")
            .cloned()
            .or_else(|| board.get("decision").cloned())
            .unwrap_or(serde_json::Value::Null);
        let candidate = summary
            .get("recommendation")
            .and_then(|item| item.get("recommended_candidate_id"))
            .cloned()
            .or_else(|| {
                summary
                    .get("candidates")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|items| items.first())
                    .and_then(|item| item.get("candidate_id"))
                    .cloned()
            })
            .unwrap_or(serde_json::Value::Null);
        let path_hit = path_execution
            .get("path")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let path_status = path_execution
            .get("status")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let reject_reason = path_execution
            .get("reason")
            .cloned()
            .or_else(|| {
                path_execution
                    .get("runtime_gate")
                    .and_then(|item| item.get("reason"))
                    .cloned()
            })
            .or_else(|| {
                path_execution
                    .get("runtime_gate")
                    .and_then(|item| item.get("error"))
                    .cloned()
            })
            .or_else(|| path_execution.get("error").cloned())
            .unwrap_or(serde_json::Value::Null);

        let recommendation_reasons = summary
            .get("recommendation")
            .and_then(|item| item.get("reasons"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();

        let top_candidate_reason = recommendation_reasons
            .iter()
            .filter_map(serde_json::Value::as_str)
            .find_map(|reason| reason.strip_prefix("top_candidate_reason=").map(str::to_string))
            .or_else(|| {
                let candidate_id = candidate.as_str()?;
                worldline_scores
                    .get("scores")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|items| {
                        items
                            .iter()
                            .find(|item| {
                                item.get("candidate_id")
                                    .and_then(serde_json::Value::as_str)
                                    .map(|id| id == candidate_id)
                                    .unwrap_or(false)
                            })
                            .and_then(|item| item.get("reasons"))
                            .and_then(serde_json::Value::as_array)
                            .and_then(|reasons| reasons.first())
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string)
                    })
            })
            .unwrap_or_else(|| "reason=unavailable".to_string());

        let worldline_weights_version = worldline_scores
            .get("scores")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("reasons"))
            .and_then(serde_json::Value::as_array)
            .and_then(|reasons| reasons.first())
            .and_then(serde_json::Value::as_str)
            .and_then(|reason| reason.split(';').next())
            .and_then(|fragment| fragment.strip_prefix("weights_version="))
            .map(str::to_string);

        let mismatch_explainer = recommendation_reasons
            .iter()
            .filter_map(serde_json::Value::as_str)
            .find(|reason| reason.contains("replay_mismatch_rate"))
            .map(str::to_string)
            .or_else(|| reject_reason.as_str().map(|item| format!("reject_reason={item}")));

        let created_at_ms = run_key
            .rsplit(':')
            .next()
            .and_then(|item| item.parse::<u64>().ok())
            .or_else(|| extract_ts(&summary))
            .unwrap_or(0);

        explain.push(serde_json::json!({
            "run_key": run_key,
            "trace_id": trace_from_summary,
            "candidate": candidate,
            "board_decision": board_decision,
            "path_hit": path_hit,
            "path_status": path_status,
            "reject_reason": reject_reason,
            "path_execution": path_execution,
            "next_gen_execution": next_gen_execution,
            "candidate_graphs": candidate_graphs,
            "worldline_scores": worldline_scores,
            "recommendation_reasons": recommendation_reasons,
            "top_candidate_reason": top_candidate_reason,
            "worldline_weights_version": worldline_weights_version,
            "mismatch_explainer": mismatch_explainer,
            "created_at_ms": created_at_ms,
        }));
    }

    explain.sort_by_key(|item| {
        item.get("created_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    Ok(explain)
}

async fn collect_signal_pipeline_explain(
    db: &StateStore,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<Vec<serde_json::Value>> {
    let mut explain = db
        .list_knowledge_by_prefix(&format!("observability:signal-explain:{session_id}:"))
        .await?
        .into_iter()
        .filter(|record| !record.key.ends_with(":latest"))
        .filter_map(record_to_json)
        .collect::<Vec<_>>();
    if let Some(trace_id) = trace_id {
        explain.retain(|item| trace_matches(item, trace_id));
    }
    explain.sort_by_key(|item| {
        item.get("created_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    Ok(explain)
}

fn collect_records_as_values(
    mut records: Vec<autoloop_state_adapter::KnowledgeRecord>,
    limit: usize,
) -> Vec<serde_json::Value> {
    records.sort_by(|left, right| left.key.cmp(&right.key));
    records
        .into_iter()
        .take(limit)
        .map(|record| {
            let parsed = serde_json::from_str::<serde_json::Value>(&record.value)
                .unwrap_or_else(|_| serde_json::json!(record.value));
            serde_json::json!({
                "key": record.key,
                "source": record.source,
                "value": parsed,
            })
        })
        .collect()
}

fn record_to_json(
    record: autoloop_state_adapter::KnowledgeRecord,
) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(&record.value)
        .ok()
        .map(|value| {
            serde_json::json!({
                "key": record.key,
                "source": record.source,
                "value": value,
            })
        })
}

fn trace_matches(value: &serde_json::Value, trace_id: &str) -> bool {
    value
        .get("trace_id")
        .and_then(serde_json::Value::as_str)
        .map(|candidate| candidate == trace_id)
        .or_else(|| {
            value
                .get("value")
                .and_then(|inner| inner.get("trace_id"))
                .and_then(serde_json::Value::as_str)
                .map(|candidate| candidate == trace_id)
        })
        .unwrap_or(false)
}

async fn get_value(db: &StateStore, key: &str) -> Result<Option<serde_json::Value>> {
    Ok(db
        .get_knowledge(key)
        .await?
        .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok()))
}

fn build_mismatch_explain(
    report: &ReplayAnalysisReport,
    plugin_traces: &[serde_json::Value],
) -> ReplayMismatchExplainRecord {
    let category = report
        .notes
        .iter()
        .find_map(|note| note.strip_prefix("mismatch_category=").map(str::to_string))
        .unwrap_or_else(|| "unknown".into());

    let mut causing_plugins = infer_causing_plugins(report, plugin_traces);
    causing_plugins.sort();
    causing_plugins.dedup();

    let summary_base = report
        .notes
        .iter()
        .find(|note| note.contains("mismatch"))
        .cloned()
        .or_else(|| {
            report
                .deviations
                .first()
                .map(|item| item.explanation.clone())
        })
        .unwrap_or_else(|| "replay mismatch detected".into());
    let summary = if causing_plugins.is_empty() {
        summary_base
    } else {
        format!(
            "{} | plugin_cause={}",
            summary_base,
            causing_plugins.join(",")
        )
    };

    let deviations = report
        .deviations
        .iter()
        .map(|item| {
            format!(
                "{} expected={} actual={} severity={}",
                item.field, item.expected, item.actual, item.severity
            )
        })
        .collect::<Vec<_>>();
    ReplayMismatchExplainRecord {
        snapshot_id: report.snapshot_id.clone(),
        trace_id: report.trace_id.clone(),
        category,
        summary,
        deterministic_boundary_respected: report.deterministic_boundary_respected,
        deviations,
        causing_plugins,
        created_at_ms: report.created_at_ms,
    }
}

fn infer_causing_plugins(
    report: &ReplayAnalysisReport,
    plugin_traces: &[serde_json::Value],
) -> Vec<String> {
    let mut plugins = report
        .notes
        .iter()
        .filter_map(|note| note.strip_prefix("plugin_cause="))
        .flat_map(|csv| csv.split(','))
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();

    if plugins.is_empty() {
        for deviation in &report.deviations {
            let field = deviation.field.to_ascii_lowercase();
            if field.contains("boundary") || field.contains("compaction") {
                plugins.push("plugin:optimizer".into());
            } else if field.contains("token") || field.contains("budget") {
                plugins.push("plugin:context-constraint".into());
            } else if field.contains("output") || field.contains("fingerprint") {
                plugins.push("plugin:proof".into());
            }
        }
    }

    if plugins.is_empty() {
        for trace in plugin_traces {
            let trace_id = trace
                .get("value")
                .and_then(|v| v.get("trace_id"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| trace.get("trace_id").and_then(serde_json::Value::as_str));
            if trace_id != Some(report.trace_id.as_str()) {
                continue;
            }
            if let Some(items) = trace
                .get("value")
                .and_then(|v| v.get("plugin_execution_traces"))
                .and_then(serde_json::Value::as_array)
            {
                for item in items {
                    if let Some(plugin_id) =
                        item.get("plugin_id").and_then(serde_json::Value::as_str)
                    {
                        plugins.push(plugin_id.to_string());
                    }
                }
            }
        }
    }

    plugins
}

#[derive(Debug, Clone)]
struct QueryRoutePolicy {
    retrieval_criteria: Vec<String>,
    multilingual: bool,
    enable_graph: bool,
    policy_ref: String,
}

async fn resolve_query_route_policy(
    db: &StateStore,
    lease: Option<&SessionLease>,
) -> Result<QueryRoutePolicy> {
    let tenant_id = lease
        .map(|item| item.tenant_id.as_str())
        .unwrap_or("tenant-default");
    let policy_ref = format!("project:{tenant_id}:memory-policy");
    let mut policy = ProjectMemoryPolicy::default();

    if let Some(record) = db.get_knowledge(&policy_ref).await? {
        if let Ok(decoded) = serde_json::from_str::<ProjectMemoryPolicy>(&record.value) {
            policy = decoded;
        } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) {
            let retrieval_criteria = value
                .get("retrieval_criteria")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(Vec::new);
            if !retrieval_criteria.is_empty() {
                policy.retrieval_criteria = retrieval_criteria;
            }
            if let Some(multilingual) = value
                .get("multilingual")
                .and_then(serde_json::Value::as_bool)
            {
                policy.multilingual = multilingual;
            }
            if let Some(enable_graph) = value
                .get("enable_graph")
                .and_then(serde_json::Value::as_bool)
            {
                policy.enable_graph = enable_graph;
            }
        }
    }

    let mut retrieval_criteria = policy
        .retrieval_criteria
        .into_iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    retrieval_criteria.sort();
    retrieval_criteria.dedup();
    if retrieval_criteria.is_empty() {
        retrieval_criteria = vec!["relevance".into(), "recency".into(), "evidence".into()];
    }

    Ok(QueryRoutePolicy {
        retrieval_criteria,
        multilingual: policy.multilingual,
        enable_graph: policy.enable_graph,
        policy_ref,
    })
}

fn apply_policy_to_metrics(
    mut metrics: serde_json::Value,
    policy: &QueryRoutePolicy,
) -> serde_json::Value {
    if let Some(object) = metrics.as_object_mut() {
        object.insert(
            "query_routing_policy".into(),
            serde_json::json!({
                "policy_ref": policy.policy_ref,
                "retrieval_criteria": policy.retrieval_criteria,
                "multilingual": policy.multilingual,
                "enable_graph": policy.enable_graph,
            }),
        );
    }
    metrics
}

fn apply_policy_to_ledger(
    mut ledger: serde_json::Value,
    policy: &QueryRoutePolicy,
    trace_id: Option<&str>,
) -> serde_json::Value {
    let keys = [
        "stage_chain",
        "step_chain",
        "budget_ledger",
        "approval_records",
        "replay_fingerprints",
        "evidence_tags",
        "memory_history",
        "memory_subledger",
        "relation_evidence_refs",
        "relation_write_proofs",
        "spend_ledger",
        "cost_attribution",
    ];
    if let Some(object) = ledger.as_object_mut() {
        for key in keys {
            if let Some(value) = object.get(key).cloned() {
                object.insert(
                    key.to_string(),
                    apply_policy_to_array(value, policy, trace_id, Some(key)),
                );
            }
        }
    }
    ledger
}

fn apply_policy_to_graph(
    mut graph: serde_json::Value,
    policy: &QueryRoutePolicy,
) -> serde_json::Value {
    if !policy.enable_graph {
        if let Some(object) = graph.as_object_mut() {
            object.insert("session_graph_snapshot".into(), serde_json::Value::Null);
            object.insert("global_graph_snapshot".into(), serde_json::Value::Null);
            object.insert(
                "org_knowledge_updates".into(),
                serde_json::Value::Array(Vec::new()),
            );
            object.insert(
                "graph_disabled_by_policy".into(),
                serde_json::json!({
                    "policy_ref": policy.policy_ref,
                    "reason": "project memory policy enable_graph=false",
                }),
            );
        }
    }
    graph
}

fn apply_policy_to_replay(
    mut replay: serde_json::Value,
    policy: &QueryRoutePolicy,
    trace_id: Option<&str>,
) -> serde_json::Value {
    let keys = [
        "snapshots",
        "analysis_reports",
        "plugin_execution_traces",
        "mismatch_explain",
        "evolution_explain",
        "signal_pipeline_explain",
    ];
    if let Some(object) = replay.as_object_mut() {
        for key in keys {
            if let Some(value) = object.get(key).cloned() {
                object.insert(
                    key.to_string(),
                    apply_policy_to_array(value, policy, trace_id, Some(key)),
                );
            }
        }
    }
    replay
}

fn apply_policy_to_array(
    value: serde_json::Value,
    policy: &QueryRoutePolicy,
    trace_id: Option<&str>,
    bucket: Option<&str>,
) -> serde_json::Value {
    let Some(items) = value.as_array() else {
        return value;
    };
    let now = current_time_ms();
    let mut scored = items
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, item)| {
            let score = score_query_item(&item, index, now, policy, trace_id, bucket);
            (score, item)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    serde_json::Value::Array(scored.into_iter().map(|(_, item)| item).collect())
}

fn score_query_item(
    item: &serde_json::Value,
    index: usize,
    now_ms: u64,
    policy: &QueryRoutePolicy,
    trace_id: Option<&str>,
    bucket: Option<&str>,
) -> f64 {
    let criteria = &policy.retrieval_criteria;
    let relevance_on = criteria.iter().any(|item| item.contains("relevance"));
    let recency_on = criteria.iter().any(|item| item.contains("recency"));
    let evidence_on = criteria.iter().any(|item| item.contains("evidence"));

    let mut score = 0.0_f64;
    if relevance_on {
        score += 1.0;
        if let Some(trace_id) = trace_id {
            if trace_matches(item, trace_id) || item.to_string().contains(trace_id) {
                score += 4.0;
            }
        }
        if let Some(bucket) = bucket {
            if bucket.contains("mismatch") && item.to_string().contains("mismatch") {
                score += 2.0;
            }
        }
    }

    if recency_on {
        score += 0.5;
        if let Some(ts) = extract_ts(item) {
            let age = now_ms.saturating_sub(ts);
            let freshness = (1_000_000_000_u64.saturating_sub(age.min(1_000_000_000)) as f64)
                / 1_000_000_000_f64;
            score += freshness * 3.0;
        } else {
            let decay = 1.0_f64 / ((index + 1) as f64);
            score += decay;
        }
    }

    if evidence_on {
        let raw = item.to_string();
        if raw.contains("evidence")
            || raw.contains("admission_evidence_ref")
            || raw.contains("guard_evidence_ref")
            || raw.contains("policy-reject")
        {
            score += 3.0;
        }
    }

    if !policy.multilingual && contains_non_ascii(item) {
        score *= 0.82;
    }

    score
}

fn extract_ts(value: &serde_json::Value) -> Option<u64> {
    const KEYS: [&str; 6] = [
        "created_at_ms",
        "updated_at_ms",
        "generated_at_ms",
        "injected_at_ms",
        "queued_at_ms",
        "fired_at_ms",
    ];
    for key in KEYS {
        if let Some(ts) = value.get(key).and_then(serde_json::Value::as_u64) {
            return Some(ts);
        }
        if let Some(ts) = value
            .get("value")
            .and_then(|inner| inner.get(key))
            .and_then(serde_json::Value::as_u64)
        {
            return Some(ts);
        }
    }
    None
}

fn contains_non_ascii(value: &serde_json::Value) -> bool {
    value.to_string().chars().any(|ch| !ch.is_ascii())
}
#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{
        StateStoreBackend, StateStoreConfig, SpendLedger, SpendLedgerKind,
    };

    use crate::config::SignalPipelineConfig;
    use crate::contracts::relation::{
        RelationEdgeType, RelationEventType,
    };
    use crate::contracts::signal::{SignalContext, SignalDecision, SignalEvent, SignalKind};
    use crate::observability::event_stream::{ReplayDeviation, persist_replay_analysis};
    use crate::observability::SignalFacade;

    #[tokio::test]
    async fn query_plane_aggregates_mismatch_explanations() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-plane";
        let trace_id = "trace-query-plane";
        let account_id = "tenant-qp:principal-qp:policy-qp";

        db.upsert_json_knowledge(
            format!("identity:session-lease:{session_id}"),
            &SessionLease {
                lease_token: "lease-query-plane".into(),
                session_id: session_id.into(),
                tenant_id: "tenant-qp".into(),
                principal_id: "principal-qp".into(),
                policy_id: "policy-qp".into(),
                expires_at_ms: current_time_ms() + 60_000,
                issued_at_ms: current_time_ms(),
            },
            "identity",
        )
        .await
        .expect("seed lease");

        db.append_spend_ledger(SpendLedger {
            ledger_id: "ledger-qp-1".into(),
            tenant_id: "tenant-qp".into(),
            account_id: account_id.into(),
            session_id: session_id.into(),
            trace_id: trace_id.into(),
            task_id: "task-qp".into(),
            capability_id: "mcp::local-mcp::invoke".into(),
            kind: SpendLedgerKind::Settle,
            amount_micros: 420,
            token_cost_micros: 300,
            tool_cost_micros: 80,
            duration_cost_micros: 40,
            reason: "settled".into(),
            created_at_ms: current_time_ms(),
        })
        .await
        .expect("seed spend ledger");

        db.upsert_json_knowledge(
            format!("graph:{session_id}:snapshot"),
            &serde_json::json!({
                "entities": [{"canonical_name":"AutoLoop"}],
                "relationships": [{"relation_type":"USES"}],
            }),
            "graph-rag",
        )
        .await
        .expect("seed graph");
        db.upsert_json_knowledge(
            format!("observability:{session_id}:route-analytics"),
            &serde_json::json!({"total_reports": 1, "guarded_reports": 0}),
            "observability",
        )
        .await
        .expect("seed metrics");
        db.upsert_json_knowledge(
            format!("observability:{session_id}:trace:1"),
            &serde_json::json!({
                "trace_id": trace_id,
                "span_name": "swarm.completed",
            }),
            "observability",
        )
        .await
        .expect("seed trace");
        db.upsert_json_knowledge(
            format!("evidence:stage:{session_id}:{trace_id}:1:execution"),
            &serde_json::json!({
                "trace_id": trace_id,
                "stage": "execution",
            }),
            "evidence-ledger",
        )
        .await
        .expect("seed evidence");

        db.upsert_json_knowledge(
            format!("memory:supermemory:history:{session_id}:job-qp:history-1"),
            &serde_json::json!({
                "history_id": "history-1",
                "session_id": session_id,
                "job_id": "job-qp",
                "trace_id": trace_id,
                "memory_id": "memory-qp-1",
                "old_memory": null,
                "new_memory": "policy replay evidence",
                "event": "ADD",
                "actor_id": "planner-agent",
                "role": "planner",
                "is_deleted": false,
                "created_at_ms": current_time_ms(),
                "updated_at_ms": current_time_ms(),
            }),
            "supermemory-kernel",
        )
        .await
        .expect("seed memory history");

        db.upsert_json_knowledge(
            format!("evidence:memory:{session_id}:history-1"),
            &serde_json::json!({
                "history_id": "history-1",
                "trace_id": trace_id,
                "event": "ADD",
                "actor_id": "planner-agent",
                "role": "planner",
            }),
            "evidence-ledger",
        )
        .await
        .expect("seed memory evidence");
        append_event(
            &db,
            "task_runs",
            trace_id,
            session_id.to_string(),
            Some("task-qp".into()),
            Some("mcp::local-mcp::invoke".into()),
            CONTRACT_VERSION,
            serde_json::json!({
                "decision": "Allow",
                "reason": "seed event",
            }),
        )
        .await
        .expect("seed event");

        persist_replay_analysis(
            &db,
            &ReplayAnalysisReport {
                snapshot_id: "replay:snapshot:qp".into(),
                session_id: session_id.into(),
                trace_id: trace_id.into(),
                replay_output_digest: "actual-digest".into(),
                matched: false,
                deterministic_boundary_respected: false,
                deviations: vec![ReplayDeviation {
                    field: "output_digest".into(),
                    expected: "expected-digest".into(),
                    actual: "actual-digest".into(),
                    severity: "high".into(),
                    explanation: "external dependency changed".into(),
                }],
                notes: vec![
                    "mismatch_category=fingerprint_mismatch".into(),
                    "replay mismatch under boundary=best_effort".into(),
                ],
                created_at_ms: current_time_ms(),
            },
        )
        .await
        .expect("seed replay analysis");

        db.upsert_json_knowledge(
            format!("replay:plugin-trace:{session_id}:1"),
            &serde_json::json!({
                "trace_id": trace_id,
                "plugin_execution_traces": [{
                    "plugin_id": "ctx-optimizer",
                    "plugin_kind": "optimizer",
                    "phase": "optimize",
                    "verdict": "pass",
                    "reason": "compiled"
                }]
            }),
            "query-engine",
        )
        .await
        .expect("seed plugin trace");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        assert!(
            view.metrics
                .get("route_analytics")
                .is_some_and(|value| !value.is_null())
        );
        assert_eq!(
            view.events
                .as_array()
                .map(|items| items.len())
                .unwrap_or_default(),
            1
        );
        let mismatch_items = view
            .replay
            .get("mismatch_explain")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(mismatch_items.len(), 1);
        let causing_plugins_len = mismatch_items
            .first()
            .and_then(|item| item.get("causing_plugins"))
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or_default();
        assert_eq!(causing_plugins_len, 1);
        assert!(
            view.replay
                .get("plugin_execution_traces")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
        let memory_history_len = view
            .ledger
            .get("memory_history")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or_default();
        let memory_subledger_len = view
            .ledger
            .get("memory_subledger")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or_default();
        assert!(memory_history_len >= 1);
        assert!(memory_subledger_len >= 1);

        let persisted = db
            .get_knowledge(&format!("observability:{session_id}:query-plane"))
            .await
            .expect("query read")
            .expect("query exists");
        assert!(persisted.value.contains("mismatch_explain"));
    }
    #[tokio::test]
    async fn query_plane_surfaces_evolution_decision_path_and_reject_reason() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-evolution";
        let trace_id = "trace-query-evolution";

        db.upsert_json_knowledge(
            format!("evo:shadow:{session_id}:process_direct:1000:summary"),
            &serde_json::json!({
                "reality": {"trace_id": trace_id},
                "board_decision": "PROMOTE_RUNTIME_UPDATE",
                "recommendation": {
                    "recommended_candidate_id": "candidate-42",
                    "reasons": [
                        "selected highest total_score candidate `candidate-42`",
                        "top_candidate_reason=weights_version=worldline-weights:hot-v1;success=1.20"
                    ]
                },
                "candidates": [{"candidate_id": "candidate-42"}],
            }),
            "evolution-os-shadow",
        )
        .await
        .expect("seed evolution summary");

        db.upsert_json_knowledge(
            format!("evo:shadow:{session_id}:process_direct:1000:worldline-scores"),
            &serde_json::json!({
                "scores": [{
                    "candidate_id": "candidate-42",
                    "reasons": [
                        "weights_version=worldline-weights:hot-v1;success=1.20,robustness=1.10,reuse=1.00,verifier=1.00,cost=1.00,latency=1.00,risk=1.00,instability=1.00,governance=1.00",
                        "positive=2.0000;negative=0.8000;retry_total=1;replay_mismatch_rate=0.030;verifier=0.900"
                    ]
                }]
            }),
            "evolution-os-shadow",
        )
        .await
        .expect("seed worldline scores");

        db.upsert_json_knowledge(
            format!("evo:shadow:{session_id}:process_direct:1000:path-execution"),
            &serde_json::json!({
                "path": "9A",
                "status": "rolled_back",
                "runtime_gate": {"verified": false, "reason": "signature verification failed"}
            }),
            "evolution-os-shadow",
        )
        .await
        .expect("seed evolution path execution");

        db.upsert_json_knowledge(
            format!("evo:shadow:{session_id}:process_direct:1000:next-gen-execution"),
            &serde_json::json!({
                "status": "rolled_back",
                "execution_mode": "canary"
            }),
            "evolution-os-shadow",
        )
        .await
        .expect("seed evolution next-gen");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        let explain = view
            .replay
            .get("evolution_explain")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(explain.len(), 1);
        let first = explain.first().cloned().unwrap_or_else(|| serde_json::json!({}));
        assert_eq!(
            first.get("candidate").and_then(serde_json::Value::as_str),
            Some("candidate-42")
        );
        assert_eq!(
            first
                .get("board_decision")
                .and_then(serde_json::Value::as_str),
            Some("PROMOTE_RUNTIME_UPDATE")
        );
        assert_eq!(
            first.get("path_hit").and_then(serde_json::Value::as_str),
            Some("9A")
        );
        assert_eq!(
            first
                .get("reject_reason")
                .and_then(serde_json::Value::as_str),
            Some("signature verification failed")
        );
        assert_eq!(
            first
                .get("top_candidate_reason")
                .and_then(serde_json::Value::as_str),
            Some("weights_version=worldline-weights:hot-v1;success=1.20")
        );
        assert_eq!(
            first
                .get("worldline_weights_version")
                .and_then(serde_json::Value::as_str),
            Some("worldline-weights:hot-v1")
        );
        assert!(
            first
                .get("mismatch_explainer")
                .and_then(serde_json::Value::as_str)
                .is_some()
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_context_objective_weights_and_breakdown() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-objective";
        let trace_id = "trace-query-objective";

        db.upsert_json_knowledge(
            "policy:context-objective-weights:latest".to_string(),
            &serde_json::json!({
                "weights": {
                    "task_utility": 1.2,
                    "distortion_penalty": 1.1,
                    "attention_mismatch_penalty": 0.9,
                    "token_cost_penalty": 1.4
                },
                "updated_at_ms": current_time_ms(),
                "reason": "test",
            }),
            "policy-engine",
        )
        .await
        .expect("seed objective weights");

        db.upsert_json_knowledge(
            format!("replay:plugin-trace:{session_id}:1"),
            &serde_json::json!({
                "trace_id": trace_id,
                "annotation_proof": {
                    "objective": {
                        "task_utility": 1.2,
                        "distortion_penalty": 1.1,
                        "attention_mismatch_penalty": 0.9,
                        "token_cost_penalty": 1.4,
                        "score": 0.82
                    },
                    "annotation": {
                        "decision_summary": "hardgate_pass_then_optimize"
                    }
                }
            }),
            "query-engine",
        )
        .await
        .expect("seed plugin trace objective");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        let weights = view
            .metrics
            .get("context_objective_weights")
            .and_then(|value| value.get("weights"))
            .expect("context objective weights present");
        assert_eq!(
            weights
                .get("task_utility")
                .and_then(serde_json::Value::as_f64),
            Some(1.2)
        );
        let breakdown = view
            .metrics
            .get("context_objective_breakdown")
            .expect("context objective breakdown present");
        assert_eq!(
            breakdown
                .get("latest_decision_summary")
                .and_then(serde_json::Value::as_str),
            Some("hardgate_pass_then_optimize")
        );
        assert!(
            breakdown
                .get("latest_objective")
                .is_some_and(|value| !value.is_null())
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_context_compiler_whitebox_fields() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-whitebox";
        let trace_id = "trace-query-whitebox";

        db.upsert_json_knowledge(
            format!("replay:plugin-trace:{session_id}:1"),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "hardgate_pass_token": "hgt:v2:test",
                "constraint_version": "constraint-v2",
                "constraint_ids": ["constraint.budget.non_zero"],
                "annotation_proof": {
                    "annotation": {
                        "prompt_pack": {
                            "message_count": 2,
                            "estimated_input_tokens": 64
                        },
                        "dropped_mapping": [{"kind": "message_span", "dropped_count": 1}],
                        "source_mapping": [{"compiled_index": 0, "source_ref": "prompt_pack:user:0"}],
                        "risk_metadata": {
                            "risk_labels": ["low"],
                            "semantic_distortion_risk": 0.1,
                            "attention_mismatch_risk": 0.1,
                            "malicious_intent_risk": 0.0
                        },
                        "decision_summary": {
                            "hardgate_pass_token": "hgt:v2:test"
                        },
                        "replay_fingerprint": "fp:test"
                    }
                }
            }),
            "query-engine",
        )
        .await
        .expect("seed whitebox trace");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        let metrics_whitebox = view
            .metrics
            .get("context_compiler_whitebox")
            .expect("metrics context compiler whitebox present");
        assert!(
            metrics_whitebox
                .get("latest")
                .and_then(|latest| latest.get("prompt_pack"))
                .is_some_and(|value| !value.is_null())
        );
        assert!(
            metrics_whitebox
                .get("latest")
                .and_then(|latest| latest.get("source_mapping"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );

        let replay_whitebox = view
            .replay
            .get("context_compiler_whitebox")
            .expect("replay context compiler whitebox present");
        assert!(
            replay_whitebox
                .get("latest")
                .and_then(|latest| latest.get("risk_metadata"))
                .is_some_and(|value| !value.is_null())
        );
    }
    #[tokio::test]
    async fn query_plane_policy_controls_graph_and_routing_surface() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-policy";
        let tenant_id = "tenant-policy";

        db.upsert_json_knowledge(
            format!("identity:session-lease:{session_id}"),
            &SessionLease {
                lease_token: "lease-query-policy".into(),
                session_id: session_id.into(),
                tenant_id: tenant_id.into(),
                principal_id: "principal-policy".into(),
                policy_id: "policy-policy".into(),
                expires_at_ms: current_time_ms() + 60_000,
                issued_at_ms: current_time_ms(),
            },
            "identity",
        )
        .await
        .expect("seed lease");

        db.upsert_json_knowledge(
            format!("project:{tenant_id}:memory-policy"),
            &serde_json::json!({
                "retrieval_criteria": ["evidence", "recency", "relevance"],
                "multilingual": false,
                "enable_graph": false
            }),
            "project-policy",
        )
        .await
        .expect("seed project policy");

        db.upsert_json_knowledge(
            format!("graph:{session_id}:snapshot"),
            &serde_json::json!({"entities":[{"canonical_name":"HiddenGraph"}]}),
            "graph-rag",
        )
        .await
        .expect("seed graph");

        db.upsert_json_knowledge(
            format!("evidence:stage:{session_id}:trace-policy:1:admission"),
            &serde_json::json!({
                "trace_id": "trace-policy",
                "created_at_ms": current_time_ms(),
                "admission_evidence_ref": "admission:1"
            }),
            "evidence-ledger",
        )
        .await
        .expect("seed evidence stage");

        let view = persist_unified_query_view(&db, session_id, Some("trace-policy"))
            .await
            .expect("query plane");

        let routing = view
            .metrics
            .get("query_routing_policy")
            .expect("routing policy present");
        assert_eq!(
            routing
                .get("enable_graph")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            routing
                .get("multilingual")
                .and_then(serde_json::Value::as_bool),
            Some(false)
        );

        assert!(
            view.graph
                .get("session_graph_snapshot")
                .is_some_and(serde_json::Value::is_null)
        );
        assert_eq!(
            view.graph
                .get("org_knowledge_updates")
                .and_then(serde_json::Value::as_array)
                .map(|items| items.len())
                .unwrap_or_default(),
            0
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_foundry_feedback_summary_and_evidence() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-foundry";
        let trace_id = "trace-query-foundry";

        db.upsert_json_knowledge(
            format!("foundry:feedback:{session_id}:{trace_id}:1:hit"),
            &serde_json::json!({
                "event_id":"foundry-feedback:route:hit:1",
                "session_id":session_id,
                "trace_id":trace_id,
                "operation":"route",
                "kind":"hit",
                "message":"ok",
                "created_at_ms": current_time_ms(),
            }),
            "foundry-feedback",
        )
        .await
        .expect("seed foundry feedback");

        db.upsert_json_knowledge(
            format!("evidence:foundry:{session_id}:1:foundry-feedback:route:hit:1"),
            &serde_json::json!({
                "event_id":"foundry-feedback:route:hit:1",
                "session_id":session_id,
                "trace_id":trace_id,
                "operation":"route",
                "kind":"hit",
                "detail":"ok",
                "created_at_ms": current_time_ms(),
            }),
            "evidence-ledger",
        )
        .await
        .expect("seed foundry evidence");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        let summary = view
            .metrics
            .get("foundry_feedback_summary")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        assert_eq!(
            summary.get("hit").and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            view.ledger
                .get("foundry_feedback_evidence")
                .and_then(serde_json::Value::as_array)
                .map(|items| items.len())
                .unwrap_or_default(),
            1
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_learning_signal_rejection_reasons() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-learning-reject";

        db.upsert_json_knowledge(
            format!("evidence:memory:{session_id}:learning-signal-reject:1"),
            &serde_json::json!({
                "reject_id":"learning-signal-reject:1",
                "session_id":session_id,
                "trace_id":"trace:learning:reject",
                "source":"hooks.learning_pipeline",
                "target":"skill_registry.promote_skill_with_verdict",
                "reason":"learning_signal.missing_evidence_ref",
                "evidence_ref": serde_json::Value::Null,
                "created_at_ms": current_time_ms(),
            }),
            "learning-signal-guard",
        )
        .await
        .expect("seed learning signal reject");

        let view = persist_unified_query_view(&db, session_id, Some("trace:learning:reject"))
            .await
            .expect("query plane");
        let rejections = view
            .ledger
            .get("learning_signal_rejections")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        assert_eq!(
            rejections
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert_eq!(
            rejections
                .get("latest_reason")
                .and_then(serde_json::Value::as_str),
            Some("learning_signal.missing_evidence_ref")
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_graph_health_summary_and_refs() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-graph-health";

        db.upsert_json_knowledge(
            format!("memory:graph:health:{session_id}:1712345678901"),
            &serde_json::json!({
                "node_count": 4,
                "edge_count": 3,
                "hub_stub": [{"node":"a","in_degree":0,"out_degree":3,"total_degree":3}],
                "fragile_bridge": [{"node":"b","component_size":4}],
                "isolated_community": [],
                "orphan": ["d"]
            }),
            "view-plane",
        )
        .await
        .expect("seed graph health");
        db.upsert_json_knowledge(
            format!("memory:graph:health:{session_id}:latest"),
            &serde_json::json!({
                "ref": format!("memory:graph:health:{session_id}:1712345678901"),
                "generated_at_ms": 1712345678901_u64,
            }),
            "view-plane",
        )
        .await
        .expect("seed graph health latest");

        let view = persist_unified_query_view(&db, session_id, None)
            .await
            .expect("query plane");

        let graph_health = view
            .graph
            .get("graph_health")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        assert!(
            graph_health
                .get("summary")
                .is_some_and(|value| !value.is_null())
        );
        assert_eq!(
            graph_health
                .get("latest_ref")
                .and_then(serde_json::Value::as_str),
            Some("memory:graph:health:session-query-graph-health:1712345678901")
        );
        assert!(
            graph_health
                .get("refs")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            view.replay
                .get("graph_health")
                .and_then(|item| item.get("summary"))
                .is_some_and(|value| !value.is_null())
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_recall_route_fallback_evidence() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-recall-fallback";
        let trace_id = "trace-query-recall-fallback";

        db.upsert_json_knowledge(
            format!("memory:episode:{session_id}:{trace_id}:1712345678901:recall"),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "stage": "recall",
                "created_at_ms": 1712345678901_u64,
                "payload": {
                    "recall": {
                        "seed_hits": ["memory:lexical:single-hit", "memory:graph:health"],
                        "cjk_lexical_hits": [{
                            "bigram":"健康",
                            "candidate_ref":"memory:graph:health",
                            "reason":"cjk_bigram_fallback"
                        }],
                        "query_route_fallback": {
                            "reason":"fast_selector_fallback",
                            "lexical_hit_count": 1,
                            "fallback_selected_refs": ["memory:graph:health"],
                            "applied": true
                        }
                    }
                }
            }),
            "episode-ledger",
        )
        .await
        .expect("seed recall stage");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");
        let fallback = view
            .graph
            .get("recall_route_fallback")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        assert!(
            fallback
                .get("latest")
                .and_then(|item| item.get("query_route_fallback"))
                .and_then(|item| item.get("reason"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == "fast_selector_fallback")
        );
        assert!(
            fallback
                .get("latest")
                .and_then(|item| item.get("cjk_lexical_hits"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
    }
    #[tokio::test]
    async fn query_plane_surfaces_semantic_lint_four_sections() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-semantic-lint";
        let trace_id = "trace-query-semantic-lint";

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        let lint = view
            .replay
            .get("semantic_lint_report")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        assert!(
            lint.get("contradictions")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            lint.get("stale")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            lint.get("gaps")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            lint.get("depth")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
    }
    #[tokio::test]
    async fn query_plane_surfaces_unified_op_log_events() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-oplog";
        let trace_id = "trace-query-oplog";

        let ops = ["ingest", "query", "lint", "graph", "refresh"];
        for (idx, op) in ops.iter().enumerate() {
            db.upsert_json_knowledge(
                format!("evidence:op-log:{session_id}:{}:evt-{idx}", 1712345678900_u64 + idx as u64),
                &serde_json::json!({
                    "event_id": format!("evt-{idx}"),
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "op_type": op,
                    "status": "ok",
                    "emitted_at_ms": 1712345678900_u64 + idx as u64,
                    "metadata": {"source": "d5"},
                    "evidence_refs": [format!("evidence:stage:{session_id}:{trace_id}:{}:execution", 1712345678900_u64 + idx as u64)],
                }),
                "evidence-ledger",
            )
            .await
            .expect("seed op log");
        }

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");
        let op_log_events = view
            .ledger
            .get("op_log_events")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(op_log_events.len(), 5);

        let mut kinds = op_log_events
            .iter()
            .filter_map(|entry| {
                entry
                    .get("value")
                    .and_then(|value| value.get("op_type"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();
        kinds.sort();
        assert_eq!(
            kinds,
            vec![
                "graph".to_string(),
                "ingest".to_string(),
                "lint".to_string(),
                "query".to_string(),
                "refresh".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_relation_graph_and_evidence_refs() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-relation";
        let trace_id = "trace-query-relation";
        let now_ms = current_time_ms();
        db.upsert_json_knowledge(
            format!("relation:state:{session_id}:{trace_id}:{}:depends", now_ms),
            &serde_json::json!({
                "kind": "relation_edge",
                "session_id": session_id,
                "trace_id": trace_id,
                "evidence_ref": "relation:evidence:depends",
                "updated_at_ms": now_ms,
                "edge": {
                    "edge_id": "edge:depends:1",
                    "from_node": "task:relation:1",
                    "to_node": "capability:write_file",
                    "edge_type": format!("{:?}", RelationEdgeType::DependsOn).to_ascii_lowercase(),
                }
            }),
            "d7-test",
        )
        .await
        .expect("seed depends edge");
        db.upsert_json_knowledge(
            format!("relation:state:{session_id}:{trace_id}:{}:approved", now_ms + 1),
            &serde_json::json!({
                "kind": "relation_edge",
                "session_id": session_id,
                "trace_id": trace_id,
                "evidence_ref": "relation:evidence:approved",
                "updated_at_ms": now_ms + 1,
                "edge": {
                    "edge_id": "edge:approved:1",
                    "from_node": "task:relation:1",
                    "to_node": "policy:relation:v1",
                    "edge_type": format!("{:?}", RelationEdgeType::ApprovedBy).to_ascii_lowercase(),
                }
            }),
            "d7-test",
        )
        .await
        .expect("seed approved edge");
        db.upsert_json_knowledge(
            format!("relation:state:{session_id}:{trace_id}:{}:event", now_ms + 2),
            &serde_json::json!({
                "kind": "relation_event",
                "session_id": session_id,
                "trace_id": trace_id,
                "updated_at_ms": now_ms + 2,
                "event": {
                    "event_id": "event:relation:approved:1",
                    "event_type": format!("{:?}", RelationEventType::EdgeUpserted).to_ascii_lowercase(),
                    "edge_id": "edge:approved:1",
                    "evidence_ref": "relation:evidence:event",
                }
            }),
            "d7-test",
        )
        .await
        .expect("seed relation event");
        db.upsert_json_knowledge(
            format!("relation:hot-index:{session_id}:{}:edge", now_ms + 3),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "relation_kind": "edge",
                "relation_ref": format!("relation:state:{session_id}:{trace_id}:{}:approved", now_ms + 1),
                "score": 1.0,
                "payload": {"edge_id":"edge:approved:1"}
            }),
            "d7-test",
        )
        .await
        .expect("seed relation hot index");
        db.upsert_json_knowledge(
            format!("relation:evidence:{session_id}:{trace_id}:{}", now_ms + 4),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "evidence_ref": "relation:evidence:approved",
            }),
            "d7-test",
        )
        .await
        .expect("seed relation evidence");
        db.upsert_json_knowledge(
            format!("relation:write_proof:{session_id}:{trace_id}:{}", now_ms + 5),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "proof_id": "proof:relation:1",
                "op": "upsert_edge",
            }),
            "d7-test",
        )
        .await
        .expect("seed relation write proof");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        let relation_graph = view
            .graph
            .get("relation_graph")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let edge_count = relation_graph
            .get("edges_current")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        assert!(edge_count >= 2, "relation edges should be surfaced");

        let event_count = relation_graph
            .get("events_append_only")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        assert!(event_count >= 1, "relation events should be surfaced");

        let graph_evidence_count = relation_graph
            .get("evidence_refs")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        assert!(graph_evidence_count >= 1, "relation evidence refs should be surfaced");

        let ledger_relation_evidence_count = view
            .ledger
            .get("relation_evidence_refs")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        assert!(
            ledger_relation_evidence_count >= 1,
            "ledger should include relation_evidence_refs"
        );

        let ledger_relation_write_proof_count = view
            .ledger
            .get("relation_write_proofs")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        assert!(
            ledger_relation_write_proof_count >= 1,
            "ledger should include relation_write_proofs"
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_signal_pipeline_explain() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-signal-explain";
        let trace_id = "trace-query-signal-explain";
        let facade = SignalFacade::new(db.clone(), &SignalPipelineConfig::default());

        facade
            .emit(SignalEvent {
                signal_id: "signal:demo:1".into(),
                kind: SignalKind::Trace,
                name: "runtime.execute.start".into(),
                context: SignalContext {
                    session_id: session_id.into(),
                    trace_id: trace_id.into(),
                    span_id: Some("span:signal:1".into()),
                    task_id: Some("task:signal:1".into()),
                    capability_id: Some("tool:write_file".into()),
                    tenant_id: Some("tenant:test".into()),
                    principal_id: Some("operator:test".into()),
                },
                attributes: std::collections::BTreeMap::new(),
                numeric_value: None,
                body: Some("start".into()),
                decision: SignalDecision {
                    accepted: true,
                    reason: None,
                    evidence_ref: Some("evidence:signal:1".into()),
                },
                emitted_at_ms: current_time_ms(),
            })
            .await
            .expect("emit signal through facade");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");

        let items = view
            .replay
            .get("signal_pipeline_explain")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0]
                .get("value")
                .and_then(|value| value.get("signal_name"))
                .and_then(serde_json::Value::as_str),
            Some("runtime.execute.start")
        );
    }

    #[tokio::test]
    async fn query_plane_surfaces_harness_round_whitebox_patch_command_test_and_rollback() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-harness-whitebox";
        let trace_id = "trace-query-harness-whitebox";

        db.upsert_json_knowledge(
            format!("harness:patch-report:{session_id}:{trace_id}:1713000000001"),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "success": false,
                "rollback_performed": true,
                "steps": [],
                "evidence_ref": format!("evidence:stage:{session_id}:{trace_id}:1713000000005:execution"),
                "created_at_ms": 1713000000001_u64,
            }),
            "test",
        )
        .await
        .expect("seed patch report");

        db.upsert_json_knowledge(
            format!("harness:shell-loop:{session_id}:{trace_id}:1713000000002"),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "success": true,
                "iterations": [{
                    "step_id": "cmd-1",
                    "command": "cargo check",
                    "status": "passed"
                }],
                "halted_reason": "completed",
                "evidence_ref": format!("evidence:stage:{session_id}:{trace_id}:1713000000006:execution"),
                "created_at_ms": 1713000000002_u64,
            }),
            "test",
        )
        .await
        .expect("seed shell report");

        db.upsert_json_knowledge(
            format!("harness:test-verifier:{session_id}:{trace_id}:1713000000003"),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "hard_pass": true,
                "hard_fail": false,
                "results": [{
                    "runner_id": "build",
                    "kind": "build",
                    "verdict": "pass"
                }],
                "summary": "hard_pass: all required runners passed",
                "evidence_ref": format!("evidence:stage:{session_id}:{trace_id}:1713000000007:verify"),
                "created_at_ms": 1713000000003_u64,
            }),
            "test",
        )
        .await
        .expect("seed test report");

        db.upsert_json_knowledge(
            format!("harness:git-checkpoint:{session_id}:{trace_id}:1713000000004"),
            &serde_json::json!({
                "session_id": session_id,
                "trace_id": trace_id,
                "success": true,
                "halted_reason": "completed",
                "steps": [{
                    "action": "rollback",
                    "success": true
                }],
                "evidence_ref": format!("evidence:stage:{session_id}:{trace_id}:1713000000008:execution"),
                "created_at_ms": 1713000000004_u64,
            }),
            "test",
        )
        .await
        .expect("seed git checkpoint report");

        for idx in 5..=8 {
            db.upsert_json_knowledge(
                format!("evidence:stage:{session_id}:{trace_id}:171300000000{idx}:execution"),
                &serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "stage": if idx == 7 { "test_verifier" } else if idx == 8 { "git_checkpoint_layer" } else { "diff_patch_engine" },
                    "created_at_ms": 1713000000000_u64 + idx as u64,
                }),
                "evidence-ledger",
            )
            .await
            .expect("seed evidence stage");
        }

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");
        let whitebox = view
            .replay
            .get("harness_whitebox")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        assert_eq!(
            whitebox
                .get("round_count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        let latest = whitebox
            .get("latest")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        assert_eq!(
            latest
                .get("trace_id")
                .and_then(serde_json::Value::as_str),
            Some(trace_id)
        );
        assert!(
            latest
                .get("commands")
                .and_then(|item| item.get("value"))
                .is_some_and(|item| !item.is_null())
        );
        assert!(
            latest
                .get("tests")
                .and_then(|item| item.get("value"))
                .is_some_and(|item| !item.is_null())
        );
        let rollback_items = latest
            .get("rollback_evidence")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            rollback_items.iter().any(|item| {
                item.get("kind")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kind| kind == "patch_rollback")
            }),
            "patch rollback evidence should be surfaced"
        );
        assert!(
            rollback_items.iter().any(|item| {
                item.get("kind")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|kind| kind == "git_rollback")
            }),
            "git rollback evidence should be surfaced"
        );
        assert!(
            latest
                .get("evidence_refs")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
    }

    #[tokio::test]
    async fn query_plane_includes_cli_event_chain_records() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let session_id = "session-query-cli-chain";
        let trace_id = "trace-query-cli-chain";

        db.upsert_json_knowledge(
            format!("observability:cli-event:{session_id}:{}", 1712345678999_u64),
            &serde_json::json!({
                "kind": "cli.frontend.prompt.request",
                "trace_id": trace_id,
                "session_id": session_id,
                "payload": {
                    "content_len": 12,
                },
                "created_at_ms": 1712345678999_u64,
            }),
            "cli-observability",
        )
        .await
        .expect("seed cli event");

        let view = persist_unified_query_view(&db, session_id, Some(trace_id))
            .await
            .expect("query plane");
        let events = view
            .events
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(events.iter().any(|event| {
            event
                .get("value")
                .and_then(|value| value.get("kind"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "cli.frontend.prompt.request")
        }));
    }
}


