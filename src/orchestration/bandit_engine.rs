use anyhow::Result;
use autoloop_state_adapter::StateStore;
use rand::thread_rng;
use rand_distr::{Beta, Distribution};

use crate::contracts::version_a::BanditStat;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BanditShadowCandidate {
    pub candidate_id: String,
    pub base_score: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BanditShadowSample {
    pub candidate_id: String,
    pub alpha: u32,
    pub beta: u32,
    pub sampled_score: f64,
    pub posterior_mean: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BanditShadowDecision {
    pub session_id: String,
    pub trace_id: String,
    pub selected_candidate_id: Option<String>,
    pub samples: Vec<BanditShadowSample>,
    pub mode: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct BanditEngine;

impl BanditEngine {
    pub async fn choose_shadow(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        candidates: &[BanditShadowCandidate],
    ) -> Result<BanditShadowDecision> {
        if candidates.is_empty() {
            return Ok(BanditShadowDecision {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                selected_candidate_id: None,
                samples: Vec::new(),
                mode: "shadow".to_string(),
                reason: "no_constraint_shield_allowed_candidates".to_string(),
            });
        }

        let mut samples = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let stat = load_stat(db, session_id, &candidate.candidate_id).await?;
            let alpha = stat.alpha.max(1) as f64;
            let beta = stat.beta.max(1) as f64;
            let sampled = Beta::new(alpha, beta)
                .map(|dist| dist.sample(&mut thread_rng()))
                .unwrap_or(stat.alpha as f64 / (stat.alpha as f64 + stat.beta as f64));
            let posterior_mean = stat.alpha as f64 / (stat.alpha as f64 + stat.beta as f64);
            samples.push(BanditShadowSample {
                candidate_id: candidate.candidate_id.clone(),
                alpha: stat.alpha,
                beta: stat.beta,
                sampled_score: sampled,
                posterior_mean,
            });
        }

        samples.sort_by(|left, right| {
            right
                .sampled_score
                .total_cmp(&left.sampled_score)
                .then_with(|| right.posterior_mean.total_cmp(&left.posterior_mean))
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });

        let selected_candidate_id = samples.first().map(|item| item.candidate_id.clone());
        Ok(BanditShadowDecision {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            selected_candidate_id,
            samples,
            mode: "shadow".to_string(),
            reason: "thompson_sampling_over_constraint_shield_allowed_candidates".to_string(),
        })
    }

    pub async fn update_outcome(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        candidate_id: &str,
        success: bool,
    ) -> Result<BanditStat> {
        let mut stat = load_stat(db, session_id, candidate_id).await?;
        if success {
            stat.alpha = stat.alpha.saturating_add(1);
            stat.success_count = stat.success_count.saturating_add(1);
        } else {
            stat.beta = stat.beta.saturating_add(1);
            stat.failure_count = stat.failure_count.saturating_add(1);
        }

        let now_ms = crate::orchestration::current_time_ms();
        let latest_key = stat_latest_key(session_id, candidate_id);
        let history_key = format!("bandit:stat:{session_id}:{candidate_id}:{now_ms}");
        let evolution_key = format!("bandit:evolution:{session_id}:{trace_id}:{candidate_id}:{now_ms}");
        let evolution_payload = serde_json::json!({
            "session_id": session_id,
            "trace_id": trace_id,
            "candidate_id": candidate_id,
            "success": success,
            "alpha": stat.alpha,
            "beta": stat.beta,
            "success_count": stat.success_count,
            "failure_count": stat.failure_count,
            "posterior_mean": stat.alpha as f64 / (stat.alpha as f64 + stat.beta as f64),
            "updated_at_ms": now_ms,
        });
        let _ = db
            .upsert_json_knowledge(latest_key, &stat, "bandit-shadow")
            .await;
        let _ = db
            .upsert_json_knowledge(history_key, &stat, "bandit-shadow")
            .await;
        let _ = db
            .upsert_json_knowledge(evolution_key, &evolution_payload, "bandit-shadow")
            .await;
        Ok(stat)
    }
}

async fn load_stat(db: &StateStore, session_id: &str, candidate_id: &str) -> Result<BanditStat> {
    let key = stat_latest_key(session_id, candidate_id);
    let maybe = db.get_knowledge(&key).await?;
    if let Some(record) = maybe {
        let parsed: BanditStat = serde_json::from_str(&record.value)?;
        return Ok(parsed);
    }
    Ok(BanditStat::default_for(candidate_id.to_string()))
}

fn stat_latest_key(session_id: &str, candidate_id: &str) -> String {
    format!("bandit:stat:{session_id}:{candidate_id}:latest")
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    fn test_db() -> StateStore {
        StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        })
    }

    #[tokio::test]
    async fn chooses_shadow_candidate_from_allowed_pool() {
        let db = test_db();
        let decision = BanditEngine::choose_shadow(
            &db,
            "session-bandit",
            "trace-bandit-1",
            &[
                BanditShadowCandidate {
                    candidate_id: "tool:a".into(),
                    base_score: 0.8,
                },
                BanditShadowCandidate {
                    candidate_id: "tool:b".into(),
                    base_score: 0.7,
                },
            ],
        )
        .await
        .expect("shadow choose");
        assert_eq!(decision.mode, "shadow");
        assert_eq!(decision.reason, "thompson_sampling_over_constraint_shield_allowed_candidates");
        assert!(!decision.samples.is_empty());
        assert!(decision.selected_candidate_id.is_some());
    }

    #[tokio::test]
    async fn updates_alpha_beta_posterior_counts() {
        let db = test_db();
        let first = BanditEngine::update_outcome(
            &db,
            "session-bandit",
            "trace-bandit-2",
            "tool:a",
            true,
        )
        .await
        .expect("first update");
        assert_eq!(first.alpha, 2);
        assert_eq!(first.beta, 1);
        assert_eq!(first.success_count, 1);
        assert_eq!(first.failure_count, 0);

        let second = BanditEngine::update_outcome(
            &db,
            "session-bandit",
            "trace-bandit-3",
            "tool:a",
            false,
        )
        .await
        .expect("second update");
        assert_eq!(second.alpha, 2);
        assert_eq!(second.beta, 2);
        assert_eq!(second.success_count, 1);
        assert_eq!(second.failure_count, 1);
    }
}
