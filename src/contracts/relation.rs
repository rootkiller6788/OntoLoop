use std::collections::BTreeMap;

pub const RELATION_CONTRACT_VERSION: &str = "relation/v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelationNodeType {
    Session,
    Task,
    Turn,
    Tool,
    Capability,
    Policy,
    Evidence,
    Skill,
    Plugin,
    Patch,
    Custom,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelationEdgeType {
    DependsOn,
    ProducedBy,
    ApprovedBy,
    BlockedBy,
    Supersedes,
    References,
    BelongsTo,
    Custom,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelationEventType {
    NodeUpserted,
    EdgeUpserted,
    EdgeRejected,
    EdgeDeleted,
    ConsistencyViolation,
    RepairProposed,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelationScope {
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub capability_id: Option<String>,
    #[serde(default)]
    pub policy_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelationReason {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub deny_reason: Option<String>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelationNode {
    #[serde(alias = "id")]
    pub node_id: String,
    #[serde(alias = "kind")]
    pub node_type: RelationNodeType,
    #[serde(default)]
    pub scope: RelationScope,
    #[serde(default, alias = "name")]
    pub display_name: Option<String>,
    #[serde(default, alias = "attrs")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelationEdge {
    #[serde(alias = "id")]
    pub edge_id: String,
    #[serde(alias = "from")]
    pub from_node_id: String,
    #[serde(alias = "to")]
    pub to_node_id: String,
    #[serde(alias = "kind")]
    pub edge_type: RelationEdgeType,
    #[serde(default)]
    pub scope: RelationScope,
    #[serde(default, alias = "why")]
    pub reason: Option<RelationReason>,
    #[serde(default)]
    pub active: bool,
    #[serde(default, alias = "attrs")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelationEvent {
    #[serde(alias = "id")]
    pub event_id: String,
    #[serde(alias = "kind")]
    pub event_type: RelationEventType,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub edge_id: Option<String>,
    #[serde(default)]
    pub scope: RelationScope,
    #[serde(default, alias = "decision_reason")]
    pub reason: Option<RelationReason>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
    pub emitted_at_ms: u64,
    #[serde(default, alias = "attrs")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RelationContract {
    #[serde(default = "default_api_version")]
    pub api_version: String,
    #[serde(default, alias = "node_list")]
    pub nodes: Vec<RelationNode>,
    #[serde(default, alias = "edge_list")]
    pub edges: Vec<RelationEdge>,
    #[serde(default, alias = "event_list")]
    pub events: Vec<RelationEvent>,
}

fn default_api_version() -> String {
    RELATION_CONTRACT_VERSION.to_string()
}

pub fn relation_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == RELATION_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("relation/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_contract_roundtrip() {
        let contract = RelationContract {
            api_version: RELATION_CONTRACT_VERSION.to_string(),
            nodes: vec![RelationNode {
                node_id: "task:1".into(),
                node_type: RelationNodeType::Task,
                scope: RelationScope {
                    tenant_id: Some("tenant-a".into()),
                    session_id: Some("session-a".into()),
                    trace_id: Some("trace-a".into()),
                    task_id: Some("task:1".into()),
                    capability_id: None,
                    policy_id: Some("policy:v1".into()),
                },
                display_name: Some("Build bill page".into()),
                metadata: BTreeMap::from([("priority".into(), "high".into())]),
            }],
            edges: vec![RelationEdge {
                edge_id: "edge:1".into(),
                from_node_id: "task:1".into(),
                to_node_id: "capability:write_file".into(),
                edge_type: RelationEdgeType::DependsOn,
                scope: RelationScope {
                    tenant_id: Some("tenant-a".into()),
                    session_id: Some("session-a".into()),
                    trace_id: Some("trace-a".into()),
                    task_id: Some("task:1".into()),
                    capability_id: Some("capability:write_file".into()),
                    policy_id: Some("policy:v1".into()),
                },
                reason: Some(RelationReason {
                    code: "dep_required".into(),
                    message: "task requires file writer".into(),
                    deny_reason: None,
                    evidence_ref: Some("evidence:1".into()),
                    replay_fp: Some("fp:1".into()),
                    metadata: BTreeMap::new(),
                }),
                active: true,
                metadata: BTreeMap::new(),
            }],
            events: vec![RelationEvent {
                event_id: "event:1".into(),
                event_type: RelationEventType::EdgeUpserted,
                node_id: Some("task:1".into()),
                edge_id: Some("edge:1".into()),
                scope: RelationScope {
                    tenant_id: Some("tenant-a".into()),
                    session_id: Some("session-a".into()),
                    trace_id: Some("trace-a".into()),
                    task_id: Some("task:1".into()),
                    capability_id: Some("capability:write_file".into()),
                    policy_id: Some("policy:v1".into()),
                },
                reason: None,
                evidence_ref: Some("evidence:1".into()),
                replay_fp: Some("fp:1".into()),
                emitted_at_ms: 1_717_000_000_100,
                metadata: BTreeMap::new(),
            }],
        };

        let raw = serde_json::to_string(&contract).expect("serialize relation contract");
        let decoded: RelationContract =
            serde_json::from_str(&raw).expect("deserialize relation contract");
        assert_eq!(decoded, contract);
    }

    #[test]
    fn relation_contract_accepts_legacy_aliases() {
        let raw = serde_json::json!({
            "api_version": "relation/v1",
            "node_list": [
                {
                    "id": "node:1",
                    "kind": "session",
                    "name": "legacy-session",
                    "attrs": { "origin": "legacy" }
                }
            ],
            "edge_list": [
                {
                    "id": "edge:1",
                    "from": "node:1",
                    "to": "node:2",
                    "kind": "references",
                    "why": {
                        "code": "legacy_ref",
                        "message": "legacy edge format"
                    },
                    "attrs": { "source": "migration" }
                }
            ],
            "event_list": [
                {
                    "id": "event:1",
                    "kind": "edge_upserted",
                    "edge_id": "edge:1",
                    "emitted_at_ms": 1_717_000_000_200u64,
                    "attrs": { "origin": "legacy" }
                }
            ]
        });

        let decoded: RelationContract =
            serde_json::from_value(raw).expect("deserialize legacy relation contract");
        assert_eq!(decoded.nodes.len(), 1);
        assert_eq!(decoded.edges.len(), 1);
        assert_eq!(decoded.events.len(), 1);
        assert_eq!(decoded.nodes[0].node_id, "node:1");
        assert_eq!(decoded.edges[0].from_node_id, "node:1");
        assert_eq!(decoded.events[0].event_id, "event:1");
    }

    #[test]
    fn relation_compat_accepts_v1_series() {
        assert!(relation_contract_compatible("relation/v1"));
        assert!(relation_contract_compatible("relation/v1.2"));
        assert!(relation_contract_compatible("RELATION/V1-beta"));
    }

    #[test]
    fn relation_compat_rejects_other_majors() {
        assert!(!relation_contract_compatible("relation/v2"));
        assert!(!relation_contract_compatible("v1"));
    }
}
