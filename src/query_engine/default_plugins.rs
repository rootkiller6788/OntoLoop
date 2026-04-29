use std::collections::BTreeMap;

use crate::{
    contracts::{
        plugin::{
            PluginApiNegotiationRequest, PluginApiNegotiationResult, PluginCapabilityDescriptor,
            PluginCompatSpec, PluginErrorCode, PluginExecutionError, PluginInvocationInput,
            PluginInvocationOutput, PluginKind, PluginManifestContract, PluginRisk,
            PluginRuntimeContract,
        },
        version::CONTRACT_VERSION,
    },
    providers::ChatMessage,
};

use super::{
    compactor::{CompactionBoundary, ContextCompactor, estimate_message_tokens},
    context_compiler::TokenBudgetFrame,
};

#[derive(Debug, Clone)]
pub struct ConstraintEvaluation {
    pub passed: bool,
    pub reason: String,
    pub budget: TokenBudgetFrame,
    pub pinned_messages: Vec<ChatMessage>,
    pub constraint_version: String,
    pub constraint_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EstimationResult {
    pub estimated_tokens: u32,
    pub relevance_score: f32,
    pub semantic_distortion_risk: f32,
    pub attention_mismatch_risk: f32,
    pub malicious_intent_risk: f32,
    pub anchor_retention_benefit: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObjectiveWeights {
    pub task_utility: f32,
    pub distortion_penalty: f32,
    pub attention_mismatch_penalty: f32,
    pub token_cost_penalty: f32,
}

impl Default for ObjectiveWeights {
    fn default() -> Self {
        Self {
            task_utility: 1.0,
            distortion_penalty: 1.0,
            attention_mismatch_penalty: 1.0,
            token_cost_penalty: 1.0,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ObjectiveScore {
    pub task_utility: f32,
    pub distortion: f32,
    pub attention_mismatch: f32,
    pub token_cost: f32,
    pub weighted_score: f32,
}

#[derive(Debug, Clone)]
pub struct OptimizationResult {
    pub messages: Vec<ChatMessage>,
    pub boundary: Option<CompactionBoundary>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct RepairResult {
    pub messages: Vec<ChatMessage>,
    pub boundary: Option<CompactionBoundary>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct ProofResult {
    pub dropped_message_count: usize,
    pub compression_ratio: f32,
    pub risk_flags: Vec<String>,
    pub notes: Vec<String>,
    pub objective: ObjectiveScore,
    pub annotation: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ContextConstraintPlugin {
    manifest: PluginManifestContract,
}

impl ContextConstraintPlugin {
    pub fn default_builtin() -> Self {
        Self {
            manifest: manifest_for(
                "plugin:context-constraint",
                PluginKind::ContextConstraint,
                PluginRisk::Medium,
                "context.constraint.evaluate",
                "v2",
            ),
        }
    }

    pub fn evaluate(
        &self,
        messages: &[ChatMessage],
        budget: &TokenBudgetFrame,
    ) -> ConstraintEvaluation {
        let has_valid_budget = budget.max_input_tokens > 0 && budget.max_output_tokens > 0;
        let mut constraint_ids = vec![
            "constraint.budget.non_zero".to_string(),
            "constraint.anchor.preserve".to_string(),
            "constraint.audit.retention".to_string(),
            "constraint.symbol.anchor".to_string(),
        ];
        if budget.max_input_tokens < budget.reserve_tokens {
            constraint_ids.push("constraint.budget.reserve_within_input".to_string());
        }
        let reason = if has_valid_budget {
            "hard constraints satisfied".to_string()
        } else {
            "invalid token budget frame".to_string()
        };
        ConstraintEvaluation {
            passed: has_valid_budget,
            reason,
            budget: budget.clone(),
            pinned_messages: messages.to_vec(),
            constraint_version: "constraint-v2".to_string(),
            constraint_ids,
        }
    }
}

impl PluginRuntimeContract for ContextConstraintPlugin {
    fn manifest(&self) -> &PluginManifestContract {
        &self.manifest
    }

    fn negotiate_api(&self, request: &PluginApiNegotiationRequest) -> PluginApiNegotiationResult {
        negotiate(self.manifest(), request)
    }

    fn invoke(
        &self,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput, PluginExecutionError> {
        Ok(PluginInvocationOutput {
            invocation_id: input.invocation_id.clone(),
            plugin_id: self.manifest.id.clone(),
            status: "ok".into(),
            payload: serde_json::json!({"constraint": "validated"}),
            evidence_refs: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SoftEstimatorPlugin {
    manifest: PluginManifestContract,
}

impl SoftEstimatorPlugin {
    pub fn default_builtin() -> Self {
        Self {
            manifest: manifest_for(
                "plugin:soft-estimator",
                PluginKind::SoftEstimator,
                PluginRisk::Low,
                "context.estimate.soft",
                "v2",
            ),
        }
    }

    pub fn estimate(
        &self,
        messages: &[ChatMessage],
        _budget: &TokenBudgetFrame,
    ) -> EstimationResult {
        let estimated = estimate_message_tokens(messages);
        let long_context_factor = (estimated as f32 / 8192.0).clamp(0.0, 1.0);
        EstimationResult {
            estimated_tokens: estimated,
            relevance_score: 1.0 - (long_context_factor * 0.12),
            semantic_distortion_risk: long_context_factor * 0.25,
            attention_mismatch_risk: long_context_factor * 0.2,
            malicious_intent_risk: 0.0,
            anchor_retention_benefit: 0.5 + ((1.0 - long_context_factor) * 0.5),
        }
    }
}

impl PluginRuntimeContract for SoftEstimatorPlugin {
    fn manifest(&self) -> &PluginManifestContract {
        &self.manifest
    }

    fn negotiate_api(&self, request: &PluginApiNegotiationRequest) -> PluginApiNegotiationResult {
        negotiate(self.manifest(), request)
    }

    fn invoke(
        &self,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput, PluginExecutionError> {
        Ok(PluginInvocationOutput {
            invocation_id: input.invocation_id.clone(),
            plugin_id: self.manifest.id.clone(),
            status: "ok".into(),
            payload: serde_json::json!({"estimator": "evaluated"}),
            evidence_refs: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct OptimizerPlugin {
    manifest: PluginManifestContract,
    compactor: ContextCompactor,
}

impl OptimizerPlugin {
    pub fn default_builtin(preserve_recent_messages: usize) -> Self {
        Self {
            manifest: manifest_for(
                "plugin:optimizer",
                PluginKind::Optimizer,
                PluginRisk::Low,
                "context.optimize",
                "v2",
            ),
            compactor: ContextCompactor::new(preserve_recent_messages),
        }
    }

    pub fn optimize(
        &self,
        messages: &[ChatMessage],
        budget: &TokenBudgetFrame,
    ) -> OptimizationResult {
        let compacted = self.compactor.compact(messages, budget);
        OptimizationResult {
            messages: compacted.messages,
            boundary: compacted.boundary,
            estimated_tokens: compacted.estimated_tokens,
        }
    }
}

impl PluginRuntimeContract for OptimizerPlugin {
    fn manifest(&self) -> &PluginManifestContract {
        &self.manifest
    }

    fn negotiate_api(&self, request: &PluginApiNegotiationRequest) -> PluginApiNegotiationResult {
        negotiate(self.manifest(), request)
    }

    fn invoke(
        &self,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput, PluginExecutionError> {
        Ok(PluginInvocationOutput {
            invocation_id: input.invocation_id.clone(),
            plugin_id: self.manifest.id.clone(),
            status: "ok".into(),
            payload: serde_json::json!({"optimizer": "applied"}),
            evidence_refs: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct RepairPlugin {
    manifest: PluginManifestContract,
}

impl RepairPlugin {
    pub fn default_builtin() -> Self {
        Self {
            manifest: manifest_for(
                "plugin:repair",
                PluginKind::Repair,
                PluginRisk::Low,
                "context.repair",
                "v2",
            ),
        }
    }

    pub fn repair(
        &self,
        optimization: OptimizationResult,
        original_messages: &[ChatMessage],
    ) -> RepairResult {
        if optimization.messages.is_empty() {
            return RepairResult {
                messages: original_messages.to_vec(),
                boundary: optimization.boundary,
                estimated_tokens: estimate_message_tokens(original_messages),
            };
        }
        RepairResult {
            messages: optimization.messages,
            boundary: optimization.boundary,
            estimated_tokens: optimization.estimated_tokens,
        }
    }
}

impl PluginRuntimeContract for RepairPlugin {
    fn manifest(&self) -> &PluginManifestContract {
        &self.manifest
    }

    fn negotiate_api(&self, request: &PluginApiNegotiationRequest) -> PluginApiNegotiationResult {
        negotiate(self.manifest(), request)
    }

    fn invoke(
        &self,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput, PluginExecutionError> {
        Ok(PluginInvocationOutput {
            invocation_id: input.invocation_id.clone(),
            plugin_id: self.manifest.id.clone(),
            status: "ok".into(),
            payload: serde_json::json!({"repair": "closed"}),
            evidence_refs: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ProofPlugin {
    manifest: PluginManifestContract,
}

impl ProofPlugin {
    pub fn default_builtin() -> Self {
        Self {
            manifest: manifest_for(
                "plugin:proof",
                PluginKind::Proof,
                PluginRisk::Low,
                "context.proof.emit",
                "v2",
            ),
        }
    }

    pub fn prove(
        &self,
        original_messages: &[ChatMessage],
        repaired_messages: &[ChatMessage],
        boundary: Option<&CompactionBoundary>,
    ) -> ProofResult {
        let original_len = original_messages.len();
        let repaired_len = repaired_messages.len();
        let dropped = original_len.saturating_sub(repaired_len);
        let compression_ratio = if original_len == 0 {
            1.0
        } else {
            repaired_len as f32 / original_len as f32
        };

        let mut notes = Vec::new();
        if let Some(boundary) = boundary {
            notes.push(format!("boundary:{}", boundary.boundary_id));
        }

        ProofResult {
            dropped_message_count: dropped,
            compression_ratio,
            risk_flags: Vec::new(),
            notes,
            objective: ObjectiveScore::default(),
            annotation: serde_json::json!({
                "dropped_mapping": [],
                "compression_stats": {
                    "original_messages": original_len,
                    "compiled_messages": repaired_len,
                    "compression_ratio": compression_ratio,
                },
                "risk_labels": [],
                "policy_hints": [],
                "replay_fingerprint": serde_json::Value::Null,
                "decision_summary": serde_json::Value::Null,
            }),
        }
    }
}

impl PluginRuntimeContract for ProofPlugin {
    fn manifest(&self) -> &PluginManifestContract {
        &self.manifest
    }

    fn negotiate_api(&self, request: &PluginApiNegotiationRequest) -> PluginApiNegotiationResult {
        negotiate(self.manifest(), request)
    }

    fn invoke(
        &self,
        input: &PluginInvocationInput,
    ) -> Result<PluginInvocationOutput, PluginExecutionError> {
        Ok(PluginInvocationOutput {
            invocation_id: input.invocation_id.clone(),
            plugin_id: self.manifest.id.clone(),
            status: "ok".into(),
            payload: serde_json::json!({"proof": "generated"}),
            evidence_refs: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct DefaultContextCompilerPlugins {
    pub constraint: ContextConstraintPlugin,
    pub estimator: SoftEstimatorPlugin,
    pub optimizer: OptimizerPlugin,
    pub repair: RepairPlugin,
    pub proof: ProofPlugin,
}

impl DefaultContextCompilerPlugins {
    pub fn new(preserve_recent_messages: usize) -> Self {
        Self {
            constraint: ContextConstraintPlugin::default_builtin(),
            estimator: SoftEstimatorPlugin::default_builtin(),
            optimizer: OptimizerPlugin::default_builtin(preserve_recent_messages),
            repair: RepairPlugin::default_builtin(),
            proof: ProofPlugin::default_builtin(),
        }
    }

    pub fn manifests(&self) -> Vec<PluginManifestContract> {
        vec![
            self.constraint.manifest().clone(),
            self.estimator.manifest().clone(),
            self.optimizer.manifest().clone(),
            self.repair.manifest().clone(),
            self.proof.manifest().clone(),
        ]
    }
}

fn manifest_for(
    id: &str,
    kind: PluginKind,
    risk: PluginRisk,
    capability_id: &str,
    api_version: &str,
) -> PluginManifestContract {
    PluginManifestContract {
        id: id.to_string(),
        plugin_id: id.to_string(),
        version: CONTRACT_VERSION.to_string(),
        kind,
        capability: PluginCapabilityDescriptor {
            capability_id: capability_id.to_string(),
            description: format!("default builtin plugin for {}", capability_id),
            scopes: vec!["context.compile".into()],
        },
        risk,
        compat: PluginCompatSpec {
            api_version: api_version.to_string(),
            compatible_api_versions: Vec::new(),
            min_core_version: CONTRACT_VERSION.to_string(),
            max_core_version: None,
        },
        name: id.to_string(),
        source: "builtin://query_engine/default".into(),
        signature_ref: Some(format!("builtin-signature:{}", id)),
        permissions: vec!["context.read".into(), "context.transform".into()],
        hooks: Vec::new(),
        commands: Vec::new(),
        event_contract_version: crate::contracts::plugin::PLUGIN_EVENT_CONTRACT_V2.to_string(),
        lifecycle_contract_version: crate::contracts::plugin::PLUGIN_LIFECYCLE_CONTRACT_V2
            .to_string(),
        isolation: crate::contracts::plugin::PluginIsolationContract::default(),
        facade: crate::contracts::plugin::PluginFacadeContract::default(),
        metadata: BTreeMap::from([("builtin".into(), "true".into())]),
    }
}

fn negotiate(
    manifest: &PluginManifestContract,
    request: &PluginApiNegotiationRequest,
) -> PluginApiNegotiationResult {
    if !manifest
        .compat
        .supports_api_version(&request.host_api_version)
    {
        return PluginApiNegotiationResult {
            accepted: false,
            plugin_id: manifest.id.clone(),
            selected_api_version: manifest.compat.api_version.clone(),
            reason: format!(
                "unsupported host api version host={} plugin={}",
                request.host_api_version, manifest.compat.api_version
            ),
        };
    }

    if let Some(missing_scope) = request.required_scopes.iter().find(|scope| {
        !manifest
            .capability
            .scopes
            .iter()
            .any(|candidate| candidate == *scope)
    }) {
        return PluginApiNegotiationResult {
            accepted: false,
            plugin_id: manifest.id.clone(),
            selected_api_version: request.host_api_version.clone(),
            reason: format!("missing required scope '{}'", missing_scope),
        };
    }

    PluginApiNegotiationResult {
        accepted: true,
        plugin_id: manifest.id.clone(),
        selected_api_version: request.host_api_version.clone(),
        reason: "api negotiation accepted".into(),
    }
}

pub fn plugin_invoke_unsupported_error(message: impl Into<String>) -> PluginExecutionError {
    PluginExecutionError {
        code: PluginErrorCode::ExecutionFailed,
        message: message.into(),
        retryable: false,
        details: BTreeMap::new(),
    }
}
