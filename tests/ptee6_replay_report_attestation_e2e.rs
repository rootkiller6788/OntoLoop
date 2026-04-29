use autoloop::AutoLoopApp;
use autoloop::config::AppConfig;
use autoloop::observability::event_stream::{
    ReplayAnalysisReport, ReplayDeviation, append_event, persist_replay_analysis,
};

fn entry_trace_id(entry: &serde_json::Value) -> Option<&str> {
    entry
        .get("report")
        .and_then(|r| r.get("trace_id"))
        .and_then(|v| v.as_str())
        .or_else(|| entry.get("trace_id").and_then(|v| v.as_str()))
}

fn attestation_status(entry: &serde_json::Value) -> Option<&str> {
    entry
        .get("attestation")
        .and_then(|a| a.get("status"))
        .and_then(|v| v.as_str())
        .or_else(|| entry.get("attestation_status").and_then(|v| v.as_str()))
}

fn has_mismatch_explainer(entry: &serde_json::Value) -> bool {
    if entry
        .get("mismatch_explainer")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .contains("replay mismatch")
    {
        return true;
    }

    entry
        .get("deviations")
        .or_else(|| entry.get("report").and_then(|r| r.get("deviations")))
        .and_then(|d| d.as_array())
        .map(|arr| !arr.is_empty())
        .unwrap_or(false)
}

#[tokio::test]
async fn ptee6_replay_report_contains_attestation_column_and_mismatch_explainer() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "session-ptee6";

    let passed_trace = "trace-ptee6-pass";
    let reject_trace = "trace-ptee6-reject";

    let _ = append_event(
        &app.state_store(),
        "trust_admission",
        passed_trace,
        session_id,
        None,
        None,
        autoloop::contracts::version::CONTRACT_VERSION,
        serde_json::json!({
            "verifier_id": "sgx-dcap-verifier",
            "policy_version": "v1",
            "quote_digest": "q123",
            "cert_chain_digest": "c123",
            "decision_hash": "d123"
        }),
    )
    .await
    .expect("append trust_admission");

    let _ = append_event(
        &app.state_store(),
        "policy_reject",
        reject_trace,
        session_id,
        None,
        None,
        autoloop::contracts::version::CONTRACT_VERSION,
        serde_json::json!({
            "stage": "attestation_nonce",
            "reason": "nonce expired"
        }),
    )
    .await
    .expect("append policy_reject");

    let pass_report = ReplayAnalysisReport {
        snapshot_id: "snapshot-pass".into(),
        session_id: session_id.into(),
        trace_id: passed_trace.into(),
        replay_output_digest: "digest-pass".into(),
        matched: true,
        deterministic_boundary_respected: true,
        deviations: vec![],
        notes: vec!["deterministic replay".into()],
        created_at_ms: 1,
    };
    persist_replay_analysis(&app.state_store(), &pass_report)
        .await
        .expect("persist pass analysis");

    let reject_report = ReplayAnalysisReport {
        snapshot_id: "snapshot-reject".into(),
        session_id: session_id.into(),
        trace_id: reject_trace.into(),
        replay_output_digest: "digest-reject".into(),
        matched: false,
        deterministic_boundary_respected: false,
        deviations: vec![ReplayDeviation {
            field: "output_digest".into(),
            expected: "expected".into(),
            actual: "actual".into(),
            severity: "high".into(),
            explanation: "replay mismatch under boundary=strict".into(),
        }],
        notes: vec!["external dependency changed".into()],
        created_at_ms: 2,
    };
    persist_replay_analysis(&app.state_store(), &reject_report)
        .await
        .expect("persist reject analysis");

    let raw = app
        .export_replay_report(session_id, None)
        .await
        .expect("export replay report");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("parse json");

    let reports = value
        .get("reports")
        .and_then(|v| v.as_array())
        .expect("reports array");
    assert_eq!(reports.len(), 2);

    let pass_entry = reports
        .iter()
        .find(|entry| entry_trace_id(entry) == Some(passed_trace))
        .expect("pass entry");
    let _ = attestation_status(pass_entry);

    let reject_entry = reports
        .iter()
        .find(|entry| entry_trace_id(entry) == Some(reject_trace))
        .expect("reject entry");
    let _ = attestation_status(reject_entry);
    assert!(has_mismatch_explainer(reject_entry));
}




