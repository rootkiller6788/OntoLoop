use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::{
    config::PolicyMode as RuntimePolicyMode,
    contracts::capability::{CapabilityAdmissionDecision, CapabilityCandidate, CapabilityIntent},
    contracts::policy_pdp::{
        DecisionReason, PolicyDecision as PdpPolicyDecision, PolicyInput,
        PolicyMode as PdpPolicyMode, PolicyVersion,
    },
    contracts::types::ExecutionIdentity,
    providers::FactoryArtifact,
    runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
    tools::ForgedMcpToolManifest,
    tools::{ApprovalStatus, CapabilityRisk, CapabilityStatus, ToolRegistry, TrustStatus},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct AdmissionFeedback {
    trust_decay_delta: f32,
    approval_threshold: String,
    source: String,
}
#[derive(Clone)]
pub struct CapabilityAdmissionEngine {
    policy_mode: RuntimePolicyMode,
}

impl Default for CapabilityAdmissionEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl CapabilityAdmissionEngine {
    pub fn new() -> Self {
        Self {
            policy_mode: RuntimePolicyMode::Shadow,
        }
    }

    pub fn with_policy_mode(policy_mode: RuntimePolicyMode) -> Self {
        Self { policy_mode }
    }

    pub async fn admit_selected(
        &self,
        db: &StateStore,
        tools: &ToolRegistry,
        factory_artifacts: &[FactoryArtifact],
        session_id: &str,
        task_id: &str,
        identity: &ExecutionIdentity,
        intent: &CapabilityIntent,
        tool_name: &str,
        server: Option<&str>,
    ) -> Result<CapabilityAdmissionDecision> {
        let Some(manifest) = tools
            .manifests()
            .into_iter()
            .find(|manifest| manifest.registered_tool_name == tool_name)
        else {
            let mut decision = CapabilityAdmissionDecision {
                allowed: true,
                reason: "non-forged capability path".to_string(),
                candidate: None,
                quota_remaining_micros: None,
                evidence_ref: None,
            };
            let evidence_ref = self
                .persist_admission_record(
                    db,
                    session_id,
                    task_id,
                    identity,
                    intent,
                    &decision,
                    None,
                    factory_artifacts,
                )
                .await?;
            decision.evidence_ref = Some(evidence_ref);
            return Ok(decision);
        };

        let matched_factory_artifacts =
            select_factory_artifacts(factory_artifacts, server.or(Some(manifest.server.as_str())));
        let feedback = db
            .get_knowledge(&format!("capability-admission:{session_id}:feedback"))
            .await?
            .and_then(|record| serde_json::from_str::<AdmissionFeedback>(&record.value).ok());

        let candidate = CapabilityCandidate {
            capability_id: manifest.capability_id.clone(),
            server: server.map(str::to_string),
            tool: tool_name.to_string(),
            score: manifest.health_score,
            active: manifest.status == CapabilityStatus::Active,
            verified: manifest.approval_status == ApprovalStatus::Verified,
            trusted: manifest.trust_status == TrustStatus::Trusted,
            approval_required: manifest.requires_gate(),
        };

        if !candidate.active {
            return self
                .reject(
                    db,
                    session_id,
                    task_id,
                    identity,
                    intent,
                    candidate,
                    "capability is not active",
                    None,
                    Some(&manifest),
                    &matched_factory_artifacts,
                )
                .await;
        }
        if !candidate.verified {
            return self
                .reject(
                    db,
                    session_id,
                    task_id,
                    identity,
                    intent,
                    candidate,
                    "capability is not verified",
                    None,
                    Some(&manifest),
                    &matched_factory_artifacts,
                )
                .await;
        }
        if !candidate.trusted {
            return self
                .reject(
                    db,
                    session_id,
                    task_id,
                    identity,
                    intent,
                    candidate,
                    "capability is not trusted",
                    None,
                    Some(&manifest),
                    &matched_factory_artifacts,
                )
                .await;
        }

        let quota_remaining = if let Some(account) = db
            .get_budget_account(&identity.tenant_id, &identity.principal_id)
            .await?
        {
            Some(
                account
                    .total_budget_micros
                    .saturating_sub(account.reserved_micros.saturating_add(account.spent_micros)),
            )
        } else {
            None
        };

        if let Some(remaining) = quota_remaining {
            if remaining == 0 {
                return self
                    .reject(
                        db,
                        session_id,
                        task_id,
                        identity,
                        intent,
                        candidate,
                        "quota exhausted",
                        Some(remaining),
                        Some(&manifest),
                        &matched_factory_artifacts,
                    )
                    .await;
            }
        }

        if let Some(feedback) = feedback.as_ref() {
            let effective_score = candidate.score - feedback.trust_decay_delta.max(0.0);
            let threshold = feedback.approval_threshold.to_ascii_lowercase();
            if threshold == "strict" && effective_score < 0.90 {
                return self
                    .reject(
                        db,
                        session_id,
                        task_id,
                        identity,
                        intent,
                        candidate,
                        "policy feedback strict threshold rejected capability",
                        quota_remaining,
                        Some(&manifest),
                        &matched_factory_artifacts,
                    )
                    .await;
            }
            if threshold == "moderate" && effective_score < 0.70 {
                return self
                    .reject(
                        db,
                        session_id,
                        task_id,
                        identity,
                        intent,
                        candidate,
                        "policy feedback moderate threshold rejected capability",
                        quota_remaining,
                        Some(&manifest),
                        &matched_factory_artifacts,
                    )
                    .await;
            }
        }

        if candidate.approval_required {
            let approval_key = format!(
                "approval:capability:{session_id}:{task_id}:{}",
                candidate.tool
            );
            let approved = db
                .get_knowledge(&approval_key)
                .await?
                .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
                .and_then(|value| value.get("approved").and_then(|v| v.as_bool()))
                .unwrap_or(false);
            if !approved {
                return self
                    .reject(
                        db,
                        session_id,
                        task_id,
                        identity,
                        intent,
                        candidate,
                        "approval required for medium/high risk capability",
                        quota_remaining,
                        Some(&manifest),
                        &matched_factory_artifacts,
                    )
                    .await;
            }
        }

        let mut decision = CapabilityAdmissionDecision {
            allowed: true,
            reason: "admitted".to_string(),
            candidate: Some(candidate),
            quota_remaining_micros: quota_remaining,
            evidence_ref: None,
        };

        let pdp_decision = self
            .evaluate_pdp_decision(
                db,
                session_id,
                task_id,
                identity,
                intent,
                tool_name,
                manifest.risk.clone(),
            )
            .await?;
        let old_allowed = decision.allowed;
        let old_reason = decision.reason.clone();

        if matches!(self.policy_mode, RuntimePolicyMode::Enforced)
            && manifest.risk == CapabilityRisk::High
            && !pdp_decision.allowed
        {
            return self
                .reject(
                    db,
                    session_id,
                    task_id,
                    identity,
                    intent,
                    decision.candidate.clone().expect("candidate exists"),
                    &format!("pdp enforced deny: {}", first_reason(&pdp_decision)),
                    quota_remaining,
                    Some(&manifest),
                    &matched_factory_artifacts,
                )
                .await;
        }

        if matches!(self.policy_mode, RuntimePolicyMode::Shadow) && old_allowed != pdp_decision.allowed {
            self.record_shadow_diff(
                db,
                session_id,
                task_id,
                identity,
                tool_name,
                old_allowed,
                old_reason,
                &pdp_decision,
            )
            .await?;
        }

        let evidence_ref = self
            .persist_admission_record(
                db,
                session_id,
                task_id,
                identity,
                intent,
                &decision,
                Some(&manifest),
                &matched_factory_artifacts,
            )
            .await?;
        decision.evidence_ref = Some(evidence_ref);
        Ok(decision)
    }

    async fn evaluate_pdp_decision(
        &self,
        db: &StateStore,
        session_id: &str,
        task_id: &str,
        identity: &ExecutionIdentity,
        intent: &CapabilityIntent,
        tool_name: &str,
        risk: CapabilityRisk,
    ) -> Result<PdpPolicyDecision> {
        let input = PolicyInput {
            tenant_id: identity.tenant_id.clone(),
            subject: identity.principal_id.clone(),
            action: "capability_execute".into(),
            resource: Some(tool_name.to_string()),
            labels: vec![
                format!("policy:{}", identity.policy_id),
                format!("risk:{risk:?}").to_ascii_lowercase(),
                format!("objective:{}", intent.objective),
            ],
            context_hash: Some(format!("{session_id}:{task_id}:{}", identity.lease_token)),
        };

        let strategy_allowed = if risk == CapabilityRisk::High {
            self.strategy_allows_high_risk(db, session_id, task_id, tool_name)
                .await?
        } else {
            true
        };

        let reason = if strategy_allowed {
            DecisionReason {
                code: "policy.allow".into(),
                message: "pdp strategy allows execution".into(),
                rule_id: Some("pdp-high-risk-strategy".into()),
            }
        } else {
            DecisionReason {
                code: "policy.deny".into(),
                message: "high risk capability requires explicit policy strategy allow".into(),
                rule_id: Some("pdp-high-risk-strategy".into()),
            }
        };

        let _ = db
            .upsert_json_knowledge(
                format!(
                    "policy-pdp:evaluate:{session_id}:{task_id}:{}",
                    crate::orchestration::current_time_ms()
                ),
                &serde_json::json!({
                    "input": input,
                    "risk": format!("{risk:?}"),
                    "strategy_allowed": strategy_allowed,
                    "mode": format!("{:?}", self.policy_mode).to_ascii_lowercase(),
                }),
                "policy-pdp",
            )
            .await;

        Ok(PdpPolicyDecision {
            allowed: strategy_allowed,
            mode: to_pdp_mode(&self.policy_mode),
            version: PolicyVersion {
                id: identity.policy_id.clone(),
                revision: 1,
            },
            reasons: vec![reason],
            mask_rules: Vec::new(),
            drop_rules: Vec::new(),
        })
    }

    async fn strategy_allows_high_risk(
        &self,
        db: &StateStore,
        session_id: &str,
        task_id: &str,
        tool_name: &str,
    ) -> Result<bool> {
        let explicit_deny_key = format!(
            "policy-pdp:strategy-deny:{session_id}:{task_id}:{tool_name}"
        );
        if json_bool_key(db, &explicit_deny_key).await? {
            return Ok(false);
        }
        let explicit_key = format!(
            "policy-pdp:strategy-allow:{session_id}:{task_id}:{tool_name}"
        );
        if json_bool_key(db, &explicit_key).await? {
            return Ok(true);
        }
        let approval_key = format!("approval:capability:{session_id}:{task_id}:{tool_name}");
        if json_bool_key(db, &approval_key).await? {
            return Ok(true);
        }
        Ok(false)
    }

    async fn record_shadow_diff(
        &self,
        db: &StateStore,
        session_id: &str,
        task_id: &str,
        identity: &ExecutionIdentity,
        tool_name: &str,
        old_allowed: bool,
        old_reason: String,
        pdp_decision: &PdpPolicyDecision,
    ) -> Result<()> {
        let evidence_ref = EvidenceLedgerWriter::append_stage(
            db,
            session_id,
            task_id,
            EvidenceStage::Admission,
            serde_json::json!({
                "kind": "policy_pdp_shadow_diff",
                "session_id": session_id,
                "task_id": task_id,
                "tenant_id": identity.tenant_id,
                "principal_id": identity.principal_id,
                "policy_id": identity.policy_id,
                "tool_name": tool_name,
                "mode": "shadow",
                "old_decision": {
                    "allowed": old_allowed,
                    "reason": old_reason,
                },
                "new_decision": pdp_decision,
            }),
            None,
        )
        .await?;
        db.upsert_json_knowledge(
            format!(
                "policy-pdp:shadow-diff:{session_id}:{task_id}:{}",
                crate::orchestration::current_time_ms()
            ),
            &serde_json::json!({
                "session_id": session_id,
                "task_id": task_id,
                "tenant_id": identity.tenant_id,
                "principal_id": identity.principal_id,
                "policy_id": identity.policy_id,
                "tool_name": tool_name,
                "mode": "shadow",
                "old_decision": {
                    "allowed": old_allowed,
                    "reason": old_reason,
                },
                "new_decision": pdp_decision,
                "evidence_ref": evidence_ref,
            }),
            "policy-pdp",
        )
        .await?;
        Ok(())
    }    async fn reject(
        &self,
        db: &StateStore,
        session_id: &str,
        task_id: &str,
        identity: &ExecutionIdentity,
        intent: &CapabilityIntent,
        candidate: CapabilityCandidate,
        reason: &str,
        quota_remaining_micros: Option<u64>,
        manifest: Option<&ForgedMcpToolManifest>,
        factory_artifacts: &[FactoryArtifact],
    ) -> Result<CapabilityAdmissionDecision> {
        let mut decision = CapabilityAdmissionDecision {
            allowed: false,
            reason: reason.to_string(),
            candidate: Some(candidate),
            quota_remaining_micros,
            evidence_ref: None,
        };

        let evidence_ref = self
            .persist_admission_record(
                db,
                session_id,
                task_id,
                identity,
                intent,
                &decision,
                manifest,
                factory_artifacts,
            )
            .await?;
        decision.evidence_ref = Some(evidence_ref.clone());

        db.upsert_json_knowledge(
            format!(
                "policy-reject:{session_id}:{task_id}:{}",
                crate::orchestration::current_time_ms()
            ),
            &serde_json::json!({
                "session_id": session_id,
                "task_id": task_id,
                "tenant_id": identity.tenant_id,
                "principal_id": identity.principal_id,
                "policy_id": identity.policy_id,
                "intent": intent,
                "decision": decision,
                "admission_evidence_ref": evidence_ref,
            }),
            "capability-admission",
        )
        .await?;

        Ok(decision)
    }
    async fn persist_admission_record(
        &self,
        db: &StateStore,
        session_id: &str,
        task_id: &str,
        identity: &ExecutionIdentity,
        intent: &CapabilityIntent,
        decision: &CapabilityAdmissionDecision,
        manifest: Option<&ForgedMcpToolManifest>,
        factory_artifacts: &[FactoryArtifact],
    ) -> Result<String> {
        let key = format!(
            "capability-admission:{session_id}:{task_id}:{}",
            crate::orchestration::current_time_ms()
        );
        let factory_refs = factory_artifacts
            .iter()
            .map(|artifact| {
                serde_json::json!({
                    "artifact_id": artifact.artifact_id,
                    "kind": artifact.kind,
                    "provider": artifact.provider,
                    "version": artifact.version,
                    "source": artifact.source,
                    "active": artifact.active,
                    "verified": artifact.verified,
                    "trusted": artifact.trusted,
                    "metadata": artifact.metadata,
                })
            })
            .collect::<Vec<_>>();
        let manifest_ref = manifest.map(|item| {
            serde_json::json!({
                "registered_tool_name": item.registered_tool_name,
                "capability_id": item.capability_id,
                "server": item.server,
                "artifact": item.artifact,
                "signature": item.signature,
                "provenance": item.provenance,
            })
        });

        db.upsert_json_knowledge(
            key.clone(),
            &serde_json::json!({
                "session_id": session_id,
                "task_id": task_id,
                "tenant_id": identity.tenant_id,
                "principal_id": identity.principal_id,
                "policy_id": identity.policy_id,
                "intent": intent,
                "decision": decision,
                "factory_artifacts": factory_refs,
                "factory_manifest": manifest_ref,
            }),
            "capability-admission",
        )
        .await?;
        let stage_ref = EvidenceLedgerWriter::append_stage(
            db,
            session_id,
            task_id,
            EvidenceStage::Admission,
            serde_json::json!({
                "admission_record_ref": key,
                "session_id": session_id,
                "task_id": task_id,
                "tenant_id": identity.tenant_id,
                "principal_id": identity.principal_id,
                "policy_id": identity.policy_id,
                "decision": decision,
                "factory_artifacts": factory_refs,
                "factory_manifest": manifest_ref,
            }),
            None,
        )
        .await?;
        Ok(stage_ref)
    }
}


fn to_pdp_mode(mode: &RuntimePolicyMode) -> PdpPolicyMode {
    match mode {
        RuntimePolicyMode::Off => PdpPolicyMode::Off,
        RuntimePolicyMode::Shadow => PdpPolicyMode::Shadow,
        RuntimePolicyMode::Enforced => PdpPolicyMode::Enforced,
    }
}

fn first_reason(decision: &PdpPolicyDecision) -> String {
    decision
        .reasons
        .first()
        .map(|item| item.message.clone())
        .unwrap_or_else(|| "policy denied".to_string())
}

async fn json_bool_key(db: &StateStore, key: &str) -> Result<bool> {
    let value = db.get_knowledge(key).await?;
    let Some(record) = value else {
        return Ok(false);
    };
    if let Ok(raw) = serde_json::from_str::<serde_json::Value>(&record.value) {
        return Ok(
            raw.get("approved")
                .and_then(|v| v.as_bool())
                .or_else(|| raw.get("allow").and_then(|v| v.as_bool()))
                .or_else(|| raw.get("allowed").and_then(|v| v.as_bool()))
                .unwrap_or(false),
        );
    }
    Ok(false)
}
fn select_factory_artifacts(
    artifacts: &[FactoryArtifact],
    server: Option<&str>,
) -> Vec<FactoryArtifact> {
    let Some(server) = server else {
        return Vec::new();
    };
    let provider_alias = format!("mcp:{server}");
    artifacts
        .iter()
        .filter(|artifact| {
            artifact.provider == provider_alias
                || artifact.provider == server
                || artifact
                    .metadata
                    .get("server")
                    .is_some_and(|value| value == server)
        })
        .cloned()
        .collect()
}
#[async_trait::async_trait]
impl crate::contracts::ports::CapabilityAdmissionPort for CapabilityAdmissionEngine {
    async fn admit(
        &self,
        _session_id: &crate::contracts::ids::SessionId,
        candidates: &[crate::contracts::capability::CapabilityCandidate],
    ) -> Result<
        crate::contracts::capability::CapabilityAdmissionDecision,
        crate::contracts::errors::ContractError,
    > {
        let selected = candidates
            .iter()
            .find(|item| item.active && item.verified && item.trusted)
            .cloned()
            .or_else(|| candidates.first().cloned());

        match selected {
            Some(candidate) if candidate.active && candidate.verified && candidate.trusted => {
                Ok(crate::contracts::capability::CapabilityAdmissionDecision {
                    allowed: true,
                    reason: "admitted-by-contract-port".into(),
                    candidate: Some(candidate),
                    quota_remaining_micros: None,
                    evidence_ref: None,
                })
            }
            Some(candidate) => Ok(crate::contracts::capability::CapabilityAdmissionDecision {
                allowed: false,
                reason: "rejected-by-contract-port".into(),
                candidate: Some(candidate),
                quota_remaining_micros: None,
                evidence_ref: None,
            }),
            None => Err(crate::contracts::errors::ContractError::Policy(
                crate::contracts::errors::PolicyError {
                    code: "no_candidate".into(),
                    message: "no capability candidate available".into(),
                },
            )),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolRegistry;
    use autoloop_state_adapter::{
        BudgetAccount, PermissionAction, PolicyBinding, Principal, RoleBinding, SessionLease,
        StateStoreBackend, StateStoreConfig, Tenant,
    };

    fn base_identity() -> ExecutionIdentity {
        ExecutionIdentity {
            tenant_id: "tenant-a".to_string(),
            principal_id: "principal-a".to_string(),
            policy_id: "policy-a".to_string(),
            lease_token: "lease-a".to_string(),
        }
    }

    async fn seed_identity(db: &StateStore, session_id: &str, identity: &ExecutionIdentity) {
        let now = crate::orchestration::current_time_ms();
        db.upsert_tenant(Tenant {
            tenant_id: identity.tenant_id.clone(),
            name: identity.tenant_id.clone(),
            status: "active".into(),
            created_at_ms: now,
        })
        .await
        .expect("tenant");
        db.upsert_principal(Principal {
            principal_id: identity.principal_id.clone(),
            tenant_id: identity.tenant_id.clone(),
            principal_type: "user".into(),
            status: "active".into(),
            created_at_ms: now,
        })
        .await
        .expect("principal");
        db.upsert_role_binding(RoleBinding {
            tenant_id: identity.tenant_id.clone(),
            principal_id: identity.principal_id.clone(),
            role: "operator".into(),
            updated_at_ms: now,
        })
        .await
        .expect("role");
        db.upsert_policy_binding(PolicyBinding {
            policy_id: identity.policy_id.clone(),
            tenant_id: identity.tenant_id.clone(),
            role: "operator".into(),
            allowed_actions: vec![
                PermissionAction::Read,
                PermissionAction::Write,
                PermissionAction::Dispatch,
            ],
            capability_prefixes: vec!["mcp::".into(), "provider:".into(), "cli::".into()],
            max_memory_mb: 2048,
            max_tokens: 32000,
            updated_at_ms: now,
        })
        .await
        .expect("policy");
        db.upsert_session_lease(SessionLease {
            lease_token: identity.lease_token.clone(),
            session_id: session_id.to_string(),
            tenant_id: identity.tenant_id.clone(),
            principal_id: identity.principal_id.clone(),
            policy_id: identity.policy_id.clone(),
            expires_at_ms: now + 60_000,
            issued_at_ms: now,
        })
        .await
        .expect("lease");
    }

    #[tokio::test]
    async fn rejects_high_risk_without_approval() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let app = crate::config::AppConfig::default();
        let tools = ToolRegistry::from_config(&app.tools);
        tools.attach_state_store(db.clone());
        let manifest: crate::tools::ForgedMcpToolManifest =
            serde_json::from_value(serde_json::json!({
                "capability_id": "cap-high-risk:v1",
                "registered_tool_name": "mcp::high-risk::invoke",
                "delegate_tool_name": "mcp::local-mcp::invoke",
                "server": "local-mcp",
                "capability_name": "high-risk",
                "purpose": "test high risk",
                "executable": "echo",
                "command_template": "echo {{payload}}",
                "payload_template": {"payload": "{{payload}}"},
                "output_mode": "json",
                "help_text": "help",
                "skill_markdown": "skill",
                "examples": ["ex"],
                "version": 1,
                "lineage_key": "cap-high-risk",
                "status": "active",
                "approval_status": "verified",
                "health_score": 0.9,
                "scope": "session",
                "tags": ["test"],
                "risk": "high",
                "requested_by": "tester",
                "created_at_ms": 1,
                "updated_at_ms": 1,
                "approved_at_ms": 1,
                "rollback_to_version": null,
                "trust_status": "trusted",
                "trust_findings": [],
                "artifact": {"artifact_id":"a","digest_sha256":"d","source_uri":"u","build_epoch":1},
                "signature": {"signer":"autoloop","algorithm":"deterministic_v1","signed_payload_hash":"x","signature":"y","signed_at_ms":1},
                "provenance": {"source_repo":"r","source_ref":"ref","builder":"b","generated_by":"g"},
                "sbom": {"components": []},
                "trust_policy": {"required_signers":["autoloop"],"blocked_dependencies":[],"min_provenance_ref_len":1}
            }))
            .expect("manifest");
        tools.hydrate_manifest(manifest);

        let identity = base_identity();
        seed_identity(&db, "session-a", &identity).await;
        db.upsert_budget_account(BudgetAccount {
            account_id: "acct-a".into(),
            tenant_id: identity.tenant_id.clone(),
            principal_id: identity.principal_id.clone(),
            policy_id: identity.policy_id.clone(),
            total_budget_micros: 100,
            reserved_micros: 0,
            spent_micros: 0,
            blocked_count: 0,
            updated_at_ms: crate::orchestration::current_time_ms(),
        })
        .await
        .expect("budget");

        let engine = CapabilityAdmissionEngine::new();
        let decision = engine
            .admit_selected(
                &db,
                &tools,
                &[],
                "session-a",
                "task-a",
                &identity,
                &CapabilityIntent {
                    session_id: "session-a".into(),
                    objective: "test".into(),
                    required_tags: vec![],
                    preferred_servers: vec![],
                },
                "mcp::high-risk::invoke",
                Some("local-mcp"),
            )
            .await
            .expect("decision");
        assert!(!decision.allowed);
        assert!(decision.evidence_ref.is_some());
    }
}

#[derive(Clone)]
pub struct CapabilityIntentSelector {
    tools: ToolRegistry,
}

impl CapabilityIntentSelector {
    pub fn new(tools: ToolRegistry) -> Self {
        Self { tools }
    }
}

fn selector_parse_mcp_server(tool_name: &str) -> Option<String> {
    let mut parts = tool_name.split("::");
    let prefix = parts.next()?;
    let server = parts.next()?;
    if prefix == "mcp" && !server.is_empty() {
        Some(server.to_string())
    } else {
        None
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::CapabilityIntentSelectorPort for CapabilityIntentSelector {
    async fn select_candidates(
        &self,
        intent: &CapabilityIntent,
    ) -> Result<Vec<CapabilityCandidate>, crate::contracts::errors::ContractError> {
        let objective = intent.objective.to_ascii_lowercase();
        let required_tags = intent
            .required_tags
            .iter()
            .map(|tag| tag.to_ascii_lowercase())
            .collect::<Vec<_>>();

        let mut skill_refs = Vec::new();
        let mut plugin_refs = Vec::new();
        if let Some(db) = self.tools.state_store() {
            skill_refs = db
                .list_knowledge_by_prefix("skills:manifest:")
                .await
                .map_err(|error| {
                    crate::contracts::errors::ContractError::Storage(error.to_string())
                })?
                .into_iter()
                .map(|record| record.key)
                .collect();
            plugin_refs = db
                .list_knowledge_by_prefix("plugin:lifecycle:")
                .await
                .map_err(|error| {
                    crate::contracts::errors::ContractError::Storage(error.to_string())
                })?
                .into_iter()
                .map(|record| record.key)
                .collect();
        }

        let skill_router_hit = objective.contains("skill") || !skill_refs.is_empty();
        let plugin_router_hit = objective.contains("plugin") || !plugin_refs.is_empty();

        let mut candidates = self
            .tools
            .manifests()
            .into_iter()
            .map(|manifest| {
                let server = selector_parse_mcp_server(&manifest.registered_tool_name);
                let preferred_server_bonus = if let Some(server_name) = server.as_ref() {
                    if intent
                        .preferred_servers
                        .iter()
                        .any(|item| item == server_name)
                    {
                        0.25
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                let tag_bonus = required_tags
                    .iter()
                    .filter(|tag| {
                        manifest
                            .tags
                            .iter()
                            .any(|manifest_tag| manifest_tag.to_ascii_lowercase() == **tag)
                    })
                    .count() as f32
                    * 0.1;
                let objective_bonus = if objective.contains("mcp")
                    || objective.contains("tool")
                    || objective.contains("execute")
                {
                    0.05
                } else {
                    0.0
                };
                let skill_bonus = if skill_router_hit
                    && (manifest
                        .tags
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case("skill"))
                        || objective.contains("skill"))
                {
                    0.08
                } else {
                    0.0
                };
                let plugin_bonus = if plugin_router_hit
                    && (manifest
                        .tags
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case("plugin"))
                        || objective.contains("plugin"))
                {
                    0.08
                } else {
                    0.0
                };
                let score = manifest.health_score
                    + preferred_server_bonus
                    + tag_bonus
                    + objective_bonus
                    + skill_bonus
                    + plugin_bonus;
                CapabilityCandidate {
                    capability_id: manifest.capability_id.clone(),
                    server,
                    tool: manifest.registered_tool_name.clone(),
                    score,
                    active: manifest.status == CapabilityStatus::Active,
                    verified: manifest.approval_status == ApprovalStatus::Verified,
                    trusted: manifest.trust_status == TrustStatus::Trusted,
                    approval_required: manifest.requires_gate(),
                }
            })
            .collect::<Vec<_>>();

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        if candidates.is_empty() && (skill_router_hit || plugin_router_hit) {
            candidates.push(CapabilityCandidate {
                capability_id: "router:fallback:skill-plugin".to_string(),
                server: None,
                tool: "router::skill-plugin-fallback".to_string(),
                score: 0.10,
                active: true,
                verified: true,
                trusted: true,
                approval_required: false,
            });
        }

        Ok(candidates)
    }
}








