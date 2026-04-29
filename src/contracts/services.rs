use std::collections::BTreeMap;

use super::ids::{SessionId, TraceId};

pub const SERVICE_GATE_TOKEN_PREFIX: &str = "svc-gate";
pub const SERVICE_GATE_TOKEN_FIELD: &str = "__gate_token";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceDomain {
    Provider,
    Tool,
    Policy,
    Plugin,
    Memory,
    Relation,
    SkillFoundry,
    Telemetry,
    SettingsSync,
    Research,
}

impl ServiceDomain {
    pub fn gate_scope(&self) -> &'static str {
        match self {
            ServiceDomain::Provider => "provider",
            ServiceDomain::Tool => "tool",
            ServiceDomain::Policy => "policy",
            ServiceDomain::Plugin => "plugin",
            ServiceDomain::Memory => "memory",
            ServiceDomain::Relation => "relation",
            ServiceDomain::SkillFoundry => "skill_foundry",
            ServiceDomain::Telemetry => "telemetry",
            ServiceDomain::SettingsSync => "settings_sync",
            ServiceDomain::Research => "research",
        }
    }

    pub fn requires_gate_token(&self) -> bool {
        matches!(
            self,
            ServiceDomain::Provider
                | ServiceDomain::Tool
                | ServiceDomain::Memory
                | ServiceDomain::Relation
        )
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceCall {
    pub session_id: SessionId,
    pub trace_id: TraceId,
    pub service_domain: ServiceDomain,
    pub service_name: String,
    pub operation: String,
    pub input: serde_json::Value,
    pub budget_scope: String,
    pub requested_at_ms: u64,
}

pub fn build_service_gate_token(
    session_id: &SessionId,
    domain: &ServiceDomain,
    issued_at_ms: u64,
) -> String {
    format!(
        "{SERVICE_GATE_TOKEN_PREFIX}:{session_id}:{}:{issued_at_ms}",
        domain.gate_scope()
    )
}

pub fn service_gate_token_valid(
    token: &str,
    session_id: &SessionId,
    domain: &ServiceDomain,
) -> bool {
    let expected_prefix = format!(
        "{SERVICE_GATE_TOKEN_PREFIX}:{session_id}:{}:",
        domain.gate_scope()
    );
    token.starts_with(&expected_prefix)
}

pub fn service_call_gate_token(input: &serde_json::Value) -> Option<&str> {
    input
        .get(SERVICE_GATE_TOKEN_FIELD)
        .and_then(serde_json::Value::as_str)
}

pub fn attach_service_gate_token(input: &mut serde_json::Value, token: String) {
    match input {
        serde_json::Value::Object(map) => {
            map.insert(
                SERVICE_GATE_TOKEN_FIELD.to_string(),
                serde_json::Value::String(token),
            );
        }
        other => {
            let wrapped_payload = other.clone();
            *other = serde_json::json!({
                "payload": wrapped_payload,
                SERVICE_GATE_TOKEN_FIELD: token,
            });
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceResult {
    pub service_name: String,
    pub operation: String,
    pub success: bool,
    pub output: serde_json::Value,
    pub error: Option<String>,
    pub latency_ms: u64,
    pub cost_micros: Option<u64>,
    pub finished_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceMediationPolicy {
    pub policy_id: String,
    pub allow_domains: Vec<ServiceDomain>,
    pub deny_operations: Vec<String>,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub circuit_breaker_profile: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServiceHealthSnapshot {
    pub service_name: String,
    pub status: String,
    pub error_rate: f32,
    pub latency_p95_ms: u64,
    pub last_incident_at_ms: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SettingsSyncPatch {
    pub tenant_id: String,
    pub scope: String,
    pub version: String,
    pub payload: serde_json::Value,
}


