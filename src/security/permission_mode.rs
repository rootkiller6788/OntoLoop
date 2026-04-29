use serde::{Deserialize, Serialize};

use crate::tools::{CapabilityRisk, ForgedMcpToolManifest};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Strict,
    Prompt,
    Auto,
    Bypass,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Strict
    }
}

impl PermissionMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" => Self::Strict,
            "prompt" => Self::Prompt,
            "auto" => Self::Auto,
            "bypass" => Self::Bypass,
            _ => Self::Strict,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Prompt => "prompt",
            Self::Auto => "auto",
            Self::Bypass => "bypass",
        }
    }

    pub fn high_risk_authorized(&self) -> bool {
        matches!(self, Self::Strict)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionModeDecisionKind {
    Allow,
    RequiresApproval,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionModeDecision {
    pub kind: PermissionModeDecisionKind,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct PermissionModeEngine {
    mode: PermissionMode,
}

impl PermissionModeEngine {
    pub fn from_sources(config_mode: &str) -> Self {
        if let Ok(env_mode) = std::env::var("AUTOLOOP_PERMISSION_MODE") {
            return Self {
                mode: PermissionMode::parse(&env_mode),
            };
        }
        Self {
            mode: PermissionMode::parse(config_mode),
        }
    }

    pub fn mode(&self) -> PermissionMode {
        self.mode
    }

    pub fn evaluate_capability(
        &self,
        manifest: Option<&ForgedMcpToolManifest>,
    ) -> PermissionModeDecision {
        let Some(manifest) = manifest else {
            return PermissionModeDecision {
                kind: PermissionModeDecisionKind::Allow,
                reason: "no forged capability manifest; permission mode leaves decision unchanged"
                    .into(),
            };
        };

        match self.mode {
            PermissionMode::Strict => {
                if manifest.risk == CapabilityRisk::High {
                    return PermissionModeDecision {
                        kind: PermissionModeDecisionKind::RequiresApproval,
                        reason: "strict mode requires explicit approval for high-risk capability"
                            .into(),
                    };
                }
                if manifest.requires_gate() {
                    return PermissionModeDecision {
                        kind: PermissionModeDecisionKind::RequiresApproval,
                        reason: "strict mode requires approval for gated capability".into(),
                    };
                }
                PermissionModeDecision {
                    kind: PermissionModeDecisionKind::Allow,
                    reason: "strict mode allows non-gated capability".into(),
                }
            }
            PermissionMode::Prompt => {
                if manifest.risk == CapabilityRisk::High {
                    return PermissionModeDecision {
                        kind: PermissionModeDecisionKind::Blocked,
                        reason:
                            "high-risk capability blocked in prompt mode; switch to strict mode"
                                .into(),
                    };
                }
                if manifest.requires_gate() {
                    return PermissionModeDecision {
                        kind: PermissionModeDecisionKind::RequiresApproval,
                        reason: "prompt mode requires approval for gated capability".into(),
                    };
                }
                PermissionModeDecision {
                    kind: PermissionModeDecisionKind::Allow,
                    reason: "prompt mode allows low-risk capability".into(),
                }
            }
            PermissionMode::Auto => {
                if manifest.risk == CapabilityRisk::High {
                    return PermissionModeDecision {
                        kind: PermissionModeDecisionKind::Blocked,
                        reason: "high-risk capability blocked in auto mode".into(),
                    };
                }
                PermissionModeDecision {
                    kind: PermissionModeDecisionKind::Allow,
                    reason: "auto mode allows low/medium capability".into(),
                }
            }
            PermissionMode::Bypass => {
                if manifest.risk == CapabilityRisk::High {
                    return PermissionModeDecision {
                        kind: PermissionModeDecisionKind::Blocked,
                        reason: "high-risk capability blocked in restricted bypass mode".into(),
                    };
                }
                PermissionModeDecision {
                    kind: PermissionModeDecisionKind::Allow,
                    reason: "restricted bypass mode allows low/medium capability".into(),
                }
            }
        }
    }
}
