use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgSharingGateInput {
    pub session_id: String,
    pub verifier_passed: bool,
    pub risk_tier: String,
    pub org_safe: bool,
    pub reusable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgSharingGateDecision {
    pub allowed: bool,
    pub reason: String,
}

pub fn evaluate(input: &OrgSharingGateInput) -> OrgSharingGateDecision {
    if !input.verifier_passed {
        return OrgSharingGateDecision {
            allowed: false,
            reason: "blocked: verifier not passed".into(),
        };
    }
    if input.risk_tier == "high" && !input.org_safe {
        return OrgSharingGateDecision {
            allowed: false,
            reason: "blocked: high-risk output not org-safe".into(),
        };
    }
    if !input.reusable {
        return OrgSharingGateDecision {
            allowed: false,
            reason: "blocked: output not reusable yet".into(),
        };
    }

    OrgSharingGateDecision {
        allowed: true,
        reason: "allowed: verifier-passed + risk-cleared + org-safe + reusable".into(),
    }
}
