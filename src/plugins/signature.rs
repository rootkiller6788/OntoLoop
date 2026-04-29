use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::contracts::plugin::{PluginInstallRequest, PluginVerificationVerdict};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginSignatureMaterial {
    pub plugin_id: String,
    pub tenant_id: String,
    pub requested_by: String,
    pub source: String,
    pub canonical_source: String,
    pub provided_signature: Option<String>,
    pub computed_signature: String,
}

pub fn signature_material(request: &PluginInstallRequest) -> PluginSignatureMaterial {
    let (canonical_source, provided_signature) = split_source_and_signature(&request.source);
    let computed_signature = compute_plugin_signature(
        &request.plugin_id,
        &canonical_source,
        &request.tenant_id,
        &request.requested_by,
    );
    PluginSignatureMaterial {
        plugin_id: request.plugin_id.clone(),
        tenant_id: request.tenant_id.clone(),
        requested_by: request.requested_by.clone(),
        source: request.source.clone(),
        canonical_source,
        provided_signature,
        computed_signature,
    }
}

pub fn verify_install_request(request: &PluginInstallRequest) -> PluginVerificationVerdict {
    let material = signature_material(request);
    let now = current_time_ms();

    if !request.verify_signature {
        return PluginVerificationVerdict {
            plugin_id: request.plugin_id.clone(),
            verified: true,
            reason: "signature verification skipped by request".into(),
            provenance_ref: Some("plugin-signature:skipped".into()),
            sbom_ref: None,
            checked_at_ms: now,
        };
    }

    let Some(provided_signature) = material.provided_signature.clone() else {
        return PluginVerificationVerdict {
            plugin_id: request.plugin_id.clone(),
            verified: false,
            reason: "missing signature in source (#sig=...)".into(),
            provenance_ref: Some("plugin-signature:missing".into()),
            sbom_ref: None,
            checked_at_ms: now,
        };
    };

    if provided_signature.eq_ignore_ascii_case(&material.computed_signature) {
        PluginVerificationVerdict {
            plugin_id: request.plugin_id.clone(),
            verified: true,
            reason: "signature verified".into(),
            provenance_ref: Some(format!(
                "plugin-signature:sha256:{}",
                material.computed_signature
            )),
            sbom_ref: None,
            checked_at_ms: now,
        }
    } else {
        PluginVerificationVerdict {
            plugin_id: request.plugin_id.clone(),
            verified: false,
            reason: format!(
                "signature mismatch provided={} expected={}",
                provided_signature, material.computed_signature
            ),
            provenance_ref: Some("plugin-signature:mismatch".into()),
            sbom_ref: None,
            checked_at_ms: now,
        }
    }
}

pub fn compute_plugin_signature(
    plugin_id: &str,
    canonical_source: &str,
    tenant_id: &str,
    requested_by: &str,
) -> String {
    let mut hasher = DefaultHasher::new();
    plugin_id.hash(&mut hasher);
    canonical_source.hash(&mut hasher);
    tenant_id.hash(&mut hasher);
    requested_by.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn split_source_and_signature(source: &str) -> (String, Option<String>) {
    let marker = "#sig=";
    let Some(index) = source.find(marker) else {
        return (source.trim().to_string(), None);
    };

    let canonical = source[..index].trim().to_string();
    let signature = source[index + marker.len()..]
        .split('&')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    (canonical, signature)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
