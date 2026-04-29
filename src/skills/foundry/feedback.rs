use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::contracts::skill_foundry::{
    FoundryPromotionPolicy, FoundrySkillLayerState, PromotionHint, SkillFoundryLayer,
};
use crate::runtime::evidence_ledger::{
    EvidenceLedgerWriter, EvidenceStage, FoundryFeedbackEvidenceRecord,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoundryFeedbackKind {
    Hit,
    Miss,
    FailedTrigger,
    MissingScript,
    BadJson,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FoundryFeedbackEvent {
    pub event_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub operation: String,
    pub kind: FoundryFeedbackKind,
    pub message: String,
    pub detail: serde_json::Value,
    pub evidence_ref: Option<String>,
    pub created_at_ms: u64,
}

pub fn default_promotion_policy(now_ms: u64) -> FoundryPromotionPolicy {
    FoundryPromotionPolicy {
        policy_id: "foundry-promotion-policy-v1".to_string(),
        s1_execution_failure_threshold: 2,
        s2_boundary_failure_threshold: 2,
        max_counted_failures: 3,
        manual_approval_required: true,
        created_at_ms: now_ms,
    }
}

pub fn record_promotion_feedback_event(
    skill_name: &str,
    from_layer: SkillFoundryLayer,
    to_layer: SkillFoundryLayer,
    trigger: &str,
    observed_failures: u32,
    evidence_refs: Vec<String>,
    now_ms: u64,
) -> PromotionHint {
    PromotionHint {
        hint_id: format!("promotion:{}:{}", skill_name, now_ms),
        from_layer,
        to_layer,
        trigger: trigger.to_string(),
        observed_failures,
        evidence_refs,
        recommended: true,
        created_at_ms: now_ms,
    }
}

pub async fn load_skill_layer_state(
    db: &StateStore,
    skill_name: &str,
) -> Result<Option<FoundrySkillLayerState>> {
    let key = format!("foundry:skill-layer:{skill_name}:latest");
    let state = db
        .get_knowledge(&key)
        .await?
        .and_then(|record| serde_json::from_str::<FoundrySkillLayerState>(&record.value).ok());
    Ok(state)
}

pub async fn persist_skill_layer_state(
    db: &StateStore,
    session_id: &str,
    trace_id: &str,
    state: &FoundrySkillLayerState,
) -> Result<()> {
    let now_ms = state.updated_at_ms;
    db.upsert_json_knowledge(
        format!("foundry:skill-layer:{}:{now_ms}", state.skill_name),
        state,
        "foundry-layer-state",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("foundry:skill-layer:{}:latest", state.skill_name),
        state,
        "foundry-layer-state",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("conversation:{session_id}:execution-feedback:{now_ms}:layer-state"),
        &serde_json::json!({
            "trace_id": trace_id,
            "state": state,
        }),
        "execution-feedback",
    )
    .await?;
    Ok(())
}

pub async fn evaluate_promotion_gate(
    db: &StateStore,
    session_id: &str,
    trace_id: &str,
    skill_name: &str,
    current_layer: SkillFoundryLayer,
    policy: &FoundryPromotionPolicy,
    now_ms: u64,
) -> Result<Option<PromotionHint>> {
    let records = db
        .list_knowledge_by_prefix(&format!("foundry:feedback:{session_id}:"))
        .await?;
    let mut events: Vec<FoundryFeedbackEvent> = records
        .into_iter()
        .filter_map(|record| serde_json::from_str::<FoundryFeedbackEvent>(&record.value).ok())
        .collect();
    events.sort_by_key(|event| event.created_at_ms);

    let streak_events = trailing_failure_streak(&events, &current_layer);
    let threshold = match current_layer {
        SkillFoundryLayer::S1PromptOnly => policy.s1_execution_failure_threshold,
        SkillFoundryLayer::S2PromptScripts => policy.s2_boundary_failure_threshold,
        SkillFoundryLayer::S3PromptMcp => return Ok(None),
    };
    if streak_events.len() < threshold as usize {
        return Ok(None);
    }

    let (to_layer, trigger) = match current_layer {
        SkillFoundryLayer::S1PromptOnly => (
            SkillFoundryLayer::S2PromptScripts,
            "execution_failure_threshold_reached",
        ),
        SkillFoundryLayer::S2PromptScripts => (
            SkillFoundryLayer::S3PromptMcp,
            "capability_boundary_threshold_reached",
        ),
        SkillFoundryLayer::S3PromptMcp => return Ok(None),
    };

    let observed_failures = (streak_events.len() as u32).min(policy.max_counted_failures.max(1));
    let evidence_refs = streak_events
        .iter()
        .filter_map(|event| event.evidence_ref.clone())
        .collect::<Vec<_>>();
    let hint = record_promotion_feedback_event(
        skill_name,
        current_layer.clone(),
        to_layer,
        trigger,
        observed_failures,
        evidence_refs,
        now_ms,
    );

    db.upsert_json_knowledge(
        format!("foundry:promotion:pending:{session_id}:{skill_name}:{now_ms}"),
        &hint,
        "foundry-promotion",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("foundry:promotion:pending:{session_id}:{skill_name}:latest"),
        &hint,
        "foundry-promotion",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("policy:foundry:promotion:{session_id}:{skill_name}:latest"),
        &serde_json::json!({
            "policy": policy,
            "threshold": threshold,
            "evaluated_layer": current_layer,
            "observed_failures": observed_failures,
            "hint_id": hint.hint_id,
            "created_at_ms": now_ms,
        }),
        "policy-engine",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("metrics:foundry:promotion:{session_id}:{now_ms}"),
        &serde_json::json!({
            "skill_name": skill_name,
            "evaluated_layer": hint.from_layer,
            "target_layer": hint.to_layer,
            "threshold": threshold,
            "observed_failures": observed_failures,
            "policy_id": policy.policy_id,
            "manual_approval_required": policy.manual_approval_required,
            "created_at_ms": now_ms,
        }),
        "metrics-foundry",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("conversation:{session_id}:execution-feedback:{now_ms}:promotion"),
        &serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "skill_name": skill_name,
            "status": "pending_approval",
            "policy": policy,
            "hint": hint,
        }),
        "execution-feedback",
    )
    .await?;
    let _stage_ref = EvidenceLedgerWriter::append_stage(
        db,
        session_id,
        trace_id,
        EvidenceStage::Learn,
        serde_json::json!({
            "gate": "foundry_promotion",
            "status": "pending_approval",
            "skill_name": skill_name,
            "policy": policy,
            "hint": hint,
        }),
        None,
    )
    .await?;

    Ok(Some(hint))
}

pub fn classify_feedback_kind(
    operation: &str,
    output: Option<&serde_json::Value>,
    error: Option<&str>,
) -> FoundryFeedbackKind {
    let lowered_error = error.unwrap_or_default().to_ascii_lowercase();
    if lowered_error.contains("json") {
        return FoundryFeedbackKind::BadJson;
    }
    if lowered_error.contains("script") {
        return FoundryFeedbackKind::MissingScript;
    }
    if lowered_error.contains("trigger") {
        return FoundryFeedbackKind::FailedTrigger;
    }
    if let Some(value) = output {
        if value.get("error").is_some() {
            return FoundryFeedbackKind::Miss;
        }
        if operation.eq_ignore_ascii_case("validate")
            && value
                .get("validation")
                .and_then(|item| item.get("passed"))
                .and_then(serde_json::Value::as_bool)
                == Some(false)
        {
            return FoundryFeedbackKind::Miss;
        }
    }
    if error.is_some() {
        FoundryFeedbackKind::Miss
    } else {
        FoundryFeedbackKind::Hit
    }
}

pub async fn persist_feedback_event(
    db: &StateStore,
    session_id: &str,
    trace_id: &str,
    operation: &str,
    kind: FoundryFeedbackKind,
    message: &str,
    detail: serde_json::Value,
    now_ms: u64,
) -> Result<FoundryFeedbackEvent> {
    let event_id = format!(
        "foundry-feedback:{}:{}:{}",
        operation,
        kind_slug(&kind),
        now_ms
    );
    let mut event = FoundryFeedbackEvent {
        event_id: event_id.clone(),
        session_id: session_id.to_string(),
        trace_id: trace_id.to_string(),
        operation: operation.to_string(),
        kind: kind.clone(),
        message: message.to_string(),
        detail: detail.clone(),
        evidence_ref: None,
        created_at_ms: now_ms,
    };

    let evidence_record = FoundryFeedbackEvidenceRecord {
        event_id: event_id.clone(),
        session_id: session_id.to_string(),
        trace_id: trace_id.to_string(),
        operation: operation.to_string(),
        kind: kind_slug(&kind).to_string(),
        detail: message.to_string(),
        created_at_ms: now_ms,
    };
    let foundry_evidence_ref =
        EvidenceLedgerWriter::append_foundry_feedback(db, session_id, &evidence_record).await?;
    let _stage_ref = EvidenceLedgerWriter::append_stage(
        db,
        session_id,
        trace_id,
        EvidenceStage::Learn,
        serde_json::json!({
            "kind": kind,
            "operation": operation,
            "message": message,
            "detail": detail,
            "foundry_evidence_ref": foundry_evidence_ref,
        }),
        None,
    )
    .await?;
    event.evidence_ref = Some(foundry_evidence_ref.clone());

    db.upsert_json_knowledge(
        format!(
            "foundry:feedback:{session_id}:{trace_id}:{now_ms}:{}",
            kind_slug(&kind)
        ),
        &event,
        "foundry-feedback",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("foundry:feedback:{session_id}:{trace_id}:latest"),
        &event,
        "foundry-feedback",
    )
    .await?;
    db.upsert_json_knowledge(
        format!("conversation:{session_id}:execution-feedback:{now_ms}:foundry"),
        &event,
        "execution-feedback",
    )
    .await?;
    Ok(event)
}

fn kind_slug(kind: &FoundryFeedbackKind) -> &'static str {
    match kind {
        FoundryFeedbackKind::Hit => "hit",
        FoundryFeedbackKind::Miss => "miss",
        FoundryFeedbackKind::FailedTrigger => "failed-trigger",
        FoundryFeedbackKind::MissingScript => "missing-script",
        FoundryFeedbackKind::BadJson => "bad-json",
    }
}

fn trailing_failure_streak<'a>(
    events: &'a [FoundryFeedbackEvent],
    current_layer: &SkillFoundryLayer,
) -> Vec<&'a FoundryFeedbackEvent> {
    let mut streak = Vec::new();
    for event in events.iter().rev() {
        if !is_execution_operation(&event.operation) {
            continue;
        }
        let is_failure = match current_layer {
            SkillFoundryLayer::S1PromptOnly => is_execution_failure_kind(&event.kind),
            SkillFoundryLayer::S2PromptScripts => is_capability_boundary_failure(event),
            SkillFoundryLayer::S3PromptMcp => false,
        };
        if is_failure {
            streak.push(event);
            continue;
        }
        break;
    }
    streak
}

fn is_execution_operation(operation: &str) -> bool {
    !matches!(
        operation,
        "intake" | "extract" | "route" | "enable" | "disable" | "approve_promotion"
    )
}

fn is_execution_failure_kind(kind: &FoundryFeedbackKind) -> bool {
    matches!(
        kind,
        FoundryFeedbackKind::Miss
            | FoundryFeedbackKind::MissingScript
            | FoundryFeedbackKind::BadJson
    )
}

fn is_capability_boundary_failure(event: &FoundryFeedbackEvent) -> bool {
    if !matches!(
        event.kind,
        FoundryFeedbackKind::FailedTrigger | FoundryFeedbackKind::Miss
    ) {
        return false;
    }
    let detail = event.detail.to_string().to_ascii_lowercase();
    let message = event.message.to_ascii_lowercase();
    [
        "capability boundary",
        "capability_boundary",
        "capability-limit",
        "requires mcp",
        "external capability",
        "admission reject",
    ]
    .iter()
    .any(|needle| message.contains(needle) || detail.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_bad_json_and_missing_script_failures() {
        let bad_json = classify_feedback_kind("validate", None, Some("json parse failed"));
        let missing_script = classify_feedback_kind("build", None, Some("script not found"));
        let hit = classify_feedback_kind("route", Some(&serde_json::json!({"ok": true})), None);

        assert!(matches!(bad_json, FoundryFeedbackKind::BadJson));
        assert!(matches!(missing_script, FoundryFeedbackKind::MissingScript));
        assert!(matches!(hit, FoundryFeedbackKind::Hit));
    }

    #[test]
    fn promotion_gate_only_promotes_one_layer() {
        let event = FoundryFeedbackEvent {
            event_id: "e1".to_string(),
            session_id: "s1".to_string(),
            trace_id: "t1".to_string(),
            operation: "build".to_string(),
            kind: FoundryFeedbackKind::MissingScript,
            message: "script missing".to_string(),
            detail: serde_json::json!({}),
            evidence_ref: Some("evidence:1".to_string()),
            created_at_ms: 1,
        };
        let events = [event.clone(), event];
        let streak = trailing_failure_streak(&events, &SkillFoundryLayer::S1PromptOnly);
        assert_eq!(streak.len(), 2);
        let policy = default_promotion_policy(2);
        assert_eq!(policy.s1_execution_failure_threshold, 2);
        let hint = record_promotion_feedback_event(
            "skill-a",
            SkillFoundryLayer::S1PromptOnly,
            SkillFoundryLayer::S2PromptScripts,
            "execution_failure_threshold_reached",
            2,
            vec!["evidence:1".to_string()],
            2,
        );
        assert_eq!(hint.from_layer, SkillFoundryLayer::S1PromptOnly);
        assert_eq!(hint.to_layer, SkillFoundryLayer::S2PromptScripts);
        assert!(hint.recommended);
    }
}


