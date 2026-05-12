pub const VERSION_A_CONTRACT_VERSION: &str = "version-a/v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintDecision {
    Allow,
    RequiresApproval,
    Block,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct E2rLimit {
    pub max_review_round: u32,
    pub max_retry: u32,
    pub max_cost_micros: u64,
    pub max_time_ms: u64,
}

impl Default for E2rLimit {
    fn default() -> Self {
        Self {
            max_review_round: 3,
            max_retry: 3,
            max_cost_micros: 0,
            max_time_ms: 0,
        }
    }
}

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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct BanditStat {
    pub target_id: String,
    pub alpha: u32,
    pub beta: u32,
    pub success_count: u64,
    pub failure_count: u64,
}

impl BanditStat {
    pub fn default_for(target_id: impl Into<String>) -> Self {
        Self {
            target_id: target_id.into(),
            alpha: 1,
            beta: 1,
            success_count: 0,
            failure_count: 0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WalTxEnvelope {
    pub wal_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub decision: ConstraintDecision,
    pub state_change: serde_json::Value,
    pub event_log: serde_json::Value,
    pub evidence_ref: String,
    pub relation_event: serde_json::Value,
    pub write_proof: serde_json::Value,
    pub replay_fingerprint: String,
    pub created_at_ms: u64,
}

pub fn version_a_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == VERSION_A_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("version-a/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_a_compat_accepts_v1_series() {
        assert!(version_a_contract_compatible("version-a/v1"));
        assert!(version_a_contract_compatible("version-a/v1.1"));
        assert!(version_a_contract_compatible("VERSION-A/V1-beta"));
    }

    #[test]
    fn version_a_compat_rejects_other_majors() {
        assert!(!version_a_contract_compatible("version-a/v2"));
        assert!(!version_a_contract_compatible("v1"));
    }

    #[test]
    fn wal_tx_envelope_roundtrip_is_stable() {
        let envelope = WalTxEnvelope {
            wal_id: "wal:1".to_string(),
            session_id: "s1".to_string(),
            trace_id: "t1".to_string(),
            decision: ConstraintDecision::Allow,
            state_change: serde_json::json!({"kind":"state"}),
            event_log: serde_json::json!({"kind":"event_log"}),
            evidence_ref: "evidence:1".to_string(),
            relation_event: serde_json::json!({"kind":"relation_event"}),
            write_proof: serde_json::json!({"kind":"write_proof"}),
            replay_fingerprint: "fp:1".to_string(),
            created_at_ms: 1,
        };
        let encoded = serde_json::to_string(&envelope).expect("serialize WalTxEnvelope");
        let decoded: WalTxEnvelope = serde_json::from_str(&encoded).expect("deserialize WalTxEnvelope");
        assert_eq!(decoded, envelope);
    }
}
