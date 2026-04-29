pub const CORE_PROTOCOL_VERSION_V2: &str = "core-protocol-v2";
pub const CORE_LIFECYCLE_CONTRACT_V2: &str = "core-lifecycle-v2";
pub const CORE_EVENT_CONTRACT_V2: &str = "core-event-v2";
pub const CORE_FACADE_CONTRACT_V2: &str = "core-facade-v2";

#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum CorePackageKind {
    GatewayCore,
    RecallCore,
    PatchCore,
    RepoCore,
    CompilerCore,
    ProvenanceCore,
    ContextCore,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CorePackageManifest {
    pub id: String,
    pub kind: CorePackageKind,
    pub version: String,
    pub protocol_version: String,
    pub lifecycle_contract_version: String,
    pub event_contract_version: String,
    pub facade_contract_version: String,
    pub facade_only: bool,
    pub owner: String,
}

impl CorePackageManifest {
    pub fn frozen_v2(
        id: impl Into<String>,
        kind: CorePackageKind,
        owner: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            version: "v2".to_string(),
            protocol_version: CORE_PROTOCOL_VERSION_V2.to_string(),
            lifecycle_contract_version: CORE_LIFECYCLE_CONTRACT_V2.to_string(),
            event_contract_version: CORE_EVENT_CONTRACT_V2.to_string(),
            facade_contract_version: CORE_FACADE_CONTRACT_V2.to_string(),
            facade_only: true,
            owner: owner.into(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CorePackageStatus {
    Ready,
    Degraded,
    Disabled,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CorePackageHealth {
    pub kind: CorePackageKind,
    pub status: CorePackageStatus,
    pub message: String,
}
