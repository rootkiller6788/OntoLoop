use std::collections::BTreeMap;

pub const ARTIFACT_DELIVERY_CONTRACT_VERSION: &str = "artifact-delivery/v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactDeliveryStatus {
    Pending,
    Written,
    Verified,
    Rejected,
    RepairRequired,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactHashAlgorithm {
    Sha256,
    Sha512,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ArtifactValidationRules {
    #[serde(default = "default_exists_required")]
    pub exists_required: bool,
    #[serde(default, alias = "min_bytes")]
    pub min_size_bytes: Option<u64>,
    #[serde(default, alias = "max_bytes")]
    pub max_size_bytes: Option<u64>,
    #[serde(default, alias = "mime")]
    pub expected_mime: Option<String>,
    #[serde(default)]
    pub readable_required: bool,
    #[serde(default)]
    pub hash_algorithm: Option<ArtifactHashAlgorithm>,
}

fn default_exists_required() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ArtifactDeliveryReason {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub deny_reason: Option<String>,
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ArtifactWriteProof {
    #[serde(alias = "artifact_path")]
    pub path: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub hash_algorithm: Option<ArtifactHashAlgorithm>,
    #[serde(default, alias = "sha256")]
    pub hash: Option<String>,
    pub readable: bool,
    pub written_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ArtifactDeliveryContract {
    #[serde(default = "default_api_version")]
    pub api_version: String,
    pub session_id: String,
    pub trace_id: String,
    #[serde(default, alias = "must_write_artifact")]
    pub requires_artifact: bool,
    #[serde(alias = "artifact_path")]
    pub target_path: String,
    #[serde(default, alias = "checks")]
    pub validation_rules: ArtifactValidationRules,
    #[serde(default)]
    pub status: Option<ArtifactDeliveryStatus>,
    #[serde(default, alias = "proof")]
    pub write_proof: Option<ArtifactWriteProof>,
    #[serde(default)]
    pub reason: Option<ArtifactDeliveryReason>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
}

fn default_api_version() -> String {
    ARTIFACT_DELIVERY_CONTRACT_VERSION.to_string()
}

impl Default for ArtifactValidationRules {
    fn default() -> Self {
        Self {
            exists_required: true,
            min_size_bytes: None,
            max_size_bytes: None,
            expected_mime: None,
            readable_required: true,
            hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
        }
    }
}

pub fn artifact_delivery_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == ARTIFACT_DELIVERY_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("artifact-delivery/v") else {
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
    fn artifact_delivery_v1_roundtrip() {
        let contract = ArtifactDeliveryContract {
            api_version: ARTIFACT_DELIVERY_CONTRACT_VERSION.to_string(),
            session_id: "session-a".into(),
            trace_id: "trace-a".into(),
            requires_artifact: true,
            target_path: "D:/AutoLoop/output/demo.html".into(),
            validation_rules: ArtifactValidationRules {
                exists_required: true,
                min_size_bytes: Some(128),
                max_size_bytes: Some(1024 * 1024),
                expected_mime: Some("text/html".into()),
                readable_required: true,
                hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
            },
            status: Some(ArtifactDeliveryStatus::Verified),
            write_proof: Some(ArtifactWriteProof {
                path: "D:/AutoLoop/output/demo.html".into(),
                size_bytes: 2048,
                mime: Some("text/html".into()),
                hash_algorithm: Some(ArtifactHashAlgorithm::Sha256),
                hash: Some("abc123".into()),
                readable: true,
                written_at_ms: 1_717_000_000_000,
            }),
            reason: None,
            evidence_ref: Some("evidence:artifact:1".into()),
            replay_fp: Some("replay:fp:1".into()),
        };

        let raw = serde_json::to_string(&contract).expect("serialize artifact contract");
        let decoded: ArtifactDeliveryContract =
            serde_json::from_str(&raw).expect("deserialize artifact contract");
        assert_eq!(decoded, contract);
    }

    #[test]
    fn artifact_delivery_accepts_legacy_aliases() {
        let raw = serde_json::json!({
            "api_version": "artifact-delivery/v1",
            "session_id": "session-a",
            "trace_id": "trace-a",
            "must_write_artifact": true,
            "artifact_path": "D:/AutoLoop/output/demo.html",
            "checks": {
                "exists_required": true,
                "min_bytes": 64,
                "max_bytes": 4096,
                "mime": "text/html",
                "readable_required": true
            },
            "proof": {
                "artifact_path": "D:/AutoLoop/output/demo.html",
                "size_bytes": 256,
                "mime": "text/html",
                "sha256": "legacy-hash",
                "readable": true,
                "written_at_ms": 1_717_000_000_000u64
            }
        });

        let decoded: ArtifactDeliveryContract =
            serde_json::from_value(raw).expect("deserialize with aliases");
        assert!(decoded.requires_artifact);
        assert_eq!(decoded.target_path, "D:/AutoLoop/output/demo.html");
        assert_eq!(decoded.validation_rules.min_size_bytes, Some(64));
        assert_eq!(
            decoded
                .write_proof
                .as_ref()
                .and_then(|proof| proof.hash.clone()),
            Some("legacy-hash".into())
        );
    }

    #[test]
    fn artifact_delivery_compat_accepts_v1_series() {
        assert!(artifact_delivery_contract_compatible("artifact-delivery/v1"));
        assert!(artifact_delivery_contract_compatible("artifact-delivery/v1.1"));
        assert!(artifact_delivery_contract_compatible("ARTIFACT-DELIVERY/V1-beta"));
    }

    #[test]
    fn artifact_delivery_compat_rejects_other_majors() {
        assert!(!artifact_delivery_contract_compatible("artifact-delivery/v2"));
        assert!(!artifact_delivery_contract_compatible("v1"));
    }
}
