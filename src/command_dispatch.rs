use std::{collections::{BTreeMap, HashMap}, fs, path::{Path, PathBuf}, sync::Arc};

use anyhow::Result;
use autoloop::{
    AutoLoopApp,
    cli_runtime::{
        BuiltinCommandRegistry, FrontendOutputFormat, FrontendStatusView,
        render_session_event_pretty, summarize_event_type_counts,
    },
    config::{AlertThresholds, ConfigProfile},
    observability::event_stream::append_event,
};

pub struct FrontendDispatchInput {
    pub session_id: String,
    pub action: String,
    pub content: Option<String>,
    pub trace_id: Option<String>,
    pub request_id: Option<String>,
    pub decision: Option<String>,
    pub reason: Option<String>,
    pub jwt: Option<String>,
    pub transport_kind: String,
    pub ttl_ms: u64,
    pub subject: Option<String>,
    pub tenant_id: Option<String>,
    pub format: String,
    pub limit: usize,
    pub output: Option<PathBuf>,
}

pub struct IdentityInput {
    pub tenant: Option<String>,
    pub principal: Option<String>,
    pub policy: Option<String>,
    pub lease_ttl_ms: u64,
}

pub struct BackgroundDispatchInput {
    pub session_id: String,
    pub action: String,
    pub task_id: Option<String>,
    pub kind: Option<String>,
    pub command: Option<String>,
    pub prompt: Option<String>,
    pub max_restarts: u32,
    pub lines: usize,
    pub output: Option<PathBuf>,
}

pub struct MemoryDispatchInput {
    pub session_id: String,
    pub plane: String,
    pub action: String,
    pub file: Option<String>,
    pub review_id: Option<String>,
    pub operator: Option<String>,
    pub reason: Option<String>,
    pub repo_root: Option<PathBuf>,
    pub clean: bool,
    pub no_infer: bool,
    pub report: bool,
    pub save: Option<PathBuf>,
    pub refresh_mode: Option<String>,
    pub page: Option<usize>,
    pub page_size: Option<usize>,
    pub output: Option<PathBuf>,
}

pub struct SystemDispatchInput {
    pub session_id: String,
    pub action: String,
    pub subaction: Option<String>,
    pub snapshot_id: Option<String>,
    pub trace_id: Option<String>,
    pub artifact_ref: Option<String>,
    pub artifact_path: Option<PathBuf>,
    pub limit: usize,
    pub profile: Option<String>,
    pub fault: Option<String>,
    pub reason: Option<String>,
    pub task_utility: Option<f32>,
    pub distortion_penalty: Option<f32>,
    pub attention_mismatch_penalty: Option<f32>,
    pub token_cost_penalty: Option<f32>,
    pub anchor_list: Option<PathBuf>,
    pub output: Option<PathBuf>,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RealTaskBenchmarkCase {
    task_id: String,
    mode: String,
    prompt: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RealTaskBenchmarkItemResult {
    task_id: String,
    mode: String,
    mode_overridden: bool,
    session_id: String,
    success: bool,
    retry_count: u32,
    failure_reason: Option<String>,
    trace_id: Option<String>,
    duration_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RealTaskBenchmarkReport {
    session_id: String,
    benchmark_id: String,
    dataset_path: String,
    total: usize,
    passed: usize,
    failed: usize,
    success_rate: f64,
    total_retry_count: u64,
    average_retry_count: f64,
    failure_reason_distribution: BTreeMap<String, usize>,
    results: Vec<RealTaskBenchmarkItemResult>,
    created_at_ms: u64,
    evidence_ref: Option<String>,
}

pub async fn dispatch_frontend(
    app: &Arc<AutoLoopApp>,
    command_registry: &BuiltinCommandRegistry,
    identity: &IdentityInput,
    input: FrontendDispatchInput,
) -> Result<()> {
    super::bind_identity_for_session(
        app,
        identity.tenant.as_deref(),
        identity.principal.as_deref(),
        identity.policy.as_deref(),
        identity.lease_ttl_ms,
        &input.session_id,
    )
    .await?;

    let body = match input.action.as_str() {
        "status" => {
            let events = app.transport.replay_session_events_v2(&input.session_id).await?;
            let latest = events.last();
            let latest_event_type = latest.map(|event| {
                match event.event_type {
                    autoloop::contracts::transport::SessionEventType::Ready => "ready",
                    autoloop::contracts::transport::SessionEventType::StateSnapshot => {
                        "state_snapshot"
                    }
                    autoloop::contracts::transport::SessionEventType::AssistantDelta => {
                        "assistant_delta"
                    }
                    autoloop::contracts::transport::SessionEventType::ToolStarted => {
                        "tool_started"
                    }
                    autoloop::contracts::transport::SessionEventType::ToolCompleted => {
                        "tool_completed"
                    }
                }
                .to_string()
            });
            let latest_emitted_at_ms = latest.map(|event| event.emitted_at_ms);
            let event_type_counts = summarize_event_type_counts(&events);
            let mut observable_snapshot_keys = command_registry.state_store().snapshot_keys().await;
            observable_snapshot_keys.sort();
            let bridge_status_raw = app.bridge_status(&input.session_id).await?;
            let bridge_status = serde_json::from_str::<serde_json::Value>(bridge_status_raw.as_str())
                .unwrap_or_else(|_| serde_json::json!({ "raw": bridge_status_raw }));

            let view = FrontendStatusView {
                session_id: input.session_id.clone(),
                transport_event_count: events.len(),
                latest_event_type,
                latest_emitted_at_ms,
                event_type_counts,
                bridge_status,
                observable_snapshot_keys,
            };
            serde_json::to_string_pretty(&view)?
        }
        "events" => {
            let format = FrontendOutputFormat::parse(&input.format).ok_or_else(|| {
                anyhow::anyhow!(
                    "unsupported frontend format '{}', expected pretty|json",
                    input.format
                )
            })?;
            let events = app.transport.replay_session_events_v2(&input.session_id).await?;
            let effective_limit = input.limit.max(1);
            let start = events.len().saturating_sub(effective_limit);
            let tail = events.into_iter().skip(start).collect::<Vec<_>>();

            match format {
                FrontendOutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
                    "session_id": input.session_id,
                    "limit": effective_limit,
                    "count": tail.len(),
                    "events": tail,
                }))?,
                FrontendOutputFormat::Pretty => {
                    let mut lines = Vec::with_capacity(tail.len() + 1);
                    lines.push(format!(
                        "session={} event_tail={} limit={}",
                        input.session_id,
                        tail.len(),
                        effective_limit
                    ));
                    for event in &tail {
                        lines.push(render_session_event_pretty(event));
                    }
                    lines.join("\n")
                }
            }
        }
        "prompt" => {
            let resolved_trace = input.trace_id.clone().unwrap_or_else(|| {
                format!(
                    "trace:{}:cli:frontend:prompt:{}",
                    input.session_id,
                    autoloop::orchestration::current_time_ms()
                )
            });
            let _ = super::append_cli_query_event(
                app,
                &input.session_id,
                &resolved_trace,
                "cli.frontend.prompt.request",
                serde_json::json!({
                    "action": "prompt",
                    "content_len": input.content.as_ref().map(|c| c.len()).unwrap_or(0),
                }),
            )
            .await;
            let content = input
                .content
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("frontend prompt requires --content <text>"))?;
            let response = app
                .frontend_bridge_prompt(&input.session_id, Some(&resolved_trace), content)
                .await?;
            let parsed = serde_json::from_str::<serde_json::Value>(&response)
                .unwrap_or_else(|_| serde_json::json!({}));
            let _ = super::append_cli_query_event(
                app,
                &input.session_id,
                &resolved_trace,
                "cli.frontend.prompt.result",
                serde_json::json!({
                    "status": parsed.get("status").cloned().unwrap_or(serde_json::Value::Null),
                    "request_id": parsed.get("request_id").cloned().unwrap_or(serde_json::Value::Null),
                    "call_id": parsed.get("call_id").cloned().unwrap_or(serde_json::Value::Null),
                    "event_count": parsed
                        .get("events")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| items.len())
                        .unwrap_or(0),
                }),
            )
            .await;
            response
        }
        "permission" => {
            let resolved_trace = format!(
                "trace:{}:cli:frontend:permission:{}",
                input.session_id,
                autoloop::orchestration::current_time_ms()
            );
            let request_id = input.request_id.as_deref().ok_or_else(|| {
                anyhow::anyhow!("frontend permission requires --request-id <id>")
            })?;
            let decision = input.decision.as_deref().ok_or_else(|| {
                anyhow::anyhow!("frontend permission requires --decision approve|reject")
            })?;
            let _ = super::append_cli_query_event(
                app,
                &input.session_id,
                &resolved_trace,
                "cli.frontend.permission.request",
                serde_json::json!({
                    "action": "permission",
                    "request_id": request_id,
                    "decision": decision,
                }),
            )
            .await;
            let response = app
                .frontend_permission_decide(
                    &input.session_id,
                    request_id,
                    decision,
                    input.reason.as_deref(),
                )
                .await?;
            let parsed = serde_json::from_str::<serde_json::Value>(&response)
                .unwrap_or_else(|_| serde_json::json!({}));
            let _ = super::append_cli_query_event(
                app,
                &input.session_id,
                &resolved_trace,
                "cli.frontend.permission.result",
                serde_json::json!({
                    "status": parsed.get("status").cloned().unwrap_or(serde_json::Value::Null),
                    "request_id": parsed.get("request_id").cloned().unwrap_or(serde_json::Value::Null),
                    "evidence_ref": parsed.get("evidence_ref").cloned().unwrap_or(serde_json::Value::Null),
                }),
            )
            .await;
            response
        }
        "attach" => {
            let resolved_trace = format!(
                "trace:{}:cli:frontend:attach:{}",
                input.session_id,
                autoloop::orchestration::current_time_ms()
            );
            let _ = super::append_cli_query_event(
                app,
                &input.session_id,
                &resolved_trace,
                "cli.frontend.attach.request",
                serde_json::json!({
                    "transport_kind": input.transport_kind,
                    "jwt": input.jwt.is_some(),
                }),
            )
            .await;
            let response = app
                .frontend_attach(
                    &input.session_id,
                    &input.transport_kind,
                    input.jwt.as_deref(),
                    input.subject.as_deref(),
                    input.tenant_id.as_deref(),
                    input.ttl_ms,
                )
                .await?;
            let parsed = serde_json::from_str::<serde_json::Value>(&response)
                .unwrap_or_else(|_| serde_json::json!({}));
            let _ = super::append_cli_query_event(
                app,
                &input.session_id,
                &resolved_trace,
                "cli.frontend.attach.result",
                serde_json::json!({
                    "status": parsed.get("status").cloned().unwrap_or(serde_json::Value::Null),
                    "attach_mode": parsed.get("attach_mode").cloned().unwrap_or(serde_json::Value::Null),
                }),
            )
            .await;
            response
        }
        _ => serde_json::json!({
            "error":"unsupported frontend action",
            "supported": [
                "status",
                "events --format pretty|json --limit <n>",
                "prompt --content <text> [--trace-id <id>]",
                "permission --request-id <id> --decision approve|reject [--reason <text>]",
                "attach [--transport-kind cli|websocket|sse|webhook|sdk] [--jwt <token>] [--subject <name>] [--tenant-id <id>] [--ttl-ms <ms>]"
            ]
        })
        .to_string(),
    };

    super::write_or_print(input.output.as_ref(), &body)
}

pub async fn dispatch_chat(
    app: &Arc<AutoLoopApp>,
    command_registry: &BuiltinCommandRegistry,
    identity: &IdentityInput,
    session_id: &str,
) -> Result<()> {
    super::bind_identity_for_session(
        app,
        identity.tenant.as_deref(),
        identity.principal.as_deref(),
        identity.policy.as_deref(),
        identity.lease_ttl_ms,
        session_id,
    )
    .await?;
    super::run_chat_repl(app, session_id, command_registry).await
}

pub async fn dispatch_background(
    app: &Arc<AutoLoopApp>,
    identity: &IdentityInput,
    input: BackgroundDispatchInput,
) -> Result<()> {
    super::bind_identity_for_session(
        app,
        identity.tenant.as_deref(),
        identity.principal.as_deref(),
        identity.policy.as_deref(),
        identity.lease_ttl_ms,
        &input.session_id,
    )
    .await?;
    let body = match input.action.as_str() {
        "start" => {
            let task_id = input
                .task_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("background start requires --task-id"))?;
            let kind = input
                .kind
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("background start requires --kind shell|agent"))?
                .to_ascii_lowercase();
            match kind.as_str() {
                "shell" => {
                    let command = input.command.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("background start --kind shell requires --command")
                    })?;
                    app.background_task_start_shell(
                        &input.session_id,
                        task_id,
                        command,
                        input.max_restarts,
                    )
                    .await?
                }
                "agent" => {
                    let prompt = input
                        .prompt
                        .as_deref()
                        .or(input.command.as_deref())
                        .ok_or_else(|| {
                            anyhow::anyhow!("background start --kind agent requires --prompt")
                        })?;
                    app.background_task_start_agent(
                        &input.session_id,
                        task_id,
                        prompt,
                        input.max_restarts,
                    )
                    .await?
                }
                _ => serde_json::json!({
                    "error": "unsupported background kind",
                    "supported": ["shell", "agent"],
                })
                .to_string(),
            }
        }
        "status" => app
            .background_task_status(&input.session_id, input.task_id.as_deref())
            .await?,
        "stop" => {
            let task_id = input
                .task_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("background stop requires --task-id"))?;
            app.background_task_stop(&input.session_id, task_id).await?
        }
        "restart" => {
            let task_id = input
                .task_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("background restart requires --task-id"))?;
            app.background_task_restart(&input.session_id, task_id).await?
        }
        "logs" | "tail" => {
            let task_id = input
                .task_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("background logs requires --task-id"))?;
            app.background_task_logs(&input.session_id, task_id, input.lines)
                .await?
        }
        _ => serde_json::json!({
            "error": "unsupported background action",
            "supported": [
                "start --task-id <id> --kind shell --command <cmd>",
                "start --task-id <id> --kind agent --prompt <prompt>",
                "status [--task-id <id>]",
                "stop --task-id <id>",
                "restart --task-id <id>",
                "logs --task-id <id> [--lines <n>]"
            ]
        })
        .to_string(),
    };
    super::write_or_print(input.output.as_ref(), &body)
}

pub async fn dispatch_memory(
    app: &Arc<AutoLoopApp>,
    identity: &IdentityInput,
    input: MemoryDispatchInput,
) -> Result<()> {
    super::bind_identity_for_session(
        app,
        identity.tenant.as_deref(),
        identity.principal.as_deref(),
        identity.policy.as_deref(),
        identity.lease_ttl_ms,
        &input.session_id,
    )
    .await?;
    let normalized_plane = input.plane.to_ascii_lowercase();
    let normalized_action = input.action.to_ascii_lowercase();
    let body = match (normalized_plane.as_str(), normalized_action.as_str()) {
        ("schema", "registry") | ("schema", "status") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let snapshot = super::SchemaRegistry::load(root)?;
            serde_json::to_string_pretty(&serde_json::json!({
                "repo_root": root.display().to_string(),
                "schema_registry": snapshot,
            }))?
        }
        ("patch", "queue")
        | ("patch", "list")
        | ("patch-review", "queue")
        | ("patch-review", "list") => {
            let items = super::PatchReviewQueue::list(&app.state_store(), &input.session_id).await?;
            serde_json::to_string_pretty(&serde_json::json!({
                "session_id": input.session_id,
                "count": items.len(),
                "items": items,
            }))?
        }
        ("patch", "approve") | ("patch-review", "approve") => {
            let review_id = input
                .review_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("memory patch approve requires --review-id"))?;
            let reviewer = input
                .operator
                .as_deref()
                .or(identity.principal.as_deref())
                .unwrap_or("principal:operator");
            let item = super::PatchReviewQueue::approve(
                &app.state_store(),
                &input.session_id,
                review_id,
                reviewer,
                input.reason.as_deref().unwrap_or("approved by operator"),
            )
            .await?;
            serde_json::to_string_pretty(&item)?
        }
        ("patch", "reject") | ("patch-review", "reject") => {
            let review_id = input
                .review_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("memory patch reject requires --review-id"))?;
            let reviewer = input
                .operator
                .as_deref()
                .or(identity.principal.as_deref())
                .unwrap_or("principal:operator");
            let item = super::PatchReviewQueue::reject(
                &app.state_store(),
                &input.session_id,
                review_id,
                reviewer,
                input.reason.as_deref().unwrap_or("rejected by operator"),
            )
            .await?;
            serde_json::to_string_pretty(&item)?
        }
        ("recall", "route") | ("recall", "status") => {
            let lease = app.state_store().get_session_lease(&input.session_id).await?;
            let tenant_id = lease
                .as_ref()
                .map(|item| item.tenant_id.as_str())
                .unwrap_or_else(|| identity.tenant.as_deref().unwrap_or("tenant:default"));
            let key = format!("memory:recall:route:{}:latest", input.session_id);
            let value = if let Some(record) = app.state_store().get_knowledge(&key).await? {
                serde_json::from_str::<serde_json::Value>(&record.value)
                    .unwrap_or_else(|_| serde_json::json!({"raw": record.value}))
            } else {
                let route = super::RecallPluginRouter::route(
                    &app.state_store(),
                    &input.session_id,
                    tenant_id,
                    "recall route status probe",
                )
                .await?;
                serde_json::json!({
                    "session_id": input.session_id,
                    "tenant_id": tenant_id,
                    "route": route,
                })
            };
            serde_json::to_string_pretty(&value)?
        }
        ("compiler", "status") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let compiler = super::compiler_status_view(app, &input.session_id, root).await?;
            serde_json::to_string_pretty(&compiler)?
        }
        ("compiler", "explain") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let target_file = input.file.as_deref().ok_or_else(|| {
                anyhow::anyhow!("memory compiler explain requires --file <relative/path.md>")
            })?;
            let explain =
                super::compiler_explain_view(app, &input.session_id, root, target_file).await?;
            serde_json::to_string_pretty(&explain)?
        }
        ("compiler", "graph") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let target_file = input.file.as_deref().ok_or_else(|| {
                anyhow::anyhow!("memory compiler graph requires --file <relative/path.md>")
            })?;
            let graph = super::compiler_graph_view(root, target_file)?;
            serde_json::to_string_pretty(&graph)?
        }
        ("graph", "export") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let artifact = super::ViewPlane::export_offline_graph(
                root,
                super::GraphExportOptions {
                    clean: input.clean,
                    no_infer: input.no_infer,
                    report: input.report,
                    save: input.save.clone(),
                },
            )?;
            serde_json::to_string_pretty(&artifact)?
        }
        ("wiki", "graph-export") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let artifact = super::ViewPlane::export_offline_graph(
                root,
                super::GraphExportOptions {
                    clean: input.clean,
                    no_infer: input.no_infer,
                    report: input.report,
                    save: input.save.clone(),
                },
            )?;
            serde_json::to_string_pretty(&serde_json::json!({
                "plane": "wiki",
                "action": "graph-export",
                "result": artifact,
            }))?
        }
        ("wiki", "lint-semantic") => {
            let view = autoloop::observability::query_plane::build_unified_query_view(
                &app.state_store(),
                &input.session_id,
                None,
            )
            .await?;
            let semantic_lint_report = view
                .replay
                .get("semantic_lint_report")
                .cloned()
                .unwrap_or_else(|| {
                    serde_json::json!({
                        "status": "unavailable",
                        "reason": "semantic lint report not found in unified query view"
                    })
                });
            serde_json::to_string_pretty(&serde_json::json!({
                "plane": "wiki",
                "action": "lint-semantic",
                "session_id": input.session_id,
                "semantic_lint_report": semantic_lint_report,
                "graph_health": view.graph.get("graph_health").cloned().unwrap_or(serde_json::json!({})),
            }))?
        }
        ("wiki", "refresh") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let requested_files = input
                .file
                .as_deref()
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mode = super::parse_refresh_plan_mode(input.refresh_mode.as_deref());
            let plan = super::HotIndexUpdater::plan_refresh_with_options(
                root,
                &requested_files,
                mode,
                input.page,
                input.page_size,
            )?;
            serde_json::to_string_pretty(&serde_json::json!({
                "plane": "wiki",
                "action": "refresh",
                "repo_root": root.display().to_string(),
                "plan": plan,
            }))?
        }
        ("wiki", "validate-only") => {
            let root = input
                .repo_root
                .as_deref()
                .unwrap_or_else(|| std::path::Path::new("D:\\AutoLoop\\autoloop-app"));
            let target_file = input.file.as_deref().ok_or_else(|| {
                anyhow::anyhow!("memory wiki validate-only requires --file <relative/path.md>")
            })?;
            let normalized = super::normalize_compiler_target(target_file);
            let source_path = root.join(&normalized);
            let content = std::fs::read_to_string(&source_path).map_err(|error| {
                anyhow::anyhow!(
                    "failed to read source file {}: {}",
                    source_path.display(),
                    error
                )
            })?;
            let validation = super::IngestValidator::validate(
                root,
                &normalized,
                &content,
                super::IngestValidationMode::ValidateOnly,
            )?;
            serde_json::to_string_pretty(&serde_json::json!({
                "plane": "wiki",
                "action": "validate-only",
                "repo_root": root.display().to_string(),
                "validation": validation,
            }))?
        }
        _ => serde_json::json!({
            "error": "unsupported memory plane/action",
            "supported": [
                "schema registry",
                "schema status",
                "patch queue",
                "patch list",
                "patch approve --review-id <id>",
                "patch reject --review-id <id>",
                "recall route",
                "recall status",
                "compiler status",
                "compiler explain --file <path>",
                "compiler graph --file <path>",
                "graph export [--repo-root <path>] [--clean] [--no-infer] [--report] [--save <path>]",
                "wiki graph-export [--repo-root <path>] [--clean] [--no-infer] [--report] [--save <path>]",
                "wiki lint-semantic",
                "wiki refresh [--file <path1,path2>] [--refresh-mode detect|dry-run|force|page] [--page <n>] [--page-size <n>]",
                "wiki validate-only --file <path>"
            ]
        })
        .to_string(),
    };
    super::write_or_print(input.output.as_ref(), &body)
}

fn resolve_profile_template_path(profile: ConfigProfile) -> PathBuf {
    let path = profile.config_path();
    if path.exists() {
        return path;
    }
    let nested = PathBuf::from("autoloop-app").join(path.clone());
    if nested.exists() {
        return nested;
    }
    path
}

fn load_profile_template(profile: ConfigProfile) -> serde_json::Value {
    let path = resolve_profile_template_path(profile);
    let exists = path.exists();
    let rendered = if exists {
        fs::read_to_string(&path).ok()
    } else {
        None
    };
    serde_json::json!({
        "profile": match profile {
            ConfigProfile::Local => "local",
            ConfigProfile::Production => "production",
        },
        "path": path.display().to_string(),
        "exists": exists,
        "template_preview": rendered
            .as_deref()
            .map(|content| content.lines().take(30).collect::<Vec<_>>().join("\n")),
    })
}

fn as_f64(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(|item| {
        item.as_f64()
            .or_else(|| item.as_u64().map(|number| number as f64))
            .or_else(|| item.as_i64().map(|number| number as f64))
    })
}

fn alert_threshold_status(
    name: &str,
    observed: f64,
    threshold: f64,
    higher_is_bad: bool,
) -> serde_json::Value {
    let breached = if higher_is_bad {
        observed > threshold
    } else {
        observed < threshold
    };
    serde_json::json!({
        "name": name,
        "observed": observed,
        "threshold": threshold,
        "breached": breached,
    })
}

fn config_doctor_view(
    app: &AutoLoopApp,
    profile: Option<&str>,
    config_path: Option<&str>,
) -> serde_json::Value {
    let cfg = &app.config;
    let selected_profile = profile
        .and_then(ConfigProfile::parse)
        .map(|item| match item {
            ConfigProfile::Local => "local",
            ConfigProfile::Production => "production",
        })
        .unwrap_or("auto");

    let mut checks = Vec::<serde_json::Value>::new();
    let mut fail_count = 0_u64;
    let mut warn_count = 0_u64;
    let mut pass_count = 0_u64;

    let mut push_check = |id: &str, severity: &str, passed: bool, message: String| {
        if passed {
            pass_count = pass_count.saturating_add(1);
        } else if severity == "fail" {
            fail_count = fail_count.saturating_add(1);
        } else {
            warn_count = warn_count.saturating_add(1);
        }
        checks.push(serde_json::json!({
            "id": id,
            "severity": severity,
            "passed": passed,
            "message": message,
        }));
    };

    let gate_ratio_ok = (0.0..=1.0).contains(&cfg.runtime.gate_enforce_ratio);
    push_check(
        "runtime.gate_enforce_ratio",
        "fail",
        gate_ratio_ok,
        if gate_ratio_ok {
            "runtime gate ratio is within [0,1]".to_string()
        } else {
            format!(
                "runtime gate ratio out of bounds: {}",
                cfg.runtime.gate_enforce_ratio
            )
        },
    );

    let report_top_k_ok = cfg.observability.report_top_k > 0;
    push_check(
        "observability.report_top_k",
        "fail",
        report_top_k_ok,
        if report_top_k_ok {
            "observability report_top_k is configured".to_string()
        } else {
            "observability report_top_k must be > 0".to_string()
        },
    );

    let signal_batch_ok = cfg.observability.signal_pipeline.batch_size > 0;
    push_check(
        "observability.signal_pipeline.batch_size",
        "fail",
        signal_batch_ok,
        if signal_batch_ok {
            "signal pipeline batch_size is configured".to_string()
        } else {
            "signal pipeline batch_size must be > 0".to_string()
        },
    );

    let signal_retry_ok = cfg.observability.signal_pipeline.max_retries <= 8;
    push_check(
        "observability.signal_pipeline.max_retries",
        "warn",
        signal_retry_ok,
        if signal_retry_ok {
            "signal pipeline max_retries is in recommended range".to_string()
        } else {
            format!(
                "signal pipeline max_retries={} is high; recommended <= 8",
                cfg.observability.signal_pipeline.max_retries
            )
        },
    );

    let storage_ok = if matches!(cfg.storage.backend, autoloop::config::StorageBackend::Postgres) {
        cfg.storage.postgres.enabled && !cfg.storage.postgres.uri.trim().is_empty()
    } else {
        true
    };
    push_check(
        "storage.postgres.enabled_uri",
        "fail",
        storage_ok,
        if storage_ok {
            "storage backend and postgres runtime config are aligned".to_string()
        } else {
            "storage backend is postgres but postgres config is disabled or uri is empty".to_string()
        },
    );

    let threshold = &cfg.observability.alert_thresholds;
    let alert_thresholds_ok = threshold.p95_latency_ms > 0.0
        && threshold.error_rate >= 0.0
        && threshold.error_rate <= 1.0
        && threshold.mttr_ms > 0.0;
    push_check(
        "observability.alert_thresholds",
        "fail",
        alert_thresholds_ok,
        if alert_thresholds_ok {
            "alert thresholds are valid".to_string()
        } else {
            "alert thresholds invalid: expect p95>0, mttr>0, error_rate in [0,1]".to_string()
        },
    );

    let status = if fail_count > 0 {
        "fail"
    } else if warn_count > 0 {
        "warn"
    } else {
        "pass"
    };

    serde_json::json!({
        "status": status,
        "selected_profile": selected_profile,
        "requested_config_path": config_path,
        "runtime_profile": cfg.deployment.profile,
        "summary": {
            "pass": pass_count,
            "warn": warn_count,
            "fail": fail_count,
        },
        "checks": checks,
        "profile_templates": [
            load_profile_template(ConfigProfile::Local),
            load_profile_template(ConfigProfile::Production),
        ],
    })
}

async fn alert_status_view(app: &Arc<AutoLoopApp>, session_id: &str) -> Result<serde_json::Value> {
    let query = autoloop::observability::query_plane::build_unified_query_view(
        &app.state_store(),
        session_id,
        None,
    )
    .await?;

    let telemetry = query.metrics.get("system_telemetry");
    let resilience = query.metrics.get("resilience");
    let collector = query.metrics.get("collector");
    let events = autoloop::observability::event_stream::list_session_events(&app.state_store(), session_id)
        .await
        .unwrap_or_default();

    let execution_started = events
        .iter()
        .filter(|event| event.kind.contains("execution.started"))
        .count() as f64;
    let execution_failed = events
        .iter()
        .filter(|event| event.kind.contains("execution.failed"))
        .count() as f64;
    let error_rate = if execution_started > 0.0 {
        execution_failed / execution_started
    } else {
        0.0
    };

    let p95_latency_ms = as_f64(collector.and_then(|value| value.get("p95_latency_ms")))
        .or_else(|| as_f64(collector.and_then(|value| value.get("latency_p95_ms"))))
        .unwrap_or(0.0);
    let mttr_ms = as_f64(resilience.and_then(|value| value.get("mttr_ms"))).unwrap_or(0.0);
    let open_circuit_count = telemetry
        .and_then(|value| value.get("open_circuit_count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let thresholds: AlertThresholds = app.config.observability.alert_thresholds.clone();
    let checks = vec![
        alert_threshold_status("p95_latency_ms", p95_latency_ms, thresholds.p95_latency_ms, true),
        alert_threshold_status("error_rate", error_rate, thresholds.error_rate, true),
        alert_threshold_status("mttr_ms", mttr_ms, thresholds.mttr_ms, true),
        alert_threshold_status(
            "open_circuit_count",
            open_circuit_count as f64,
            thresholds.open_circuit_count as f64,
            true,
        ),
    ];

    let breached = checks
        .iter()
        .filter(|item| {
            item.get("breached")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .cloned()
        .collect::<Vec<_>>();
    let status = if breached.is_empty() {
        "ok"
    } else if breached.len() >= 2 {
        "critical"
    } else {
        "degraded"
    };

    Ok(serde_json::json!({
        "status": status,
        "session_id": session_id,
        "observed": {
            "p95_latency_ms": p95_latency_ms,
            "error_rate": error_rate,
            "mttr_ms": mttr_ms,
            "open_circuit_count": open_circuit_count,
            "execution_started": execution_started,
            "execution_failed": execution_failed,
        },
        "thresholds": thresholds,
        "checks": checks,
        "breaches": breached,
    }))
}

fn default_real_task_benchmark_dataset_path() -> PathBuf {
    PathBuf::from("deploy")
        .join("benchmarks")
        .join("d12_real_tasks_v1.json")
}

fn load_real_task_benchmark_cases(path: &Path) -> Result<Vec<RealTaskBenchmarkCase>> {
    let raw = fs::read_to_string(path)?;
    let cases = serde_json::from_str::<Vec<RealTaskBenchmarkCase>>(&raw)?;
    if cases.len() < 50 {
        anyhow::bail!(
            "real task benchmark dataset requires at least 50 cases, got {}",
            cases.len()
        );
    }
    Ok(cases)
}

fn normalize_failure_reason(reason: &str) -> String {
    let lowered = reason.to_ascii_lowercase();
    if lowered.contains("artifact") && lowered.contains("proof") {
        "artifact_proof_missing".to_string()
    } else if lowered.contains("permission") || lowered.contains("policy") {
        "permission_or_policy".to_string()
    } else if lowered.contains("budget") || lowered.contains("token") {
        "budget".to_string()
    } else if lowered.contains("timeout") {
        "timeout".to_string()
    } else if lowered.contains("tool") {
        "tool".to_string()
    } else if lowered.contains("test verifier") || lowered.contains("test") {
        "test_verifier".to_string()
    } else if lowered.contains("compile") {
        "compile".to_string()
    } else {
        "unknown".to_string()
    }
}

async fn latest_runtime_decision_for_session(
    app: &Arc<AutoLoopApp>,
    session_id: &str,
) -> Result<Option<serde_json::Value>> {
    let prefix = format!("runtime:decision:{session_id}:");
    let records = app.state_store().list_knowledge_by_prefix(&prefix).await?;
    let latest = records
        .iter()
        .filter_map(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
        .max_by_key(|value| {
            value
                .get("decision_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        });
    Ok(latest)
}

async fn run_real_task_benchmark(
    app: &Arc<AutoLoopApp>,
    base_session_id: &str,
    dataset_path: &Path,
    limit: usize,
) -> Result<RealTaskBenchmarkReport> {
    let cases = load_real_task_benchmark_cases(dataset_path)?;
    let effective_total = limit.max(1).min(cases.len());
    let selected = cases.into_iter().take(effective_total).collect::<Vec<_>>();
    let benchmark_id = format!(
        "d12-real-benchmark-{}",
        autoloop::orchestration::current_time_ms()
    );
    let mut results = Vec::with_capacity(selected.len());
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut retry_total = 0u64;
    let mut failure_distribution: HashMap<String, usize> = HashMap::new();
    let case_timeout_ms = std::env::var("AUTOLOOP_BENCHMARK_CASE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(8_000)
        .max(1_000);
    let shadow_safe = std::env::var("AUTOLOOP_BENCHMARK_SHADOW_SAFE")
        .ok()
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "on"))
        .unwrap_or(false);

    for case in selected {
        let session_id = format!("{base_session_id}:bench:{}", case.task_id);
        let started = autoloop::orchestration::current_time_ms();
        let requested_mode = case.mode.to_ascii_lowercase();
        let mode_overridden = shadow_safe && requested_mode == "swarm";
        let effective_mode = if mode_overridden {
            "direct"
        } else {
            requested_mode.as_str()
        };
        let execution = match effective_mode {
            "swarm" => tokio::time::timeout(
                std::time::Duration::from_millis(case_timeout_ms),
                app.process_requirement_swarm(&session_id, &case.prompt),
            )
            .await
            .map_err(|_| anyhow::anyhow!("benchmark case timeout at {}ms", case_timeout_ms))
            .and_then(|result| result),
            _ => tokio::time::timeout(
                std::time::Duration::from_millis(case_timeout_ms),
                app.process_direct(&session_id, &case.prompt),
            )
            .await
            .map_err(|_| anyhow::anyhow!("benchmark case timeout at {}ms", case_timeout_ms))
            .and_then(|result| result),
        };
        let decision = latest_runtime_decision_for_session(app, &session_id).await?;
        let retry_count = decision
            .as_ref()
            .and_then(|value| value.get("execute"))
            .map(|execute| {
                execute
                    .get("provider_retry_count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    + execute
                        .get("tool_retry_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0)
            })
            .unwrap_or(0) as u32;
        retry_total = retry_total.saturating_add(retry_count as u64);
        let trace_id = decision
            .as_ref()
            .and_then(|value| value.get("trace_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);

        let (success, failure_reason) = match execution {
            Ok(_) => {
                let rejected = decision
                    .as_ref()
                    .and_then(|value| value.get("decision"))
                    .and_then(serde_json::Value::as_str)
                    .map(|status| status.eq_ignore_ascii_case("reject"))
                    .unwrap_or(false);
                if rejected {
                    let reason = decision
                        .as_ref()
                        .and_then(|value| value.get("reasons"))
                        .and_then(serde_json::Value::as_array)
                        .and_then(|reasons| reasons.first())
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("rejected_without_reason")
                        .to_string();
                    (false, Some(reason))
                } else {
                    (true, None)
                }
            }
            Err(error) => (false, Some(error.to_string())),
        };
        if success {
            passed = passed.saturating_add(1);
        } else {
            failed = failed.saturating_add(1);
            let reason = normalize_failure_reason(failure_reason.as_deref().unwrap_or("unknown"));
            *failure_distribution.entry(reason).or_insert(0) += 1;
        }
        results.push(RealTaskBenchmarkItemResult {
            task_id: case.task_id,
            mode: effective_mode.to_string(),
            mode_overridden,
            session_id,
            success,
            retry_count,
            failure_reason,
            trace_id,
            duration_ms: autoloop::orchestration::current_time_ms().saturating_sub(started),
        });
    }

    let success_rate = if effective_total == 0 {
        0.0
    } else {
        passed as f64 / effective_total as f64
    };
    let average_retry_count = if effective_total == 0 {
        0.0
    } else {
        retry_total as f64 / effective_total as f64
    };
    let mut distribution = BTreeMap::new();
    for (key, value) in failure_distribution {
        distribution.insert(key, value);
    }
    let now = autoloop::orchestration::current_time_ms();
    let report_key = format!("benchmark:d12:{base_session_id}:{benchmark_id}");
    let report = RealTaskBenchmarkReport {
        session_id: base_session_id.to_string(),
        benchmark_id: benchmark_id.clone(),
        dataset_path: dataset_path.to_string_lossy().to_string(),
        total: effective_total,
        passed,
        failed,
        success_rate,
        total_retry_count: retry_total,
        average_retry_count,
        failure_reason_distribution: distribution,
        results,
        created_at_ms: now,
        evidence_ref: Some(report_key.clone()),
    };
    app.state_store()
        .upsert_json_knowledge(report_key.clone(), &report, "d12-real-benchmark")
        .await?;
    app.state_store()
        .upsert_json_knowledge(
            format!("benchmark:d12:{base_session_id}:latest"),
            &report,
            "d12-real-benchmark",
        )
        .await?;
    let _ = append_event(
        app.state_store(),
        "benchmark.d12.real_task.completed",
        format!("trace:{base_session_id}:benchmark:{benchmark_id}"),
        base_session_id.to_string(),
        None,
        Some("system:benchmark".to_string()),
        autoloop::contracts::version::CONTRACT_VERSION,
        serde_json::json!({
            "benchmark_id": benchmark_id,
            "dataset_path": dataset_path.to_string_lossy(),
            "total": report.total,
            "passed": report.passed,
            "failed": report.failed,
            "success_rate": report.success_rate,
            "average_retry_count": report.average_retry_count,
            "failure_reason_distribution": report.failure_reason_distribution,
            "evidence_ref": report_key,
        }),
    )
    .await;
    Ok(report)
}

pub async fn dispatch_system(
    app: &Arc<AutoLoopApp>,
    command_registry: &BuiltinCommandRegistry,
    identity: &IdentityInput,
    input: SystemDispatchInput,
) -> Result<()> {
    super::bind_identity_for_session(
        app,
        identity.tenant.as_deref(),
        identity.principal.as_deref(),
        identity.policy.as_deref(),
        identity.lease_ttl_ms,
        &input.session_id,
    )
    .await?;
    let body = match input.action.as_str() {
        "relation" => {
            let subaction = input.subaction.as_deref().unwrap_or("status");
            match subaction {
                "status" => {
                    let view = super::system_relation_status_view(
                        app,
                        &input.session_id,
                        input.limit.max(1),
                    )
                    .await?;
                    serde_json::to_string_pretty(&view)?
                }
                "explain" => {
                    let view = super::system_relation_explain_view(
                        app,
                        &input.session_id,
                        input.trace_id.as_deref(),
                        input.limit.max(1),
                    )
                    .await?;
                    serde_json::to_string_pretty(&view)?
                }
                "graph" => {
                    let view = super::system_relation_graph_view(
                        app,
                        &input.session_id,
                        input.trace_id.as_deref(),
                        input.limit.max(1),
                    )
                    .await?;
                    serde_json::to_string_pretty(&view)?
                }
                "check" => {
                    let view = super::system_relation_check_view(
                        app,
                        &input.session_id,
                        input.trace_id.as_deref(),
                        input.limit.max(1),
                        false,
                        input.reason.as_deref(),
                    )
                    .await?;
                    serde_json::to_string_pretty(&view)?
                }
                "repair" => {
                    let view = super::system_relation_check_view(
                        app,
                        &input.session_id,
                        input.trace_id.as_deref(),
                        input.limit.max(1),
                        true,
                        input.reason.as_deref(),
                    )
                    .await?;
                    serde_json::to_string_pretty(&view)?
                }
                _ => serde_json::json!({
                    "error": "unsupported relation subaction",
                    "supported": ["status", "explain", "graph", "check", "repair"]
                })
                .to_string(),
            }
        }
        "artifact" => {
            let subaction = input.subaction.as_deref().unwrap_or("proof");
            match subaction {
                "proof" | "query" => {
                    let view = super::system_artifact_proof_view(
                        app,
                        &input.session_id,
                        input.artifact_ref.as_deref(),
                        input.artifact_path.as_deref(),
                        input.trace_id.as_deref(),
                        input.limit.max(1),
                    )
                    .await?;
                    serde_json::to_string_pretty(&view)?
                }
                _ => serde_json::json!({
                    "error": "unsupported artifact subaction",
                    "supported": ["proof"]
                })
                .to_string(),
            }
        }
        "signal" => {
            let signal_action = input.subaction.as_deref().unwrap_or("status");
            match signal_action {
                "status" => {
                    let view = super::system_signal_status_view(app, &input.session_id).await?;
                    serde_json::to_string_pretty(&view)?
                }
                "explain" => {
                    let view = super::system_signal_explain_view(
                        app,
                        &input.session_id,
                        input.trace_id.as_deref(),
                    )
                    .await?;
                    serde_json::to_string_pretty(&view)?
                }
                "drain" => {
                    let view = super::system_signal_drain_view(app, &input.session_id).await?;
                    serde_json::to_string_pretty(&view)?
                }
                _ => serde_json::json!({
                    "error": "unsupported signal subaction",
                    "supported": ["status", "explain", "drain"]
                })
                .to_string(),
            }
        }
        "benchmark" => {
            let benchmark_action = input.subaction.as_deref().unwrap_or("run");
            match benchmark_action {
                "run" => {
                    let dataset_path = input
                        .artifact_path
                        .clone()
                        .unwrap_or_else(default_real_task_benchmark_dataset_path);
                    let report = run_real_task_benchmark(
                        app,
                        &input.session_id,
                        &dataset_path,
                        input.limit.max(1),
                    )
                    .await?;
                    serde_json::to_string_pretty(&report)?
                }
                "status" => {
                    let latest = app
                        .state_store()
                        .get_knowledge(&format!("benchmark:d12:{}:latest", input.session_id))
                        .await?;
                    match latest {
                        Some(value) => value.value,
                        None => serde_json::json!({
                            "status": "not_found",
                            "session_id": input.session_id,
                            "hint": "run `autocog system benchmark run` first",
                        })
                        .to_string(),
                    }
                }
                _ => serde_json::json!({
                    "error": "unsupported benchmark subaction",
                    "supported": [
                        "run [--artifact-path <dataset.json>] [--limit <n>]",
                        "status"
                    ]
                })
                .to_string(),
            }
        }
        "status" => {
            let outcome = command_registry
                .dispatch(
                    "system.status",
                    app.clone(),
                    autoloop::cli_runtime::DispatchArgs {
                        session_id: input.session_id.clone(),
                        action: "status".to_string(),
                        params: BTreeMap::new(),
                    },
                )
                .await?;
            outcome.body.unwrap_or(app.system_status().await?)
        }
        "permission-mode" => serde_json::to_string_pretty(&app.runtime.permission_mode_status())?,
        "config" => {
            let config_action = input.subaction.as_deref().unwrap_or("doctor");
            match config_action {
                "doctor" => {
                    let config_env = std::env::var("AUTOLOOP_CONFIG").ok();
                    let view = config_doctor_view(
                        app,
                        input.profile.as_deref(),
                        config_env.as_deref(),
                    );
                    serde_json::to_string_pretty(&view)?
                }
                "template" => {
                    let profile = input
                        .profile
                        .as_deref()
                        .and_then(ConfigProfile::parse)
                        .unwrap_or(ConfigProfile::Local);
                    serde_json::to_string_pretty(&load_profile_template(profile))?
                }
                _ => serde_json::json!({
                    "error": "unsupported config subaction",
                    "supported": ["doctor", "template [--profile local|production]"]
                })
                .to_string(),
            }
        }
        "alert" => {
            let alert_action = input.subaction.as_deref().unwrap_or("status");
            match alert_action {
                "status" => {
                    let view = alert_status_view(app, &input.session_id).await?;
                    serde_json::to_string_pretty(&view)?
                }
                "drill" => {
                    let trace_id = format!(
                        "trace:{}:ops:alert-drill:{}",
                        input.session_id,
                        autoloop::orchestration::current_time_ms()
                    );
                    let reason = input
                        .reason
                        .clone()
                        .unwrap_or_else(|| "ops alert drill".to_string());
                    let payload = serde_json::json!({
                        "status": "raised",
                        "severity": "critical",
                        "reason": reason,
                        "source": "system.alert.drill",
                    });
                    let event = autoloop::observability::event_stream::append_event(
                        &app.state_store(),
                        "alert.raised",
                        trace_id.clone(),
                        input.session_id.clone(),
                        None,
                        Some("ops:alert".into()),
                        autoloop::contracts::version::CONTRACT_VERSION,
                        payload.clone(),
                    )
                    .await?;
                    app.state_store()
                        .upsert_json_knowledge(
                            format!("observability:{}:alert:latest", input.session_id),
                            &serde_json::json!({
                                "session_id": input.session_id,
                                "trace_id": trace_id,
                                "status": "raised",
                                "severity": "critical",
                                "reason": reason,
                                "event_id": event.event_id,
                                "raised_at_ms": event.created_at_ms,
                            }),
                            "ops-alert",
                        )
                        .await?;
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "raised",
                        "session_id": input.session_id,
                        "event_id": event.event_id,
                        "trace_id": trace_id,
                        "payload": payload,
                    }))?
                }
                _ => serde_json::json!({
                    "error": "unsupported alert subaction",
                    "supported": ["status", "drill [--reason <text>]"]
                })
                .to_string(),
            }
        }
        "self-heal" => {
            let heal_action = input.subaction.as_deref().unwrap_or("drill");
            match heal_action {
                "drill" => {
                    let profile_kind = super::parse_degrade_profile(
                        input.profile.as_deref().unwrap_or("queue_throttle"),
                    );
                    let trace_id = format!(
                        "trace:{}:ops:self-heal:{}",
                        input.session_id,
                        autoloop::orchestration::current_time_ms()
                    );
                    let degrade_reason = format!(
                        "self-heal drill degrade: {}",
                        input.reason.as_deref().unwrap_or("ops drill")
                    );
                    let applied = app
                        .runtime
                        .apply_degrade_profile(
                            &app.state_store(),
                            &input.session_id,
                            &trace_id,
                            profile_kind,
                            &degrade_reason,
                        )
                        .await?;
                    let recovered = app
                        .runtime
                        .recover_from_degrade(
                            &app.state_store(),
                            &input.session_id,
                            input.reason
                                .as_deref()
                                .unwrap_or("self-heal drill recover"),
                        )
                        .await?;
                    let _ = autoloop::observability::event_stream::append_event(
                        &app.state_store(),
                        "ops.self_heal.drill",
                        trace_id,
                        input.session_id.clone(),
                        None,
                        Some("ops:self-heal".into()),
                        autoloop::contracts::version::CONTRACT_VERSION,
                        serde_json::json!({
                            "degrade_record_id": applied.record_id,
                            "degrade_profile": applied.profile,
                            "recovered": recovered.as_ref().map(|record| record.recovered).unwrap_or(false),
                            "mttr_ms": recovered.as_ref().and_then(|record| record.mttr_ms),
                        }),
                    )
                    .await;
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": if recovered.as_ref().map(|record| record.recovered).unwrap_or(false) { "recovered" } else { "degraded" },
                        "degrade": applied,
                        "recover": recovered,
                    }))?
                }
                _ => serde_json::json!({
                    "error": "unsupported self-heal subaction",
                    "supported": ["drill [--profile queue_throttle|provider_fallback|mcp_conservative] [--reason <text>]"]
                })
                .to_string(),
            }
        }
        "health" => {
            let outcome = command_registry
                .dispatch(
                    "system.health",
                    app.clone(),
                    autoloop::cli_runtime::DispatchArgs {
                        session_id: input.session_id.clone(),
                        action: "health".to_string(),
                        params: BTreeMap::new(),
                    },
                )
                .await?;
            if let Some(body) = outcome.body {
                body
            } else {
                serde_json::to_string_pretty(&serde_json::json!({
                    "research": app.research.health_report(),
                    "system": serde_json::from_str::<serde_json::Value>(&app.system_status().await?)
                        .unwrap_or_else(|_| serde_json::json!({})),
                }))?
            }
        }
        "update" => serde_json::json!({
            "status": "noop",
            "note": "binary self-update is not implemented yet"
        })
        .to_string(),
        "deploy" => {
            let anchors = input
                .anchor_list
                .as_ref()
                .and_then(|path| super::parse_anchor_list(path).ok())
                .map(|rows| rows.len())
                .unwrap_or(0);
            serde_json::json!({
                "status": "ready",
                "artifacts": ["Dockerfile", "docker-compose.yml", "deploy/k8s/autoloop-deployment.yaml"],
                "anchor_batch_size": anchors,
            })
            .to_string()
        }
        "backup" => serde_json::json!({
            "status": "ready",
            "script": "deploy/backup/backup.ps1",
        })
        .to_string(),
        "restore" => serde_json::json!({
            "status": "ready",
            "script": "deploy/backup/restore.ps1",
        })
        .to_string(),
        "approve" => {
            app.operator_decision(
                &input.session_id,
                true,
                input.reason.as_deref().unwrap_or("approved by operator"),
            )
            .await?
        }
        "reject" => {
            app.operator_decision(
                &input.session_id,
                false,
                input.reason.as_deref().unwrap_or("rejected by operator"),
            )
            .await?
        }
        "export" => {
            let query_view = autoloop::observability::query_plane::persist_unified_query_view(
                &app.state_store(),
                &input.session_id,
                input.trace_id.as_deref(),
            )
            .await?;
            serde_json::to_string_pretty(&serde_json::json!({
                "session_id": input.session_id,
                "system_status": serde_json::from_str::<serde_json::Value>(&app.system_status().await?)
                    .unwrap_or_else(|_| serde_json::json!({})),
                "dashboard": serde_json::from_str::<serde_json::Value>(&app.export_dashboard_snapshot(&input.session_id).await?)
                    .unwrap_or_else(|_| serde_json::json!({})),
                "query_view": query_view,
                "knowledge_graph": serde_json::from_str::<serde_json::Value>(&app.export_knowledge(&input.session_id, "graph").await?)
                    .unwrap_or_else(|_| serde_json::json!({})),
            }))?
        }
        "dashboard" => app.export_dashboard_snapshot(&input.session_id).await?,
        "replay-report" => app
            .export_replay_report(&input.session_id, input.snapshot_id.as_deref())
            .await?,
        "query" => {
            let view = autoloop::observability::query_plane::persist_unified_query_view(
                &app.state_store(),
                &input.session_id,
                input.trace_id.as_deref(),
            )
            .await?;
            serde_json::to_string_pretty(&view)?
        }
        "objective-weights" => {
            let any_update = input.task_utility.is_some()
                || input.distortion_penalty.is_some()
                || input.attention_mismatch_penalty.is_some()
                || input.token_cost_penalty.is_some();
            if any_update {
                app.set_context_objective_weights(
                    input.task_utility,
                    input.distortion_penalty,
                    input.attention_mismatch_penalty,
                    input.token_cost_penalty,
                    input.reason.as_deref(),
                )
                .await?
            } else {
                app.get_context_objective_weights().await?
            }
        }
        "replay" => {
            if let Some(snapshot_id) = input.snapshot_id.as_deref() {
                app.run_replay_snapshot(snapshot_id).await?
            } else {
                app.export_session_replay(&input.session_id).await?
            }
        }
        "degrade" => {
            let parsed =
                super::parse_degrade_profile(input.profile.as_deref().unwrap_or("manual_takeover"));
            let applied = app
                .runtime
                .apply_degrade_profile(
                    &app.state_store(),
                    &input.session_id,
                    &format!(
                        "operator:{}:{}",
                        input.session_id,
                        autoloop::orchestration::current_time_ms()
                    ),
                    parsed,
                    input.reason.as_deref().unwrap_or("operator requested degrade"),
                )
                .await?;
            serde_json::to_string_pretty(&applied)?
        }
        "recover" => {
            let recovered = app
                .runtime
                .recover_from_degrade(
                    &app.state_store(),
                    &input.session_id,
                    input.reason.as_deref().unwrap_or("operator requested recovery"),
                )
                .await?;
            serde_json::to_string_pretty(&serde_json::json!({
                "session": input.session_id,
                "recovered": recovered,
            }))?
        }
        "chaos" => {
            let profile_kind =
                super::parse_degrade_profile(input.profile.as_deref().unwrap_or("queue_throttle"));
            let now = autoloop::orchestration::current_time_ms();
            let case = autoloop::runtime::ChaosCase {
                case_id: format!("{}-{}", input.fault.as_deref().unwrap_or("fault"), now),
                name: format!("chaos-{}", input.fault.as_deref().unwrap_or("fault")),
                fault: input
                    .fault
                    .clone()
                    .unwrap_or_else(|| "provider_down".to_string()),
                expected_profile: profile_kind,
                target: "runtime".into(),
                injected_at_ms: now,
            };
            let record = app
                .runtime
                .run_chaos_case(&app.state_store(), &input.session_id, case)
                .await?;
            serde_json::to_string_pretty(&record)?
        }
        "serve" => {
            autoloop::dashboard_server::run_dashboard_server(app.clone(), &input.host, input.port)
                .await?;
            return Ok(());
        }
        _ => serde_json::json!({
            "error":"unsupported system action",
            "supported": [
                "status",
                "health",
                "config doctor",
                "config template [--profile local|production]",
                "query",
                "replay",
                "replay-report",
                "alert status",
                "alert drill [--reason <text>]",
                "self-heal drill [--profile queue_throttle|provider_fallback|mcp_conservative] [--reason <text>]",
                "objective-weights",
                "degrade",
                "recover",
                "chaos",
                "export",
                "dashboard",
                "serve",
                "signal status",
                "signal explain",
                "signal drain",
                "benchmark run [--artifact-path <dataset.json>] [--limit <n>]",
                "benchmark status",
                "relation status",
                "relation explain [--trace-id <id>] [--limit <n>]",
                "relation graph [--trace-id <id>] [--limit <n>]",
                "relation check [--limit <n>]",
                "relation repair [--trace-id <id>] [--limit <n>] [--reason <text>]",
                "artifact proof [--artifact-ref <evidence_ref>] [--artifact-path <file>] [--trace-id <id>] [--limit <n>]"
            ]
        }).to_string(),
    };
    super::write_or_print(input.output.as_ref(), &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, time::{SystemTime, UNIX_EPOCH}};

    use autoloop::{AutoLoopApp, cli_runtime::BuiltinCommandRegistry, config::AppConfig};

    #[tokio::test]
    async fn frontend_events_json_handles_empty_session_event_tail() {
        let app = Arc::new(AutoLoopApp::new(AppConfig::default()));
        let command_registry = BuiltinCommandRegistry::new();
        let identity = IdentityInput {
            tenant: None,
            principal: None,
            policy: None,
            lease_ttl_ms: 3_600_000,
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis();
        let output = std::env::temp_dir().join(format!("ontoloop-empty-events-{timestamp}.json"));
        let input = FrontendDispatchInput {
            session_id: format!("empty-events-session-{timestamp}"),
            action: "events".to_string(),
            content: None,
            trace_id: None,
            request_id: None,
            decision: None,
            reason: None,
            jwt: None,
            transport_kind: "cli".to_string(),
            ttl_ms: 3_600_000,
            subject: None,
            tenant_id: None,
            format: "json".to_string(),
            limit: 20,
            output: Some(output.clone()),
        };

        dispatch_frontend(&app, &command_registry, &identity, input)
            .await
            .expect("dispatch frontend events");
        let raw = std::fs::read_to_string(&output).expect("read output");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse output json");
        assert_eq!(
            parsed.get("count").and_then(serde_json::Value::as_u64),
            Some(0)
        );
        assert_eq!(
            parsed
                .get("events")
                .and_then(serde_json::Value::as_array)
                .map(|items| items.len()),
            Some(0)
        );

        let _ = std::fs::remove_file(output);
    }

    #[tokio::test]
    async fn system_config_doctor_reports_status() {
        let app = Arc::new(AutoLoopApp::new(AppConfig::default()));
        let command_registry = BuiltinCommandRegistry::new();
        let identity = IdentityInput {
            tenant: None,
            principal: None,
            policy: None,
            lease_ttl_ms: 3_600_000,
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_millis();
        let output = std::env::temp_dir().join(format!("ontoloop-config-doctor-{timestamp}.json"));

        dispatch_system(
            &app,
            &command_registry,
            &identity,
            SystemDispatchInput {
                session_id: format!("config-doctor-session-{timestamp}"),
                action: "config".to_string(),
                subaction: Some("doctor".to_string()),
                snapshot_id: None,
                trace_id: None,
                artifact_ref: None,
                artifact_path: None,
                limit: 50,
                profile: Some("local".to_string()),
                fault: None,
                reason: None,
                task_utility: None,
                distortion_penalty: None,
                attention_mismatch_penalty: None,
                token_cost_penalty: None,
                anchor_list: None,
                output: Some(output.clone()),
                host: "127.0.0.1".to_string(),
                port: 8787,
            },
        )
        .await
        .expect("dispatch system config doctor");
        let raw = std::fs::read_to_string(&output).expect("read output");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse output");
        let status = parsed
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        assert!(matches!(status, "pass" | "warn" | "fail"));

        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn d12_benchmark_dataset_has_at_least_50_cases() {
        let path = default_real_task_benchmark_dataset_path();
        let cases = load_real_task_benchmark_cases(&path).expect("load benchmark dataset");
        assert!(
            cases.len() >= 50,
            "benchmark dataset must include at least 50 real tasks"
        );
    }

    #[test]
    fn normalize_failure_reason_maps_budget_and_permission() {
        assert_eq!(
            normalize_failure_reason("token budget exceeded in compile phase"),
            "budget"
        );
        assert_eq!(
            normalize_failure_reason("permission denied by policy"),
            "permission_or_policy"
        );
    }
}
