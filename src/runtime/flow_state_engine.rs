use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use autoloop_state_adapter::StateStore;

use crate::contracts::{
    errors::ContractError,
    flow::{FlowMapSnapshot, FlowNodeRecord, FlowNodeState, FlowStateFlags, FlowStatePatch},
    ids::SessionId,
    ports::FlowStateEnginePort,
};
use crate::orchestration::current_time_ms;

const FLOW_SCOPE: &str = "flow-state-engine";

#[derive(Debug, Clone)]
pub struct FlowRuntimeUpdate {
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub state: Option<FlowNodeState>,
    pub reason: String,
    pub side_effect_state: Option<String>,
    pub budget_state: Option<String>,
    pub trigger_state: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone)]
pub struct FlowStateEngine {
    db: StateStore,
}

impl FlowStateEngine {
    pub fn new(db: StateStore) -> Self {
        Self { db }
    }

    pub async fn apply_runtime_update(&self, update: FlowRuntimeUpdate) -> Result<()> {
        let session_id = update.session_id.clone();
        let flow_id = flow_id_for_session(&session_id);
        let mut snapshot = self
            .fetch_flow_map_internal(&session_id)
            .await?
            .unwrap_or_else(|| default_flow_map(&session_id));

        let node_index = snapshot
            .nodes
            .iter()
            .position(|node| node.node_id == update.task_id)
            .or_else(|| {
                snapshot.nodes.push(FlowNodeRecord {
                    node_id: update.task_id.clone(),
                    task_id: update.task_id.clone(),
                    capability_id: update.capability_id.clone(),
                    state: FlowNodeState::Pending,
                    depends_on: Vec::new(),
                    started_at_ms: None,
                    ended_at_ms: None,
                    flags: FlowStateFlags::default(),
                    metadata: BTreeMap::new(),
                });
                snapshot.nodes.len().checked_sub(1)
            })
            .ok_or_else(|| anyhow!("failed to allocate flow node for task {}", update.task_id))?;

        let from_state = snapshot.nodes[node_index].state.clone();
        let to_state = update.state.clone().unwrap_or(from_state.clone());

        let mut metadata = update.metadata;
        metadata.insert("session_id".into(), session_id.clone());
        metadata.insert("trace_id".into(), update.trace_id);
        metadata.insert("task_id".into(), update.task_id.clone());
        metadata.insert("capability_id".into(), update.capability_id.clone());
        if let Some(side_effect) = update.side_effect_state {
            metadata.insert("side_effect_state".into(), side_effect);
        }
        if let Some(budget) = update.budget_state {
            metadata.insert("budget_state".into(), budget);
        }
        if let Some(trigger) = update.trigger_state {
            metadata.insert("trigger_state".into(), trigger);
        }

        let patch = FlowStatePatch {
            flow_id,
            node_id: update.task_id,
            from: Some(from_state),
            to: to_state,
            reason: update.reason,
            trace_id: metadata
                .get("trace_id")
                .cloned()
                .unwrap_or_else(|| "trace:unknown".to_string()),
            updated_at_ms: current_time_ms(),
            metadata,
        };
        self.apply_patch_internal(&patch).await
    }

    pub async fn set_budget_state(
        &self,
        session_id: &str,
        trace_id: &str,
        task_id: &str,
        capability_id: &str,
        budget_state: &str,
        reason: &str,
    ) -> Result<()> {
        self.apply_runtime_update(FlowRuntimeUpdate {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            task_id: task_id.to_string(),
            capability_id: capability_id.to_string(),
            state: None,
            reason: reason.to_string(),
            side_effect_state: None,
            budget_state: Some(budget_state.to_string()),
            trigger_state: None,
            metadata: BTreeMap::new(),
        })
        .await
    }

    async fn fetch_flow_map_internal(&self, session_id: &str) -> Result<Option<FlowMapSnapshot>> {
        let Some(record) = self.db.get_knowledge(&flow_map_key(session_id)).await? else {
            return Ok(None);
        };
        let snapshot = serde_json::from_str::<FlowMapSnapshot>(&record.value).map_err(|error| {
            anyhow!(
                "failed to deserialize flow map snapshot for session {}: {}",
                session_id,
                error
            )
        })?;
        Ok(Some(snapshot))
    }

    async fn upsert_flow_map_internal(&self, snapshot: &FlowMapSnapshot) -> Result<()> {
        self.db
            .upsert_json_knowledge(flow_map_key(&snapshot.session_id), snapshot, FLOW_SCOPE)
            .await?;
        self.db
            .upsert_knowledge(
                flow_index_key(&snapshot.flow_id),
                snapshot.session_id.clone(),
                FLOW_SCOPE.into(),
            )
            .await?;
        for node in &snapshot.nodes {
            self.persist_node_views(snapshot, node).await?;
        }
        Ok(())
    }

    async fn persist_node_views(
        &self,
        snapshot: &FlowMapSnapshot,
        node: &FlowNodeRecord,
    ) -> Result<()> {
        let state_view = serde_json::json!({
            "session_id": snapshot.session_id,
            "flow_id": snapshot.flow_id,
            "plan_id": snapshot.plan_id,
            "run_id": snapshot.run_id,
            "node_id": node.node_id,
            "task_id": node.task_id,
            "capability_id": node.capability_id,
            "state": format!("{:?}", node.state).to_ascii_lowercase(),
            "started_at_ms": node.started_at_ms,
            "ended_at_ms": node.ended_at_ms,
            "updated_at_ms": snapshot.updated_at_ms,
        });
        self.db
            .upsert_json_knowledge(
                node_state_key(&snapshot.session_id, &node.node_id),
                &state_view,
                FLOW_SCOPE,
            )
            .await?;

        self.db
            .upsert_json_knowledge(
                side_effect_key(&snapshot.session_id, &node.node_id),
                &serde_json::json!({
                    "value": node.flags.side_effect_state,
                    "updated_at_ms": snapshot.updated_at_ms,
                }),
                FLOW_SCOPE,
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                budget_state_key(&snapshot.session_id, &node.node_id),
                &serde_json::json!({
                    "value": node.flags.budget_state,
                    "updated_at_ms": snapshot.updated_at_ms,
                }),
                FLOW_SCOPE,
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                trigger_state_key(&snapshot.session_id, &node.node_id),
                &serde_json::json!({
                    "value": node.flags.trigger_state,
                    "updated_at_ms": snapshot.updated_at_ms,
                }),
                FLOW_SCOPE,
            )
            .await?;
        Ok(())
    }

    async fn apply_patch_internal(&self, patch: &FlowStatePatch) -> Result<()> {
        let session_id = if let Some(session) = patch.metadata.get("session_id") {
            session.clone()
        } else if let Some(record) = self
            .db
            .get_knowledge(&flow_index_key(&patch.flow_id))
            .await?
        {
            record.value
        } else {
            parse_session_from_flow_id(&patch.flow_id)
                .unwrap_or_else(|| "session:unknown".to_string())
        };

        let mut snapshot = self
            .fetch_flow_map_internal(&session_id)
            .await?
            .unwrap_or_else(|| default_flow_map(&session_id));
        if snapshot.flow_id.is_empty() {
            snapshot.flow_id = patch.flow_id.clone();
        }
        if snapshot.flow_id != patch.flow_id {
            snapshot.flow_id = patch.flow_id.clone();
        }

        let node_index = snapshot
            .nodes
            .iter()
            .position(|node| node.node_id == patch.node_id)
            .or_else(|| {
                let mut flags = FlowStateFlags::default();
                if let Some(value) = patch.metadata.get("side_effect_state") {
                    flags.side_effect_state = value.clone();
                }
                if let Some(value) = patch.metadata.get("budget_state") {
                    flags.budget_state = value.clone();
                }
                if let Some(value) = patch.metadata.get("trigger_state") {
                    flags.trigger_state = value.clone();
                }
                snapshot.nodes.push(FlowNodeRecord {
                    node_id: patch.node_id.clone(),
                    task_id: patch
                        .metadata
                        .get("task_id")
                        .cloned()
                        .unwrap_or_else(|| patch.node_id.clone()),
                    capability_id: patch
                        .metadata
                        .get("capability_id")
                        .cloned()
                        .unwrap_or_else(|| "capability:unknown".to_string()),
                    state: patch.to.clone(),
                    depends_on: parse_depends_on(patch.metadata.get("depends_on")),
                    started_at_ms: if patch.to == FlowNodeState::Running {
                        Some(patch.updated_at_ms)
                    } else {
                        None
                    },
                    ended_at_ms: if is_terminal_state(&patch.to) {
                        Some(patch.updated_at_ms)
                    } else {
                        None
                    },
                    flags,
                    metadata: BTreeMap::new(),
                });
                snapshot.nodes.len().checked_sub(1)
            })
            .ok_or_else(|| anyhow!("failed to allocate node for patch {}", patch.node_id))?;

        let node = snapshot
            .nodes
            .get_mut(node_index)
            .ok_or_else(|| anyhow!("failed to fetch mutable flow node {}", patch.node_id))?;

        if let Some(expected_from) = patch.from.as_ref() {
            if *expected_from != node.state {
                node.metadata.insert(
                    "transition_warning".into(),
                    format!(
                        "state mismatch before patch: expected={:?}, actual={:?}",
                        expected_from, node.state
                    ),
                );
            }
        }

        node.state = patch.to.clone();
        if patch.to == FlowNodeState::Running && node.started_at_ms.is_none() {
            node.started_at_ms = Some(patch.updated_at_ms);
        }
        if is_terminal_state(&patch.to) {
            node.ended_at_ms = Some(patch.updated_at_ms);
        }
        if let Some(value) = patch.metadata.get("side_effect_state") {
            node.flags.side_effect_state = value.clone();
        }
        if let Some(value) = patch.metadata.get("budget_state") {
            node.flags.budget_state = value.clone();
        }
        if let Some(value) = patch.metadata.get("trigger_state") {
            node.flags.trigger_state = value.clone();
        }
        for (key, value) in &patch.metadata {
            node.metadata.insert(key.clone(), value.clone());
        }

        snapshot.updated_at_ms = patch.updated_at_ms;
        if snapshot.plan_id.is_empty() {
            snapshot.plan_id = "plan:runtime".into();
        }
        if snapshot.run_id.is_empty() {
            snapshot.run_id = format!("run:{}:active", session_id);
        }

        self.upsert_flow_map_internal(&snapshot).await?;
        self.db
            .upsert_json_knowledge(
                flow_patch_key(
                    &session_id,
                    &patch.flow_id,
                    &patch.node_id,
                    patch.updated_at_ms,
                ),
                patch,
                FLOW_SCOPE,
            )
            .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl FlowStateEnginePort for FlowStateEngine {
    async fn upsert_flow_map(&self, snapshot: &FlowMapSnapshot) -> Result<(), ContractError> {
        self.upsert_flow_map_internal(snapshot)
            .await
            .map_err(|error| ContractError::Storage(error.to_string()))
    }

    async fn apply_patch(&self, patch: &FlowStatePatch) -> Result<(), ContractError> {
        self.apply_patch_internal(patch)
            .await
            .map_err(|error| ContractError::Storage(error.to_string()))
    }

    async fn fetch_flow_map(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<FlowMapSnapshot>, ContractError> {
        self.fetch_flow_map_internal(session_id.as_ref())
            .await
            .map_err(|error| ContractError::Storage(error.to_string()))
    }
}

fn flow_id_for_session(session_id: &str) -> String {
    format!("flow:{}", session_id)
}

fn parse_session_from_flow_id(flow_id: &str) -> Option<String> {
    flow_id.strip_prefix("flow:").map(str::to_string)
}

fn flow_map_key(session_id: &str) -> String {
    format!("flow:map:{}:active", session_id)
}

fn flow_index_key(flow_id: &str) -> String {
    format!("flow:index:{}", flow_id)
}

fn flow_patch_key(session_id: &str, flow_id: &str, node_id: &str, updated_at_ms: u64) -> String {
    format!(
        "flow:patch:{}:{}:{}:{}",
        session_id,
        sanitize_key_part(flow_id),
        sanitize_key_part(node_id),
        updated_at_ms
    )
}

fn node_state_key(session_id: &str, node_id: &str) -> String {
    format!(
        "flow:node:{}:{}:state",
        session_id,
        sanitize_key_part(node_id)
    )
}

fn side_effect_key(session_id: &str, node_id: &str) -> String {
    format!(
        "flow:node:{}:{}:side_effect_state",
        session_id,
        sanitize_key_part(node_id)
    )
}

fn budget_state_key(session_id: &str, node_id: &str) -> String {
    format!(
        "flow:node:{}:{}:budget_state",
        session_id,
        sanitize_key_part(node_id)
    )
}

fn trigger_state_key(session_id: &str, node_id: &str) -> String {
    format!(
        "flow:node:{}:{}:trigger_state",
        session_id,
        sanitize_key_part(node_id)
    )
}

fn sanitize_key_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ':' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
}

fn default_flow_map(session_id: &str) -> FlowMapSnapshot {
    FlowMapSnapshot {
        flow_id: flow_id_for_session(session_id),
        session_id: session_id.to_string(),
        plan_id: "plan:runtime".into(),
        run_id: format!("run:{}:active", session_id),
        nodes: Vec::new(),
        updated_at_ms: current_time_ms(),
    }
}

fn parse_depends_on(raw: Option<&String>) -> Vec<String> {
    raw.map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default()
}

fn is_terminal_state(state: &FlowNodeState) -> bool {
    matches!(
        state,
        FlowNodeState::Succeeded
            | FlowNodeState::Failed
            | FlowNodeState::Blocked
            | FlowNodeState::Skipped
            | FlowNodeState::Degraded
            | FlowNodeState::Replayed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn runtime_updates_maintain_flow_map_and_state_views() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let engine = FlowStateEngine::new(db.clone());

        engine
            .apply_runtime_update(FlowRuntimeUpdate {
                session_id: "session-flow".into(),
                trace_id: "trace-flow".into(),
                task_id: "task-flow".into(),
                capability_id: "mcp::local-mcp::invoke".into(),
                state: Some(FlowNodeState::Pending),
                reason: "dispatch".into(),
                side_effect_state: Some("none".into()),
                budget_state: Some("unknown".into()),
                trigger_state: Some("runtime.dispatch".into()),
                metadata: BTreeMap::new(),
            })
            .await
            .expect("pending update");

        engine
            .set_budget_state(
                "session-flow",
                "trace-flow",
                "task-flow",
                "mcp::local-mcp::invoke",
                "reserved",
                "budget reserve",
            )
            .await
            .expect("budget reserve");

        engine
            .apply_runtime_update(FlowRuntimeUpdate {
                session_id: "session-flow".into(),
                trace_id: "trace-flow".into(),
                task_id: "task-flow".into(),
                capability_id: "mcp::local-mcp::invoke".into(),
                state: Some(FlowNodeState::Succeeded),
                reason: "execution completed".into(),
                side_effect_state: Some("applied".into()),
                budget_state: Some("consumed".into()),
                trigger_state: None,
                metadata: BTreeMap::new(),
            })
            .await
            .expect("succeeded update");

        let map = engine
            .fetch_flow_map_internal("session-flow")
            .await
            .expect("fetch map")
            .expect("map exists");
        assert_eq!(map.nodes.len(), 1);
        assert_eq!(map.nodes[0].state, FlowNodeState::Succeeded);
        assert_eq!(map.nodes[0].flags.side_effect_state, "applied");
        assert_eq!(map.nodes[0].flags.budget_state, "consumed");
        assert_eq!(map.nodes[0].flags.trigger_state, "runtime.dispatch");

        let state_view = db
            .get_knowledge("flow:node:session-flow:task-flow:state")
            .await
            .expect("state view")
            .expect("state view exists");
        assert!(state_view.value.contains("succeeded"));

        let budget_view = db
            .get_knowledge("flow:node:session-flow:task-flow:budget_state")
            .await
            .expect("budget view")
            .expect("budget view exists");
        assert!(budget_view.value.contains("consumed"));
    }
}

