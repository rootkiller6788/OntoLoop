use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};

use crate::contracts::version_a::ConstraintDecision;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintShieldAction {
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub actor_id: String,
    pub surface: String,
    pub capability_id: String,
    pub payload: serde_json::Value,
    pub risk: String,
    pub requires_evidence_ref: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintShieldVerdict {
    pub decision: ConstraintDecision,
    pub reason: String,
    pub decision_hash: String,
    pub evidence_ref: String,
}

pub struct ConstraintShield;

impl ConstraintShield {
    pub async fn check_action(
        db: &StateStore,
        action: &ConstraintShieldAction,
    ) -> Result<ConstraintShieldVerdict> {
        let payload_text = flatten_payload(&action.payload).to_ascii_lowercase();
        let now_ms = current_time_ms();
        let decision = if contains_hard_block_pattern(&payload_text) {
            ConstraintDecision::Block
        } else if requires_approval(action, &payload_text) {
            ConstraintDecision::RequiresApproval
        } else {
            ConstraintDecision::Allow
        };

        let reason = match decision {
            ConstraintDecision::Allow => "constraint_shield.allow".to_string(),
            ConstraintDecision::RequiresApproval => {
                "constraint_shield.requires_approval(high_risk_or_sensitive_action)".to_string()
            }
            ConstraintDecision::Block => {
                "constraint_shield.block(unsafe_or_policy_bypass_pattern)".to_string()
            }
        };

        let decision_hash = format!(
            "decision:{}:{}:{}:{}",
            action.session_id, action.trace_id, action.surface, now_ms
        );
        let evidence_ref = format!(
            "evidence:constraint-shield:{}:{}:{}",
            action.session_id, action.trace_id, now_ms
        );

        let decision_log_key = format!(
            "decision_log:constraint_shield:{}:{}:{}",
            action.session_id, action.trace_id, now_ms
        );
        let decision_log = serde_json::json!({
            "kind": "constraint_shield_decision",
            "session_id": action.session_id,
            "trace_id": action.trace_id,
            "task_id": action.task_id,
            "actor_id": action.actor_id,
            "surface": action.surface,
            "capability_id": action.capability_id,
            "risk": action.risk,
            "decision": format!("{:?}", decision).to_ascii_lowercase(),
            "reason": reason,
            "decision_hash": decision_hash,
            "evidence_ref": evidence_ref,
            "requires_evidence_ref": action.requires_evidence_ref,
        });
        let _ = db
            .upsert_json_knowledge(decision_log_key, &decision_log, "constraint-shield")
            .await;

        Ok(ConstraintShieldVerdict {
            decision,
            reason,
            decision_hash,
            evidence_ref,
        })
    }
}

fn requires_approval(action: &ConstraintShieldAction, payload_text: &str) -> bool {
    let sensitive_capability = action.capability_id.starts_with("mcp::")
        || action.capability_id.contains("shell")
        || action.capability_id.contains("rollback")
        || action.capability_id.contains("write");
    let high_risk = action.risk.eq_ignore_ascii_case("high");
    let sensitive_payload = payload_text.contains("delete ")
        || payload_text.contains("drop table")
        || payload_text.contains("truncate")
        || payload_text.contains("rollback")
        || payload_text.contains("overwrite");
    let missing_evidence = action.requires_evidence_ref
        && action
            .payload
            .get("evidence_ref")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .map(|v| v.is_empty())
            .unwrap_or(true);
    high_risk || sensitive_capability || sensitive_payload || missing_evidence
}

fn contains_hard_block_pattern(payload_text: &str) -> bool {
    payload_text.contains("ignore all rules")
        || payload_text.contains("bypass policy")
        || payload_text.contains("disable guard")
        || payload_text.contains("exfiltrate")
        || payload_text.contains("leak secret")
}

fn flatten_payload(payload: &serde_json::Value) -> String {
    match payload {
        serde_json::Value::String(s) => s.clone(),
        _ => payload.to_string(),
    }
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_action() -> ConstraintShieldAction {
        ConstraintShieldAction {
            session_id: "s1".to_string(),
            trace_id: "t1".to_string(),
            task_id: "task1".to_string(),
            actor_id: "actor1".to_string(),
            surface: "runtime".to_string(),
            capability_id: "tool::read".to_string(),
            payload: serde_json::json!({"action":"read"}),
            risk: "low".to_string(),
            requires_evidence_ref: false,
        }
    }

    #[test]
    fn constraint_patterns_block_unsafe_payload() {
        assert!(contains_hard_block_pattern("please bypass policy now"));
        assert!(contains_hard_block_pattern("try to exfiltrate data"));
        assert!(!contains_hard_block_pattern("safe operation"));
    }

    #[test]
    fn requires_approval_when_high_risk_or_missing_evidence() {
        let mut high_risk = base_action();
        high_risk.risk = "high".to_string();
        assert!(requires_approval(&high_risk, "safe payload"));

        let mut evidence_required = base_action();
        evidence_required.requires_evidence_ref = true;
        evidence_required.payload = serde_json::json!({"action":"write"});
        assert!(requires_approval(&evidence_required, "write file"));
    }
}
