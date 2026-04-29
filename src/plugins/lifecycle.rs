use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Result, anyhow, bail};
use async_trait::async_trait;
use tokio::sync::RwLock;

use autoloop_state_adapter::StateStore;

use crate::contracts::{
    capability::CapabilityCandidate,
    errors::{ContractError, RuntimeError},
    ids::SessionId,
    plugin::{
        PLUGIN_API_VERSION_V2, PLUGIN_EVENT_CONTRACT_V2, PLUGIN_LIFECYCLE_CONTRACT_V2,
        PluginCapabilityDescriptor, PluginCompatSpec, PluginFacadeContract, PluginInstallRequest,
        PluginIsolationContract, PluginKind, PluginLifecycleEvent, PluginManifestContract,
        PluginRisk, PluginRolloutMode, PluginState, PluginVerificationVerdict,
    },
    ports::{CapabilityAdmissionPort, PluginLifecyclePort},
    version::CONTRACT_VERSION,
};
use crate::observability::event_stream::digest_value;
use crate::runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage};
use crate::security::capability_admission::CapabilityAdmissionEngine;

use super::compatibility_loader::PluginCompatibilityLoader;
use super::facade_guard::enforce_manifest_contract;
use super::host::{ExternalPluginLoadRequest, PluginHost, SubprocessPluginLoader};
use super::signature::{
    compute_plugin_signature, signature_material, split_source_and_signature, verify_install_request,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginRuntimeRecord {
    pub plugin_id: String,
    pub current_manifest: PluginManifestContract,
    pub state: PluginState,
    pub verified: bool,
    pub history: Vec<PluginManifestContract>,
    pub last_event: Option<PluginLifecycleEvent>,
    pub last_verdict: Option<PluginVerificationVerdict>,
    pub rollout_mode: Option<PluginRolloutMode>,
    pub rollout_traffic_percent: Option<u8>,
}

#[derive(Clone)]
pub struct PluginLifecycleManager {
    state_store: StateStore,
    records: Arc<RwLock<BTreeMap<String, PluginRuntimeRecord>>>,
    host: PluginHost,
}

impl PluginLifecycleManager {
    pub fn new(state_store: StateStore) -> Self {
        Self {
            state_store,
            records: Arc::new(RwLock::new(BTreeMap::new())),
            host: PluginHost::new(PLUGIN_API_VERSION_V2),
        }
    }

    pub async fn install(&self, request: &PluginInstallRequest) -> Result<PluginManifestContract> {
        self.emit_hook(
            "pre_install",
            &request.plugin_id,
            serde_json::json!({
                "source": request.source,
                "tenant_id": request.tenant_id,
                "requested_by": request.requested_by,
            }),
        )
        .await?;

        let material = signature_material(request);
        if material.provided_signature.is_none() {
            let verdict = crate::contracts::plugin::PluginVerificationVerdict {
                plugin_id: request.plugin_id.clone(),
                verified: false,
                reason: "plugin install rejected: unsigned source (missing #sig=...)".to_string(),
                provenance_ref: Some("plugin-signature:missing".to_string()),
                sbom_ref: None,
                checked_at_ms: current_time_ms(),
            };
            self.persist_verdict(&verdict).await?;
            bail!("plugin install rejected: {}", verdict.reason);
        }

        if !request.verify_signature {
            let verdict = crate::contracts::plugin::PluginVerificationVerdict {
                plugin_id: request.plugin_id.clone(),
                verified: false,
                reason: "plugin install rejected: verify_signature=false is not allowed".to_string(),
                provenance_ref: Some("plugin-signature:bypass-blocked".to_string()),
                sbom_ref: None,
                checked_at_ms: current_time_ms(),
            };
            self.persist_verdict(&verdict).await?;
            bail!("plugin install rejected: {}", verdict.reason);
        }

        let verdict = verify_install_request(request);
        if !verdict.verified {
            self.persist_verdict(&verdict).await?;
            bail!("plugin install rejected: {}", verdict.reason);
        }

        let (canonical_source, _) = split_source_and_signature(&request.source);
        let now = current_time_ms();
        let signature = compute_plugin_signature(
            &request.plugin_id,
            &canonical_source,
            &request.tenant_id,
            &request.requested_by,
        );
        let manifest = PluginManifestContract {
            id: request.plugin_id.clone(),
            plugin_id: request.plugin_id.clone(),
            version: "v2".into(),
            kind: plugin_kind_from_source(&canonical_source),
            capability: PluginCapabilityDescriptor {
                capability_id: format!("plugin:{}", request.plugin_id),
                description: "plugin runtime capability".into(),
                scopes: vec!["plugin.invoke".into()],
            },
            risk: PluginRisk::Medium,
            compat: PluginCompatSpec {
                api_version: PLUGIN_API_VERSION_V2.into(),
                compatible_api_versions: vec![PLUGIN_API_VERSION_V2.into()],
                min_core_version: PLUGIN_API_VERSION_V2.into(),
                max_core_version: None,
            },
            name: request.plugin_id.clone(),
            source: canonical_source,
            signature_ref: Some(signature),
            permissions: vec!["read".into()],
            hooks: vec![
                "pre_install".into(),
                "post_install".into(),
                "post_enable".into(),
                "post_disable".into(),
                "post_update".into(),
                "post_rollback".into(),
            ],
            commands: vec![format!("plugin:{}:invoke", request.plugin_id)],
            event_contract_version: PLUGIN_EVENT_CONTRACT_V2.to_string(),
            lifecycle_contract_version: PLUGIN_LIFECYCLE_CONTRACT_V2.to_string(),
            isolation: PluginIsolationContract::default(),
            facade: PluginFacadeContract::default(),
            metadata: BTreeMap::from([
                ("tenant_id".into(), request.tenant_id.clone()),
                ("requested_by".into(), request.requested_by.clone()),
                ("installed_at_ms".into(), now.to_string()),
            ]),
        };

        enforce_manifest_contract(&manifest, false)?;

        let _admission_ref = self
            .enforce_plugin_capability_admission(
                &manifest,
                &verdict,
                "install",
                &request.requested_by,
                Some(&request.tenant_id),
            )
            .await?;

        let lifecycle_event = PluginLifecycleEvent {
            plugin_id: request.plugin_id.clone(),
            from_state: None,
            to_state: PluginState::Installed,
            reason: "plugin installed".into(),
            operator: request.requested_by.clone(),
            at_ms: now,
        };

        let record = PluginRuntimeRecord {
            plugin_id: request.plugin_id.clone(),
            current_manifest: manifest.clone(),
            state: PluginState::Installed,
            verified: verdict.verified,
            history: vec![manifest.clone()],
            last_event: Some(lifecycle_event.clone()),
            last_verdict: Some(verdict.clone()),
            rollout_mode: None,
            rollout_traffic_percent: None,
        };

        self.records
            .write()
            .await
            .insert(request.plugin_id.clone(), record);
        self.persist_manifest(&manifest, &PluginState::Installed)
            .await?;
        self.persist_event(&lifecycle_event).await?;
        self.persist_verdict(&verdict).await?;
        self.persist_index().await?;

        self.emit_hook(
            "post_install",
            &request.plugin_id,
            serde_json::to_value(&manifest).unwrap_or_else(|_| serde_json::json!({})),
        )
        .await?;
        Ok(manifest)
    }

    pub async fn enable(
        &self,
        plugin_id: &str,
        operator: &str,
        reason: &str,
    ) -> Result<PluginManifestContract> {
        self.transition_state(plugin_id, PluginState::Enabled, operator, reason)
            .await
    }

    pub async fn disable(
        &self,
        plugin_id: &str,
        operator: &str,
        reason: &str,
    ) -> Result<PluginManifestContract> {
        self.transition_state(plugin_id, PluginState::Disabled, operator, reason)
            .await
    }

    pub async fn update(
        &self,
        plugin_id: &str,
        source: Option<&str>,
        operator: &str,
    ) -> Result<PluginManifestContract> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(plugin_id)
            .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
        let previous = record.current_manifest.clone();
        let from_state = record.state.clone();

        let mut next = previous.clone();
        next.version = bump_version(&previous.version);
        if let Some(source) = source {
            let (canonical, _) = split_source_and_signature(source);
            if !canonical.is_empty() {
                next.source = canonical;
            }
        }
        next.metadata
            .insert("updated_by".into(), operator.to_string());
        next.metadata
            .insert("updated_at_ms".into(), current_time_ms().to_string());
        next.signature_ref = Some(compute_plugin_signature(
            plugin_id,
            &next.source,
            next.metadata
                .get("tenant_id")
                .map(String::as_str)
                .unwrap_or("tenant:default"),
            next.metadata
                .get("requested_by")
                .map(String::as_str)
                .unwrap_or(operator),
        ));

        enforce_manifest_contract(&next, false)?;
        record.current_manifest = next.clone();
        record.history.push(next.clone());
        record.state = PluginState::Installed;
        let event = PluginLifecycleEvent {
            plugin_id: plugin_id.to_string(),
            from_state: Some(from_state),
            to_state: PluginState::Installed,
            reason: "plugin updated".into(),
            operator: operator.to_string(),
            at_ms: current_time_ms(),
        };
        record.last_event = Some(event.clone());
        drop(records);

        self.persist_manifest(&next, &PluginState::Installed)
            .await?;
        self.persist_event(&event).await?;
        self.persist_index().await?;
        self.emit_hook(
            "post_update",
            plugin_id,
            serde_json::json!({"version": next.version}),
        )
        .await?;
        Ok(next)
    }

    pub async fn rollback(
        &self,
        plugin_id: &str,
        operator: &str,
        reason: &str,
    ) -> Result<PluginManifestContract> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(plugin_id)
            .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
        if record.history.len() < 2 {
            bail!("plugin '{}' has no previous version to rollback", plugin_id);
        }
        let rolled_back_from = record.current_manifest.clone();
        let previous = record.history[record.history.len() - 2].clone();
        record.current_manifest = previous.clone();
        record.state = PluginState::RolledBack;
        let event = PluginLifecycleEvent {
            plugin_id: plugin_id.to_string(),
            from_state: Some(PluginState::Installed),
            to_state: PluginState::RolledBack,
            reason: if reason.trim().is_empty() {
                "plugin rollback".into()
            } else {
                reason.to_string()
            },
            operator: operator.to_string(),
            at_ms: current_time_ms(),
        };
        record.last_event = Some(event.clone());
        drop(records);

        self.persist_manifest(&previous, &PluginState::RolledBack)
            .await?;
        self.persist_event(&event).await?;
        self.persist_index().await?;
        self.emit_hook(
            "post_rollback",
            plugin_id,
            serde_json::json!({
                "rolled_back_from": rolled_back_from.version,
                "active_version": previous.version,
            }),
        )
        .await?;
        Ok(previous)
    }

    pub async fn rollout(
        &self,
        plugin_id: &str,
        mode: PluginRolloutMode,
        traffic_percent: Option<u8>,
        operator: &str,
        reason: &str,
    ) -> Result<PluginManifestContract> {
        let normalized_percent = match &mode {
            PluginRolloutMode::Shadow => 0,
            PluginRolloutMode::Canary => traffic_percent.unwrap_or(10).clamp(1, 99),
            PluginRolloutMode::Full => 100,
            PluginRolloutMode::Rollback => 0,
        };

        if matches!(&mode, PluginRolloutMode::Rollback) {
            return self
                .rollback(
                    plugin_id,
                    operator,
                    if reason.trim().is_empty() {
                        "plugin rollout rollback"
                    } else {
                        reason
                    },
                )
                .await;
        }

        if !matches!(&mode, PluginRolloutMode::Shadow) {
            let _ = self
                .enable(
                    plugin_id,
                    operator,
                    if reason.trim().is_empty() {
                        "plugin rollout enable"
                    } else {
                        reason
                    },
                )
                .await?;
        }

        let mut records = self.records.write().await;
        let record = records
            .get_mut(plugin_id)
            .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
        record.rollout_mode = Some(mode.clone());
        record.rollout_traffic_percent = Some(normalized_percent);
        record
            .current_manifest
            .metadata
            .insert("rollout_mode".into(), rollout_mode_label(&mode).to_string());
        record.current_manifest.metadata.insert(
            "rollout_traffic_percent".into(),
            normalized_percent.to_string(),
        );
        record.current_manifest.metadata.insert(
            "rollout_updated_at_ms".into(),
            current_time_ms().to_string(),
        );

        let manifest = record.current_manifest.clone();
        drop(records);

        self.persist_manifest(&manifest, &manifest_state_for_mode(&mode))
            .await?;
        self.persist_rollout_event(plugin_id, &mode, normalized_percent, operator, reason)
            .await?;
        self.persist_index().await?;
        self.emit_hook(
            "post_rollout",
            plugin_id,
            serde_json::json!({
                "mode": mode,
                "traffic_percent": normalized_percent,
                "reason": reason,
                "operator": operator,
            }),
        )
        .await?;
        Ok(manifest)
    }

    pub async fn quick_rollback(
        &self,
        plugin_id: &str,
        operator: &str,
    ) -> Result<PluginManifestContract> {
        self.rollback(plugin_id, operator, "plugin quick rollback")
            .await
    }
    pub async fn verify(&self, plugin_id: &str) -> Result<PluginVerificationVerdict> {
        let mut records = self.records.write().await;
        let record = records
            .get_mut(plugin_id)
            .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
        let manifest = &record.current_manifest;
        let tenant = manifest
            .metadata
            .get("tenant_id")
            .map(String::as_str)
            .unwrap_or("tenant:default");
        let requested_by = manifest
            .metadata
            .get("requested_by")
            .map(String::as_str)
            .unwrap_or("unknown");
        let expected = compute_plugin_signature(plugin_id, &manifest.source, tenant, requested_by);
        let verified = manifest
            .signature_ref
            .as_ref()
            .map(|sig| sig.eq_ignore_ascii_case(&expected))
            .unwrap_or(false);
        let verdict = PluginVerificationVerdict {
            plugin_id: plugin_id.to_string(),
            verified,
            reason: if verified {
                "plugin signature verified".into()
            } else {
                "plugin signature verification failed".into()
            },
            provenance_ref: Some(format!("plugin-signature:{}", manifest.version)),
            sbom_ref: None,
            checked_at_ms: current_time_ms(),
        };
        record.verified = verified;
        record.last_verdict = Some(verdict.clone());
        drop(records);
        self.persist_verdict(&verdict).await?;
        Ok(verdict)
    }

    pub async fn status(&self, plugin_id: &str) -> Result<Option<PluginRuntimeRecord>> {
        Ok(self.records.read().await.get(plugin_id).cloned())
    }

    pub async fn list(&self) -> Result<Vec<PluginRuntimeRecord>> {
        Ok(self.records.read().await.values().cloned().collect())
    }

    pub async fn discover_compatibility(&self, root: &str) -> Result<serde_json::Value> {
        let report = PluginCompatibilityLoader::discover(root)?;
        let mut admission_preview = Vec::new();
        for entry in &report.entries {
            let Some(manifest) = entry.manifest.as_ref() else {
                continue;
            };
            let tenant_id = manifest
                .metadata
                .get("tenant_id")
                .cloned()
                .unwrap_or_else(|| "tenant:default".to_string());
            let session_key = SessionId::from(format!("plugin-session:{tenant_id}:compat"));
            let candidate = CapabilityCandidate {
                capability_id: manifest.capability.capability_id.clone(),
                server: Some("plugin-runtime".into()),
                tool: manifest.id.clone(),
                score: risk_score(manifest.risk.clone()),
                active: true,
                verified: manifest.signature_ref.is_some(),
                trusted: manifest.signature_ref.is_some(),
                approval_required: matches!(manifest.risk, PluginRisk::High | PluginRisk::Critical),
            };
            let engine = CapabilityAdmissionEngine::new();
            let decision = engine.admit(&session_key, &[candidate]).await;
            let preview = match decision {
                Ok(value) => serde_json::json!({
                    "plugin_id": manifest.id,
                    "allowed": value.allowed,
                    "reason": value.reason,
                    "candidate": value.candidate,
                }),
                Err(error) => serde_json::json!({
                    "plugin_id": manifest.id,
                    "allowed": false,
                    "reason": error.to_string(),
                }),
            };
            admission_preview.push(preview);
        }

        let payload = serde_json::json!({
            "report": report,
            "admission_preview": admission_preview,
        });

        self.state_store
            .upsert_json_knowledge(
                format!("plugin:compat:discover:{}", current_time_ms()),
                &payload,
                "plugin-compat-loader",
            )
            .await?;
        Ok(payload)
    }
    pub async fn host_status(&self) -> Result<serde_json::Value> {
        let manifests = self.host.list_manifests().await;
        Ok(serde_json::json!({
            "host_api_version": self.host.resolver().host_api_version(),
            "loaded_runtime_count": manifests.len(),
            "loaded_plugins": manifests.into_iter().map(|manifest| manifest.id).collect::<Vec<_>>(),
        }))
    }

    pub async fn host_load_subprocess(
        &self,
        plugin_id: &str,
        entrypoint: &str,
        operator: &str,
    ) -> Result<PluginManifestContract> {
        let record = self
            .status(plugin_id)
            .await?
            .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
        enforce_manifest_contract(&record.current_manifest, false)?;
        if !matches!(record.state, PluginState::Enabled) {
            bail!("plugin '{}' must be enabled before host-load", plugin_id);
        }
        let verdict = self.verify(plugin_id).await?;
        let tenant_id = record.current_manifest.metadata.get("tenant_id").map(String::as_str);
        let _ = self
            .enforce_plugin_capability_admission(
                &record.current_manifest,
                &verdict,
                "host_load",
                operator,
                tenant_id,
            )
            .await?;

        let request = ExternalPluginLoadRequest {
            manifest: record.current_manifest.clone(),
            entrypoint: entrypoint.to_string(),
            metadata: BTreeMap::from([
                ("loaded_by".to_string(), operator.to_string()),
                ("loaded_at_ms".to_string(), current_time_ms().to_string()),
            ]),
        };
        let loader = SubprocessPluginLoader::default();
        self.host.load_external(request, &loader).await?;

        self.state_store
            .upsert_json_knowledge(
                format!("plugin:host:{}:loaded:{}", plugin_id, current_time_ms()),
                &serde_json::json!({
                    "plugin_id": plugin_id,
                    "entrypoint": entrypoint,
                    "operator": operator,
                    "isolation": "subprocess",
                }),
                "plugin-host",
            )
            .await?;
        Ok(record.current_manifest)
    }

    pub async fn host_load_default_bundle(
        &self,
        tenant_id: &str,
        operator: &str,
        entrypoint: &str,
    ) -> Result<serde_json::Value> {
        let defaults = default_host_bundle_specs();
        let mut loaded = Vec::with_capacity(defaults.len());

        for spec in defaults {
            let existing = self.status(spec.plugin_id).await?;
            if existing.is_none() {
                let signature = compute_plugin_signature(
                    spec.plugin_id,
                    spec.install_source,
                    tenant_id,
                    operator,
                );
                let request = PluginInstallRequest {
                    plugin_id: spec.plugin_id.to_string(),
                    source: format!("{}#sig={}", spec.install_source, signature),
                    requested_by: operator.to_string(),
                    tenant_id: tenant_id.to_string(),
                    verify_signature: true,
                };
                let _ = self.install(&request).await?;
            }

            let _ = self
                .enable(
                    spec.plugin_id,
                    operator,
                    "day9-10 default plugin bundle enable",
                )
                .await?;
            let loaded_manifest = self
                .host_load_subprocess(spec.plugin_id, entrypoint, operator)
                .await?;

            loaded.push(serde_json::json!({
                "plugin_id": spec.plugin_id,
                "kind": spec.kind,
                "install_source": spec.install_source,
                "version": loaded_manifest.version,
                "capability_id": loaded_manifest.capability.capability_id,
                "entrypoint": entrypoint,
            }));
        }

        Ok(serde_json::json!({
            "status": "ok",
            "bundle": "day9-10-default-subprocess",
            "count": loaded.len(),
            "loaded": loaded,
        }))
    }

    pub async fn host_invoke(
        &self,
        plugin_id: &str,
        session_id: &str,
        tenant_id: &str,
        principal_id: &str,
        capability_id: Option<&str>,
        payload: serde_json::Value,
    ) -> Result<crate::contracts::plugin::PluginInvocationOutput> {
        let record = self
            .status(plugin_id)
            .await?
            .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
        enforce_manifest_contract(&record.current_manifest, false)?;

        let input = crate::contracts::plugin::PluginInvocationInput {
            invocation_id: format!("plugin-invoke:{}:{}", plugin_id, current_time_ms()),
            plugin_id: plugin_id.to_string(),
            session_id: session_id.to_string(),
            tenant_id: tenant_id.to_string(),
            principal_id: principal_id.to_string(),
            capability_id: capability_id
                .map(str::to_string)
                .unwrap_or_else(|| record.current_manifest.capability.capability_id.clone()),
            payload,
            metadata: BTreeMap::from([
                ("facade_channel".to_string(), "plugin-facade-v2".to_string()),
                ("invoked_at_ms".to_string(), current_time_ms().to_string()),
            ]),
        };

        self.host.invoke_by_id(plugin_id, &input).await
    }
    async fn transition_state(
        &self,
        plugin_id: &str,
        target: PluginState,
        operator: &str,
        reason: &str,
    ) -> Result<PluginManifestContract> {
        if matches!(target, PluginState::Enabled) {
            let manifest = self
                .status(plugin_id)
                .await?
                .map(|record| record.current_manifest)
                .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
            enforce_manifest_contract(&manifest, false)?;
            let verdict = self.verify(plugin_id).await?;
            let tenant_id = manifest.metadata.get("tenant_id").map(String::as_str);
            self.enforce_plugin_capability_admission(
                &manifest, &verdict, "enable", operator, tenant_id,
            )
            .await?;
        }

        let mut records = self.records.write().await;
        let record = records
            .get_mut(plugin_id)
            .ok_or_else(|| anyhow!("plugin '{}' not found", plugin_id))?;
        let from_state = record.state.clone();
        record.state = target.clone();
        record.current_manifest.metadata.insert(
            "last_state_transition_at_ms".into(),
            current_time_ms().to_string(),
        );
        let event = PluginLifecycleEvent {
            plugin_id: plugin_id.to_string(),
            from_state: Some(from_state),
            to_state: target.clone(),
            reason: reason.to_string(),
            operator: operator.to_string(),
            at_ms: current_time_ms(),
        };
        record.last_event = Some(event.clone());
        let manifest = record.current_manifest.clone();
        drop(records);

        self.persist_manifest(&manifest, &target).await?;
        self.persist_event(&event).await?;
        self.persist_index().await?;
        let hook = match target {
            PluginState::Enabled => "post_enable",
            PluginState::Disabled => "post_disable",
            PluginState::Deprecated => "post_deprecate",
            PluginState::RolledBack => "post_rollback",
            PluginState::Installed => "post_install",
        };
        self.emit_hook(
            hook,
            plugin_id,
            serde_json::json!({"state": serde_json::to_value(&target).unwrap_or(serde_json::json!("unknown"))}),
        )
        .await?;
        Ok(manifest)
    }

    async fn enforce_plugin_capability_admission(
        &self,
        manifest: &PluginManifestContract,
        verdict: &PluginVerificationVerdict,
        action: &str,
        operator: &str,
        tenant_override: Option<&str>,
    ) -> Result<String> {
        let tenant_id = tenant_override
            .map(str::to_string)
            .or_else(|| manifest.metadata.get("tenant_id").cloned())
            .unwrap_or_else(|| "tenant:default".to_string());
        let policy_version = manifest
            .metadata
            .get("policy_version")
            .cloned()
            .unwrap_or_else(|| CONTRACT_VERSION.to_string());
        let session_id = format!("plugin-session:{}", tenant_id);
        let session_key = SessionId::from(session_id.as_str());

        let candidate = CapabilityCandidate {
            capability_id: manifest.capability.capability_id.clone(),
            server: Some("plugin-runtime".into()),
            tool: manifest.id.clone(),
            score: risk_score(manifest.risk.clone()),
            active: true,
            verified: verdict.verified,
            trusted: verdict.verified,
            approval_required: matches!(manifest.risk, PluginRisk::High | PluginRisk::Critical),
        };
        let engine = CapabilityAdmissionEngine::new();
        let decision = engine
            .admit(&session_key, &[candidate.clone()])
            .await
            .map_err(|error| anyhow!("plugin capability admission failed: {}", error))?;

        let signature_digest = digest_value(&serde_json::json!(manifest.signature_ref));
        let decision_payload = serde_json::json!({
            "plugin_id": manifest.id,
            "action": action,
            "tenant_id": tenant_id,
            "operator": operator,
            "policy_version": policy_version,
            "signature_digest": signature_digest,
            "decision": decision,
            "candidate": candidate,
            "verified": verdict.verified,
        });
        let decision_hash = digest_value(&decision_payload);

        let admission_ref_key = format!(
            "plugin-capability-admission:{}:{}:{}",
            manifest.id,
            action,
            current_time_ms()
        );
        self.state_store
            .upsert_json_knowledge(
                admission_ref_key.clone(),
                &serde_json::json!({
                    "plugin_id": manifest.id,
                    "action": action,
                    "tenant_id": tenant_id,
                    "operator": operator,
                    "policy_version": policy_version,
                    "signature_digest": signature_digest,
                    "decision_hash": decision_hash,
                    "decision": decision,
                    "candidate": candidate,
                }),
                "plugin-capability-admission",
            )
            .await?;

        let stage_ref = EvidenceLedgerWriter::append_stage(
            &self.state_store,
            &session_id,
            &format!("plugin-admission:{}:{}", manifest.id, action),
            EvidenceStage::Admission,
            serde_json::json!({
                "admission_record_ref": admission_ref_key,
                "plugin_id": manifest.id,
                "action": action,
                "tenant_id": tenant_id,
                "policy_version": policy_version,
                "signature_digest": signature_digest,
                "decision_hash": decision_hash,
                "decision": decision,
            }),
            None,
        )
        .await?;

        if !decision.allowed {
            self.state_store
                .upsert_json_knowledge(
                    format!(
                        "policy-reject:{}:{}:{}",
                        manifest.id,
                        action,
                        current_time_ms()
                    ),
                    &serde_json::json!({
                        "plugin_id": manifest.id,
                        "action": action,
                        "tenant_id": tenant_id,
                        "policy_version": policy_version,
                        "signature_digest": signature_digest,
                        "decision_hash": decision_hash,
                        "decision": decision,
                        "admission_evidence_ref": stage_ref,
                    }),
                    "plugin-capability-admission",
                )
                .await?;
            bail!("plugin capability admission rejected: {}", decision.reason);
        }

        Ok(stage_ref)
    }

    async fn emit_hook(
        &self,
        hook: &str,
        plugin_id: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!("plugin:hook:{plugin_id}:{hook}:{}", current_time_ms()),
                &payload,
                "plugin-lifecycle",
            )
            .await?;
        Ok(())
    }

    async fn persist_manifest(
        &self,
        manifest: &PluginManifestContract,
        state: &PluginState,
    ) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!("plugin:lifecycle:{}:latest", manifest.id),
                &serde_json::json!({
                    "plugin_id": manifest.id,
                    "state": state,
                    "manifest": manifest,
                }),
                "plugin-lifecycle",
            )
            .await?;
        Ok(())
    }

    async fn persist_rollout_event(
        &self,
        plugin_id: &str,
        mode: &PluginRolloutMode,
        traffic_percent: u8,
        operator: &str,
        reason: &str,
    ) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!("plugin:lifecycle:{plugin_id}:rollout:{}", current_time_ms()),
                &serde_json::json!({
                    "plugin_id": plugin_id,
                    "mode": mode,
                    "traffic_percent": traffic_percent,
                    "operator": operator,
                    "reason": reason,
                    "at_ms": current_time_ms(),
                }),
                "plugin-lifecycle",
            )
            .await?;
        Ok(())
    }
    async fn persist_event(&self, event: &PluginLifecycleEvent) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!(
                    "plugin:lifecycle:{}:events:{}",
                    event.plugin_id, event.at_ms
                ),
                event,
                "plugin-lifecycle",
            )
            .await?;
        Ok(())
    }

    async fn persist_verdict(&self, verdict: &PluginVerificationVerdict) -> Result<()> {
        self.state_store
            .upsert_json_knowledge(
                format!(
                    "plugin:lifecycle:{}:verdict:{}",
                    verdict.plugin_id, verdict.checked_at_ms
                ),
                verdict,
                "plugin-signature",
            )
            .await?;
        Ok(())
    }

    async fn persist_index(&self) -> Result<()> {
        let records = self.records.read().await;
        let index = records
            .iter()
            .map(|(plugin_id, item)| {
                serde_json::json!({
                    "plugin_id": plugin_id,
                    "state": item.state,
                    "version": item.current_manifest.version,
                    "verified": item.verified,
                    "rollout_mode": item.rollout_mode,
                    "rollout_traffic_percent": item.rollout_traffic_percent,
                })
            })
            .collect::<Vec<_>>();
        drop(records);
        self.state_store
            .upsert_json_knowledge(
                "plugin:lifecycle:index".to_string(),
                &index,
                "plugin-lifecycle",
            )
            .await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct DefaultHostBundleSpec {
    plugin_id: &'static str,
    kind: &'static str,
    install_source: &'static str,
}

fn default_host_bundle_specs() -> [DefaultHostBundleSpec; 5] {
    [
        DefaultHostBundleSpec {
            plugin_id: "plugin:graph-projection",
            kind: "graph_projection",
            install_source: "builtin://graph-projection",
        },
        DefaultHostBundleSpec {
            plugin_id: "plugin:vector-projection",
            kind: "vector_projection",
            install_source: "builtin://vector-projection",
        },
        DefaultHostBundleSpec {
            plugin_id: "plugin:search-projection",
            kind: "search_projection",
            install_source: "builtin://search-projection",
        },
        DefaultHostBundleSpec {
            plugin_id: "plugin:supermemory-federation",
            kind: "supermemory_federation",
            install_source: "builtin://supermemory-federation",
        },
        DefaultHostBundleSpec {
            plugin_id: "plugin:source-adapter",
            kind: "source_adapter",
            install_source: "builtin://source-adapter",
        },
    ]
}

#[async_trait]
impl PluginLifecyclePort for PluginLifecycleManager {
    async fn install_plugin(
        &self,
        request: &PluginInstallRequest,
    ) -> Result<PluginManifestContract, ContractError> {
        self.install(request).await.map_err(|error| {
            ContractError::Runtime(RuntimeError {
                code: "plugin_install_failed".into(),
                message: error.to_string(),
            })
        })
    }

    async fn apply_lifecycle_event(
        &self,
        event: &PluginLifecycleEvent,
    ) -> Result<(), ContractError> {
        let reason = if event.reason.trim().is_empty() {
            "lifecycle event".to_string()
        } else {
            event.reason.clone()
        };
        let result = match event.to_state {
            PluginState::Enabled => {
                self.enable(&event.plugin_id, &event.operator, &reason)
                    .await
            }
            PluginState::Disabled => {
                self.disable(&event.plugin_id, &event.operator, &reason)
                    .await
            }
            PluginState::RolledBack => {
                self.rollback(&event.plugin_id, &event.operator, &reason)
                    .await
            }
            PluginState::Installed => self.update(&event.plugin_id, None, &event.operator).await,
            PluginState::Deprecated => {
                self.disable(&event.plugin_id, &event.operator, &reason)
                    .await
            }
        };
        result.map(|_| ()).map_err(|error| {
            ContractError::Runtime(RuntimeError {
                code: "plugin_transition_failed".into(),
                message: error.to_string(),
            })
        })
    }

    async fn verify_plugin(
        &self,
        plugin_id: &str,
    ) -> Result<PluginVerificationVerdict, ContractError> {
        self.verify(plugin_id).await.map_err(|error| {
            ContractError::Runtime(RuntimeError {
                code: "plugin_verify_failed".into(),
                message: error.to_string(),
            })
        })
    }
}

fn plugin_kind_from_source(source: &str) -> PluginKind {
    let lowered = source.to_ascii_lowercase();
    if lowered.contains("supermemory") {
        return PluginKind::SupermemoryFederation;
    }
    if lowered.contains("graph") {
        return PluginKind::GraphProjection;
    }
    if lowered.contains("vector") {
        return PluginKind::VectorProjection;
    }
    if lowered.contains("search") {
        return PluginKind::SearchProjection;
    }
    PluginKind::SourceAdapter
}

fn bump_version(previous: &str) -> String {
    let trimmed = previous.trim();
    let digits = trimmed
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .trim();
    let parsed = digits.parse::<u32>().ok().unwrap_or(0);
    format!("v{}", parsed.saturating_add(1))
}

fn rollout_mode_label(mode: &PluginRolloutMode) -> &'static str {
    match mode {
        PluginRolloutMode::Shadow => "shadow",
        PluginRolloutMode::Canary => "canary",
        PluginRolloutMode::Full => "full",
        PluginRolloutMode::Rollback => "rollback",
    }
}
fn manifest_state_for_mode(mode: &PluginRolloutMode) -> PluginState {
    match mode {
        PluginRolloutMode::Shadow => PluginState::Installed,
        PluginRolloutMode::Canary => PluginState::Enabled,
        PluginRolloutMode::Full => PluginState::Enabled,
        PluginRolloutMode::Rollback => PluginState::RolledBack,
    }
}
fn risk_score(risk: PluginRisk) -> f32 {
    match risk {
        PluginRisk::Low => 0.98,
        PluginRisk::Medium => 0.82,
        PluginRisk::High => 0.65,
        PluginRisk::Critical => 0.55,
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    use super::{PluginLifecycleManager, compute_plugin_signature};

    #[tokio::test]
    async fn compatibility_discovery_compiles_layers_and_returns_admission_preview() {
        let mut root = std::env::temp_dir();
        root.push(format!("autoloop-plugin-compat-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("skills")).expect("mkdir skills");
        fs::write(root.join("skills").join("analysis.md"), "# analysis").expect("skill file");
        fs::write(
            root.join("plugin.json"),
            r#"{
                "id":"plugin:compat-preview",
                "kind":"tool",
                "signature_ref":"sig-compat",
                "hooks":["pre_tool_use"],
                "mcpServers":{"local-mcp":{"url":"http://localhost"}},
                "skills":["planner"]
            }"#,
        )
        .expect("plugin json");

        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let manager = PluginLifecycleManager::new(db);
        let payload = manager
            .discover_compatibility(root.to_str().expect("root str"))
            .await
            .expect("discover");

        let entries = payload
            .get("report")
            .and_then(|value| value.get("entries"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let target = entries
            .iter()
            .find(|entry| {
                entry
                    .get("plugin_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("plugin:compat-preview")
            })
            .expect("compat entry");
        let commands = target
            .get("manifest")
            .and_then(|value| value.get("commands"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            commands.iter().any(|item| item.as_str() == Some("mcp::local-mcp::invoke")),
            "expected compiled mcp command"
        );

        let admission_preview = payload
            .get("admission_preview")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            admission_preview.iter().any(|item| {
                item.get("plugin_id").and_then(serde_json::Value::as_str)
                    == Some("plugin:compat-preview")
            }),
            "expected admission preview record for compiled compatibility plugin"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn day9_10_default_bundle_loads_five_subprocess_plugin_kinds() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let manager = PluginLifecycleManager::new(db.clone());

        let report = manager
            .host_load_default_bundle(
                "tenant:day9-10",
                "principal:day9-10",
                "proc://cmd /c exit 0",
            )
            .await
            .expect("load default bundle");
        assert_eq!(
            report
                .get("count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            5
        );

        let status = manager.host_status().await.expect("host status");
        let loaded = status
            .get("loaded_plugins")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            loaded
                .iter()
                .any(|value| value.as_str() == Some("plugin:graph-projection"))
        );
        assert!(
            loaded
                .iter()
                .any(|value| value.as_str() == Some("plugin:vector-projection"))
        );
        assert!(
            loaded
                .iter()
                .any(|value| value.as_str() == Some("plugin:search-projection"))
        );
        assert!(
            loaded
                .iter()
                .any(|value| value.as_str() == Some("plugin:supermemory-federation"))
        );
        assert!(
            loaded
                .iter()
                .any(|value| value.as_str() == Some("plugin:source-adapter"))
        );

        let host_records = db
            .list_knowledge_by_prefix("plugin:host:plugin:")
            .await
            .expect("host records");
        assert!(
            host_records.len() >= 5,
            "expected host load records for five default plugins, got {}",
            host_records.len()
        );
    }

    #[tokio::test]
    async fn host_load_path_records_host_load_admission_evidence() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let manager = PluginLifecycleManager::new(db.clone());

        let plugin_id = "plugin:host-load-admission";
        let tenant = "tenant:host-load";
        let operator = "principal:host-load";
        let source = "builtin://host-load-admission";
        let signature = compute_plugin_signature(plugin_id, source, tenant, operator);

        manager
            .install(&crate::contracts::plugin::PluginInstallRequest {
                plugin_id: plugin_id.to_string(),
                source: format!("{source}#sig={signature}"),
                requested_by: operator.to_string(),
                tenant_id: tenant.to_string(),
                verify_signature: true,
            })
            .await
            .expect("install");
        manager
            .enable(plugin_id, operator, "enable for host load")
            .await
            .expect("enable");
        manager
            .host_load_subprocess(plugin_id, "proc://cmd /c exit 0", operator)
            .await
            .expect("host load");

        let admission_records = db
            .list_knowledge_by_prefix("plugin-capability-admission:plugin:host-load-admission:host_load:")
            .await
            .expect("admission records");
        assert!(
            !admission_records.is_empty(),
            "expected host_load admission evidence record"
        );
    }
    #[tokio::test]
    async fn unsigned_install_is_rejected_even_if_verify_flag_false() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let manager = PluginLifecycleManager::new(db);
        let result = manager
            .install(&crate::contracts::plugin::PluginInstallRequest {
                plugin_id: "plugin:unsigned".to_string(),
                source: "builtin://unsigned-plugin".to_string(),
                requested_by: "principal:test".to_string(),
                tenant_id: "tenant:test".to_string(),
                verify_signature: false,
            })
            .await;
        assert!(result.is_err());
        let message = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(message.contains("unsigned source") || message.contains("verify_signature=false"));
    }

    #[tokio::test]
    async fn day14_gray_release_rollout_shadow_canary_full() {
        let db = autoloop_state_adapter::StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let manager = PluginLifecycleManager::new(db);
        let plugin_id = "plugin:day14-rollout";
        let tenant_id = "tenant:day14";
        let operator = "principal:day14";

        let signature = compute_plugin_signature(
            plugin_id,
            "builtin://day14-rollout",
            tenant_id,
            operator,
        );
        let installed = manager
            .install(&crate::contracts::plugin::PluginInstallRequest {
                plugin_id: plugin_id.to_string(),
                source: format!("builtin://day14-rollout#sig={}", signature),
                requested_by: operator.to_string(),
                tenant_id: tenant_id.to_string(),
                verify_signature: true,
            })
            .await
            .expect("install");
        assert_eq!(installed.plugin_id, plugin_id);

        manager
            .rollout(
                plugin_id,
                crate::contracts::plugin::PluginRolloutMode::Shadow,
                Some(0),
                operator,
                "shadow stage",
            )
            .await
            .expect("shadow rollout");
        let status_shadow = manager
            .status(plugin_id)
            .await
            .expect("status")
            .expect("present");
        assert_eq!(
            status_shadow.rollout_mode,
            Some(crate::contracts::plugin::PluginRolloutMode::Shadow)
        );
        assert_eq!(status_shadow.rollout_traffic_percent, Some(0));

        manager
            .rollout(
                plugin_id,
                crate::contracts::plugin::PluginRolloutMode::Canary,
                Some(30),
                operator,
                "canary stage",
            )
            .await
            .expect("canary rollout");
        let status_canary = manager
            .status(plugin_id)
            .await
            .expect("status")
            .expect("present");
        assert_eq!(
            status_canary.rollout_mode,
            Some(crate::contracts::plugin::PluginRolloutMode::Canary)
        );
        assert_eq!(status_canary.rollout_traffic_percent, Some(30));

        manager
            .rollout(
                plugin_id,
                crate::contracts::plugin::PluginRolloutMode::Full,
                Some(100),
                operator,
                "full stage",
            )
            .await
            .expect("full rollout");
        let status_full = manager
            .status(plugin_id)
            .await
            .expect("status")
            .expect("present");
        assert_eq!(
            status_full.rollout_mode,
            Some(crate::contracts::plugin::PluginRolloutMode::Full)
        );
        assert_eq!(status_full.rollout_traffic_percent, Some(100));
    }

    #[tokio::test]
    async fn day14_one_click_acceptance_gray_release_plus_regression() {
        let app = crate::AutoLoopApp::new(crate::config::AppConfig::default());
        let session_id = "day14-one-click";
        let tenant_id = "tenant:day14";
        let operator = "principal:day14";
        let plugin_id = "plugin:day14-one-click";
        let signature =
            compute_plugin_signature(plugin_id, "builtin://day14-one-click", tenant_id, operator);

        app.ensure_session_identity(session_id, tenant_id, operator, "policy:day14", 3_600_000)
            .await
            .expect("identity");
        app.plugin_install(
            plugin_id,
            &format!("builtin://day14-one-click#sig={signature}"),
            operator,
            tenant_id,
            true,
        )
        .await
        .expect("install");
        app.plugin_enable(plugin_id, operator, "enable")
            .await
            .expect("enable");
        app.plugin_rollout(plugin_id, "shadow", Some(0), operator, Some("shadow"))
            .await
            .expect("shadow");
        app.plugin_rollout(plugin_id, "canary", Some(30), operator, Some("canary"))
            .await
            .expect("canary");
        app.plugin_rollout(plugin_id, "full", Some(100), operator, Some("full"))
            .await
            .expect("full");

        let status = app.plugin_status(plugin_id).await.expect("status");
        let status_json: serde_json::Value = serde_json::from_str(&status).expect("status json");
        assert_eq!(
            status_json
                .get("rollout_mode")
                .and_then(serde_json::Value::as_str),
            Some("full")
        );
        assert_eq!(
            status_json
                .get("rollout_traffic_percent")
                .and_then(serde_json::Value::as_u64),
            Some(100)
        );

        let response = app
            .process_requirement_swarm(session_id, "day14 regression acceptance")
            .await
            .expect("swarm");
        assert!(!response.trim().is_empty());
        let trace_id = format!("trace:{session_id}:day14");
        let query = crate::observability::query_plane::persist_unified_query_view(
            &app.state_store(),
            session_id,
            Some(&trace_id),
        )
        .await
        .expect("query");
        assert!(query.metrics.is_object());
        assert!(query.ledger.is_object());
        assert!(query.replay.is_object());

        app.plugin_update(plugin_id, Some("builtin://day14-one-click-v2"), operator)
            .await
            .expect("update");
        app.plugin_quick_rollback(plugin_id, operator)
            .await
            .expect("rollback");
        let rolled = app.plugin_status(plugin_id).await.expect("rollback status");
        let rolled_json: serde_json::Value = serde_json::from_str(&rolled).expect("rolled json");
        assert_eq!(
            rolled_json.get("state").and_then(serde_json::Value::as_str),
            Some("rolled_back")
        );
    }
}



