use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Trace,
    Metric,
    Log,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalContext {
    pub session_id: String,
    pub trace_id: String,
    #[serde(default)]
    pub span_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub capability_id: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub principal_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalReason {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub processor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalDecision {
    pub accepted: bool,
    #[serde(default)]
    pub reason: Option<SignalReason>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignalEvent {
    pub signal_id: String,
    pub kind: SignalKind,
    pub name: String,
    pub context: SignalContext,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    #[serde(default)]
    pub numeric_value: Option<f64>,
    #[serde(default)]
    pub body: Option<String>,
    pub decision: SignalDecision,
    pub emitted_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_contract_v1_roundtrip() {
        let event = SignalEvent {
            signal_id: "signal:s-1:1".into(),
            kind: SignalKind::Trace,
            name: "runtime.execute.start".into(),
            context: SignalContext {
                session_id: "s-1".into(),
                trace_id: "trace:s-1:1".into(),
                span_id: Some("span:s-1:1".into()),
                task_id: Some("task:s-1:1".into()),
                capability_id: Some("tool:write_file".into()),
                tenant_id: Some("tenant-a".into()),
                principal_id: Some("operator-a".into()),
            },
            attributes: BTreeMap::from([
                ("runtime_class".into(), "tool_sandboxed".into()),
                ("policy_mode".into(), "shadow".into()),
            ]),
            numeric_value: Some(1.0),
            body: Some("execution-started".into()),
            decision: SignalDecision {
                accepted: true,
                reason: None,
                evidence_ref: Some("evidence:stage:s-1:trace:s-1:1:1".into()),
            },
            emitted_at_ms: 1_710_000_001_000,
        };

        let raw = serde_json::to_string(&event).expect("serialize signal event");
        let decoded: SignalEvent = serde_json::from_str(&raw).expect("deserialize signal event");
        assert_eq!(decoded, event);
    }
}
