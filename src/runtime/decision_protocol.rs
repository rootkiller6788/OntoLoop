use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeDecisionKind {
    Accept,
    Repair,
    Reject,
    Escalate,
}

impl RuntimeDecisionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Repair => "repair",
            Self::Reject => "reject",
            Self::Escalate => "escalate",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionGuardObservation {
    pub surface: String,
    pub capability_id: String,
    pub decision: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionThresholds {
    pub repair_below: f32,
    pub escalate_below: f32,
}

impl Default for DecisionThresholds {
    fn default() -> Self {
        Self {
            repair_below: 0.35,
            escalate_below: 0.05,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedDecisionInput {
    pub hardgate_passed: bool,
    pub compile_failed: bool,
    pub compaction_applied: bool,
    pub max_iterations_reached: bool,
    pub provider_retry_count: usize,
    pub tool_retry_count: usize,
    pub guard_observations: Vec<ExecutionGuardObservation>,
    pub verifier_score: f32,
    pub forced_hint: Option<RuntimeDecisionKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedDecisionOutput {
    pub kind: RuntimeDecisionKind,
    pub reasons: Vec<String>,
    pub verifier_score: f32,
    pub forced: bool,
}

pub fn load_thresholds_from_env() -> DecisionThresholds {
    let raw = std::env::var("AUTOLOOP_CONTEXT_DECISION_THRESHOLDS").unwrap_or_default();
    if raw.trim().is_empty() {
        return DecisionThresholds::default();
    }
    serde_json::from_str::<DecisionThresholds>(&raw)
        .unwrap_or_else(|_| DecisionThresholds::default())
}

pub fn parse_decision_hint(input: &str) -> Option<RuntimeDecisionKind> {
    let lowered = input.to_ascii_lowercase();
    if lowered.contains("[decision:accept]") || lowered.contains("decision=accept") {
        return Some(RuntimeDecisionKind::Accept);
    }
    if lowered.contains("[decision:repair]") || lowered.contains("decision=repair") {
        return Some(RuntimeDecisionKind::Repair);
    }
    if lowered.contains("[decision:reject]") || lowered.contains("decision=reject") {
        return Some(RuntimeDecisionKind::Reject);
    }
    if lowered.contains("[decision:escalate]") || lowered.contains("decision=escalate") {
        return Some(RuntimeDecisionKind::Escalate);
    }
    None
}

pub fn evaluate_unified_decision(
    input: UnifiedDecisionInput,
    thresholds: &DecisionThresholds,
) -> UnifiedDecisionOutput {
    if let Some(forced) = input.forced_hint {
        return UnifiedDecisionOutput {
            kind: forced,
            reasons: vec!["forced decision hint matched".into()],
            verifier_score: input.verifier_score,
            forced: true,
        };
    }

    let guard_blocked = input
        .guard_observations
        .iter()
        .any(|entry| entry.decision.eq_ignore_ascii_case("blocked"));
    let guard_requires_approval = input
        .guard_observations
        .iter()
        .any(|entry| entry.decision.eq_ignore_ascii_case("requires_approval"));

    if input.compile_failed || !input.hardgate_passed || guard_blocked {
        return UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Reject,
            reasons: vec![
                "hard constraints or runtime guard failed".into(),
                format!(
                    "compile_failed={} hardgate_passed={} guard_blocked={}",
                    input.compile_failed, input.hardgate_passed, guard_blocked
                ),
            ],
            verifier_score: input.verifier_score,
            forced: false,
        };
    }

    if input.max_iterations_reached
        || guard_requires_approval
        || input.verifier_score <= thresholds.escalate_below
    {
        return UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Escalate,
            reasons: vec![
                "verification threshold exceeded escalation boundary".into(),
                format!(
                    "max_iterations_reached={} guard_requires_approval={} verifier_score={:.3}",
                    input.max_iterations_reached, guard_requires_approval, input.verifier_score
                ),
            ],
            verifier_score: input.verifier_score,
            forced: false,
        };
    }

    if input.provider_retry_count > 0
        || input.tool_retry_count > 0
        || input.compaction_applied
        || input.verifier_score <= thresholds.repair_below
    {
        return UnifiedDecisionOutput {
            kind: RuntimeDecisionKind::Repair,
            reasons: vec![
                "repair branch selected by retry/quality signal".into(),
                format!(
                    "provider_retries={} tool_retries={} compaction_applied={} verifier_score={:.3}",
                    input.provider_retry_count,
                    input.tool_retry_count,
                    input.compaction_applied,
                    input.verifier_score
                ),
            ],
            verifier_score: input.verifier_score,
            forced: false,
        };
    }

    UnifiedDecisionOutput {
        kind: RuntimeDecisionKind::Accept,
        reasons: vec!["all runtime thresholds passed".into()],
        verifier_score: input.verifier_score,
        forced: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_hint_has_highest_priority() {
        let decision = evaluate_unified_decision(
            UnifiedDecisionInput {
                hardgate_passed: false,
                compile_failed: true,
                compaction_applied: false,
                max_iterations_reached: false,
                provider_retry_count: 0,
                tool_retry_count: 0,
                guard_observations: Vec::new(),
                verifier_score: 1.0,
                forced_hint: Some(RuntimeDecisionKind::Repair),
            },
            &DecisionThresholds::default(),
        );
        assert_eq!(decision.kind, RuntimeDecisionKind::Repair);
        assert!(decision.forced);
    }

    #[test]
    fn reject_wins_when_guard_blocks() {
        let decision = evaluate_unified_decision(
            UnifiedDecisionInput {
                hardgate_passed: true,
                compile_failed: false,
                compaction_applied: false,
                max_iterations_reached: false,
                provider_retry_count: 0,
                tool_retry_count: 0,
                guard_observations: vec![ExecutionGuardObservation {
                    surface: "tool".into(),
                    capability_id: "mcp::write".into(),
                    decision: "blocked".into(),
                    reason: "policy".into(),
                }],
                verifier_score: 0.9,
                forced_hint: None,
            },
            &DecisionThresholds::default(),
        );
        assert_eq!(decision.kind, RuntimeDecisionKind::Reject);
    }
}
