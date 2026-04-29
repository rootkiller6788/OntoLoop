use serde_json::{Map, Value, json};

pub fn canonical_json_string(value: &Value) -> String {
    fn normalize(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut sorted = Map::new();
                let mut keys = map.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                for key in keys {
                    if let Some(inner) = map.get(&key) {
                        sorted.insert(key, normalize(inner));
                    }
                }
                Value::Object(sorted)
            }
            Value::Array(items) => Value::Array(items.iter().map(normalize).collect()),
            _ => value.clone(),
        }
    }

    serde_json::to_string(&normalize(value)).unwrap_or_else(|_| "{}".to_string())
}

pub fn build_fingerprint(
    namespace: &str,
    schema_version: &str,
    seed_version: &str,
    replay_version: &str,
    payload: &Value,
) -> String {
    let wrapped = json!({
        "schema_version": schema_version,
        "seed_version": seed_version,
        "replay_version": replay_version,
        "payload": payload,
    });
    let canonical_payload = canonical_json_string(&wrapped);
    let digest = digest_of_parts(&[
        namespace,
        schema_version,
        seed_version,
        replay_version,
        &canonical_payload,
    ]);
    format!("{namespace}:{digest}")
}

pub fn build_chain_fingerprint(
    schema_version: &str,
    seed_version: &str,
    replay_version: &str,
    components: &[(&str, &str)],
) -> String {
    let payload = json!({
        "components": components
            .iter()
            .map(|(domain, fingerprint)| json!({
                "domain": domain,
                "fingerprint": fingerprint,
            }))
            .collect::<Vec<_>>(),
    });
    build_fingerprint(
        "evoreplaychain",
        schema_version,
        seed_version,
        replay_version,
        &payload,
    )
}

pub fn default_version_drift_explainer(component: &str, replay_version: &str) -> String {
    format!(
        "fingerprint_drift_expected_when_{component}_replay_version_changes(current={replay_version})"
    )
}

fn digest_of_parts(parts: &[&str]) -> String {
    let payload = parts.join("::");
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in payload.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_payload_and_versions_keep_fingerprint_stable() {
        let payload = json!({
            "b": 2,
            "a": { "y": 2, "x": 1 }
        });
        let first = build_fingerprint("test", "schema/v1", "seed/v1", "replay/v1", &payload);
        let second = build_fingerprint(
            "test",
            "schema/v1",
            "seed/v1",
            "replay/v1",
            &json!({"a":{"x":1,"y":2},"b":2}),
        );
        assert_eq!(first, second);
    }

    #[test]
    fn version_changes_produce_explainable_drift() {
        let payload = json!({"x": 1});
        let first = build_fingerprint("test", "schema/v1", "seed/v1", "replay/v1", &payload);
        let second = build_fingerprint("test", "schema/v1", "seed/v1", "replay/v2", &payload);
        assert_ne!(first, second);
        assert!(
            default_version_drift_explainer("test", "replay/v2").contains("replay_version")
        );
    }
}
