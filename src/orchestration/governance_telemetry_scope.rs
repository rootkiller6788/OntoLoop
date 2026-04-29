use serde::{Deserialize, Serialize};

use crate::contracts::org::OrganizationContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceTelemetryScope {
    pub scope_id: String,
    pub session_id: String,
    pub tenant_scope: String,
    pub risk_tier: String,
    pub privacy_level: String,
    pub approval_required: bool,
    pub retention_hours: u32,
    pub redaction_fields: Vec<String>,
}

impl GovernanceTelemetryScope {
    pub fn compile(org: &OrganizationContext) -> Self {
        let approval_required = org
            .approval_policy
            .to_ascii_lowercase()
            .contains("approval_required");
        let blocked = org.quotas.blocked_count;
        let risk_tier = if approval_required || blocked >= 3 {
            "high"
        } else if blocked > 0 {
            "medium"
        } else {
            "low"
        }
        .to_string();

        let privacy_level = if org.role.to_ascii_lowercase().contains("admin") {
            "restricted"
        } else {
            "internal"
        }
        .to_string();

        let retention_hours = match risk_tier.as_str() {
            "high" => 24,
            "medium" => 72,
            _ => 168,
        };

        let mut redaction_fields = vec![
            "identity.lease_token".to_string(),
            "payload.raw".to_string(),
            "secrets".to_string(),
        ];
        if privacy_level == "restricted" {
            redaction_fields.push("principal_id".to_string());
            redaction_fields.push("kb_refs".to_string());
        }

        Self {
            scope_id: format!("gov-scope:{}", org.session_id),
            session_id: org.session_id.clone(),
            tenant_scope: org.tenant_id.clone(),
            risk_tier,
            privacy_level,
            approval_required,
            retention_hours,
            redaction_fields,
        }
    }
}
