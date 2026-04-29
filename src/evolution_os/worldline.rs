use serde::{Deserialize, Serialize};

use crate::contracts::evolution_os::{CandidateGraph, WorldlineScore};

const WORLDLINE_REPLAY_SCHEMA_VERSION: &str = "worldline-replay/v1";
const WORLDLINE_REPLAY_SEED_VERSION: &str = "worldline-seed/v1";
const DEFAULT_WORLDLINE_WEIGHTS_VERSION: &str = "worldline-weights:default-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorldlineWeights {
    pub success_weight: f32,
    pub robustness_weight: f32,
    pub reuse_weight: f32,
    pub verifier_weight: f32,
    pub cost_weight: f32,
    pub latency_weight: f32,
    pub risk_weight: f32,
    pub instability_weight: f32,
    pub governance_weight: f32,
}

impl Default for WorldlineWeights {
    fn default() -> Self {
        Self {
            success_weight: 1.0,
            robustness_weight: 1.0,
            reuse_weight: 1.0,
            verifier_weight: 1.0,
            cost_weight: 1.0,
            latency_weight: 1.0,
            risk_weight: 1.0,
            instability_weight: 1.0,
            governance_weight: 1.0,
        }
    }
}

impl WorldlineWeights {
    pub fn sanitize(&self) -> Self {
        Self {
            success_weight: clamp_weight(self.success_weight),
            robustness_weight: clamp_weight(self.robustness_weight),
            reuse_weight: clamp_weight(self.reuse_weight),
            verifier_weight: clamp_weight(self.verifier_weight),
            cost_weight: clamp_weight(self.cost_weight),
            latency_weight: clamp_weight(self.latency_weight),
            risk_weight: clamp_weight(self.risk_weight),
            instability_weight: clamp_weight(self.instability_weight),
            governance_weight: clamp_weight(self.governance_weight),
        }
    }

    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let weights = value.get("weights").unwrap_or(value);
        Some(Self {
            success_weight: weights.get("success_weight")?.as_f64()? as f32,
            robustness_weight: weights.get("robustness_weight")?.as_f64()? as f32,
            reuse_weight: weights.get("reuse_weight")?.as_f64()? as f32,
            verifier_weight: weights.get("verifier_weight")?.as_f64()? as f32,
            cost_weight: weights.get("cost_weight")?.as_f64()? as f32,
            latency_weight: weights.get("latency_weight")?.as_f64()? as f32,
            risk_weight: weights.get("risk_weight")?.as_f64()? as f32,
            instability_weight: weights.get("instability_weight")?.as_f64()? as f32,
            governance_weight: weights.get("governance_weight")?.as_f64()? as f32,
        })
    }
}

fn clamp_weight(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 3.0)
    } else {
        1.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelemetryReplaySnapshot {
    pub verifier_score: f32,
    pub provider_retry_count: u32,
    pub tool_retry_count: u32,
    pub replay_mismatch_rate: f32,
    pub deterministic_boundary_respected: bool,
    pub latency_p95_ms: u64,
    #[serde(default)]
    pub worldline_weights: Option<WorldlineWeights>,
    #[serde(default)]
    pub weights_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldlineRecommendation {
    pub recommended_candidate_id: String,
    pub confidence: f32,
    pub ranked_candidates: Vec<(String, f32)>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct WorldlineEvaluator;

impl WorldlineEvaluator {
    pub fn score(&self, candidate: &CandidateGraph) -> WorldlineScore {
        self.score_with_snapshot(candidate, None)
    }

    pub fn score_with_snapshot(
        &self,
        candidate: &CandidateGraph,
        snapshot: Option<&TelemetryReplaySnapshot>,
    ) -> WorldlineScore {
        let telemetry = snapshot.cloned().unwrap_or_default();
        let weights = telemetry
            .worldline_weights
            .clone()
            .unwrap_or_default()
            .sanitize();
        let retry_total = telemetry.provider_retry_count + telemetry.tool_retry_count;
        let retry_penalty = (retry_total as f32 / 12.0).min(0.35);
        let replay_penalty = telemetry.replay_mismatch_rate.clamp(0.0, 1.0) * 0.4;
        let boundary_penalty = if telemetry.deterministic_boundary_respected {
            0.0
        } else {
            0.1
        };

        let telemetry_verifier = telemetry.verifier_score.clamp(0.0, 1.0);
        let baseline_verifier = (0.95 - candidate.expected_risk_score * 0.3).clamp(0.0, 1.0);
        let verifier_confidence = if snapshot.is_some() {
            (baseline_verifier * 0.6 + telemetry_verifier * 0.4).clamp(0.0, 1.0)
        } else {
            baseline_verifier
        };

        let telemetry_latency_penalty = if telemetry.latency_p95_ms == 0 {
            0.0
        } else {
            (telemetry.latency_p95_ms as f32 / 20_000.0).min(0.3)
        };

        let task_success = (1.0 - candidate.expected_risk_score * 0.4 - replay_penalty * 0.2)
            .clamp(0.0, 1.0);
        let robustness = (1.0
            - candidate.expected_risk_score * 0.5
            - replay_penalty * 0.6
            - retry_penalty * 0.4
            - boundary_penalty * 0.5)
            .clamp(0.0, 1.0);
        let reuse_gain = (0.6 + (1.0 - replay_penalty) * 0.15).clamp(0.0, 1.0);
        let cost_penalty = (candidate.expected_cost_micros as f32 / 1_000_000.0).min(1.0);
        let latency_penalty =
            ((candidate.expected_latency_ms as f32 / 10_000.0) + telemetry_latency_penalty).min(1.0);
        let risk_penalty = candidate.expected_risk_score.clamp(0.0, 1.0);
        let instability_penalty =
            (0.08 + retry_penalty + replay_penalty * 0.25 + boundary_penalty).clamp(0.0, 1.0);
        let governance_violation_penalty = if verifier_confidence < 0.55 {
            0.2
        } else {
            0.0
        };

        let positive_score = task_success * weights.success_weight
            + robustness * weights.robustness_weight
            + reuse_gain * weights.reuse_weight
            + verifier_confidence * weights.verifier_weight;
        let negative_score = cost_penalty * weights.cost_weight
            + latency_penalty * weights.latency_weight
            + risk_penalty * weights.risk_weight
            + instability_penalty * weights.instability_weight
            + governance_violation_penalty * weights.governance_weight;

        let total_score = positive_score - negative_score;
        let weights_version = telemetry
            .weights_version
            .clone()
            .unwrap_or_else(|| DEFAULT_WORLDLINE_WEIGHTS_VERSION.to_string());
        let replay_fingerprint = worldline_replay_fingerprint(
            candidate,
            &telemetry,
            &weights,
            &weights_version,
            retry_total,
            retry_penalty,
            replay_penalty,
            boundary_penalty,
            task_success,
            robustness,
            reuse_gain,
            verifier_confidence,
            cost_penalty,
            latency_penalty,
            risk_penalty,
            instability_penalty,
            governance_violation_penalty,
            total_score,
        );

        WorldlineScore {
            candidate_id: candidate.candidate_id.clone(),
            task_success,
            robustness,
            reuse_gain,
            verifier_confidence,
            cost_penalty,
            latency_penalty,
            risk_penalty,
            instability_penalty,
            governance_violation_penalty,
            total_score,
            reasons: vec![
                format!(
                    "weights_version={weights_version};success={:.2},robustness={:.2},reuse={:.2},verifier={:.2},cost={:.2},latency={:.2},risk={:.2},instability={:.2},governance={:.2}",
                    weights.success_weight,
                    weights.robustness_weight,
                    weights.reuse_weight,
                    weights.verifier_weight,
                    weights.cost_weight,
                    weights.latency_weight,
                    weights.risk_weight,
                    weights.instability_weight,
                    weights.governance_weight
                ),
                format!(
                    "positive={positive_score:.4};negative={negative_score:.4};retry_total={retry_total};replay_mismatch_rate={:.3};verifier={:.3}",
                    telemetry.replay_mismatch_rate.clamp(0.0, 1.0),
                    verifier_confidence
                ),
                format!(
                    "worldline_replay_fingerprint={replay_fingerprint};schema_version={WORLDLINE_REPLAY_SCHEMA_VERSION};seed_version={WORLDLINE_REPLAY_SEED_VERSION};weights_version={weights_version}"
                ),
            ],
            scored_at_ms: candidate.generated_at_ms,
        }
    }

    pub fn recommend(
        &self,
        scores: &[WorldlineScore],
        snapshot: Option<&TelemetryReplaySnapshot>,
    ) -> Option<WorldlineRecommendation> {
        if scores.is_empty() {
            return None;
        }
        let mut ranked = scores
            .iter()
            .map(|item| (item.candidate_id.clone(), item.total_score))
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });

        let best = ranked.first()?.clone();
        let second = ranked.get(1).map(|item| item.1).unwrap_or(best.1 - 0.15);
        let margin = (best.1 - second).max(0.0);
        let telemetry_boost = snapshot
            .map(|item| item.verifier_score.clamp(0.0, 1.0) * 0.25)
            .unwrap_or(0.15);
        let confidence = (0.5 + margin.min(0.35) + telemetry_boost).clamp(0.0, 1.0);

        let best_reason = scores
            .iter()
            .find(|item| item.candidate_id == best.0)
            .and_then(|item| item.reasons.first().cloned())
            .unwrap_or_else(|| "reason=unavailable".to_string());

        Some(WorldlineRecommendation {
            recommended_candidate_id: best.0.clone(),
            confidence,
            ranked_candidates: ranked,
            reasons: vec![
                format!("selected highest total_score candidate `{}`", best.0),
                format!("score_margin={:.3}", margin),
                format!("confidence={:.3}", confidence),
                format!("top_candidate_reason={best_reason}"),
                "tie_breaker=candidate_id_lexicographic_for_equal_scores".to_string(),
                "recommendation-only: no automatic production graph switch".to_string(),
            ],
        })
    }
}

fn qf(value: f32) -> String {
    format!("{:.6}", value)
}

#[allow(clippy::too_many_arguments)]
fn worldline_replay_fingerprint(
    candidate: &CandidateGraph,
    telemetry: &TelemetryReplaySnapshot,
    weights: &WorldlineWeights,
    weights_version: &str,
    retry_total: u32,
    retry_penalty: f32,
    replay_penalty: f32,
    boundary_penalty: f32,
    task_success: f32,
    robustness: f32,
    reuse_gain: f32,
    verifier_confidence: f32,
    cost_penalty: f32,
    latency_penalty: f32,
    risk_penalty: f32,
    instability_penalty: f32,
    governance_violation_penalty: f32,
    total_score: f32,
) -> String {
    let payload = serde_json::json!({
        "schema_version": WORLDLINE_REPLAY_SCHEMA_VERSION,
        "seed_version": WORLDLINE_REPLAY_SEED_VERSION,
        "weights_version": weights_version,
        "candidate": {
            "candidate_id": candidate.candidate_id,
            "graph_version": candidate.graph_version,
            "expected_cost_micros": candidate.expected_cost_micros,
            "expected_latency_ms": candidate.expected_latency_ms,
            "expected_risk_score": qf(candidate.expected_risk_score),
            "generated_at_ms": candidate.generated_at_ms,
        },
        "telemetry": {
            "verifier_score": qf(telemetry.verifier_score),
            "provider_retry_count": telemetry.provider_retry_count,
            "tool_retry_count": telemetry.tool_retry_count,
            "replay_mismatch_rate": qf(telemetry.replay_mismatch_rate),
            "deterministic_boundary_respected": telemetry.deterministic_boundary_respected,
            "latency_p95_ms": telemetry.latency_p95_ms,
        },
        "weights": {
            "success_weight": qf(weights.success_weight),
            "robustness_weight": qf(weights.robustness_weight),
            "reuse_weight": qf(weights.reuse_weight),
            "verifier_weight": qf(weights.verifier_weight),
            "cost_weight": qf(weights.cost_weight),
            "latency_weight": qf(weights.latency_weight),
            "risk_weight": qf(weights.risk_weight),
            "instability_weight": qf(weights.instability_weight),
            "governance_weight": qf(weights.governance_weight),
        },
        "derived": {
            "retry_total": retry_total,
            "retry_penalty": qf(retry_penalty),
            "replay_penalty": qf(replay_penalty),
            "boundary_penalty": qf(boundary_penalty),
            "task_success": qf(task_success),
            "robustness": qf(robustness),
            "reuse_gain": qf(reuse_gain),
            "verifier_confidence": qf(verifier_confidence),
            "cost_penalty": qf(cost_penalty),
            "latency_penalty": qf(latency_penalty),
            "risk_penalty": qf(risk_penalty),
            "instability_penalty": qf(instability_penalty),
            "governance_violation_penalty": qf(governance_violation_penalty),
            "total_score": qf(total_score),
        }
    });
    let canonical_payload = canonical_json_string(&payload);
    digest_of_parts(&[
        WORLDLINE_REPLAY_SCHEMA_VERSION,
        WORLDLINE_REPLAY_SEED_VERSION,
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

fn digest_of_parts(parts: &[&str]) -> String {
    let payload = parts.join("::");
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in payload.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("worldlinefp:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, cost: u64, latency: u64, risk: f32) -> CandidateGraph {
        CandidateGraph {
            candidate_id: id.to_string(),
            reality_snapshot_id: "r1".to_string(),
            graph_version: "g1".to_string(),
            node_ids: vec!["n1".to_string()],
            edges: vec![],
            budget_allocation: std::collections::BTreeMap::new(),
            expected_cost_micros: cost,
            expected_latency_ms: latency,
            expected_risk_score: risk,
            generated_at_ms: 1,
        }
    }

    #[test]
    fn weights_can_shift_candidate_preference() {
        let eval = WorldlineEvaluator;
        let cheap_high_risk = candidate("cheap-risky", 10_000, 1200, 0.75);
        let expensive_low_risk = candidate("exp-safe", 700_000, 1200, 0.10);

        let default_a = eval.score(&cheap_high_risk);
        let default_b = eval.score(&expensive_low_risk);
        assert!(default_b.total_score > default_a.total_score);

        let bias_cost = TelemetryReplaySnapshot {
            worldline_weights: Some(WorldlineWeights {
                risk_weight: 0.2,
                cost_weight: 2.6,
                ..WorldlineWeights::default()
            }),
            weights_version: Some("worldline-weights:hot:cost-bias".to_string()),
            ..TelemetryReplaySnapshot::default()
        };

        let biased_a = eval.score_with_snapshot(&cheap_high_risk, Some(&bias_cost));
        let biased_b = eval.score_with_snapshot(&expensive_low_risk, Some(&bias_cost));
        assert!(
            biased_a.total_score > biased_b.total_score,
            "hot-updated weights should change top candidate"
        );
    }

    #[test]
    fn recommendation_reasons_are_stable_for_equal_scores() {
        let eval = WorldlineEvaluator;
        let score_a = WorldlineScore {
            candidate_id: "candidate-a".to_string(),
            task_success: 1.0,
            robustness: 1.0,
            reuse_gain: 1.0,
            verifier_confidence: 1.0,
            cost_penalty: 0.0,
            latency_penalty: 0.0,
            risk_penalty: 0.0,
            instability_penalty: 0.0,
            governance_violation_penalty: 0.0,
            total_score: 1.0,
            reasons: vec!["reason-a".to_string()],
            scored_at_ms: 1,
        };
        let score_b = WorldlineScore {
            candidate_id: "candidate-b".to_string(),
            reasons: vec!["reason-b".to_string()],
            ..score_a.clone()
        };

        let first = eval
            .recommend(&[score_b.clone(), score_a.clone()], None)
            .expect("recommendation");
        let second = eval
            .recommend(&[score_a, score_b], None)
            .expect("recommendation");

        assert_eq!(
            first.recommended_candidate_id,
            second.recommended_candidate_id,
            "top candidate should be stable regardless of input ordering"
        );
        assert_eq!(first.reasons, second.reasons);
    }

    fn extract_replay_fp(score: &WorldlineScore) -> String {
        let marker = "worldline_replay_fingerprint=";
        let line = score
            .reasons
            .iter()
            .find(|item| item.contains(marker))
            .expect("worldline replay fingerprint reason");
        let start = line.find(marker).expect("marker") + marker.len();
        let tail = &line[start..];
        tail.split(';').next().unwrap_or_default().to_string()
    }

    #[test]
    fn worldline_replay_fingerprint_stable_for_same_input_and_version() {
        let eval = WorldlineEvaluator;
        let item = candidate("candidate-stable", 250_000, 1600, 0.22);
        let snapshot = TelemetryReplaySnapshot {
            verifier_score: 0.87,
            provider_retry_count: 1,
            tool_retry_count: 0,
            replay_mismatch_rate: 0.07,
            deterministic_boundary_respected: true,
            latency_p95_ms: 1500,
            worldline_weights: Some(WorldlineWeights::default()),
            weights_version: Some("worldline-weights:fixed-v2".to_string()),
        };
        let first = eval.score_with_snapshot(&item, Some(&snapshot));
        let second = eval.score_with_snapshot(&item, Some(&snapshot));
        assert_eq!(
            extract_replay_fp(&first),
            extract_replay_fp(&second),
            "same input + same weights version must keep replay fingerprint stable"
        );
    }

    #[test]
    fn worldline_replay_fingerprint_changes_when_weights_version_changes() {
        let eval = WorldlineEvaluator;
        let item = candidate("candidate-version", 250_000, 1600, 0.22);
        let snapshot_a = TelemetryReplaySnapshot {
            verifier_score: 0.87,
            provider_retry_count: 1,
            tool_retry_count: 0,
            replay_mismatch_rate: 0.07,
            deterministic_boundary_respected: true,
            latency_p95_ms: 1500,
            worldline_weights: Some(WorldlineWeights::default()),
            weights_version: Some("worldline-weights:fixed-v2".to_string()),
        };
        let snapshot_b = TelemetryReplaySnapshot {
            weights_version: Some("worldline-weights:fixed-v3".to_string()),
            ..snapshot_a.clone()
        };
        let first = eval.score_with_snapshot(&item, Some(&snapshot_a));
        let second = eval.score_with_snapshot(&item, Some(&snapshot_b));
        assert_ne!(
            extract_replay_fp(&first),
            extract_replay_fp(&second),
            "weights version changes must produce a different replay fingerprint"
        );
    }
}
