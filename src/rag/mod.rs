mod graph_core;
mod model;
mod org_knowledge;
mod org_sharing_gate;
mod retrieval;

pub use graph_core::{GraphModule, ReducerContext, heuristic_extract_chunk_graph, normalize_key};
pub use org_knowledge::{
    OrgKnowledgePublisher, OrgKnowledgeSnapshot, SharedKnowledgePortAdapter, SharedKnowledgeUpdate,
};
pub use org_sharing_gate::{
    OrgSharingGateDecision, OrgSharingGateInput, evaluate as evaluate_org_sharing_gate,
};

pub use model::{
    CommunityRecord, DatabaseSnapshot, DocumentRecord, DocumentStatus, EntityRecord, QueryContext,
    QueryMode, RelationshipRecord,
};

use crate::tools::ForgedMcpToolManifest;
use anyhow::{Result, bail};
use serde::Serialize;

use crate::{
    config::RagConfig,
    orchestration::{ExecutionReport, SwarmTask, ValidationReport},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagStrategy {
    GraphRag,
    FirstPrinciplesExtraction,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphKnowledgeUpdate {
    pub document_id: u64,
    pub local_context_summary: String,
    pub global_context_summary: String,
    pub task_capability_map_summary: String,
    pub snapshot_json: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GraphRoutingSignals {
    pub document_count: usize,
    pub entity_count: usize,
    pub relationship_count: usize,
    pub community_count: usize,
    pub top_entities: Vec<String>,
    pub capability_surfaces: Vec<String>,
    pub forged_capability_count: usize,
    pub has_dense_graph: bool,
    pub needs_more_extraction: bool,
    pub prefers_mcp_execution: bool,
    pub prefers_cli_execution: bool,
    pub strategy_warm_count: usize,
    pub strategy_cold_count: usize,
}

#[derive(Debug, Clone)]
pub struct RagSubsystem {
    strategies: Vec<RagStrategy>,
}

impl RagSubsystem {
    pub fn from_config(config: &RagConfig) -> Self {
        let mut strategies = Vec::new();
        if config.enable_graph_rag {
            strategies.push(RagStrategy::GraphRag);
        }
        if config.enable_first_principles_extraction {
            strategies.push(RagStrategy::FirstPrinciplesExtraction);
        }
        Self { strategies }
    }

    pub fn validate(&self) -> Result<()> {
        if self.strategies.is_empty() {
            bail!("at least one rag strategy should be enabled");
        }
        Ok(())
    }

    pub fn strategies(&self) -> &[RagStrategy] {
        &self.strategies
    }

    pub fn build_knowledge_update(
        &self,
        session_id: &str,
        request: &str,
        ceo_summary: &str,
        tasks: &[SwarmTask],
        execution_reports: &[ExecutionReport],
        validation: &ValidationReport,
    ) -> GraphKnowledgeUpdate {
        let mut module = GraphModule::default();
        let ctx = ReducerContext {
            caller: "autoloop-rag".into(),
            timestamp_ms: 0,
        };
        let corpus = format!(
            "Session: {session_id}\nRequest: {request}\nCEO: {ceo_summary}\nTasks: {}\nReports: {}\nValidation: {}",
            serde_json::to_string(tasks).unwrap_or_default(),
            serde_json::to_string(execution_reports).unwrap_or_default(),
            validation.summary
        );
        let ingest = module.ingest_document_with_heuristics(
            &ctx,
            format!("session-{session_id}-knowledge"),
            format!("state_store://knowledge/{session_id}"),
            corpus,
            64,
            12,
        );

        let local = module.joint_query_context(ingest.document_id, request, 4, 6, 3);
        let global = module.global_query_context(ingest.document_id, ceo_summary, 3);
        let snapshot_json =
            serde_json::to_string(&module.snapshot()).unwrap_or_else(|_| "{}".into());

        GraphKnowledgeUpdate {
            document_id: ingest.document_id,
            local_context_summary: local.summary,
            global_context_summary: global.summary,
            task_capability_map_summary: summarize_task_capability_map(tasks, execution_reports),
            snapshot_json,
        }
    }

    pub fn graph_routing_signals(&self, snapshot_json: &str) -> GraphRoutingSignals {
        let snapshot = match serde_json::from_str::<DatabaseSnapshot>(snapshot_json) {
            Ok(snapshot) => snapshot,
            Err(_) => return GraphRoutingSignals::default(),
        };

        let entity_count = snapshot.entities.len();
        let relationship_count = snapshot.relationships.len();
        let community_count = snapshot.communities.len();
        let top_entities = snapshot
            .entities
            .iter()
            .take(12)
            .map(|entity| entity.canonical_name.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let prefers_mcp_execution = top_entities
            .iter()
            .any(|name| name.contains("mcp") || name.contains("api") || name.contains("server"));
        let prefers_cli_execution = top_entities.iter().any(|name| {
            name.contains("cli")
                || name.contains("shell")
                || name.contains("terminal")
                || name.contains("file")
        });
        let capability_surfaces = snapshot
            .entities
            .iter()
            .filter(|entity| {
                entity.entity_type == "Capability"
                    || entity.normalized_name.contains("mcp")
                    || entity
                        .description
                        .to_ascii_lowercase()
                        .contains("forged capability")
            })
            .map(|entity| entity.canonical_name.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let forged_capability_count = capability_surfaces.len();

        GraphRoutingSignals {
            document_count: snapshot.documents.len(),
            entity_count,
            relationship_count,
            community_count,
            top_entities,
            capability_surfaces,
            forged_capability_count,
            has_dense_graph: entity_count >= 6 && relationship_count >= 4,
            needs_more_extraction: entity_count < 4 || relationship_count < 3,
            prefers_mcp_execution,
            prefers_cli_execution,
            strategy_warm_count: 0,
            strategy_cold_count: 0,
        }
    }

    pub fn apply_strategy_memory_layers(
        &self,
        signals: &mut GraphRoutingSignals,
        strategy_layers: &serde_json::Value,
    ) {
        let warm = strategy_layers
            .get("warm_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        let cold = strategy_layers
            .get("cold_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize;
        signals.strategy_warm_count = warm;
        signals.strategy_cold_count = cold;
        if warm >= 3 {
            signals.prefers_mcp_execution = true;
        }
        if cold >= warm.saturating_add(2) {
            signals.needs_more_extraction = true;
        }
    }

    pub fn augment_snapshot_with_forged_capabilities(
        &self,
        snapshot_json: &str,
        manifests: &[ForgedMcpToolManifest],
    ) -> String {
        let mut snapshot = match serde_json::from_str::<DatabaseSnapshot>(snapshot_json) {
            Ok(snapshot) => snapshot,
            Err(_) => return snapshot_json.to_string(),
        };
        if manifests.is_empty() {
            return snapshot_json.to_string();
        }

        let document_id = snapshot
            .documents
            .iter()
            .map(|doc| doc.id)
            .max()
            .unwrap_or(0)
            + 1;
        let mut next_entity_id = snapshot
            .entities
            .iter()
            .map(|entity| entity.id)
            .max()
            .unwrap_or(0)
            + 1;
        let mut next_relationship_id = snapshot
            .relationships
            .iter()
            .map(|relationship| relationship.id)
            .max()
            .unwrap_or(0)
            + 1;
        let next_community_id = snapshot
            .communities
            .iter()
            .map(|community| community.id)
            .max()
            .unwrap_or(0)
            + 1;

        snapshot.documents.push(DocumentRecord {
            id: document_id,
            title: "forged-capability-catalog".into(),
            source_uri: format!(
                "state_store://knowledge/forged-capabilities/{}",
                manifests.len()
            ),
            raw_text: manifests
                .iter()
                .map(|manifest| format!("{} {}", manifest.registered_tool_name, manifest.purpose))
                .collect::<Vec<_>>()
                .join("\n"),
            status: DocumentStatus::GraphReady,
            created_at_ms: 0,
            chunk_count: 0,
            entity_count: 0,
            relationship_count: 0,
        });

        let orchestrator_entity_id = upsert_entity(
            &mut snapshot.entities,
            &mut next_entity_id,
            document_id,
            "AutoLoop CLI Agent".into(),
            "Agent".into(),
            "Agent that forges and dispatches reusable MCP capability surfaces.".into(),
        );

        let mut community_members = vec![orchestrator_entity_id];
        let mut community_relationships = Vec::new();

        for manifest in manifests {
            let capability_entity_id = upsert_entity(
                &mut snapshot.entities,
                &mut next_entity_id,
                document_id,
                manifest.registered_tool_name.clone(),
                "Capability".into(),
                format!(
                    "Forged capability for {} via {}",
                    manifest.purpose, manifest.server
                ),
            );
            let server_entity_id = upsert_entity(
                &mut snapshot.entities,
                &mut next_entity_id,
                document_id,
                format!("MCP Server {}", manifest.server),
                "Platform".into(),
                format!(
                    "Execution surface for forged capability {}",
                    manifest.server
                ),
            );

            community_members.push(capability_entity_id);
            community_members.push(server_entity_id);

            snapshot.relationships.push(RelationshipRecord {
                id: next_relationship_id,
                document_id,
                source_entity_id: orchestrator_entity_id,
                target_entity_id: capability_entity_id,
                relation_type: "FORGES".into(),
                weight: 12,
                confidence: 95,
                evidence_chunk_ids: Vec::new(),
                description: manifest.purpose.clone(),
            });
            community_relationships.push(next_relationship_id);
            next_relationship_id += 1;

            snapshot.relationships.push(RelationshipRecord {
                id: next_relationship_id,
                document_id,
                source_entity_id: capability_entity_id,
                target_entity_id: server_entity_id,
                relation_type: "DISPATCHES_VIA".into(),
                weight: 12,
                confidence: 95,
                evidence_chunk_ids: Vec::new(),
                description: manifest.command_template.clone(),
            });
            community_relationships.push(next_relationship_id);
            next_relationship_id += 1;
        }

        community_members.sort_unstable();
        community_members.dedup();

        snapshot.communities.push(CommunityRecord {
            id: next_community_id,
            document_id,
            label: "Forged MCP Capabilities".into(),
            member_entity_ids: community_members.clone(),
            relationship_ids: community_relationships.clone(),
            rank: (community_members.len() as u32 * 8) + community_relationships.len() as u32 * 6,
            summary: format!(
                "Forged capability layer with {} reusable MCP surfaces.",
                manifests.len()
            ),
        });

        if let Some(document) = snapshot
            .documents
            .iter_mut()
            .find(|document| document.id == document_id)
        {
            document.entity_count = community_members.len() as u32;
            document.relationship_count = community_relationships.len() as u32;
        }

        serde_json::to_string(&snapshot).unwrap_or_else(|_| snapshot_json.to_string())
    }

    pub fn merge_incremental_snapshot(
        &self,
        existing_snapshot_json: Option<&str>,
        new_snapshot_json: &str,
        tasks: &[SwarmTask],
        manifests: &[ForgedMcpToolManifest],
    ) -> String {
        let Some(existing_snapshot_json) = existing_snapshot_json else {
            return self.augment_snapshot_with_task_capabilities(
                new_snapshot_json,
                tasks,
                manifests,
            );
        };
        let existing = serde_json::from_str::<DatabaseSnapshot>(existing_snapshot_json);
        let incoming = serde_json::from_str::<DatabaseSnapshot>(new_snapshot_json);
        let (Ok(mut existing), Ok(incoming)) = (existing, incoming) else {
            return self.augment_snapshot_with_task_capabilities(
                new_snapshot_json,
                tasks,
                manifests,
            );
        };

        let mut entity_remap = std::collections::HashMap::<u64, u64>::new();

        for document in incoming.documents {
            if !existing
                .documents
                .iter()
                .any(|item| item.source_uri == document.source_uri)
            {
                existing.documents.push(document);
            }
        }
        for entity in incoming.entities {
            if let Some(found) = existing.entities.iter_mut().find(|item| {
                item.normalized_name == entity.normalized_name
                    || semantic_entity_match(item, &entity)
            }) {
                entity_remap.insert(entity.id, found.id);
                found.salience = found.salience.max(entity.salience) + 1;
                found.mention_count += entity.mention_count;
                found.weight = found.weight.max(entity.weight);
                if found.description.len() < entity.description.len()
                    || incoming_is_newer(entity.first_document_id, found.first_document_id)
                {
                    found.description = entity.description;
                }
            } else {
                entity_remap.insert(entity.id, entity.id);
                existing.entities.push(entity);
            }
        }
        for mut relationship in incoming.relationships {
            relationship.source_entity_id = entity_remap
                .get(&relationship.source_entity_id)
                .copied()
                .unwrap_or(relationship.source_entity_id);
            relationship.target_entity_id = entity_remap
                .get(&relationship.target_entity_id)
                .copied()
                .unwrap_or(relationship.target_entity_id);
            if let Some(existing_relationship) = existing.relationships.iter_mut().find(|item| {
                item.source_entity_id == relationship.source_entity_id
                    && item.target_entity_id == relationship.target_entity_id
                    && item.relation_type == relationship.relation_type
            }) {
                existing_relationship.weight = existing_relationship
                    .weight
                    .max(relationship.weight)
                    .saturating_add(1);
                existing_relationship.confidence = existing_relationship
                    .confidence
                    .max(relationship.confidence);
                if existing_relationship.description.len() < relationship.description.len() {
                    existing_relationship.description = relationship.description;
                }
            } else {
                existing.relationships.push(relationship);
            }
        }
        for community in incoming.communities {
            if let Some(found) = existing
                .communities
                .iter_mut()
                .find(|item| item.label == community.label)
            {
                found.rank = found.rank.max(community.rank).saturating_add(2);
                found.summary = if found.summary.len() >= community.summary.len() {
                    found.summary.clone()
                } else {
                    community.summary
                };
            } else {
                existing.communities.push(community);
            }
        }

        let merged_json =
            serde_json::to_string(&existing).unwrap_or_else(|_| new_snapshot_json.to_string());
        self.augment_snapshot_with_task_capabilities(&merged_json, tasks, manifests)
    }

    pub fn augment_snapshot_with_task_capabilities(
        &self,
        snapshot_json: &str,
        tasks: &[SwarmTask],
        manifests: &[ForgedMcpToolManifest],
    ) -> String {
        let mut snapshot = match serde_json::from_str::<DatabaseSnapshot>(snapshot_json) {
            Ok(snapshot) => snapshot,
            Err(_) => return snapshot_json.to_string(),
        };
        let mut next_entity_id = snapshot
            .entities
            .iter()
            .map(|entity| entity.id)
            .max()
            .unwrap_or(0)
            + 1;
        let mut next_relationship_id = snapshot
            .relationships
            .iter()
            .map(|relationship| relationship.id)
            .max()
            .unwrap_or(0)
            + 1;
        let task_document_id = snapshot
            .documents
            .iter()
            .map(|doc| doc.id)
            .max()
            .unwrap_or(0)
            + 1;

        snapshot.documents.push(DocumentRecord {
            id: task_document_id,
            title: "task-capability-map".into(),
            source_uri: format!(
                "state_store://knowledge/task-capability-map/{}",
                tasks.len()
            ),
            raw_text: summarize_task_capability_map(tasks, &[]),
            status: DocumentStatus::GraphReady,
            created_at_ms: 0,
            chunk_count: 0,
            entity_count: 0,
            relationship_count: 0,
        });

        for task in tasks {
            let task_entity_id = upsert_entity(
                &mut snapshot.entities,
                &mut next_entity_id,
                task_document_id,
                format!("Task {}", task.role),
                "Task".into(),
                task.objective.clone(),
            );
            for manifest in manifests {
                if task.role == "Execution"
                    || task
                        .objective
                        .to_ascii_lowercase()
                        .contains(&manifest.capability_name.to_ascii_lowercase())
                {
                    let capability_entity_id = upsert_entity(
                        &mut snapshot.entities,
                        &mut next_entity_id,
                        task_document_id,
                        manifest.registered_tool_name.clone(),
                        "Capability".into(),
                        format!("Capability candidate for {}", task.role),
                    );
                    let duplicate = snapshot.relationships.iter().any(|item| {
                        item.source_entity_id == task_entity_id
                            && item.target_entity_id == capability_entity_id
                            && item.relation_type == "CAN_USE"
                    });
                    if !duplicate {
                        snapshot.relationships.push(RelationshipRecord {
                            id: next_relationship_id,
                            document_id: task_document_id,
                            source_entity_id: task_entity_id,
                            target_entity_id: capability_entity_id,
                            relation_type: "CAN_USE".into(),
                            weight: 10,
                            confidence: 88,
                            evidence_chunk_ids: Vec::new(),
                            description: task.objective.clone(),
                        });
                        next_relationship_id += 1;
                    }
                }
            }
        }

        serde_json::to_string(&snapshot).unwrap_or_else(|_| snapshot_json.to_string())
    }
}

fn summarize_task_capability_map(
    tasks: &[SwarmTask],
    execution_reports: &[ExecutionReport],
) -> String {
    let task_summary = tasks
        .iter()
        .map(|task| format!("{} -> {}", task.role, task.objective))
        .collect::<Vec<_>>()
        .join("; ");
    let execution_summary = execution_reports
        .iter()
        .filter_map(|report| {
            report
                .tool_used
                .as_ref()
                .map(|tool| format!("{} used {}", report.task.role, tool))
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("tasks: {task_summary}; execution: {execution_summary}")
}

fn semantic_entity_match(left: &EntityRecord, right: &EntityRecord) -> bool {
    if left.entity_type != right.entity_type {
        return false;
    }
    let left_terms = left
        .normalized_name
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    let right_terms = right
        .normalized_name
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    let overlap = left_terms.intersection(&right_terms).count();
    overlap >= 2
        || left.normalized_name.contains(&right.normalized_name)
        || right.normalized_name.contains(&left.normalized_name)
}

fn incoming_is_newer(incoming_document_id: u64, existing_document_id: u64) -> bool {
    incoming_document_id >= existing_document_id
}

fn upsert_entity(
    entities: &mut Vec<EntityRecord>,
    next_entity_id: &mut u64,
    document_id: u64,
    canonical_name: String,
    entity_type: String,
    description: String,
) -> u64 {
    let normalized_name = normalize_key(&canonical_name);
    if let Some(existing) = entities
        .iter_mut()
        .find(|entity| entity.normalized_name == normalized_name)
    {
        existing.salience += 1;
        existing.mention_count += 1;
        existing.weight += 8;
        return existing.id;
    }

    let id = *next_entity_id;
    *next_entity_id += 1;
    entities.push(EntityRecord {
        id,
        canonical_name,
        normalized_name,
        entity_type,
        description,
        salience: 1,
        mention_count: 1,
        degree: 1,
        weight: 16,
        first_document_id: document_id,
    });
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::AppConfig,
        tools::{CliOutputMode, ForgedMcpToolManifest},
    };

    #[test]
    fn rag_builds_graph_update_from_swarm_outputs() {
        let rag = RagSubsystem::from_config(&AppConfig::default().rag);
        let update = rag.build_knowledge_update(
            "session-1",
            "Build StateStore-native GraphRAG memory",
            "CEO wants a swarm plan with execution and validation",
            &[SwarmTask {
                task_id: "knowledge-graph-build".into(),
                agent_name: "knowledge-agent".into(),
                role: "GraphRAG".into(),
                objective: "Extract entities and relationships".into(),
                depends_on: Vec::new(),
            }],
            &[ExecutionReport {
                task: SwarmTask {
                    task_id: "execution-mcp-tools".into(),
                    agent_name: "execution-agent".into(),
                    role: "Execution".into(),
                    objective: "Run MCP tools".into(),
                    depends_on: Vec::new(),
                },
                output: "StateStore stores schedule events and knowledge snapshots.".into(),
                tool_used: Some("mcp::local-mcp::invoke".into()),
                mcp_server: Some("local-mcp".into()),
                invocation_payload: Some("{\"server\":\"local-mcp\"}".into()),
                outcome_score: 4,
                route_variant: "control".into(),
                control_score: 4,
                treatment_score: 4,
                guard_decision: "Allow".into(),
            }],
            &ValidationReport {
                ready: true,
                summary: "All criteria covered.".into(),
                follow_up_tasks: Vec::new(),
                verifier_summary: "Verifier Pass".into(),
            },
        );

        assert!(update.snapshot_json.contains("relationships"));
        assert!(update.local_context_summary.contains("GraphRAG"));
    }

    #[test]
    fn graph_routing_signals_detect_forged_capability_surfaces() {
        let rag = RagSubsystem::from_config(&AppConfig::default().rag);
        let update = rag.build_knowledge_update(
            "session-capability",
            "Build agent-native MCP execution",
            "CEO wants reusable execution surfaces",
            &[SwarmTask {
                task_id: "execution-use-mcp".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Use MCP".into(),
                depends_on: Vec::new(),
            }],
            &[ExecutionReport {
                task: SwarmTask {
                    task_id: "execution-use-mcp-report".into(),
                    agent_name: "execution-agent".into(),
                    role: "Execution".into(),
                    objective: "Use MCP".into(),
                    depends_on: Vec::new(),
                },
                output: "completed".into(),
                tool_used: Some("mcp::local-mcp::invoke".into()),
                mcp_server: Some("local-mcp".into()),
                invocation_payload: Some("{}".into()),
                outcome_score: 4,
                route_variant: "control".into(),
                control_score: 4,
                treatment_score: 4,
                guard_decision: "Allow".into(),
            }],
            &ValidationReport {
                ready: true,
                summary: "ok".into(),
                follow_up_tasks: Vec::new(),
                verifier_summary: "Verifier Pass".into(),
            },
        );
        let augmented = rag.augment_snapshot_with_forged_capabilities(
            &update.snapshot_json,
            &[ForgedMcpToolManifest {
                registered_tool_name: "mcp::local-mcp::diagram-export".into(),
                delegate_tool_name: "mcp::local-mcp::invoke".into(),
                server: "local-mcp".into(),
                capability_name: "diagram-export".into(),
                purpose: "Export diagrams".into(),
                executable: "diagram-cli".into(),
                command_template: "diagram-cli export --project {{project}}".into(),
                payload_template: serde_json::json!({"server":"local-mcp"}),
                output_mode: CliOutputMode::Json,
                working_directory: Some(".".into()),
                success_signal: Some("completed".into()),
                help_text: "help".into(),
                skill_markdown: "# skill".into(),
                examples: vec!["diagram-cli export".into()],
                ..ForgedMcpToolManifest::default()
            }],
        );
        let signals = rag.graph_routing_signals(&augmented);

        assert!(signals.forged_capability_count >= 1);
        assert!(
            signals
                .capability_surfaces
                .iter()
                .any(|name| name.contains("mcp::local-mcp::diagram-export"))
        );
        assert!(signals.prefers_mcp_execution);
    }

    #[test]
    fn rag_merges_incremental_snapshot_and_task_capability_edges() {
        let rag = RagSubsystem::from_config(&AppConfig::default().rag);
        let base = rag.build_knowledge_update(
            "session-a",
            "Build execution graph",
            "CEO routes execution",
            &[SwarmTask {
                task_id: "execution-catalog-capability".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Use catalog capability".into(),
                depends_on: Vec::new(),
            }],
            &[],
            &ValidationReport {
                ready: true,
                summary: "ok".into(),
                follow_up_tasks: Vec::new(),
                verifier_summary: "pass".into(),
            },
        );
        let merged = rag.merge_incremental_snapshot(
            Some(&base.snapshot_json),
            &base.snapshot_json,
            &[SwarmTask {
                task_id: "execution-catalog-capability-merge".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Use catalog capability".into(),
                depends_on: Vec::new(),
            }],
            &[ForgedMcpToolManifest {
                registered_tool_name: "mcp::local-mcp::catalog-exec".into(),
                delegate_tool_name: "mcp::local-mcp::invoke".into(),
                server: "local-mcp".into(),
                capability_name: "catalog-exec".into(),
                purpose: "Execute via catalog".into(),
                executable: "autoloop-cli".into(),
                command_template: "autoloop-cli task execution --objective {{objective}}".into(),
                payload_template: serde_json::json!({"server":"local-mcp"}),
                output_mode: CliOutputMode::Json,
                working_directory: Some(".".into()),
                success_signal: Some("completed".into()),
                help_text: "help".into(),
                skill_markdown: "# skill".into(),
                examples: vec![],
                ..ForgedMcpToolManifest::default()
            }],
        );

        assert!(merged.contains("CAN_USE"));
        assert!(merged.contains("catalog-exec"));
    }

    #[test]
    fn incremental_merge_strengthens_duplicate_relationships() {
        let rag = RagSubsystem::from_config(&AppConfig::default().rag);
        let base = rag.build_knowledge_update(
            "session-merge-a",
            "Build graph memory",
            "CEO routes execution",
            &[SwarmTask {
                task_id: "execution-persist-memory".into(),
                agent_name: "execution-agent".into(),
                role: "Execution".into(),
                objective: "Persist graph memory".into(),
                depends_on: Vec::new(),
            }],
            &[ExecutionReport {
                task: SwarmTask {
                    task_id: "execution-persist-memory-report".into(),
                    agent_name: "execution-agent".into(),
                    role: "Execution".into(),
                    objective: "Persist graph memory".into(),
                    depends_on: Vec::new(),
                },
                output: "StateStore stores graph memory and execution traces.".into(),
                tool_used: Some("mcp::local-mcp::invoke".into()),
                mcp_server: Some("local-mcp".into()),
                invocation_payload: Some("{}".into()),
                outcome_score: 4,
                route_variant: "control".into(),
                control_score: 4,
                treatment_score: 4,
                guard_decision: "Allow".into(),
            }],
            &ValidationReport {
                ready: true,
                summary: "ok".into(),
                follow_up_tasks: Vec::new(),
                verifier_summary: "pass".into(),
            },
        );
        let merged = rag.merge_incremental_snapshot(
            Some(&base.snapshot_json),
            &base.snapshot_json,
            &[],
            &[],
        );

        assert!(merged.contains("graph memory"));
        assert!(merged.contains("relationships"));
    }
}

