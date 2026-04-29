use super::frozen_manifest;
use super::protocol::{CorePackageKind, CorePackageManifest};

pub struct GatewayCore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayInput {
    pub session_id: String,
    pub tenant_id: String,
    pub intent: String,
    pub actor: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayDecision {
    pub accepted: bool,
    pub scope: String,
    pub reason: String,
    pub normalized_intent: String,
}

impl GatewayCore {
    pub fn manifest_frozen() -> CorePackageManifest {
        frozen_manifest(
            "core.gateway",
            CorePackageKind::GatewayCore,
            "memory-gateway",
        )
    }

    pub fn decide(input: &GatewayInput) -> GatewayDecision {
        let normalized_intent = input.intent.trim().to_string();
        GatewayDecision {
            accepted: !normalized_intent.is_empty(),
            scope: format!("tenant:{}:memory", input.tenant_id),
            reason: if normalized_intent.is_empty() {
                "empty intent rejected".to_string()
            } else {
                "accepted by gateway policy".to_string()
            },
            normalized_intent,
        }
    }
}
