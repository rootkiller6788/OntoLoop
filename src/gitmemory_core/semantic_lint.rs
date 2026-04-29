use crate::observability::event_stream::ReplayAnalysisReport;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SemanticLintFinding {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SemanticLintReport {
    pub report_id: String,
    pub session_id: String,
    pub trace_id: Option<String>,
    pub generated_at_ms: u64,
    pub contradictions: Vec<SemanticLintFinding>,
    pub stale: Vec<SemanticLintFinding>,
    pub gaps: Vec<SemanticLintFinding>,
    pub depth: Vec<SemanticLintFinding>,
}

pub fn build_semantic_lint_report(
    session_id: &str,
    trace_id: Option<&str>,
    analyses: &[ReplayAnalysisReport],
    plugin_traces: &[serde_json::Value],
    graph_health: &serde_json::Value,
) -> SemanticLintReport {
    let contradictions = collect_contradictions(analyses);
    let stale = collect_stale(analyses);
    let gaps = collect_gaps(analyses, graph_health);
    let depth = collect_depth(plugin_traces, graph_health);
    let generated_at_ms = current_time_ms();
    let report_id = format!(
        "semantic-lint:{}:{}:{}",
        session_id,
        trace_id.unwrap_or("all"),
        generated_at_ms
    );

    SemanticLintReport {
        report_id,
        session_id: session_id.to_string(),
        trace_id: trace_id.map(str::to_string),
        generated_at_ms,
        contradictions: default_if_empty(
            contradictions,
            "contradictions.none",
            "No contradiction signal detected from replay analysis.",
        ),
        stale: default_if_empty(
            stale,
            "stale.none",
            "No stale-content signal detected from replay analysis.",
        ),
        gaps: default_if_empty(
            gaps,
            "gaps.none",
            "No data-gap signal detected from graph/replay synthesis.",
        ),
        depth: default_if_empty(
            depth,
            "depth.none",
            "No shallow-depth signal detected from current traces.",
        ),
    }
}

fn collect_contradictions(analyses: &[ReplayAnalysisReport]) -> Vec<SemanticLintFinding> {
    analyses
        .iter()
        .filter_map(|item| {
            if item.matched && item.deterministic_boundary_respected {
                return None;
            }
            let contradiction_note = item
                .notes
                .iter()
                .find(|note| note.to_ascii_lowercase().contains("contradiction"))
                .cloned()
                .or_else(|| {
                    item.deviations
                        .iter()
                        .map(|dev| dev.explanation.clone())
                        .find(|text| text.to_ascii_lowercase().contains("contradiction"))
                });
            contradiction_note.map(|summary| SemanticLintFinding {
                id: format!("contradiction:{}", item.snapshot_id),
                summary,
                refs: vec![format!("replay:snapshot:{}", item.snapshot_id)],
            })
        })
        .collect()
}

fn collect_stale(analyses: &[ReplayAnalysisReport]) -> Vec<SemanticLintFinding> {
    analyses
        .iter()
        .filter_map(|item| {
            let stale_note = item
                .notes
                .iter()
                .find(|note| {
                    let lowered = note.to_ascii_lowercase();
                    lowered.contains("stale") || lowered.contains("superseded")
                })
                .cloned();
            stale_note.map(|summary| SemanticLintFinding {
                id: format!("stale:{}", item.snapshot_id),
                summary,
                refs: vec![format!("replay:snapshot:{}", item.snapshot_id)],
            })
        })
        .collect()
}

fn collect_gaps(
    analyses: &[ReplayAnalysisReport],
    graph_health: &serde_json::Value,
) -> Vec<SemanticLintFinding> {
    let mut findings = Vec::<SemanticLintFinding>::new();

    let orphan_count = graph_health
        .get("summary")
        .and_then(|summary| summary.get("orphan"))
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    if orphan_count > 0 {
        findings.push(SemanticLintFinding {
            id: "gaps.graph_orphan".to_string(),
            summary: format!(
                "Graph health reports {} orphan node(s); retrieval coverage may be incomplete.",
                orphan_count
            ),
            refs: vec!["graph_health.summary.orphan".to_string()],
        });
    }

    let replay_gap = analyses.iter().find_map(|item| {
        item.notes
            .iter()
            .find(|note| note.to_ascii_lowercase().contains("gap"))
            .cloned()
            .map(|summary| SemanticLintFinding {
                id: format!("gaps:{}", item.snapshot_id),
                summary,
                refs: vec![format!("replay:snapshot:{}", item.snapshot_id)],
            })
    });
    if let Some(item) = replay_gap {
        findings.push(item);
    }

    findings
}

fn collect_depth(
    plugin_traces: &[serde_json::Value],
    graph_health: &serde_json::Value,
) -> Vec<SemanticLintFinding> {
    let mut findings = Vec::<SemanticLintFinding>::new();

    let trace_count = plugin_traces.len();
    if trace_count < 2 {
        findings.push(SemanticLintFinding {
            id: "depth.trace_window_small".to_string(),
            summary: format!(
                "Only {} plugin trace sample(s) available; synthesis depth may be shallow.",
                trace_count
            ),
            refs: vec!["replay.plugin_execution_traces".to_string()],
        });
    }

    let fragile_bridge_count = graph_health
        .get("summary")
        .and_then(|summary| summary.get("fragile_bridge"))
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    if fragile_bridge_count > 0 {
        findings.push(SemanticLintFinding {
            id: "depth.fragile_bridge".to_string(),
            summary: format!(
                "Graph health reports {} fragile bridge node(s); concept depth is uneven.",
                fragile_bridge_count
            ),
            refs: vec!["graph_health.summary.fragile_bridge".to_string()],
        });
    }

    findings
}

fn default_if_empty(
    findings: Vec<SemanticLintFinding>,
    fallback_id: &str,
    fallback_summary: &str,
) -> Vec<SemanticLintFinding> {
    if findings.is_empty() {
        vec![SemanticLintFinding {
            id: fallback_id.to_string(),
            summary: fallback_summary.to_string(),
            refs: Vec::new(),
        }]
    } else {
        findings
    }
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
    use crate::observability::event_stream::ReplayDeviation;

    #[test]
    fn semantic_lint_report_contains_four_sections_with_defaults() {
        let report = build_semantic_lint_report(
            "session-a",
            Some("trace-a"),
            &[],
            &[],
            &serde_json::json!({}),
        );
        assert_eq!(report.session_id, "session-a");
        assert_eq!(report.trace_id.as_deref(), Some("trace-a"));
        assert!(!report.contradictions.is_empty());
        assert!(!report.stale.is_empty());
        assert!(!report.gaps.is_empty());
        assert!(!report.depth.is_empty());
    }

    #[test]
    fn semantic_lint_extracts_contradiction_and_stale_signals() {
        let analyses = vec![ReplayAnalysisReport {
            snapshot_id: "snap-1".to_string(),
            session_id: "session-a".to_string(),
            trace_id: "trace-a".to_string(),
            replay_output_digest: "digest-1".to_string(),
            matched: false,
            deterministic_boundary_respected: false,
            deviations: vec![ReplayDeviation {
                field: "facts".to_string(),
                expected: "A".to_string(),
                actual: "B".to_string(),
                severity: "high".to_string(),
                explanation: "contradiction detected in source claims".to_string(),
            }],
            notes: vec!["stale summary should be refreshed".to_string()],
            created_at_ms: 1,
        }];
        let report = build_semantic_lint_report(
            "session-a",
            Some("trace-a"),
            &analyses,
            &[],
            &serde_json::json!({}),
        );
        assert!(
            report
                .contradictions
                .iter()
                .any(|item| item.id.starts_with("contradiction:"))
        );
        assert!(report.stale.iter().any(|item| item.id.starts_with("stale:")));
    }
}
