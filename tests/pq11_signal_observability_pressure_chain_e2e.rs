use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use autoloop::config::{AppConfig, SignalPipelineConfig};
use autoloop::contracts::signal::{SignalContext, SignalDecision, SignalEvent, SignalKind};
use autoloop::observability::{SignalFacade, event_stream::list_session_events};
use autoloop::AutoLoopApp;

fn pressure_event(
    session_id: &str,
    trace_id: &str,
    kind: SignalKind,
    idx: usize,
) -> SignalEvent {
    let name = match kind {
        SignalKind::Trace => "runtime.trace.step",
        SignalKind::Metric => "runtime.metric.latency_ms",
        SignalKind::Log => "runtime.log.tool",
    };
    let mut attributes = BTreeMap::new();
    attributes.insert("rate_limit_per_minute".into(), "10000".into());
    attributes.insert("sample_ratio".into(), "1.0".into());
    attributes.insert("pressure_run".into(), "pq11-d10".into());
    attributes.insert("chain_step".into(), idx.to_string());

    SignalEvent {
        signal_id: format!("signal:pressure:{idx}"),
        kind,
        name: name.into(),
        context: SignalContext {
            session_id: session_id.into(),
            trace_id: trace_id.into(),
            span_id: Some(format!("span:pressure:{idx}")),
            task_id: Some("task:signal:pressure".into()),
            capability_id: Some("capability:signal-facade".into()),
            tenant_id: Some("tenant:pq11".into()),
            principal_id: Some("principal:pq11".into()),
        },
        attributes,
        numeric_value: Some((idx % 97) as f64 + 0.5),
        body: Some(format!("pressure-body-{idx}")),
        decision: SignalDecision {
            accepted: true,
            reason: None,
            evidence_ref: Some(format!("evidence:seed:pressure:{idx}")),
        },
        emitted_at_ms: autoloop::orchestration::current_time_ms(),
    }
}

#[tokio::test]
async fn signal_observability_pressure_chain_is_facade_only_no_bypass() -> Result<()> {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq11-signal-pressure-chain";
    let trace_id = "trace:pq11:signal-pressure";
    let facade = SignalFacade::new(app.state_store().clone(), &SignalPipelineConfig::default());

    let mut events = Vec::new();
    let mut expected_trace = 0usize;
    let mut expected_metric = 0usize;
    let mut expected_log = 0usize;
    let total = 180usize;
    for idx in 0..total {
        let kind = match idx % 3 {
            0 => {
                expected_trace += 1;
                SignalKind::Trace
            }
            1 => {
                expected_metric += 1;
                SignalKind::Metric
            }
            _ => {
                expected_log += 1;
                SignalKind::Log
            }
        };
        events.push(pressure_event(session_id, trace_id, kind, idx));
    }

    let output = facade.emit_batch(events).await?;
    assert_eq!(output.requested, total);
    assert_eq!(output.flushed, total);
    assert_eq!(output.outputs.len(), total);
    assert!(
        output.outputs.iter().all(|item| item.accepted),
        "pressure batch should not be dropped in this test profile"
    );
    assert!(
        output
            .outputs
            .iter()
            .all(|item| item.query_explain_ref.is_some()),
        "every signal should emit query explain ref"
    );

    let signal_records = app
        .state_store()
        .list_knowledge_by_prefix(&format!("signal:events:{session_id}:"))
        .await?;
    let mut observed_signal_ids = BTreeSet::new();
    let mut kind_by_signal_id = BTreeMap::<String, String>::new();
    for record in signal_records {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&record.value) else {
            continue;
        };
        let event = value.get("event").cloned().unwrap_or(serde_json::json!({}));
        let Some(signal_id) = event.get("signal_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !signal_id.starts_with("signal:pressure:") {
            continue;
        }
        let signal_id = signal_id.to_string();
        observed_signal_ids.insert(signal_id.clone());
        assert_eq!(
            event
                .get("attributes")
                .and_then(|v| v.get("signal_processor_order"))
                .and_then(serde_json::Value::as_str),
            Some("redact>filter>sample>rate_limit"),
            "signal must pass pipeline processors in fixed order"
        );
        assert_eq!(
            event
                .get("attributes")
                .and_then(|v| v.get("signal_pipeline_mode"))
                .and_then(serde_json::Value::as_str),
            Some("shadow"),
            "signal must carry pipeline mode injected by facade pipeline"
        );

        if let Some(kind) = event.get("kind").and_then(serde_json::Value::as_str) {
            kind_by_signal_id.insert(signal_id, kind.to_string());
        }
    }

    assert_eq!(
        observed_signal_ids.len(),
        total,
        "all pressure signals should be persisted exactly once (latest key excluded)"
    );
    let trace_count = kind_by_signal_id
        .values()
        .filter(|kind| kind.as_str() == "trace")
        .count();
    let metric_count = kind_by_signal_id
        .values()
        .filter(|kind| kind.as_str() == "metric")
        .count();
    let log_count = kind_by_signal_id
        .values()
        .filter(|kind| kind.as_str() == "log")
        .count();
    assert_eq!(trace_count, expected_trace);
    assert_eq!(metric_count, expected_metric);
    assert_eq!(log_count, expected_log);

    let event_log = list_session_events(app.state_store(), session_id).await?;
    let sink_events = event_log
        .iter()
        .filter(|evt| evt.kind == "signal.pipeline.sink" && evt.trace_id == trace_id)
        .collect::<Vec<_>>();
    assert_eq!(
        sink_events.len(),
        total,
        "event stream should record one sink event per pressure signal"
    );

    let explain_refs = output
        .outputs
        .iter()
        .filter_map(|item| item.query_explain_ref.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        explain_refs.len(),
        total,
        "every pressure signal should produce a unique explain ref"
    );
    for explain_ref in explain_refs {
        let explain = app.state_store().get_knowledge(&explain_ref).await?;
        assert!(
            explain.is_some(),
            "query explain record should exist for ref {explain_ref}"
        );
    }

    Ok(())
}
