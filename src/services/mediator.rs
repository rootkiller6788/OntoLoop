use std::collections::BTreeMap;

use anyhow::Result;
use async_trait::async_trait;
use autoloop_state_adapter::StateStore;
use ontoloop_core::{
    ConstitutionAuditRecord, CorePromotionDecision, CoreRolloutStage, ProductionWriteInput,
    transition_with_audit,
};

use crate::{
    contracts::{
        errors::{ContractError, RuntimeError},
        relation::{RelationContract, RelationEdge, RelationEvent},
        ports::ServiceMediatorPort,
        services::{
            ServiceCall, ServiceDomain, ServiceHealthSnapshot, ServiceResult, SettingsSyncPatch,
            service_call_gate_token, service_gate_token_valid,
        },
    },
    memory::MemorySubsystem,
    observability::ObservabilityKernel,
    plugins::{MarkdownPreprocessPlugin, MarkdownPreprocessRequest, PluginLifecycleManager},
    providers::{ChatMessage, ProviderRegistry},
    security::policy_host::collect_policy_status,
    skills::SkillRegistry,
    tools::ToolRegistry,
};

use super::mcp_manager::{McpManager, McpResourceDescriptor};
use super::relation_facade::RelationFacade;

#[derive(Clone)]
pub struct ServiceMediator {
    providers: ProviderRegistry,
    tools: ToolRegistry,
    plugins: PluginLifecycleManager,
    mcp: McpManager,
    memory: MemorySubsystem,
    relation: RelationFacade,
    skills: SkillRegistry,
    observability: ObservabilityKernel,
    state_store: StateStore,
    policy_bundle_root: std::path::PathBuf,
}

impl ServiceMediator {
    pub fn new(
        providers: ProviderRegistry,
        tools: ToolRegistry,
        plugins: PluginLifecycleManager,
        mcp: McpManager,
        memory: MemorySubsystem,
        skills: SkillRegistry,
        observability: ObservabilityKernel,
        state_store: StateStore,
    ) -> Self {
        Self {
            providers,
            tools,
            plugins,
            mcp,
            memory,
            relation: RelationFacade::new(state_store.clone()),
            skills,
            observability,
            state_store,
            policy_bundle_root: std::path::PathBuf::from("deploy/runtime/policy"),
        }
    }

    pub async fn mediate_call(&self, call: &ServiceCall) -> Result<ServiceResult> {
        self.enforce_gate_token(call)?;
        let started_at_ms = current_time_ms();
        let execution = self.dispatch(call).await;
        let finished_at_ms = current_time_ms();
        let latency_ms = finished_at_ms.saturating_sub(started_at_ms);

        match execution {
            Ok(output) => Ok(ServiceResult {
                service_name: call.service_name.clone(),
                operation: call.operation.clone(),
                success: true,
                output,
                error: None,
                latency_ms,
                cost_micros: None,
                finished_at_ms,
            }),
            Err(error) => Ok(ServiceResult {
                service_name: call.service_name.clone(),
                operation: call.operation.clone(),
                success: false,
                output: serde_json::json!({}),
                error: Some(error.to_string()),
                latency_ms,
                cost_micros: None,
                finished_at_ms,
            }),
        }
    }

    pub async fn health_snapshot(&self) -> Result<Vec<ServiceHealthSnapshot>> {
        let mcp_snapshot = self.mcp.aggregate(&self.tools).await;
        let policy_status = collect_policy_status(&self.state_store, &self.policy_bundle_root)
            .await
            .ok();
        Ok(vec![
            ServiceHealthSnapshot {
                service_name: "provider".into(),
                status: if self.providers.len() > 0 {
                    "healthy".into()
                } else {
                    "degraded".into()
                },
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::from([("registered".into(), self.providers.len().to_string())]),
            },
            ServiceHealthSnapshot {
                service_name: "tool".into(),
                status: if self.tools.len() > 0 {
                    "healthy".into()
                } else {
                    "degraded".into()
                },
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::from([("registered".into(), self.tools.len().to_string())]),
            },
            ServiceHealthSnapshot {
                service_name: "policy".into(),
                status: policy_status
                    .as_ref()
                    .map(|status| status.runtime.status.clone())
                    .unwrap_or_else(|| "degraded".into()),
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::from([
                    (
                        "runtime_mode".into(),
                        policy_status
                            .as_ref()
                            .map(|status| status.runtime.mode.clone())
                            .unwrap_or_else(|| "unknown".into()),
                    ),
                    (
                        "bundle_version".into(),
                        policy_status
                            .as_ref()
                            .and_then(|status| status.bundle.active_policy_version.clone())
                            .unwrap_or_else(|| "unknown".into()),
                    ),
                ]),
            },
            ServiceHealthSnapshot {
                service_name: "plugin".into(),
                status: "healthy".into(),
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::new(),
            },
            ServiceHealthSnapshot {
                service_name: "mcp".into(),
                status: if mcp_snapshot.total_servers == 0 {
                    "degraded".into()
                } else if mcp_snapshot.connected_servers == mcp_snapshot.total_servers {
                    "healthy".into()
                } else {
                    "degraded".into()
                },
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::from([
                    ("servers".into(), mcp_snapshot.total_servers.to_string()),
                    (
                        "connected_servers".into(),
                        mcp_snapshot.connected_servers.to_string(),
                    ),
                    ("tools".into(), mcp_snapshot.total_tools.to_string()),
                    ("resources".into(), mcp_snapshot.total_resources.to_string()),
                ]),
            },
            ServiceHealthSnapshot {
                service_name: "memory".into(),
                status: "healthy".into(),
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::from([(
                    "targets".into(),
                    self.memory.load_targets().len().to_string(),
                )]),
            },
            ServiceHealthSnapshot {
                service_name: "telemetry".into(),
                status: if self.observability.validate().is_ok() {
                    "healthy".into()
                } else {
                    "degraded".into()
                },
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::new(),
            },
            ServiceHealthSnapshot {
                service_name: "settings_sync".into(),
                status: "healthy".into(),
                error_rate: 0.0,
                latency_p95_ms: 0,
                last_incident_at_ms: None,
                metadata: BTreeMap::new(),
            },
        ])
    }

    async fn dispatch(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        match call.service_domain {
            ServiceDomain::Provider => self.handle_provider(call).await,
            ServiceDomain::Tool => self.handle_tool(call).await,
            ServiceDomain::Policy => self.handle_policy(call).await,
            ServiceDomain::Plugin => self.handle_plugin(call).await,
            ServiceDomain::Memory => self.handle_memory(call).await,
            ServiceDomain::Relation => self.handle_relation(call).await,
            ServiceDomain::SkillFoundry => self.handle_skill_foundry(call).await,
            ServiceDomain::Telemetry => self.handle_telemetry(call).await,
            ServiceDomain::SettingsSync => self.handle_settings_sync(call).await,
            ServiceDomain::Research => Ok(serde_json::json!({
                "status": "accepted",
                "note": "research domain is handled by research kernel"
            })),
        }
    }

    fn enforce_gate_token(&self, call: &ServiceCall) -> Result<()> {
        if !call.service_domain.requires_gate_token() {
            return Ok(());
        }
        let Some(token) = service_call_gate_token(&call.input) else {
            anyhow::bail!(
                "service gate token required for domain '{}' but missing field '{}'",
                call.service_domain.gate_scope(),
                crate::contracts::services::SERVICE_GATE_TOKEN_FIELD
            );
        };
        if !service_gate_token_valid(token, &call.session_id, &call.service_domain) {
            anyhow::bail!(
                "service gate token invalid for session '{}' domain '{}'",
                call.session_id,
                call.service_domain.gate_scope()
            );
        }
        Ok(())
    }

    async fn handle_provider(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        let messages = call
            .input
            .get("messages")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        let messages = serde_json::from_value::<Vec<ChatMessage>>(messages).unwrap_or_default();
        let model = call.input.get("model").and_then(serde_json::Value::as_str);
        let response =
            ProviderRegistry::chat_with_policy(&self.providers, &messages, model).await?;
        Ok(serde_json::to_value(response).unwrap_or_else(|_| serde_json::json!({})))
    }

    async fn handle_tool(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        match call.operation.as_str() {
            "mcp_status" => {
                let snapshot = self.mcp.aggregate(&self.tools).await;
                return Ok(serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({})));
            }
            "mcp_upsert_connection" => {
                let server = call
                    .input
                    .get("server")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("local-mcp");
                let connected = call
                    .input
                    .get("connected")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                let last_error = call
                    .input
                    .get("last_error")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let state = self
                    .mcp
                    .upsert_connection(server, connected, last_error)
                    .await?;
                return Ok(serde_json::to_value(state).unwrap_or_else(|_| serde_json::json!({})));
            }
            "mcp_register_resource" => {
                let resource = McpResourceDescriptor {
                    server: call
                        .input
                        .get("server")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("local-mcp")
                        .to_string(),
                    resource_id: call
                        .input
                        .get("resource_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("resource:unknown")
                        .to_string(),
                    kind: call
                        .input
                        .get("kind")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("generic")
                        .to_string(),
                    capability_id: call
                        .input
                        .get("capability_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string),
                };
                self.mcp.register_resource(resource).await?;
                let snapshot = self.mcp.aggregate(&self.tools).await;
                return Ok(serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({})));
            }
            _ => {}
        }

        let tool_name = call
            .input
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(call.service_name.as_str());
        let arguments = call
            .input
            .get("arguments")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("{}");
        let result = ToolRegistry::execute(&self.tools, tool_name, arguments).await?;
        Ok(serde_json::to_value(result).unwrap_or_else(|_| serde_json::json!({})))
    }

    async fn handle_policy(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        let status = collect_policy_status(&self.state_store, &self.policy_bundle_root).await?;
        match call.operation.as_str() {
            "status" | "runtime_status" => {
                Ok(serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({})))
            }
            "runtime" => Ok(
                serde_json::to_value(status.runtime).unwrap_or_else(|_| serde_json::json!({})),
            ),
            "bundle" => Ok(
                serde_json::to_value(status.bundle).unwrap_or_else(|_| serde_json::json!({})),
            ),
            "discovery" => Ok(
                serde_json::to_value(status.discovery).unwrap_or_else(|_| serde_json::json!({})),
            ),
            _ => Ok(serde_json::json!({
                "error": "unsupported policy operation",
                "operation": call.operation,
                "supported": ["status", "runtime_status", "runtime", "bundle", "discovery"],
            })),
        }
    }

    async fn handle_plugin(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        let plugin_id = call
            .input
            .get("plugin_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(call.service_name.as_str());
        let operator = call
            .input
            .get("operator")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("service-mediator");
        let reason = call
            .input
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("service mediation");

        match call.operation.as_str() {
            "install" => {
                let source = call
                    .input
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("builtin://plugin-default#sig=0");
                let requested_by = call
                    .input
                    .get("requested_by")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(operator);
                let tenant_id = call
                    .input
                    .get("tenant_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("tenant:default");
                let verify_signature = call
                    .input
                    .get("verify_signature")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);
                let request = crate::contracts::plugin::PluginInstallRequest {
                    plugin_id: plugin_id.to_string(),
                    source: source.to_string(),
                    requested_by: requested_by.to_string(),
                    tenant_id: tenant_id.to_string(),
                    verify_signature,
                };
                let manifest = self.plugins.install(&request).await?;
                Ok(serde_json::to_value(manifest).unwrap_or_else(|_| serde_json::json!({})))
            }
            "enable" => {
                let manifest = self.plugins.enable(plugin_id, operator, reason).await?;
                Ok(serde_json::to_value(manifest).unwrap_or_else(|_| serde_json::json!({})))
            }
            "disable" => {
                let manifest = self.plugins.disable(plugin_id, operator, reason).await?;
                Ok(serde_json::to_value(manifest).unwrap_or_else(|_| serde_json::json!({})))
            }
            "update" => {
                let source = call.input.get("source").and_then(serde_json::Value::as_str);
                let manifest = self.plugins.update(plugin_id, source, operator).await?;
                Ok(serde_json::to_value(manifest).unwrap_or_else(|_| serde_json::json!({})))
            }
            "rollback" => {
                let manifest = self.plugins.rollback(plugin_id, operator, reason).await?;
                Ok(serde_json::to_value(manifest).unwrap_or_else(|_| serde_json::json!({})))
            }
            "verify" => {
                let verdict = self.plugins.verify(plugin_id).await?;
                Ok(serde_json::to_value(verdict).unwrap_or_else(|_| serde_json::json!({})))
            }
            "status" => {
                let status = self.plugins.status(plugin_id).await?;
                Ok(serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({})))
            }
            "list" => {
                let records = self.plugins.list().await?;
                Ok(serde_json::to_value(records).unwrap_or_else(|_| serde_json::json!([])))
            }
            _ => Ok(serde_json::json!({
                "error": "unsupported plugin operation",
                "operation": call.operation,
            })),
        }
    }

    async fn handle_memory(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        match call.operation.as_str() {
            "ingest_preprocess" => {
                let request = serde_json::from_value::<MarkdownPreprocessRequest>(call.input.clone())
                    .unwrap_or(MarkdownPreprocessRequest {
                        source_path: None,
                        content: call
                            .input
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        declared_format: call
                            .input
                            .get("format")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        options: call
                            .input
                            .get("options")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({})),
                    });
                let result = MarkdownPreprocessPlugin::preprocess(&request)?;
                let provenance_key = format!(
                    "memory:provenance:{}:{}:markdown-preprocess:{}",
                    call.session_id,
                    call.trace_id,
                    current_time_ms()
                );
                self.state_store
                    .upsert_json_knowledge(
                        provenance_key.clone(),
                        &serde_json::json!({
                            "session_id": call.session_id,
                            "trace_id": call.trace_id,
                            "operation": call.operation,
                            "plugin_id": result.plugin_id,
                            "source_path": request.source_path,
                            "source_format": result.source_format,
                            "target_format": result.target_format,
                            "content_digest": result.content_digest,
                            "bytes_in": result.bytes_in,
                            "bytes_out": result.bytes_out,
                            "warnings": result.warnings,
                            "metadata": result.metadata,
                        }),
                        "markdown-preprocess",
                    )
                    .await?;
                self.state_store
                    .upsert_json_knowledge(
                        format!(
                            "memory:provenance:{}:{}:markdown-preprocess:latest",
                            call.session_id, call.trace_id
                        ),
                        &serde_json::json!({
                            "ref": provenance_key,
                            "updated_at_ms": current_time_ms(),
                        }),
                        "markdown-preprocess",
                    )
                    .await?;
                Ok(serde_json::json!({
                    "markdown": result.markdown,
                    "source_format": result.source_format,
                    "target_format": result.target_format,
                    "content_digest": result.content_digest,
                    "bytes_in": result.bytes_in,
                    "bytes_out": result.bytes_out,
                    "warnings": result.warnings,
                    "provenance_ref": provenance_key,
                }))
            }
            "build_context" => {
                let user_input = call
                    .input
                    .get("user_input")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let session_history = call
                    .input
                    .get("session_history")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                let session_history =
                    serde_json::from_value::<Vec<ChatMessage>>(session_history).unwrap_or_default();
                let context = self
                    .memory
                    .build_memory_context_for(user_input, &session_history);
                Ok(serde_json::json!({ "context": context }))
            }
            "strategy_layers" => {
                let layers = self
                    .memory
                    .strategy_memory_layers(&self.state_store, call.session_id.as_ref())
                    .await?;
                Ok(layers)
            }
            _ => Ok(serde_json::json!({
                "error": "unsupported memory operation",
                "operation": call.operation,
            })),
        }
    }

    async fn handle_relation(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        match call.operation.as_str() {
            "upsert_contract" => {
                let contract =
                    serde_json::from_value::<RelationContract>(call.input.clone()).unwrap_or(
                        RelationContract {
                            api_version: crate::contracts::version::RELATION_CONTRACT_VERSION
                                .to_string(),
                            nodes: Vec::new(),
                            edges: Vec::new(),
                            events: Vec::new(),
                        },
                    );
                self.relation
                    .upsert_contract(
                        call.session_id.as_ref(),
                        call.trace_id.as_ref(),
                        &contract,
                        "service-mediator:relation",
                    )
                    .await
            }
            "upsert_edge" => {
                let edge = serde_json::from_value::<RelationEdge>(call.input.clone())?;
                self.relation
                    .upsert_edge(
                        call.session_id.as_ref(),
                        call.trace_id.as_ref(),
                        &edge,
                        "service-mediator:relation",
                    )
                    .await
            }
            "append_event" => {
                let event = serde_json::from_value::<RelationEvent>(call.input.clone())?;
                self.relation
                    .append_event(
                        call.session_id.as_ref(),
                        call.trace_id.as_ref(),
                        &event,
                        "service-mediator:relation",
                    )
                    .await
            }
            "list_edges" => {
                let limit = call
                    .input
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(100) as usize;
                let edges = self
                    .relation
                    .list_edges(call.session_id.as_ref(), limit)
                    .await?;
                Ok(serde_json::json!({
                    "session_id": call.session_id,
                    "count": edges.len(),
                    "edges": edges,
                }))
            }
            "list_events" => {
                let limit = call
                    .input
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(100) as usize;
                let events = self
                    .relation
                    .list_events(call.session_id.as_ref(), limit)
                    .await?;
                Ok(serde_json::json!({
                    "session_id": call.session_id,
                    "count": events.len(),
                    "events": events,
                }))
            }
            "list_hot_index" => {
                let limit = call
                    .input
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(100) as usize;
                let entries = self
                    .relation
                    .list_hot_index(call.session_id.as_ref(), limit)
                    .await?;
                Ok(serde_json::json!({
                    "session_id": call.session_id,
                    "count": entries.len(),
                    "hot_index": entries,
                }))
            }
            _ => Ok(serde_json::json!({
                "error": "unsupported relation operation",
                "operation": call.operation,
                "supported": [
                    "upsert_contract",
                    "upsert_edge",
                    "append_event",
                    "list_edges",
                    "list_events",
                    "list_hot_index"
                ],
            })),
        }
    }

    async fn handle_skill_foundry(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        let now_ms = current_time_ms();
        let builder = call
            .input
            .get("builder")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("foundry-skill");
        let source = call
            .input
            .get("source")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("manual://inline");
        let markdown = call
            .input
            .get("markdown")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("json artifact");
        let operator = call
            .input
            .get("requested_by")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("principal:operator");

        let intake_seed = crate::contracts::skill_foundry::FoundryIntake {
            intake_id: format!("intake:{}:{}", call.session_id, now_ms),
            task_name: builder.to_string(),
            concrete_examples: vec![source.to_string()],
            negative_examples: vec![],
            expected_output: markdown.to_string(),
            existing_software: vec![source.to_string()],
            existing_apis: vec![],
            existing_scripts: vec![],
            requested_by: operator.to_string(),
            session_id: call.session_id.to_string(),
            created_at_ms: now_ms,
        };
        let intake = crate::skills::foundry::normalize_intake(intake_seed);
        let extraction = crate::skills::foundry::extract_first_principles(&intake);
        let suggested_route = crate::skills::foundry::route_layer(&extraction, now_ms);
        let suggestion = crate::skills::foundry::promotion_suggestion(&extraction, &suggested_route, now_ms);

        let layer_state = crate::skills::foundry::load_skill_layer_state(&self.state_store, builder).await?;
        let mut effective_route = suggested_route.clone();
        if let Some(state) = &layer_state {
            effective_route.selected_layer = state.current_layer.clone();
            effective_route
                .policy_notes
                .push(format!("layer_state_override={:?}", state.current_layer));
        }

        let promotion_policy = self
            .state_store
            .get_knowledge("policy:foundry:promotion:latest")
            .await?
            .and_then(|record| {
                serde_json::from_str::<crate::contracts::skill_foundry::FoundryPromotionPolicy>(&record.value).ok()
            })
            .unwrap_or_else(|| crate::skills::foundry::default_promotion_policy(now_ms));

        self.state_store
            .upsert_json_knowledge(
                format!("policy:foundry:promotion:session:{}:latest", call.session_id),
                &promotion_policy,
                "policy-engine",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("metrics:foundry:promotion-policy:{}:{}", call.session_id, now_ms),
                &serde_json::json!({
                    "policy_id": promotion_policy.policy_id,
                    "s1_execution_failure_threshold": promotion_policy.s1_execution_failure_threshold,
                    "s2_boundary_failure_threshold": promotion_policy.s2_boundary_failure_threshold,
                    "max_counted_failures": promotion_policy.max_counted_failures,
                    "manual_approval_required": promotion_policy.manual_approval_required,
                    "created_at_ms": now_ms,
                }),
                "metrics-foundry",
            )
            .await?;

        let mut output = match call.operation.as_str() {
            "intake" => serde_json::json!(intake),
            "extract" => serde_json::json!({
                "intake": intake,
                "extraction": extraction,
            }),
            "route" => serde_json::json!({
                "intake": intake,
                "extraction": extraction,
                "route": effective_route.clone(),
                "route_suggested": suggested_route.clone(),
                "current_layer_state": layer_state,
                "suggestion": suggestion,
                "promotion_policy": promotion_policy,
                "output_file": "route_decision.json",
            }),
            "build" => {
                let build = crate::skills::foundry::build_skill_skeleton(
                    builder,
                    &effective_route,
                    "skills",
                    now_ms,
                )?;
                serde_json::json!({
                    "route": effective_route.clone(),
                    "route_suggested": suggested_route.clone(),
                    "build": build,
                })
            }
            "validate" => {
                let built = crate::skills::foundry::build_skill_skeleton(
                    builder,
                    &effective_route,
                    "skills",
                    now_ms,
                )?;
                let validation = crate::skills::foundry::validate_skill_contract(&built, now_ms);
                serde_json::json!({
                    "package": built,
                    "validation": validation,
                    "route": effective_route.clone(),
                    "route_suggested": suggested_route.clone(),
                    "output_file": "validation_report.json",
                })
            }
            "package" => {
                let packaged = crate::skills::foundry::package_skill(
                    builder,
                    "v1",
                    &effective_route,
                    "skills",
                    now_ms,
                );
                serde_json::json!(packaged)
            }
            "install" => {
                let built = crate::skills::foundry::build_skill_skeleton(
                    builder,
                    &effective_route,
                    "skills",
                    now_ms,
                )?;
                let packaged = crate::skills::foundry::package_skill(
                    &built.skill_name,
                    "v1",
                    &effective_route,
                    &built.artifact_path,
                    now_ms,
                );
                let source_ref = format!("foundry://{}", packaged.artifact_path.replace('\\', "/"));
                let skill_markdown =
                    std::fs::read_to_string(format!("{}/SKILL.md", packaged.artifact_path))
                        .unwrap_or_else(|_| "# skill\nfoundry generated".to_string());
                let learning_signal = crate::memory::LearningSignal {
                    signal_id: format!("foundry-signal:{}:{}", builder, now_ms),
                    session_id: call.session_id.to_string(),
                    trace_id: call.trace_id.to_string(),
                    source: "services.mediator.foundry.install".to_string(),
                    evidence_ref: format!(
                        "evidence:tag:{}:foundry-install:{}",
                        call.session_id, now_ms
                    ),
                    metadata: std::collections::BTreeMap::from([
                        ("operation".to_string(), "install".to_string()),
                        ("skill_name".to_string(), builder.to_string()),
                    ]),
                };
                let manifest = self
                    .skills
                    .install_from_package(&packaged, &source_ref, &skill_markdown, &learning_signal)
                    .await?;
                serde_json::json!({
                    "package": packaged,
                    "install": manifest,
                })
            }
            "enable" => {
                let learning_signal = crate::memory::LearningSignal {
                    signal_id: format!("foundry-signal:{}:{}", builder, now_ms),
                    session_id: call.session_id.to_string(),
                    trace_id: call.trace_id.to_string(),
                    source: "services.mediator.foundry.enable".to_string(),
                    evidence_ref: format!(
                        "evidence:tag:{}:foundry-enable:{}",
                        call.session_id, now_ms
                    ),
                    metadata: std::collections::BTreeMap::from([
                        ("operation".to_string(), "enable".to_string()),
                        ("skill_name".to_string(), builder.to_string()),
                    ]),
                };
                let manifest = self
                    .skills
                    .set_enabled(builder, true, "foundry enable", &learning_signal)
                    .await?;
                serde_json::json!(manifest)
            }
            "disable" => {
                let learning_signal = crate::memory::LearningSignal {
                    signal_id: format!("foundry-signal:{}:{}", builder, now_ms),
                    session_id: call.session_id.to_string(),
                    trace_id: call.trace_id.to_string(),
                    source: "services.mediator.foundry.disable".to_string(),
                    evidence_ref: format!(
                        "evidence:tag:{}:foundry-disable:{}",
                        call.session_id, now_ms
                    ),
                    metadata: std::collections::BTreeMap::from([
                        ("operation".to_string(), "disable".to_string()),
                        ("skill_name".to_string(), builder.to_string()),
                    ]),
                };
                let manifest = self
                    .skills
                    .set_enabled(builder, false, "foundry disable", &learning_signal)
                    .await?;
                serde_json::json!(manifest)
            }
            "approve_promotion" => {
                let hint_id = call
                    .input
                    .get("hint_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let pending = self
                    .state_store
                    .get_knowledge(&format!(
                        "foundry:promotion:pending:{}:{}:latest",
                        call.session_id, builder
                    ))
                    .await?
                    .and_then(|record| {
                        serde_json::from_str::<crate::contracts::skill_foundry::PromotionHint>(&record.value).ok()
                    });
                if pending.is_none() {
                    serde_json::json!({
                        "error": "no pending promotion hint found",
                        "skill_name": builder,
                    })
                } else {
                    let hint = pending.expect("pending checked");
                    if !hint_id.is_empty() && hint.hint_id != hint_id {
                        serde_json::json!({
                            "error": "hint_id mismatch",
                            "expected": hint.hint_id,
                            "provided": hint_id,
                        })
                    } else {
                    let write_gate = self
                        .build_skill_promotion_gate(
                            call.session_id.as_ref(),
                            call.trace_id.as_ref(),
                            true,
                            "skill_foundry.approve_promotion",
                        )
                        .await?;
                    if !write_gate
                        .get("production_write_allowed")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                        || !self.skill_promotion_triad_present(&write_gate)
                    {
                        let deny_reason = if !write_gate
                            .get("production_write_allowed")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false)
                        {
                            write_gate.get("deny_reason").cloned().unwrap_or(serde_json::json!("production_write_gate_denied"))
                        } else {
                            serde_json::json!("missing_gate_triad")
                        };
                        serde_json::json!({
                            "status": "blocked",
                            "error": "production_write_gate_denied",
                            "board_decision": write_gate.get("board_decision").cloned(),
                            "policy_allow": write_gate.get("policy_allow").cloned(),
                            "evidence_ref": write_gate.get("evidence_ref").cloned(),
                            "deny_reason": deny_reason,
                            "gate": write_gate,
                            "skill_name": builder,
                        })
                    } else {
                    let previous_layer = layer_state
                        .as_ref()
                        .map(|state| state.current_layer.clone())
                        .unwrap_or(suggested_route.selected_layer.clone());
                    let layer_update = crate::contracts::skill_foundry::FoundrySkillLayerState {
                        skill_name: builder.to_string(),
                        current_layer: hint.to_layer.clone(),
                        approved_hint_id: Some(hint.hint_id.clone()),
                        reason: "promotion_approved".to_string(),
                        updated_at_ms: now_ms,
                        board_decision: write_gate
                            .get("board_decision")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        policy_allow: write_gate
                            .get("policy_allow")
                            .and_then(serde_json::Value::as_bool),
                        evidence_ref: write_gate
                            .get("evidence_ref")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                        deny_reason: write_gate
                            .get("deny_reason")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_string),
                    };
                    crate::skills::foundry::persist_skill_layer_state(
                        &self.state_store,
                        call.session_id.as_ref(),
                        call.trace_id.as_ref(),
                        &layer_update,
                    )
                    .await?;
                    let approval = serde_json::json!({
                        "hint_id": hint.hint_id,
                        "skill_name": builder,
                        "approved_by": operator,
                        "approved_at_ms": now_ms,
                        "status": "approved",
                        "manual_approval_required": promotion_policy.manual_approval_required,
                        "from_layer": previous_layer,
                        "to_layer": layer_update.current_layer,
                        "board_decision": layer_update.board_decision,
                        "policy_allow": layer_update.policy_allow,
                        "evidence_ref": layer_update.evidence_ref,
                        "deny_reason": layer_update.deny_reason,
                        "gate": write_gate,
                    });
                    self.state_store
                        .upsert_json_knowledge(
                            format!(
                                "foundry:promotion:approval:{}:{}:{}",
                                call.session_id, builder, now_ms
                            ),
                            &approval,
                            "foundry-promotion",
                        )
                        .await?;
                    approval
                    }
                    }
                }
            }
            _ => serde_json::json!({
                "error": "unsupported foundry operation",
                "operation": call.operation,
            }),
        };
        let feedback_kind =
            crate::skills::foundry::classify_feedback_kind(&call.operation, Some(&output), None);
        let feedback_message = output
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("foundry operation completed");
        let _feedback = crate::skills::foundry::persist_feedback_event(
            &self.state_store,
            call.session_id.as_ref(),
            call.trace_id.as_ref(),
            &call.operation,
            feedback_kind,
            feedback_message,
            output.clone(),
            now_ms,
        )
        .await?;
        let promotion_hint = crate::skills::foundry::evaluate_promotion_gate(
            &self.state_store,
            call.session_id.as_ref(),
            call.trace_id.as_ref(),
            builder,
            effective_route.selected_layer.clone(),
            &promotion_policy,
            now_ms,
        )
        .await?;
        if let Some(hint) = promotion_hint {
            let gate_payload = serde_json::json!({
                "status": "pending_human_approval",
                "auto_apply": false,
                "hint": hint,
                "policy_id": promotion_policy.policy_id,
                "manual_approval_required": promotion_policy.manual_approval_required,
            });
            if let Some(object) = output.as_object_mut() {
                object.insert("promotion_gate".to_string(), gate_payload);
            } else {
                output = serde_json::json!({
                    "result": output,
                    "promotion_gate": gate_payload,
                });
            }
        }
        Ok(output)
    }
    async fn handle_telemetry(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        let dashboard_key = format!("observability:{}:dashboard", call.session_id);
        let dashboard = self
            .state_store
            .get_knowledge(&dashboard_key)
            .await?
            .map(|record| record.value)
            .unwrap_or_else(|| "{}".to_string());
        Ok(serde_json::json!({
            "dashboard": serde_json::from_str::<serde_json::Value>(&dashboard).unwrap_or_else(|_| serde_json::json!({})),
            "session_id": call.session_id,
            "operation": call.operation,
        }))
    }

    async fn handle_settings_sync(&self, call: &ServiceCall) -> Result<serde_json::Value> {
        let patch = serde_json::from_value::<SettingsSyncPatch>(call.input.clone()).unwrap_or(
            SettingsSyncPatch {
                tenant_id: "tenant:default".into(),
                scope: "runtime".into(),
                version: "v1".into(),
                payload: call.input.clone(),
            },
        );
        self.state_store
            .upsert_json_knowledge(
                format!(
                    "settings-sync:{}:{}:{}",
                    patch.tenant_id, patch.scope, patch.version
                ),
                &patch,
                "service-mediator",
            )
            .await?;
        Ok(serde_json::to_value(patch).unwrap_or_else(|_| serde_json::json!({})))
    }

    async fn build_skill_promotion_gate(
        &self,
        session_id: &str,
        trace_id: &str,
        policy_allow: bool,
        scope: &str,
    ) -> Result<serde_json::Value> {
        let now_ms = current_time_ms();
        let evidence_ref = format!(
            "evidence:skill-promotion:write-gate:{session_id}:{trace_id}:{scope}:{now_ms}"
        );
        let audit = ConstitutionAuditRecord {
            evidence_ref: evidence_ref.clone(),
            audit_source: "skill-promotion-write-gate".to_string(),
            policy_allow,
            policy_version: crate::contracts::version::CONTRACT_VERSION.to_string(),
            decision_hash: format!("decision:{session_id}:{trace_id}:{scope}:{now_ms}"),
        };
        let state = transition_with_audit(
            None,
            &ProductionWriteInput {
                board_decision: CorePromotionDecision::PromoteTemplate,
                rollout_stage: CoreRolloutStage::Full,
            },
            &audit,
        );
        let decision = state.decision();
        let gate = serde_json::json!({
            "board_decision": format!("{:?}", decision.board_decision),
            "policy_allow": decision.policy_allow,
            "evidence_ref": decision.evidence_ref,
            "deny_reason": decision.deny_reason,
            "production_write_allowed": decision.production_write_allowed,
            "rollout_stage": format!("{:?}", decision.rollout_stage),
            "constitution_state_hash": state.state_hash(),
        });
        self.state_store
            .upsert_json_knowledge(
                evidence_ref,
                &serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "scope": scope,
                    "gate": gate,
                    "audit": audit,
                    "created_at_ms": now_ms,
                }),
                "skill-promotion-write-gate",
            )
            .await?;
        Ok(gate)
    }

    fn skill_promotion_triad_present(&self, gate: &serde_json::Value) -> bool {
        let has_board_decision = gate
            .get("board_decision")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        let policy_allow = gate
            .get("policy_allow")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let has_evidence_ref = gate
            .get("evidence_ref")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.trim().is_empty());
        has_board_decision && policy_allow && has_evidence_ref
    }
}

#[async_trait]
impl ServiceMediatorPort for ServiceMediator {
    async fn mediate(&self, call: &ServiceCall) -> Result<ServiceResult, ContractError> {
        self.mediate_call(call).await.map_err(|error| {
            ContractError::Runtime(RuntimeError {
                code: "service_mediation_failed".into(),
                message: error.to_string(),
            })
        })
    }

    async fn health(&self) -> Result<Vec<ServiceHealthSnapshot>, ContractError> {
        self.health_snapshot().await.map_err(|error| {
            ContractError::Runtime(RuntimeError {
                code: "service_health_failed".into(),
                message: error.to_string(),
            })
        })
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}




