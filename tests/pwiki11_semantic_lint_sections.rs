use autoloop::{
    observability::event_stream::{ReplayAnalysisReport, ReplayDeviation},
    plugins::gitmemory_core::semantic_lint::build_semantic_lint_report,
};

#[test]
fn pwiki11_semantic_lint_sections_are_present() {
    let analyses = vec![ReplayAnalysisReport {
        snapshot_id: "snap-1".to_string(),
        session_id: "session-1".to_string(),
        trace_id: "trace-1".to_string(),
        replay_output_digest: "digest-1".to_string(),
        matched: false,
        deterministic_boundary_respected: false,
        deviations: vec![ReplayDeviation {
            field: "facts".to_string(),
            expected: "A".to_string(),
            actual: "B".to_string(),
            severity: "high".to_string(),
            explanation: "contradiction in source claims".to_string(),
        }],
        notes: vec!["stale summary needs refresh".to_string()],
        created_at_ms: 1,
    }];

    let report = build_semantic_lint_report(
        "session-1",
        Some("trace-1"),
        &analyses,
        &[serde_json::json!({"plugin":"graph"})],
        &serde_json::json!({
            "summary": {
                "orphan": ["node-a"],
                "fragile_bridge": ["node-b"]
            }
        }),
    );

    assert!(!report.contradictions.is_empty());
    assert!(!report.stale.is_empty());
    assert!(!report.gaps.is_empty());
    assert!(!report.depth.is_empty());
}



