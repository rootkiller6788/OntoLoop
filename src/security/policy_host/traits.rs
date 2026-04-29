use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::contracts::policy_pdp::{PolicyDecision, PolicyMode, PolicyVersion};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedPolicyInput {
    pub tenant: String,
    pub principal: String,
    pub capability: String,
    pub tool: String,
    #[serde(default)]
    pub context: Value,
}

impl UnifiedPolicyInput {
    pub fn to_eval_value(&self) -> Value {
        serde_json::json!({
            "tenant": self.tenant.clone(),
            "principal": self.principal.clone(),
            "capability": self.capability.clone(),
            "tool": self.tool.clone(),
            "context": self.context.clone(),
            "tenant_id": self.tenant.clone(),
            "principal_id": self.principal.clone(),
            "capability_id": self.capability.clone(),
            "tool_name": self.tool.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyHostMetadata {
    pub policy_id: String,
    pub policy_version: PolicyVersion,
    pub mode: PolicyMode,
    pub wasm_entrypoint: String,
}

#[async_trait]
pub trait PolicyHost: Send + Sync {
    fn metadata(&self) -> &PolicyHostMetadata;

    async fn evaluate(&self, input: &UnifiedPolicyInput) -> Result<PolicyDecision>;
}

