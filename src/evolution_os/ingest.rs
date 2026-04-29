use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::{
    contracts::evolution_os::RealitySnapshot,
    evolution_os::{
        proposal_hub::ExternalProposalSignals, worldline::TelemetryReplaySnapshot,
    },
};

const REALITY_FINGERPRINT_SCHEMA_VERSION: &str = "reality-fp/v3";
const REALITY_FINGERPRINT_SEED_VERSION: &str = "reality-seed/v1";

#[derive(Debug, Clone)]
pub struct IngestInput {
    pub session_id: String,
    pub trace_id: String,
    pub tenant_id: String,
    pub policy_version: String,
    pub runtime_mode: String,
    pub available_tools: Vec<String>,
    pub memory_refs: Vec<String>,
    pub graph_refs: Vec<String>,
    pub repo_refs: Vec<String>,
    pub policy_refs: Vec<String>,
    pub tool_refs: Vec<String>,
    pub budget_micros: u64,
    pub latency_budget_ms: u64,
    pub budget_profile: BTreeMap<String, u64>,
    pub now_ms: u64,
    pub telemetry_replay: Option<TelemetryReplaySnapshot>,
    pub proposal_signals: Option<ExternalProposalSignals>,
}

#[derive(Debug, Clone, Default)]
pub struct CanonicalRealityIngestor;

impl CanonicalRealityIngestor {
    pub fn ingest(&self, input: IngestInput) -> Result<RealitySnapshot> {
        if input.session_id.trim().is_empty() {
            bail!("session_id is required");
        }
        if input.trace_id.trim().is_empty() {
            bail!("trace_id is required");
        }
        if input.tenant_id.trim().is_empty() {
            bail!("tenant_id is required");
        }
        if input.budget_micros == 0 {
            bail!("budget_micros must be > 0");
        }

        let available_tools = normalize_refs(input.available_tools);
        let memory_refs = normalize_refs(input.memory_refs);
        let graph_refs = normalize_refs(input.graph_refs);

        let repo_refs = {
            let refs = normalize_refs(input.repo_refs);
            if refs.is_empty() {
                vec![format!("repo:session:{}", input.session_id)]
            } else {
                refs
            }
        };

        let policy_refs = {
            let mut refs = normalize_refs(input.policy_refs);
            refs.push(format!("policy_version:{}", input.policy_version));
            refs.push(format!("tenant:{}", input.tenant_id));
            normalize_refs(refs)
        };

        let tool_refs = {
            let refs = normalize_refs(input.tool_refs);
            if refs.is_empty() {
                available_tools.clone()
            } else {
                refs
            }
        };

        let mut budget_profile = input.budget_profile;
        if budget_profile.is_empty() {
            budget_profile.insert("token_budget".to_string(), input.budget_micros);
            budget_profile.insert("latency_budget_ms".to_string(), input.latency_budget_ms);
        }

        let repo_digest = digest_of_refs("repo", &repo_refs);
        let memory_digest = digest_of_refs("memory", &memory_refs);
        let graph_digest = digest_of_refs("graph", &graph_refs);
        let policy_digest = digest_of_refs("policy", &policy_refs);
        let tool_digest = digest_of_refs("tool", &tool_refs);
        let budget_digest = digest_of_budget(input.budget_micros, input.latency_budget_ms, &budget_profile);

        let reality_fingerprint = canonical_reality_fingerprint(
            &input.session_id,
            &input.trace_id,
            &input.tenant_id,
            &input.policy_version,
            &input.runtime_mode,
            &available_tools,
            &memory_refs,
            &graph_refs,
            &repo_refs,
            &policy_refs,
            &tool_refs,
            input.budget_micros,
            input.latency_budget_ms,
            &budget_profile,
            &repo_digest,
            &memory_digest,
            &graph_digest,
            &policy_digest,
            &tool_digest,
            &budget_digest,
        );

        let fingerprint_prefix = reality_fingerprint.chars().take(12).collect::<String>();

        Ok(RealitySnapshot {
            snapshot_id: format!(
                "reality:{}:{}:{}",
                input.session_id, input.now_ms, fingerprint_prefix
            ),
            session_id: input.session_id,
            trace_id: input.trace_id,
            tenant_id: input.tenant_id,
            policy_version: input.policy_version,
            runtime_mode: input.runtime_mode,
            available_tools,
            memory_refs,
            graph_refs,
            budget_micros: input.budget_micros,
            latency_budget_ms: input.latency_budget_ms,
            repo_refs,
            policy_refs,
            tool_refs,
            budget_profile,
            repo_digest,
            memory_digest,
            graph_digest,
            policy_digest,
            tool_digest,
            budget_digest,
            reality_fingerprint,
            created_at_ms: input.now_ms,
        })
    }
}

fn normalize_refs(values: Vec<String>) -> Vec<String> {
    let mut refs = values
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    refs.sort();
    refs.dedup();
    refs
}

fn digest_of_refs(domain: &str, refs: &[String]) -> String {
    let payload = if refs.is_empty() {
        format!("{domain}|<empty>")
    } else {
        format!("{domain}|{}", refs.join("|"))
    };
    digest_of_parts(&[domain, &payload])
}

fn digest_of_budget(
    budget_micros: u64,
    latency_budget_ms: u64,
    budget_profile: &BTreeMap<String, u64>,
) -> String {
    let mut pieces = vec![
        format!("budget_micros={budget_micros}"),
        format!("latency_budget_ms={latency_budget_ms}"),
    ];
    for (key, value) in budget_profile {
        pieces.push(format!("{key}={value}"));
    }
    digest_of_parts(&["budget", &pieces.join("|")])
}

fn digest_of_parts(parts: &[&str]) -> String {
    let payload = parts.join("::");
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in payload.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

#[allow(clippy::too_many_arguments)]
fn canonical_reality_fingerprint(
    session_id: &str,
    trace_id: &str,
    tenant_id: &str,
    policy_version: &str,
    runtime_mode: &str,
    available_tools: &[String],
    memory_refs: &[String],
    graph_refs: &[String],
    repo_refs: &[String],
    policy_refs: &[String],
    tool_refs: &[String],
    budget_micros: u64,
    latency_budget_ms: u64,
    budget_profile: &BTreeMap<String, u64>,
    repo_digest: &str,
    memory_digest: &str,
    graph_digest: &str,
    policy_digest: &str,
    tool_digest: &str,
    budget_digest: &str,
) -> String {
    let payload = serde_json::json!({
        "schema_version": REALITY_FINGERPRINT_SCHEMA_VERSION,
        "seed_version": REALITY_FINGERPRINT_SEED_VERSION,
        "session_id": session_id,
        "trace_id": trace_id,
        "tenant_id": tenant_id,
        "policy_version": policy_version,
        "runtime_mode": runtime_mode,
        "available_tools": available_tools,
        "memory_refs": memory_refs,
        "graph_refs": graph_refs,
        "repo_refs": repo_refs,
        "policy_refs": policy_refs,
        "tool_refs": tool_refs,
        "budget_micros": budget_micros,
        "latency_budget_ms": latency_budget_ms,
        "budget_profile": budget_profile,
        "repo_digest": repo_digest,
        "memory_digest": memory_digest,
        "graph_digest": graph_digest,
        "policy_digest": policy_digest,
        "tool_digest": tool_digest,
        "budget_digest": budget_digest,
    });
    let canonical_payload = canonical_json_string(&payload);
    digest_of_parts(&[
        REALITY_FINGERPRINT_SCHEMA_VERSION,
        REALITY_FINGERPRINT_SEED_VERSION,
        &canonical_payload,
    ])
}

fn canonical_json_string(value: &serde_json::Value) -> String {
    fn normalize(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut sorted = serde_json::Map::new();
                let mut keys = map.keys().cloned().collect::<Vec<_>>();
                keys.sort();
                for key in keys {
                    if let Some(inner) = map.get(&key) {
                        sorted.insert(key, normalize(inner));
                    }
                }
                serde_json::Value::Object(sorted)
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(normalize).collect())
            }
            _ => value.clone(),
        }
    }

    serde_json::to_string(&normalize(value)).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(now_ms: u64) -> IngestInput {
        IngestInput {
            session_id: "s1".to_string(),
            trace_id: "trace:s1:ingest".to_string(),
            tenant_id: "tenant-a".to_string(),
            policy_version: "policy-v2".to_string(),
            runtime_mode: "shadow".to_string(),
            available_tools: vec!["tool:b".into(), "tool:a".into()],
            memory_refs: vec!["memory:latest".into()],
            graph_refs: vec!["graph:latest".into()],
            repo_refs: vec!["repo://autoloop-app".into()],
            policy_refs: vec!["policy:tenant-a:default".into()],
            tool_refs: vec![],
            budget_micros: 100_000,
            latency_budget_ms: 2_000,
            budget_profile: BTreeMap::new(),
            now_ms,
            telemetry_replay: None,
            proposal_signals: None,
        }
    }

    #[test]
    fn same_input_produces_same_reality_fingerprint() {
        let ingestor = CanonicalRealityIngestor;
        let first = ingestor.ingest(sample_input(1)).expect("first ingest");
        let second = ingestor.ingest(sample_input(2)).expect("second ingest");
        assert_eq!(first.reality_fingerprint, second.reality_fingerprint);
        assert_eq!(first.repo_digest, second.repo_digest);
        assert_eq!(first.memory_digest, second.memory_digest);
        assert_eq!(first.graph_digest, second.graph_digest);
        assert_eq!(first.policy_digest, second.policy_digest);
        assert_eq!(first.tool_digest, second.tool_digest);
        assert_eq!(first.budget_digest, second.budget_digest);
    }

    #[test]
    fn snapshot_id_still_changes_with_time() {
        let ingestor = CanonicalRealityIngestor;
        let first = ingestor.ingest(sample_input(100)).expect("first ingest");
        let second = ingestor.ingest(sample_input(200)).expect("second ingest");
        assert_ne!(first.snapshot_id, second.snapshot_id);
        assert_eq!(first.reality_fingerprint, second.reality_fingerprint);
    }

    #[test]
    fn reality_fingerprint_stable_for_same_input_version() {
        let ingestor = CanonicalRealityIngestor;
        let first = ingestor.ingest(sample_input(1710000010000)).expect("first ingest");
        let second = ingestor
            .ingest(sample_input(1710000010000))
            .expect("second ingest");
        assert_eq!(
            first.reality_fingerprint, second.reality_fingerprint,
            "same input + same schema/seed version should keep deterministic fingerprint"
        );
    }
}
