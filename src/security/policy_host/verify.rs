use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::contracts::policy_pdp::PolicyVersion;

use super::bundle::LoadedPolicyBundle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundleVerifyRequirements {
    pub policy_version: PolicyVersion,
    pub capabilities_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundleVerifyResult {
    pub verified: bool,
    pub bundle_hash: String,
    pub signature_digest: Option<String>,
    pub reasons: Vec<String>,
}

pub trait PolicyBundleVerifier: Send + Sync {
    fn verify(
        &self,
        bundle: &LoadedPolicyBundle,
        requirements: &PolicyBundleVerifyRequirements,
    ) -> Result<PolicyBundleVerifyResult>;
}

#[derive(Debug, Clone, Default)]
pub struct DefaultPolicyBundleVerifier;

impl PolicyBundleVerifier for DefaultPolicyBundleVerifier {
    fn verify(
        &self,
        bundle: &LoadedPolicyBundle,
        requirements: &PolicyBundleVerifyRequirements,
    ) -> Result<PolicyBundleVerifyResult> {
        let mut reasons = Vec::new();
        let bundle_hash = compute_bundle_hash(bundle)?;

        if bundle.manifest.policy_version != requirements.policy_version {
            reasons.push(format!(
                "policy_version mismatch expected={}#{} got={}#{}",
                requirements.policy_version.id,
                requirements.policy_version.revision,
                bundle.manifest.policy_version.id,
                bundle.manifest.policy_version.revision
            ));
        }

        let meta_capabilities = metadata_string(bundle, "capabilities_version");
        match meta_capabilities {
            Some(value) if value == requirements.capabilities_version => {}
            Some(value) => reasons.push(format!(
                "capabilities_version mismatch expected={} got={}",
                requirements.capabilities_version, value
            )),
            None => reasons.push("capabilities_version missing in manifest metadata".into()),
        }

        let declared_bundle_hash = metadata_string(bundle, "bundle_hash");
        match declared_bundle_hash {
            Some(value) if value == bundle_hash => {}
            Some(value) => reasons.push(format!(
                "bundle_hash mismatch declared={} computed={}",
                value, bundle_hash
            )),
            None => reasons.push("bundle_hash missing in manifest metadata".into()),
        }

        let signature_digest = metadata_string(bundle, "signature_digest");
        match signature_digest.as_ref() {
            Some(value) => {
                let computed_signature = compute_signature_digest(
                    &bundle.manifest.policy_id,
                    &bundle.manifest.policy_version,
                    &bundle_hash,
                    &requirements.capabilities_version,
                );
                if *value != computed_signature {
                    reasons.push(format!(
                        "signature_digest mismatch declared={} computed={}",
                        value, computed_signature
                    ));
                }
            }
            None => reasons.push("signature_digest missing in manifest metadata".into()),
        }

        Ok(PolicyBundleVerifyResult {
            verified: reasons.is_empty(),
            bundle_hash,
            signature_digest,
            reasons,
        })
    }
}

pub fn enforce_verified(result: &PolicyBundleVerifyResult) -> Result<()> {
    if result.verified {
        return Ok(());
    }
    bail!(
        "bundle verification failed: {}",
        result.reasons.join("; ")
    )
}

fn compute_bundle_hash(bundle: &LoadedPolicyBundle) -> Result<String> {
    let bytes = fs::read(&bundle.source_archive)?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    bundle.manifest.policy_id.hash(&mut hasher);
    bundle
        .manifest
        .policy_version
        .id
        .hash(&mut hasher);
    bundle
        .manifest
        .policy_version
        .revision
        .hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

fn compute_signature_digest(
    policy_id: &str,
    policy_version: &PolicyVersion,
    bundle_hash: &str,
    capabilities_version: &str,
) -> String {
    let mut hasher = DefaultHasher::new();
    policy_id.hash(&mut hasher);
    policy_version.id.hash(&mut hasher);
    policy_version.revision.hash(&mut hasher);
    bundle_hash.hash(&mut hasher);
    capabilities_version.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn metadata_string(bundle: &LoadedPolicyBundle, key: &str) -> Option<String> {
    bundle
        .manifest
        .metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::security::policy_host::bundle::PolicyBundleManifest;

    #[test]
    fn verify_rejects_when_required_fields_mismatch() {
        let temp_root = std::env::temp_dir().join(format!(
            "policy-verify-test-{}-{}",
            std::process::id(),
            current_time_ms()
        ));
        std::fs::create_dir_all(&temp_root).expect("temp dir");
        let archive_path = temp_root.join("bundle.tar.gz");
        std::fs::write(&archive_path, b"fake-bundle").expect("archive");

        let bundle = LoadedPolicyBundle {
            source_archive: archive_path,
            extracted_dir: temp_root.clone(),
            manifest_path: temp_root.join("manifest.json"),
            wasm_path: temp_root.join("policy.wasm"),
            data_path: temp_root.join("data.json"),
            manifest: PolicyBundleManifest {
                policy_id: "policy-a".into(),
                policy_version: PolicyVersion {
                    id: "v1".into(),
                    revision: 1,
                },
                wasm_entrypoint: "eval".into(),
                wasm_file: "policy.wasm".into(),
                data_file: "data.json".into(),
                metadata: json!({
                    "capabilities_version": "caps-v1",
                    "bundle_hash": "wrong",
                    "signature_digest": "wrong"
                }),
            },
            data: json!({"ok": true}),
        };

        let requirements = PolicyBundleVerifyRequirements {
            policy_version: PolicyVersion {
                id: "v2".into(),
                revision: 2,
            },
            capabilities_version: "caps-v2".into(),
        };

        let verifier = DefaultPolicyBundleVerifier;
        let result = verifier.verify(&bundle, &requirements).expect("verification");
        assert!(!result.verified);
        assert!(!result.reasons.is_empty());
    }

    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}
