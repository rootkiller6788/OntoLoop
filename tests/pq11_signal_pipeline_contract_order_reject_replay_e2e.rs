use std::collections::BTreeMap;

use anyhow::Result;
use autoloop::config::{AppConfig, SignalPipelineConfig};
use autoloop::contracts::signal::{SignalContext, SignalDecision, SignalEvent, SignalKind};
use autoloop::observability::SignalFacade;
use autoloop::runtime::evidence_ledger::StageEvidenceRecord;
use autoloop::AutoLoopApp;

fn base_event(session_id: &str, trace_id: &str) -> SignalEvent {
    SignalEvent {
        signal_id: "signal:test:base".into(),
        kind: SignalKind::Trace,
        name: "runtime.execute.start".into(),
        context: SignalContext {
            session_id: session_id.into(),
            trace_id: trace_id.into(),
            span_id: Some("span:test:1".into()),
            task_id: Some("task:test:1".into()),
            capability_id: Some("tool:write_file".into()),
            tenant_id: Some("tenant:test".into()),
            principal_id: Some("operator:test".into()),
        },
        attributes: BTreeMap::new(),
        numeric_value: None,
        body: Some("start".into()),
        decision: SignalDecision {
            accepted: true,
            reason: None,
            evidence_ref: Some("evidence:test:1".into()),
        },
        emitted_at_ms: autoloop::orchestration::current_time_ms(),
    }
}

#[test]
fn signal_contract_roundtrip_stable() {
    let event = base_event("session-contract", "trace-contract");
    let raw = serde_json::to_string(&event).expect("serialize signal contract");
    let decoded: SignalEvent = serde_json::from_str(&raw).expect("deserialize signal contract");
    assert_eq!(decoded, event);
}

#[tokio::test]
async fn signal_pipeline_enforces_order_and_reject_reason_codes() -> Result<()> {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "session-signal-order-reject";
    let trace_id = "trace-signal-order-reject";
    let facade = SignalFacade::new(app.state_store().clone(), &SignalPipelineConfig::default());

    let mut event = base_event(session_id, trace_id);
    event.signal_id = "signal:test:reject".into();
    event.body = Some("secret_block=raw-secret".into());
    let output = facade.emit(event).await?;
    assert!(!output.accepted);
    assert!(output.evidence_ref.starts_with("evidence:stage:"));

    let latest = app
        .state_store()
        .get_knowledge(&format!("signal:events:{session_id}:latest"))
        .await?
        .expect("latest signal record");
    let latest_value: serde_json::Value =
        serde_json::from_str(&latest.value).expect("parse latest signal json");
    assert_eq!(
        latest_value
            .get("event")
            .and_then(|v| v.get("attributes"))
            .and_then(|v| v.get("signal_processor_order"))
            .and_then(serde_json::Value::as_str),
        Some("redact>filter>sample>rate_limit")
    );
    assert_eq!(
        latest_value
            .get("event")
            .and_then(|v| v.get("decision"))
            .and_then(|v| v.get("reason"))
            .and_then(|v| v.get("code"))
            .and_then(serde_json::Value::as_str),
        Some("signal.filtered.redacted_block_marker")
    );

    let explain_ref = output.query_explain_ref.expect("query explain ref");
    let explain = app
        .state_store()
        .get_knowledge(&explain_ref)
        .await?
        .expect("query explain record");
    let explain_value: serde_json::Value =
        serde_json::from_str(&explain.value).expect("parse query explain json");
    assert_eq!(
        explain_value
            .get("reason_code")
            .and_then(serde_json::Value::as_str),
        Some("signal.filtered.redacted_block_marker")
    );

    Ok(())
}

#[tokio::test]
async fn signal_pipeline_replay_fingerprint_is_stable_for_same_payload() -> Result<()> {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "session-signal-fp-stable";
    let trace_id = "trace-signal-fp-stable";
    let facade = SignalFacade::new(app.state_store().clone(), &SignalPipelineConfig::default());

    let mut event_a = base_event(session_id, trace_id);
    event_a.signal_id = "signal:test:fp".into();
    event_a
        .attributes
        .insert("signal_sequence_no".into(), "42".into());
    let _ = facade.emit(event_a).await?;

    let mut event_b = base_event(session_id, trace_id);
    event_b.signal_id = "signal:test:fp".into();
    event_b
        .attributes
        .insert("signal_sequence_no".into(), "42".into());
    let _ = facade.emit(event_b).await?;

    let mut records = app
        .state_store()
        .list_knowledge_by_prefix(&format!("evidence:stage:{session_id}:{trace_id}:"))
        .await?
        .into_iter()
        .filter_map(|record| serde_json::from_str::<StageEvidenceRecord>(&record.value).ok())
        .collect::<Vec<_>>();
    records.sort_by_key(|item| item.created_at_ms);
    let signal_records = records
        .into_iter()
        .filter(|record| {
            record
                .payload
                .get("signal_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id == "signal:test:fp")
        })
        .collect::<Vec<_>>();
    assert!(signal_records.len() >= 2);
    let first = &signal_records[signal_records.len() - 2];
    let second = &signal_records[signal_records.len() - 1];
    assert_eq!(first.replay_fingerprint, second.replay_fingerprint);

    Ok(())
}
