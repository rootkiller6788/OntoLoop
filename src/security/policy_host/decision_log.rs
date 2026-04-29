use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::policy_pdp::{DropRule, MaskRule, PolicyDecision, PolicyVersion};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionLogPolicy {
    #[serde(default)]
    pub policy_version: Option<PolicyVersion>,
    #[serde(default)]
    pub mask_rules: Vec<MaskRule>,
    #[serde(default)]
    pub drop_rules: Vec<DropRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DecisionLogArtifact {
    #[serde(default)]
    pub policy_version: Option<PolicyVersion>,
    #[serde(default)]
    pub masked_fields: Vec<String>,
    #[serde(default)]
    pub dropped_fields: Vec<String>,
    #[serde(default)]
    pub applied_mask_rule_ids: Vec<String>,
    #[serde(default)]
    pub applied_drop_rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionLogSanitizeOutcome {
    pub payload: Value,
    #[serde(default)]
    pub artifact: Option<DecisionLogArtifact>,
}

pub fn sanitize_decision_payload(payload: Value) -> DecisionLogSanitizeOutcome {
    let Some(policy) = detect_policy(&payload) else {
        return DecisionLogSanitizeOutcome {
            payload,
            artifact: None,
        };
    };

    let mut sanitized = payload;
    let mut artifact = DecisionLogArtifact {
        policy_version: policy.policy_version.clone(),
        ..DecisionLogArtifact::default()
    };

    for rule in &policy.drop_rules {
        let selector = parse_selector(&rule.selector);
        let mut hits = Vec::new();
        apply_drop_rule(&mut sanitized, &selector, String::new(), &mut hits);
        if !hits.is_empty() {
            artifact.applied_drop_rule_ids.push(rule.id.clone());
            artifact.dropped_fields.extend(hits);
        }
    }

    for rule in &policy.mask_rules {
        let selector = parse_selector(&rule.selector);
        let mut hits = Vec::new();
        apply_mask_rule(
            &mut sanitized,
            &selector,
            String::new(),
            &rule.strategy,
            &mut hits,
        );
        if !hits.is_empty() {
            artifact.applied_mask_rule_ids.push(rule.id.clone());
            artifact.masked_fields.extend(hits);
        }
    }

    artifact.masked_fields.sort();
    artifact.masked_fields.dedup();
    artifact.dropped_fields.sort();
    artifact.dropped_fields.dedup();
    artifact.applied_mask_rule_ids.sort();
    artifact.applied_mask_rule_ids.dedup();
    artifact.applied_drop_rule_ids.sort();
    artifact.applied_drop_rule_ids.dedup();

    if artifact.masked_fields.is_empty() && artifact.dropped_fields.is_empty() {
        return DecisionLogSanitizeOutcome {
            payload: sanitized,
            artifact: None,
        };
    }

    if let Some(object) = sanitized.as_object_mut() {
        object.insert(
            "decision_log_artifact".into(),
            serde_json::to_value(&artifact).unwrap_or_else(|_| Value::Null),
        );
    }

    DecisionLogSanitizeOutcome {
        payload: sanitized,
        artifact: Some(artifact),
    }
}

fn detect_policy(payload: &Value) -> Option<DecisionLogPolicy> {
    payload
        .get("decision_log_policy")
        .and_then(|value| serde_json::from_value::<DecisionLogPolicy>(value.clone()).ok())
        .or_else(|| {
            payload
                .get("pdp_decision")
                .and_then(|value| serde_json::from_value::<PolicyDecision>(value.clone()).ok())
                .map(policy_from_decision)
        })
        .or_else(|| {
            payload
                .get("policy_decision")
                .and_then(|value| serde_json::from_value::<PolicyDecision>(value.clone()).ok())
                .map(policy_from_decision)
        })
}

fn policy_from_decision(decision: PolicyDecision) -> DecisionLogPolicy {
    DecisionLogPolicy {
        policy_version: Some(decision.version),
        mask_rules: decision.mask_rules,
        drop_rules: decision.drop_rules,
    }
}

fn parse_selector(selector: &str) -> Vec<&str> {
    selector
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn apply_drop_rule(value: &mut Value, selector: &[&str], base_path: String, hits: &mut Vec<String>) {
    if selector.is_empty() {
        return;
    }
    if let Some((head, tail)) = selector.split_first() {
        if tail.is_empty() {
            remove_leaf(value, head, base_path, hits);
            return;
        }
        descend_children(value, head, base_path, |child, path| {
            apply_drop_rule(child, tail, path, hits);
        });
    }
}

fn apply_mask_rule(
    value: &mut Value,
    selector: &[&str],
    base_path: String,
    strategy: &str,
    hits: &mut Vec<String>,
) {
    if selector.is_empty() {
        return;
    }
    if let Some((head, tail)) = selector.split_first() {
        if tail.is_empty() {
            mask_leaf(value, head, base_path, strategy, hits);
            return;
        }
        descend_children(value, head, base_path, |child, path| {
            apply_mask_rule(child, tail, path, strategy, hits);
        });
    }
}

fn descend_children<F>(value: &mut Value, segment: &str, base_path: String, mut func: F)
where
    F: FnMut(&mut Value, String),
{
    match value {
        Value::Object(map) => {
            if segment == "*" {
                for (key, child) in map.iter_mut() {
                    let path = join_path(&base_path, key);
                    func(child, path);
                }
            } else if let Some(child) = map.get_mut(segment) {
                let path = join_path(&base_path, segment);
                func(child, path);
            }
        }
        Value::Array(items) => {
            if segment == "*" {
                for (index, child) in items.iter_mut().enumerate() {
                    let path = join_path(&base_path, &index.to_string());
                    func(child, path);
                }
            } else if let Ok(index) = segment.parse::<usize>() {
                if let Some(child) = items.get_mut(index) {
                    let path = join_path(&base_path, segment);
                    func(child, path);
                }
            }
        }
        _ => {}
    }
}

fn remove_leaf(value: &mut Value, segment: &str, base_path: String, hits: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if segment == "*" {
                let keys = map.keys().cloned().collect::<Vec<_>>();
                for key in keys {
                    map.remove(&key);
                    hits.push(join_path(&base_path, &key));
                }
            } else if map.remove(segment).is_some() {
                hits.push(join_path(&base_path, segment));
            }
        }
        Value::Array(items) => {
            if segment == "*" {
                let len = items.len();
                for index in 0..len {
                    hits.push(join_path(&base_path, &index.to_string()));
                }
                items.clear();
            } else if let Ok(index) = segment.parse::<usize>() {
                if index < items.len() {
                    items.remove(index);
                    hits.push(join_path(&base_path, segment));
                }
            }
        }
        _ => {}
    }
}

fn mask_leaf(
    value: &mut Value,
    segment: &str,
    base_path: String,
    strategy: &str,
    hits: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            if segment == "*" {
                let keys = map.keys().cloned().collect::<Vec<_>>();
                for key in keys {
                    if let Some(current) = map.get_mut(&key) {
                        *current = masked_value(current.clone(), strategy);
                        hits.push(join_path(&base_path, &key));
                    }
                }
            } else if let Some(current) = map.get_mut(segment) {
                *current = masked_value(current.clone(), strategy);
                hits.push(join_path(&base_path, segment));
            }
        }
        Value::Array(items) => {
            if segment == "*" {
                for (index, current) in items.iter_mut().enumerate() {
                    *current = masked_value(current.clone(), strategy);
                    hits.push(join_path(&base_path, &index.to_string()));
                }
            } else if let Ok(index) = segment.parse::<usize>() {
                if let Some(current) = items.get_mut(index) {
                    *current = masked_value(current.clone(), strategy);
                    hits.push(join_path(&base_path, segment));
                }
            }
        }
        _ => {}
    }
}

fn masked_value(value: Value, strategy: &str) -> Value {
    match strategy.to_ascii_lowercase().as_str() {
        "full" | "redact" => Value::String("[MASKED]".into()),
        "last4" => match value {
            Value::String(raw) => {
                if raw.len() <= 4 {
                    Value::String("[MASKED]".into())
                } else {
                    Value::String(format!("***{}", &raw[raw.len() - 4..]))
                }
            }
            _ => Value::String("[MASKED]".into()),
        },
        "hash" => {
            let mut hasher = DefaultHasher::new();
            value.to_string().hash(&mut hasher);
            Value::String(format!("hash:{:016x}", hasher.finish()))
        }
        unknown => Value::String(format!("[MASKED:{}]", unknown)),
    }
}

fn join_path(base: &str, segment: &str) -> String {
    if base.is_empty() {
        segment.to_string()
    } else {
        format!("{base}.{segment}")
    }
}

pub fn summarize_decision_log_artifacts(records: &[Value]) -> Value {
    let mut mask_count = 0_u64;
    let mut drop_count = 0_u64;
    let mut masked_fields = Vec::<String>::new();
    let mut dropped_fields = Vec::<String>::new();
    let mut versions = Vec::<String>::new();

    for record in records {
        let artifact = extract_artifact(record);
        let Some(artifact) = artifact else {
            continue;
        };
        mask_count += artifact
            .get("masked_fields")
            .and_then(Value::as_array)
            .map(|values| values.len() as u64)
            .unwrap_or(0);
        drop_count += artifact
            .get("dropped_fields")
            .and_then(Value::as_array)
            .map(|values| values.len() as u64)
            .unwrap_or(0);
        if let Some(items) = artifact.get("masked_fields").and_then(Value::as_array) {
            for item in items {
                if let Some(path) = item.as_str() {
                    masked_fields.push(path.to_string());
                }
            }
        }
        if let Some(items) = artifact.get("dropped_fields").and_then(Value::as_array) {
            for item in items {
                if let Some(path) = item.as_str() {
                    dropped_fields.push(path.to_string());
                }
            }
        }
        if let Some(version) = artifact
            .get("policy_version")
            .and_then(|value| value.get("id"))
            .and_then(Value::as_str)
        {
            versions.push(version.to_string());
        }
    }

    masked_fields.sort();
    masked_fields.dedup();
    dropped_fields.sort();
    dropped_fields.dedup();
    versions.sort();
    versions.dedup();

    serde_json::json!({
        "mask_applied_count": mask_count,
        "drop_applied_count": drop_count,
        "masked_fields": masked_fields,
        "dropped_fields": dropped_fields,
        "policy_versions": versions,
    })
}

fn extract_artifact(record: &Value) -> Option<&Value> {
    record
        .get("value")
        .and_then(|value| value.get("payload"))
        .and_then(|payload| payload.get("decision_log_artifact"))
        .or_else(|| record.get("payload").and_then(|payload| payload.get("decision_log_artifact")))
        .or_else(|| record.get("decision_log_artifact"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_decision_payload_applies_mask_and_drop_rules() {
        let payload = serde_json::json!({
            "tenant_id": "tenant-a",
            "secret": "abcd-efgh-1234",
            "token": "tok-001",
            "nested": {
                "secret": "nested-xyz"
            },
            "decision_log_policy": {
                "policy_version": {"id":"policy-v2","revision":2},
                "mask_rules": [
                    {"id":"mask-secret","selector":"secret","strategy":"last4","reason":"pii"},
                    {"id":"mask-nested","selector":"nested.secret","strategy":"full","reason":"pii"}
                ],
                "drop_rules": [
                    {"id":"drop-token","selector":"token","reason":"secret"}
                ]
            }
        });

        let output = sanitize_decision_payload(payload);
        let token = output.payload.get("token");
        assert!(token.is_none());
        assert_eq!(
            output.payload.get("secret").and_then(Value::as_str),
            Some("***1234")
        );
        assert_eq!(
            output
                .payload
                .get("nested")
                .and_then(|item| item.get("secret"))
                .and_then(Value::as_str),
            Some("[MASKED]")
        );
        assert!(output
            .payload
            .get("decision_log_artifact")
            .is_some_and(|value| value.is_object()));
    }

    #[test]
    fn summarize_artifacts_collects_counts() {
        let records = vec![
            serde_json::json!({
                "value": {
                    "payload": {
                        "decision_log_artifact": {
                            "policy_version": {"id":"policy-v2","revision":2},
                            "masked_fields": ["a.b"],
                            "dropped_fields": ["token"],
                        }
                    }
                }
            }),
            serde_json::json!({
                "value": {
                    "payload": {
                        "decision_log_artifact": {
                            "policy_version": {"id":"policy-v2","revision":2},
                            "masked_fields": ["a.c"],
                            "dropped_fields": [],
                        }
                    }
                }
            }),
        ];
        let summary = summarize_decision_log_artifacts(&records);
        assert_eq!(
            summary
                .get("mask_applied_count")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            summary
                .get("drop_applied_count")
                .and_then(Value::as_u64),
            Some(1)
        );
    }
}

