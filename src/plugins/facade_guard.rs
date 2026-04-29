use anyhow::{Result, bail};

use crate::contracts::plugin::{
    PLUGIN_EVENT_CONTRACT_V2, PLUGIN_FACADE_CONTRACT_V2, PLUGIN_LIFECYCLE_CONTRACT_V2,
    PluginFacadeContract, PluginInvocationInput, PluginIsolationMode, PluginKind,
    PluginManifestContract,
};

pub fn enforce_manifest_contract(
    manifest: &PluginManifestContract,
    allow_builtin_relaxation: bool,
) -> Result<()> {
    if manifest.facade.contract_version != PLUGIN_FACADE_CONTRACT_V2 {
        bail!(
            "plugin '{}' violates facade contract version: expected={}, got={}",
            manifest.id,
            PLUGIN_FACADE_CONTRACT_V2,
            manifest.facade.contract_version
        );
    }
    if manifest.event_contract_version != PLUGIN_EVENT_CONTRACT_V2 {
        bail!(
            "plugin '{}' violates event contract version: expected={}, got={}",
            manifest.id,
            PLUGIN_EVENT_CONTRACT_V2,
            manifest.event_contract_version
        );
    }
    if manifest.lifecycle_contract_version != PLUGIN_LIFECYCLE_CONTRACT_V2 {
        bail!(
            "plugin '{}' violates lifecycle contract version: expected={}, got={}",
            manifest.id,
            PLUGIN_LIFECYCLE_CONTRACT_V2,
            manifest.lifecycle_contract_version
        );
    }
    if !manifest.facade.facade_only {
        bail!(
            "plugin '{}' violates facade contract: facade_only must be true",
            manifest.id
        );
    }
    if manifest.facade.allow_repo_direct {
        bail!(
            "plugin '{}' violates facade contract: direct repo access is forbidden",
            manifest.id
        );
    }
    if manifest.facade.allow_patch_state_direct {
        bail!(
            "plugin '{}' violates facade contract: direct patch-state access is forbidden",
            manifest.id
        );
    }
    if !matches!(manifest.isolation.mode, PluginIsolationMode::Subprocess) {
        bail!(
            "plugin '{}' violates isolation contract: subprocess mode is required",
            manifest.id
        );
    }

    let is_builtin = manifest
        .metadata
        .get("builtin")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !(allow_builtin_relaxation && is_builtin) {
        match manifest.kind {
            PluginKind::GraphProjection
            | PluginKind::VectorProjection
            | PluginKind::SearchProjection
            | PluginKind::SupermemoryFederation
            | PluginKind::SourceAdapter => {}
            _ => {
                bail!(
                    "plugin '{}' kind '{:?}' is outside day1/day2 allow-list",
                    manifest.id,
                    manifest.kind
                );
            }
        }
    }
    Ok(())
}

pub fn enforce_invocation_contract(
    facade: &PluginFacadeContract,
    input: &PluginInvocationInput,
) -> Result<()> {
    if !facade.facade_only {
        bail!("plugin facade contract violation: facade_only=false");
    }
    if input.metadata.get("facade_channel").map(String::as_str) != Some(PLUGIN_FACADE_CONTRACT_V2) {
        bail!("plugin facade contract violation: missing facade_channel=plugin-facade-v2");
    }

    let forbidden = [
        "repo",
        "canonical",
        "git",
        "patch_state",
        "patchstate",
        "patch_state_ref",
    ];
    if !facade.allow_repo_direct && contains_forbidden_token(&input.payload, &forbidden) {
        bail!("plugin facade contract violation: direct repo/git payload is forbidden");
    }
    if !facade.allow_patch_state_direct && contains_forbidden_token(&input.payload, &forbidden) {
        bail!("plugin facade contract violation: direct patch-state payload is forbidden");
    }

    for (key, value) in &input.metadata {
        let lowered = format!(
            "{}={}",
            key.to_ascii_lowercase(),
            value.to_ascii_lowercase()
        );
        if forbidden.iter().any(|token| lowered.contains(token)) {
            bail!(
                "plugin facade contract violation: forbidden metadata key/value '{}={}'",
                key,
                value
            );
        }
    }
    Ok(())
}

fn contains_forbidden_token(value: &serde_json::Value, tokens: &[&str]) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(_) => false,
        serde_json::Value::Number(_) => false,
        serde_json::Value::String(text) => {
            let lowered = text.to_ascii_lowercase();
            tokens.iter().any(|token| lowered.contains(token))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| contains_forbidden_token(item, tokens)),
        serde_json::Value::Object(map) => map.iter().any(|(key, value)| {
            let lowered = key.to_ascii_lowercase();
            tokens.iter().any(|token| lowered.contains(token))
                || contains_forbidden_token(value, tokens)
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::contracts::plugin::{
        PluginCapabilityDescriptor, PluginCompatSpec, PluginInvocationInput, PluginKind,
        PluginManifestContract, PluginRisk,
    };

    use super::{enforce_invocation_contract, enforce_manifest_contract};

    fn sample_manifest() -> PluginManifestContract {
        PluginManifestContract {
            id: "plugin:test".into(),
            plugin_id: "plugin:test".into(),
            version: "v2".into(),
            kind: PluginKind::SourceAdapter,
            capability: PluginCapabilityDescriptor {
                capability_id: "plugin:test:invoke".into(),
                description: "test".into(),
                scopes: vec!["plugin.invoke".into()],
            },
            risk: PluginRisk::Low,
            compat: PluginCompatSpec {
                api_version: "v2".into(),
                compatible_api_versions: vec!["v2".into()],
                min_core_version: "v2".into(),
                max_core_version: None,
            },
            name: "plugin-test".into(),
            source: "builtin://test".into(),
            signature_ref: None,
            permissions: vec![],
            hooks: vec![],
            commands: vec![],
            event_contract_version: "plugin-event-v2".into(),
            lifecycle_contract_version: "plugin-lifecycle-v2".into(),
            isolation: crate::contracts::plugin::PluginIsolationContract::default(),
            facade: crate::contracts::plugin::PluginFacadeContract::default(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn manifest_guard_rejects_wrong_facade_contract_version() {
        let mut manifest = sample_manifest();
        manifest.facade.contract_version = "plugin-facade-v1".into();
        let result = enforce_manifest_contract(&manifest, false);
        assert!(result.is_err());
    }

    #[test]
    fn invocation_guard_rejects_repo_bypass_payload() {
        let input = PluginInvocationInput {
            invocation_id: "inv".into(),
            plugin_id: "plugin:test".into(),
            session_id: "session:test".into(),
            tenant_id: "tenant:test".into(),
            principal_id: "principal:test".into(),
            capability_id: "plugin:test:invoke".into(),
            payload: serde_json::json!({"repo":"direct-access"}),
            metadata: BTreeMap::from([("facade_channel".into(), "plugin-facade-v2".into())]),
        };
        let result = enforce_invocation_contract(
            &crate::contracts::plugin::PluginFacadeContract::default(),
            &input,
        );
        assert!(result.is_err());
    }
}
