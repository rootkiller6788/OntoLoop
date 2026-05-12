#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RankingWeight {
    pub capability_match: f32,
    pub success_rate: f32,
    pub trust_score: f32,
    pub risk_score: f32,
    pub cost_score: f32,
}

impl Default for RankingWeight {
    fn default() -> Self {
        Self {
            capability_match: 0.30,
            success_rate: 0.25,
            trust_score: 0.20,
            risk_score: 0.15,
            cost_score: 0.10,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RankingFeatures {
    pub capability_match: f32,
    pub success_rate: f32,
    pub trust_score: f32,
    pub risk_score: f32,
    pub cost_score: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RankedCandidate {
    pub candidate_id: String,
    pub score: f32,
    pub rank_reason: String,
    pub features: RankingFeatures,
}

#[derive(Debug, Clone)]
pub struct RankingEngine {
    pub weights: RankingWeight,
}

impl Default for RankingEngine {
    fn default() -> Self {
        Self {
            weights: RankingWeight::default(),
        }
    }
}

impl RankingEngine {
    pub fn score(&self, features: &RankingFeatures) -> f32 {
        let capability = clamp01(features.capability_match);
        let success = clamp01(features.success_rate);
        let trust = clamp01(features.trust_score);
        let risk = clamp01(features.risk_score);
        let cost = clamp01(features.cost_score);
        self.weights.capability_match * capability
            + self.weights.success_rate * success
            + self.weights.trust_score * trust
            - self.weights.risk_score * risk
            - self.weights.cost_score * cost
    }

    pub fn rank_reason(&self, features: &RankingFeatures, score: f32) -> String {
        format!(
            "rank_score={score:.3} (0.30*capability_match={:.2} + 0.25*success_rate={:.2} + 0.20*trust_score={:.2} - 0.15*risk_score={:.2} - 0.10*cost_score={:.2})",
            clamp01(features.capability_match),
            clamp01(features.success_rate),
            clamp01(features.trust_score),
            clamp01(features.risk_score),
            clamp01(features.cost_score),
        )
    }

    pub fn rank_candidates(
        &self,
        candidates: impl IntoIterator<Item = (String, RankingFeatures)>,
    ) -> Vec<RankedCandidate> {
        let mut ranked = candidates
            .into_iter()
            .map(|(candidate_id, features)| {
                let score = self.score(&features);
                let rank_reason = self.rank_reason(&features, score);
                RankedCandidate {
                    candidate_id,
                    score,
                    rank_reason,
                    features,
                }
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.candidate_id.cmp(&b.candidate_id))
        });
        ranked
    }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranking_is_stable_and_prefers_higher_score() {
        let engine = RankingEngine::default();
        let ranked = engine.rank_candidates(vec![
            (
                "candidate-low".to_string(),
                RankingFeatures {
                    capability_match: 0.6,
                    success_rate: 0.5,
                    trust_score: 0.5,
                    risk_score: 0.8,
                    cost_score: 0.8,
                },
            ),
            (
                "candidate-high".to_string(),
                RankingFeatures {
                    capability_match: 0.9,
                    success_rate: 0.9,
                    trust_score: 0.8,
                    risk_score: 0.1,
                    cost_score: 0.2,
                },
            ),
        ]);
        assert_eq!(ranked.first().map(|c| c.candidate_id.as_str()), Some("candidate-high"));
        assert!(ranked[0].rank_reason.contains("rank_score="));
    }
}
