use std::collections::BTreeMap;

use super::ids::{SessionId, TraceId};

pub const WIKI_COMPAT_CONTRACT_VERSION: &str = "wiki-compat/v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    Extracted,
    Inferred,
    Ambiguous,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RefreshMode {
    DryRun,
    Force,
    Page,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryRouteReason {
    LexicalHit,
    CjkBigramFallback,
    GraphNeighborExpansion,
    FastSelectorFallback,
    PolicyConstrained,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LintFinding {
    pub finding_type: String,
    pub severity: LintSeverity,
    pub summary: String,
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct LintReport {
    pub report_id: String,
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub generated_at_ms: u64,
    #[serde(default)]
    pub contradictions: Vec<LintFinding>,
    #[serde(default)]
    pub stale_content: Vec<LintFinding>,
    #[serde(default)]
    pub data_gaps: Vec<LintFinding>,
    #[serde(default)]
    pub depth_gaps: Vec<LintFinding>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct OpLogEvent {
    pub event_id: String,
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub op_type: String,
    pub status: String,
    pub emitted_at_ms: u64,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WikiCompatContract {
    pub api_version: String,
    pub edge_type: EdgeType,
    pub lint_report: LintReport,
    pub refresh_mode: RefreshMode,
    pub query_route_reason: QueryRouteReason,
    pub op_log_event: OpLogEvent,
}

pub fn wiki_compat_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == WIKI_COMPAT_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("wiki-compat/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wiki_compat_accepts_v1_series() {
        assert!(wiki_compat_contract_compatible("wiki-compat/v1"));
        assert!(wiki_compat_contract_compatible("wiki-compat/v1.0"));
        assert!(wiki_compat_contract_compatible("WIKI-COMPAT/V1-beta"));
    }

    #[test]
    fn wiki_compat_rejects_other_majors() {
        assert!(!wiki_compat_contract_compatible("wiki-compat/v2"));
        assert!(!wiki_compat_contract_compatible("sandbox/v2"));
        assert!(!wiki_compat_contract_compatible("v1"));
    }
}
