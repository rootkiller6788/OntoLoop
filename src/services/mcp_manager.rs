use std::{collections::BTreeMap, sync::Arc};

use anyhow::Result;
use tokio::sync::RwLock;

use crate::tools::ToolRegistry;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpConnectionState {
    pub server: String,
    pub connected: bool,
    pub last_heartbeat_ms: Option<u64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpResourceDescriptor {
    pub server: String,
    pub resource_id: String,
    pub kind: String,
    pub capability_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpToolAggregate {
    pub server: String,
    pub tool_names: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct McpAggregateSnapshot {
    pub total_servers: usize,
    pub connected_servers: usize,
    pub total_tools: usize,
    pub total_resources: usize,
    pub tool_aggregates: Vec<McpToolAggregate>,
    pub resources: Vec<McpResourceDescriptor>,
    pub connections: Vec<McpConnectionState>,
}

#[derive(Clone, Default)]
pub struct McpManager {
    connections: Arc<RwLock<BTreeMap<String, McpConnectionState>>>,
    resources: Arc<RwLock<BTreeMap<String, McpResourceDescriptor>>>,
}

impl McpManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn upsert_connection(
        &self,
        server: &str,
        connected: bool,
        last_error: Option<String>,
    ) -> Result<McpConnectionState> {
        let normalized = normalize_server(server);
        let state = McpConnectionState {
            server: normalized.clone(),
            connected,
            last_heartbeat_ms: Some(current_time_ms()),
            last_error,
        };
        self.connections
            .write()
            .await
            .insert(normalized, state.clone());
        Ok(state)
    }

    pub async fn register_resource(&self, resource: McpResourceDescriptor) -> Result<()> {
        let key = format!(
            "{}::{}::{}",
            normalize_server(&resource.server),
            resource.kind,
            resource.resource_id
        );
        self.resources.write().await.insert(key, resource);
        Ok(())
    }

    pub async fn aggregate(&self, tools: &ToolRegistry) -> McpAggregateSnapshot {
        let mcp_tools = tools.mcp_tool_names();
        let mut grouped = BTreeMap::<String, Vec<String>>::new();
        for tool_name in mcp_tools {
            let server = parse_mcp_server(&tool_name).unwrap_or_else(|| "unknown".to_string());
            grouped.entry(server).or_default().push(tool_name);
        }

        let mut connections = self.connections.read().await.clone();
        for server in grouped.keys() {
            connections.entry(server.clone()).or_insert(McpConnectionState {
                server: server.clone(),
                connected: true,
                last_heartbeat_ms: None,
                last_error: None,
            });
        }

        let mut tool_aggregates = grouped
            .into_iter()
            .map(|(server, mut tool_names)| {
                tool_names.sort();
                McpToolAggregate { server, tool_names }
            })
            .collect::<Vec<_>>();
        tool_aggregates.sort_by(|a, b| a.server.cmp(&b.server));

        let mut resources = self
            .resources
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        resources.sort_by(|a, b| {
            a.server
                .cmp(&b.server)
                .then(a.kind.cmp(&b.kind))
                .then(a.resource_id.cmp(&b.resource_id))
        });

        let mut connection_rows = connections.into_values().collect::<Vec<_>>();
        connection_rows.sort_by(|a, b| a.server.cmp(&b.server));

        McpAggregateSnapshot {
            total_servers: connection_rows.len(),
            connected_servers: connection_rows.iter().filter(|row| row.connected).count(),
            total_tools: tool_aggregates
                .iter()
                .map(|row| row.tool_names.len())
                .sum::<usize>(),
            total_resources: resources.len(),
            tool_aggregates,
            resources,
            connections: connection_rows,
        }
    }
}

fn parse_mcp_server(tool_name: &str) -> Option<String> {
    let mut parts = tool_name.splitn(4, "::");
    if parts.next() != Some("mcp") {
        return None;
    }
    parts.next().map(normalize_server)
}

fn normalize_server(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        "unknown".to_string()
    } else {
        normalized
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[tokio::test]
    async fn aggregate_includes_connection_and_tool_resource_views() {
        let tools = ToolRegistry::from_config(&AppConfig::default().tools);
        let manager = McpManager::new();
        manager
            .upsert_connection("local-mcp", true, None)
            .await
            .expect("upsert connection");
        manager
            .register_resource(McpResourceDescriptor {
                server: "local-mcp".into(),
                resource_id: "res://catalog".into(),
                kind: "catalog".into(),
                capability_id: Some("mcp::local-mcp::invoke".into()),
            })
            .await
            .expect("register resource");

        let snapshot = manager.aggregate(&tools).await;
        assert!(snapshot.total_servers >= 1);
        assert!(snapshot.total_tools >= 1);
        assert_eq!(snapshot.total_resources, 1);
        assert!(snapshot.connections.iter().any(|row| row.server == "local-mcp"));
    }
}
