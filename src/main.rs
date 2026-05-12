use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use autoloop::plugins::gitmemory_core::patch_review_queue::PatchReviewQueue;
use autoloop::plugins::gitmemory_core::patch_core::{PatchOp, PatchOpKind, PatchPlan};
use autoloop::plugins::gitmemory_core::recall_plugin_router::RecallPluginRouter;
use autoloop::plugins::gitmemory_core::schema_registry::SchemaRegistry;
use autoloop::plugins::gitmemory_core::{
    hot_index_updater::{HotIndexUpdater, RefreshPlanMode},
    ingest_validator::{IngestValidationMode, IngestValidator},
};
use autoloop::plugins::gitmemory_core::{graph_export::GraphExportOptions, view_plane::ViewPlane};
use autoloop::runtime::trigger_runtime::TriggerRuntimeEngine;
use autoloop::runtime::DegradeProfileKind;
use autoloop::session::{audit::StateAuditSink, machine::WorkflowMachine};
use autoloop::observability::event_stream::append_event;
use autoloop::observability::query_plane::build_unified_query_view;
use autoloop::{
    AutoLoopApp,
    cli_runtime::{
        BuiltinCommandRegistry, DispatchArgs,
    },
    config::AppConfig,
};
use clap::{Parser, Subcommand};
use tokio::time::{Duration, sleep};
use tracing_subscriber::EnvFilter;

mod command_dispatch;

#[derive(Parser, Debug)]
#[command(name = "ontoloop")]
#[command(about = "OntoLoop — sovereign AI harness. Plan | Lite | Full | Test")]
#[command(version = "0.1.0")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long)]
    profile: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long)]
    message: Option<String>,

    #[arg(long, default_value = "cli:direct")]
    session: String,

    #[arg(long, default_value_t = false)]
    swarm: bool,

    #[arg(long)]
    tenant: Option<String>,

    #[arg(long)]
    principal: Option<String>,

    #[arg(long)]
    policy: Option<String>,

    #[arg(long, default_value_t = 3_600_000)]
    lease_ttl_ms: u64,

    #[arg(long = "permission-mode")]
    permission_mode: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Focus {
        #[arg()]
        anchor: Option<String>,
        #[arg(long)]
        list: bool,
        #[arg(long)]
        status: bool,
        #[arg(long)]
        board: bool,
        #[arg(long)]
        delete: bool,
        #[arg(long)]
        add: Option<String>,
        #[arg(long = "anchor-id")]
        anchor_id: Option<String>,
        #[arg(long)]
        time: Option<String>,
        #[arg(long)]
        region: Option<String>,
        #[arg(long = "core-source")]
        core_source: Option<String>,
        #[arg(long = "update-cycle")]
        update_cycle: Option<String>,
    },
    Mcp {
        #[arg()]
        action: String,
        #[arg(long = "anchor-id")]
        anchor_id: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        input: Option<PathBuf>,
        #[arg(long)]
        tool: Option<String>,
    },
    Knowledge {
        #[arg()]
        action: String,
        #[arg(long = "anchor-id")]
        anchor_id: Option<String>,
        #[arg(long = "anchor-list")]
        anchor_list: Option<PathBuf>,
        #[arg(long = "snapshot-id")]
        snapshot_id: Option<String>,
        #[arg(long, default_value = "graph")]
        r#type: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Crawl {
        #[arg()]
        action: String,
        #[arg(long = "anchor-id")]
        anchor_id: Option<String>,
        #[arg()]
        anchor: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Trigger {
        #[arg()]
        action: String,
        #[arg(long = "anchor-id")]
        anchor_id: Option<String>,
        #[arg(long)]
        schedule: Option<String>,
        #[arg(long)]
        topic: Option<String>,
        #[arg(long, default_value = "webhook")]
        actor: String,
        #[arg(long, default_value_t = false)]
        run_now: bool,
        #[arg(long)]
        payload: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Bridge {
        #[arg()]
        action: String,
        #[arg(long = "anchor-id")]
        anchor_id: Option<String>,
        #[arg(long = "transport-kind", default_value = "cli")]
        transport_kind: String,
        #[arg(long, default_value = "bridge:operator")]
        subject: String,
        #[arg(long = "jwt")]
        jwt: Option<String>,
        #[arg(long, default_value_t = 3_600_000)]
        ttl_ms: u64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Skill {
        #[arg()]
        action: String,
        #[arg()]
        skill: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        markdown: Option<String>,
        #[arg(long)]
        builder: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Org {
        #[arg()]
        action: String,
        #[arg(long = "anchor-id")]
        anchor_id: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Plugin {
        #[arg()]
        action: String,
        #[arg()]
        plugin: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long = "traffic-percent")]
        traffic_percent: Option<u8>,
        #[arg(long, default_value_t = true)]
        verify_signature: bool,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Session {
        #[arg()]
        action: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Background {
        #[arg()]
        action: String,
        #[arg(long = "task-id")]
        task_id: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        command: Option<String>,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long, default_value_t = 0)]
        max_restarts: u32,
        #[arg(long, default_value_t = 50)]
        lines: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Memory {
        #[arg()]
        plane: String,
        #[arg()]
        action: String,
        #[arg(long = "file")]
        file: Option<String>,
        #[arg(long = "review-id")]
        review_id: Option<String>,
        #[arg(long)]
        operator: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long = "repo-root")]
        repo_root: Option<PathBuf>,
        #[arg(long, default_value_t = false)]
        clean: bool,
        #[arg(long = "no-infer", default_value_t = false)]
        no_infer: bool,
        #[arg(long, default_value_t = false)]
        report: bool,
        #[arg(long = "save")]
        save: Option<PathBuf>,
        #[arg(long = "refresh-mode")]
        refresh_mode: Option<String>,
        #[arg(long)]
        page: Option<usize>,
        #[arg(long = "page-size")]
        page_size: Option<usize>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Frontend {
        #[arg()]
        action: String,
        #[arg(long)]
        content: Option<String>,
        #[arg(long = "trace-id")]
        trace_id: Option<String>,
        #[arg(long = "request-id")]
        request_id: Option<String>,
        #[arg(long)]
        decision: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long = "jwt")]
        jwt: Option<String>,
        #[arg(long, default_value = "cli")]
        transport_kind: String,
        #[arg(long, default_value_t = 3_600_000)]
        ttl_ms: u64,
        #[arg(long)]
        subject: Option<String>,
        #[arg(long = "tenant-id")]
        tenant_id: Option<String>,
        #[arg(long, default_value = "pretty")]
        format: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Chat {},
    System {
        #[arg()]
        action: String,
        #[arg()]
        subaction: Option<String>,
        #[arg(long = "snapshot-id")]
        snapshot_id: Option<String>,
        #[arg(long = "trace-id")]
        trace_id: Option<String>,
        #[arg(long = "artifact-ref")]
        artifact_ref: Option<String>,
        #[arg(long = "artifact-path")]
        artifact_path: Option<PathBuf>,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long)]
        profile: Option<String>,
        #[arg(long)]
        fault: Option<String>,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long = "task-utility")]
        task_utility: Option<f32>,
        #[arg(long = "distortion-penalty")]
        distortion_penalty: Option<f32>,
        #[arg(long = "attention-mismatch-penalty")]
        attention_mismatch_penalty: Option<f32>,
        #[arg(long = "token-cost-penalty")]
        token_cost_penalty: Option<f32>,
        #[arg(long = "anchor-list")]
        anchor_list: Option<PathBuf>,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8787)]
        port: u16,
    },
    Tui {
        #[arg(long)]
        mode: Option<String>,
    },
}

fn main() -> Result<()> {
    if let Ok(task) = std::env::var("AUTOLOOP_SMOKE_TASK") {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(32 * 1024 * 1024)
            .build()?;
        return runtime.block_on(Box::pin(run_smoke_frontend_task(&task)));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(32 * 1024 * 1024)
        .build()?;
    runtime.block_on(async {
        tokio::spawn(async { run_main().await })
            .await
            .map_err(|error| anyhow::anyhow!("main runtime worker join error: {error}"))?
    })
}

async fn run_main() -> Result<()> {
    if std::env::var("AUTOLOOP_TRACING_INIT")
        .map(|value| value == "1")
        .unwrap_or(false)
    {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
            )
            .try_init();
    }

    let cli = Cli::parse();
    let identity_tenant = cli.tenant.clone();
    let identity_principal = cli.principal.clone();
    let identity_policy = cli.policy.clone();
    let identity_lease_ttl_ms = cli.lease_ttl_ms;
    let permission_mode_override = cli.permission_mode.clone();
    let config = match AppConfig::load_with_profile_hint(cli.config.as_deref(), cli.profile.as_deref()) {
        Ok(cfg) => cfg,
        Err(_) => {
            eprintln!("[ontoloop] Config not found, using defaults (set OPENAI_API_KEY env var)");
            AppConfig::default()
        }
    };
    let mut config = config;
    let tui_config = config.clone();
apply_provider_env_overrides(&mut config);

    if let Some(mode) = permission_mode_override.as_deref() {
        unsafe {
            std::env::set_var("AUTOLOOP_PERMISSION_MODE", mode);
        }
    }
    let app = AutoLoopApp::try_new(config)?;
    let report = app.bootstrap().await?;
    let mut _startup_workflow_machine = WorkflowMachine::new(
        "system:bootstrap",
        Arc::new(StateAuditSink::with_source(
            app.state_store().clone(),
            "cli-bootstrap",
        )),
    );
    let app = Arc::new(app);
    let command_registry = BuiltinCommandRegistry::new();

    if let Some(command) = cli.command {
        match command {
            Commands::Focus {
                anchor,
                list,
                status,
                board,
                delete,
                add,
                anchor_id,
                time,
                region,
                core_source,
                update_cycle,
            } => {
                let identity_session = anchor_id.as_deref().unwrap_or("cli:focus");
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    identity_session,
                )
                .await?;
                if list {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&app.list_focus_anchors().await?)?
                    );
                } else if status {
                    let focus_session = anchor_id.as_deref().unwrap_or("cli:focus").to_string();
                    let outcome = command_registry
                        .dispatch(
                            "focus.status",
                            app.clone(),
                            DispatchArgs {
                                session_id: focus_session.clone(),
                                action: "status".to_string(),
                                params: BTreeMap::new(),
                            },
                        )
                        .await?;
                    if let Some(body) = outcome.body {
                        println!("{}", body);
                    } else {
                        println!("{}", app.focus_status(&focus_session).await?);
                    }
                } else if board {
                    let session = anchor_id.as_deref().unwrap_or("cli:focus");
                    let board = app
                        .state_store()
                        .get_knowledge(&format!("focus-board:{session}:latest"))
                        .await?
                        .map(|record| record.value)
                        .unwrap_or_else(|| "{}".to_string());
                    println!("{board}");
                } else if delete {
                    println!(
                        "{}",
                        app.delete_focus_anchor(anchor_id.as_deref().unwrap_or("cli:focus"))
                            .await?
                    );
                } else if let Some(extra) = add {
                    let session = anchor_id.unwrap_or_else(|| "cli:focus".into());
                    let response = app.process_requirement_swarm(&session, &extra).await?;
                    println!("{response}");
                } else if let Some(anchor) = anchor {
                    let session = anchor_id.unwrap_or_else(|| "cli:focus".into());
                    let anchor_request = compose_anchor_request(
                        &anchor,
                        time.as_deref(),
                        region.as_deref(),
                        core_source.as_deref(),
                        update_cycle.as_deref(),
                    );
                    let response = app
                        .process_requirement_swarm(&session, &anchor_request)
                        .await?;
                    println!("{response}");
                } else {
                    println!("{}", app.system_status().await?);
                }
            }
            Commands::Mcp {
                action,
                anchor_id,
                output,
                input,
                tool,
            } => {
                let identity_session = anchor_id.as_deref().unwrap_or(&cli.session);
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    identity_session,
                )
                .await?;
                let body = match action.as_str() {
                    "status" => app
                        .focus_status(anchor_id.as_deref().unwrap_or("cli:focus"))
                        .await?,
                    "export" => app.export_mcp_catalog().await?,
                    "import" => {
                        let raw = if let Some(path) = input.as_ref() {
                            fs::read_to_string(path)?
                        } else {
                            "[]".into()
                        };
                        app.import_mcp_catalog(&raw).await?
                    }
                    "optimize" => serde_json::json!({
                        "status": "accepted",
                        "note": "runtime and learning loop already perform bounded autonomous optimization"
                    })
                    .to_string(),
                    "verify" | "deprecate" | "rollback" => app
                        .govern_mcp_capability(&action, tool.as_deref().unwrap_or("mcp::local-mcp::invoke"))
                        .await?,
                    _ => serde_json::json!({"error":"unsupported plugin action"}).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Knowledge {
                action,
                anchor_id,
                anchor_list,
                snapshot_id,
                r#type,
                output,
            } => {
                let anchor = anchor_id.unwrap_or_else(|| "cli:focus".into());
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &anchor,
                )
                .await?;
                let body = match action.as_str() {
                    "export" => app.export_knowledge(&anchor, &r#type).await?,
                    "batch-export" => {
                        let path = anchor_list.as_ref().ok_or_else(|| {
                            anyhow::anyhow!("--anchor-list is required for knowledge batch-export")
                        })?;
                        let anchors = parse_anchor_list(path)?;
                        let mut batch = Vec::with_capacity(anchors.len());
                        for anchor_id in anchors {
                            let payload = app.export_knowledge(&anchor_id, &r#type).await?;
                            let parsed = serde_json::from_str::<serde_json::Value>(&payload)
                                .unwrap_or_else(|_| serde_json::json!({"raw": payload}));
                            batch.push(serde_json::json!({
                                "anchor_id": anchor_id,
                                "type": r#type,
                                "payload": parsed,
                            }));
                        }
                        serde_json::to_string_pretty(&serde_json::json!({
                            "status": "ok",
                            "count": batch.len(),
                            "exports": batch,
                        }))?
                    }
                    "check" => app.focus_status(&anchor).await?,
                    "index" => app.export_knowledge(&anchor, "index").await?,
                    "replay-report" => {
                        app.export_replay_report(&anchor, snapshot_id.as_deref())
                            .await?
                    }
                    _ => serde_json::json!({"error":"unsupported plugin action"}).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Crawl {
                action,
                anchor_id,
                anchor,
                output,
            } => {
                let session = anchor_id.unwrap_or_else(|| "cli:focus".into());
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &session,
                )
                .await?;
                let body = match action.as_str() {
                    "run" => {
                        let anchor_text = anchor.unwrap_or_else(|| session.clone());
                        let report = app
                            .research
                            .run_anchor_research(&app.state_store(), &session, &anchor_text)
                            .await?;
                        let scheduled = app
                            .research
                            .schedule_follow_up_research(&app.state_store(), &session, &session, &report)
                            .await?;
                        serde_json::to_string_pretty(&serde_json::json!({
                            "report": report,
                            "scheduled_follow_ups": scheduled,
                        }))?
                    }
                    "status" => serde_json::json!({
                        "report": serde_json::from_str::<serde_json::Value>(&app.export_knowledge(&session, "research").await?)
                            .unwrap_or_else(|_| serde_json::json!({})),
                        "follow_up": serde_json::from_str::<serde_json::Value>(&app.export_knowledge(&session, "research-follow-up").await?)
                            .unwrap_or_else(|_| serde_json::json!({})),
                        "proxy_forensics": serde_json::from_str::<serde_json::Value>(&app.export_knowledge(&session, "research-proxy").await?)
                            .unwrap_or_else(|_| serde_json::json!({})),
                        "health": app.research.health_report(),
                        "backend": format!("{:?}", app.config.research.backend),
                        "live_fetch_enabled": app.config.research.live_fetch_enabled,
                        "dynamic_render": app.config.research.prefer_dynamic_render,
                        "proxy_pool_size": app.config.research.proxy_pool.len(),
                    }).to_string(),
                    "pause" => serde_json::json!({"status":"accepted","note":"crawl pause intent recorded; scheduled follow-ups can be drained by policy"}).to_string(),
                    "resume" => serde_json::json!({"status":"accepted","note":"crawl resume accepted; next run will continue autonomous research scheduling"}).to_string(),
                    _ => serde_json::json!({"error":"unsupported plugin action"}).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Trigger {
                action,
                anchor_id,
                schedule,
                topic,
                payload,
                actor,
                run_now,
                output,
            } => {
                let session = anchor_id.unwrap_or_else(|| "cli:focus".into());
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &session,
                )
                .await?;
                let body = match action.as_str() {
                    "set" => {
                        let topic = schedule.unwrap_or_else(|| "manual".into());
                        let target = "mcp::local-mcp::invoke".to_string();
                        let arg = payload.unwrap_or_else(|| "{}".into());
                        let created = app
                            .state_store()
                            .create_schedule_event(
                                session.clone(),
                                topic,
                                target,
                                arg,
                                session.clone(),
                            )
                            .await?;
                        serde_json::to_string_pretty(&created)?
                    }
                    "webhook" => {
                        let engine = TriggerRuntimeEngine::new(app.state_store().clone());
                        let webhook_topic = topic
                            .clone()
                            .or_else(|| schedule.clone())
                            .unwrap_or_else(|| "external".to_string());
                        let event = engine
                            .ingest_webhook_event(&session, &webhook_topic, payload.clone(), &actor)
                            .await?;
                        if run_now {
                            let app_ref = &app;
                            let run_report = engine
                                .run_worker_once(&session, |queued| {
                                    let target_session = queued.session_id.clone();
                                    let topic = queued.topic.clone();
                                    let payload = queued.payload.clone();
                                    async move {
                                        let prompt = if payload.trim().is_empty() {
                                            format!("Webhook trigger fired: {}", topic)
                                        } else {
                                            format!(
                                                "Webhook trigger fired: {}\nPayload: {}",
                                                topic, payload
                                            )
                                        };
                                        let _ = app_ref
                                            .process_direct(&target_session, &prompt)
                                            .await?;
                                        Ok(())
                                    }
                                })
                                .await?;
                            serde_json::to_string_pretty(&serde_json::json!({
                                "status": "webhook_ingested_and_executed",
                                "event": event,
                                "worker": run_report,
                            }))?
                        } else {
                            serde_json::to_string_pretty(&serde_json::json!({
                                "status": "webhook_ingested",
                                "event": event,
                            }))?
                        }
                    }
                    "list" => {
                        let outcome = command_registry
                            .dispatch(
                                "trigger.list",
                                app.clone(),
                                DispatchArgs {
                                    session_id: session.clone(),
                                    action: "list".to_string(),
                                    params: BTreeMap::new(),
                                },
                            )
                            .await?;
                        if let Some(body) = outcome.body {
                            body
                        } else {
                            serde_json::to_string_pretty(
                                &app.state_store().list_schedule_events(&session).await?,
                            )?
                        }
                    },
                    "run" => app.run_trigger_worker_once(&session).await?,
                    "daemon" => {
                        let interval_secs = schedule
                            .as_deref()
                            .and_then(|raw| raw.parse::<u64>().ok())
                            .unwrap_or(5);
                        println!(
                            "{}",
                            serde_json::json!({"status":"daemon_started","session":session,"interval_secs":interval_secs})
                        );
                        loop {
                            match app.run_trigger_worker_once(&session).await {
                                Ok(report) => println!("{report}"),
                                Err(error) => println!(
                                    "{}",
                                    serde_json::json!({"status":"daemon_tick_failed","session":session,"error":error.to_string()})
                                ),
                            }
                            sleep(Duration::from_secs(interval_secs)).await;
                        }
                    }
                    "cancel" => {
                        let events = app.state_store().list_schedule_events(&session).await?;
                        for event in events {
                            let _ = app
                                .state_store()
                                .update_schedule_status(event.id, "cancelled")
                                .await;
                        }
                        serde_json::json!({"status":"cancelled","session":session}).to_string()
                    }
                    _ => serde_json::json!({"error":"unsupported plugin action"}).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Bridge {
                action,
                anchor_id,
                transport_kind,
                subject,
                jwt,
                ttl_ms,
                output,
            } => {
                let session = anchor_id.unwrap_or_else(|| cli.session.clone());
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &session,
                )
                .await?;
                let lease = app.state_store().get_session_lease(&session).await?;
                let tenant_id = lease
                    .as_ref()
                    .map(|item| item.tenant_id.as_str())
                    .unwrap_or_else(|| identity_tenant.as_deref().unwrap_or("tenant:default"));
                let body = match action.as_str() {
                    "start" => {
                        app.bridge_start(&session, &transport_kind, &subject, tenant_id, ttl_ms)
                            .await?
                    }
                    "status" => {
                        let outcome = command_registry
                            .dispatch(
                                "bridge.status",
                                app.clone(),
                                DispatchArgs {
                                    session_id: session.clone(),
                                    action: "status".to_string(),
                                    params: BTreeMap::new(),
                                },
                            )
                            .await?;
                        outcome.body.unwrap_or(app.bridge_status(&session).await?)
                    },
                    "stop" => app.bridge_stop(&session).await?,
                    "issue-jwt" => {
                        app.bridge_issue_jwt(&session, &subject, tenant_id, ttl_ms)
                            .await?
                    }
                    "remote-start" => {
                        let token = jwt.as_deref().ok_or_else(|| {
                            anyhow::anyhow!("--jwt is required for bridge remote-start")
                        })?;
                        app.bridge_remote_start(&session, &transport_kind, token, ttl_ms)
                            .await?
                    }
                    "remote-status" => {
                        let outcome = command_registry
                            .dispatch(
                                "bridge.remote-status",
                                app.clone(),
                                DispatchArgs {
                                    session_id: session.clone(),
                                    action: "remote-status".to_string(),
                                    params: BTreeMap::new(),
                                },
                            )
                            .await?;
                        outcome.body.unwrap_or(app.bridge_remote_status(&session).await?)
                    },
                    "remote-stop" => app.bridge_remote_stop(&session).await?,
                    _ => serde_json::json!({"error":"unsupported plugin action"}).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Skill {
                action,
                skill,
                source,
                markdown,
                builder,
                output,
            } => {
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &cli.session,
                )
                .await?;
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0);
                let skill_id = skill.as_deref().unwrap_or("skill:default");
                let body = match action.as_str() {
                    "foundry" => {
                        let sub_action = skill.as_deref().unwrap_or("status").to_ascii_lowercase();
                        let mediation_call = autoloop::contracts::services::ServiceCall {
                            session_id: cli.session.clone().into(),
                            trace_id: format!("trace:foundry:{}:{}", sub_action, now_ms).into(),
                            service_domain: autoloop::contracts::services::ServiceDomain::SkillFoundry,
                            service_name: "skill_foundry".to_string(),
                            operation: sub_action.clone(),
                            input: serde_json::json!({
                                "builder": builder.as_deref().unwrap_or("foundry-skill"),
                                "source": source.as_deref().unwrap_or("manual://inline"),
                                "hint_id": source.as_deref().unwrap_or(""),
                                "markdown": markdown.as_deref().unwrap_or("json artifact"),
                                "requested_by": identity_principal.as_deref().unwrap_or("principal:operator"),
                            }),
                            budget_scope: "skill_foundry".to_string(),
                            requested_at_ms: now_ms,
                        };
                        let mediated = app.service_mediate(&mediation_call).await?;
                        if sub_action == "route" {
                            if let Ok(result) =
                                serde_json::from_str::<autoloop::contracts::services::ServiceResult>(&mediated)
                            {
                                if let Ok(pretty) = serde_json::to_string_pretty(&result.output) {
                                    let _ = std::fs::write("route_decision.json", pretty.as_bytes());
                                }
                            }
                        }
                        if sub_action == "validate" {
                            if let Ok(result) =
                                serde_json::from_str::<autoloop::contracts::services::ServiceResult>(&mediated)
                            {
                                if let Ok(pretty) = serde_json::to_string_pretty(&result.output) {
                                    let _ = std::fs::write("validation_report.json", pretty.as_bytes());
                                }
                            }
                        }
                        mediated
                    },
                    "list" => app.skill_list().await?,
                    "register" | "add" => {
                        let src = source.as_deref().unwrap_or("manual://inline");
                        let md = markdown.as_deref().unwrap_or("# skill\nno content");
                        app.skill_register(skill_id, src, md).await?
                    },
                    "install" => {
                        let route = autoloop::contracts::skill_foundry::RouteDecision {
                            decision_id: format!("route:skill-install:{}:{}", skill_id, now_ms),
                            selected_layer:
                                autoloop::contracts::skill_foundry::SkillFoundryLayer::S1PromptOnly,
                            risk_level: "low".to_string(),
                            confidence: 0.9,
                            reasons: vec!["manual skill install".to_string()],
                            rejected_layers: vec![],
                            policy_notes: vec!["manual_install=true".to_string()],
                            created_at_ms: now_ms,
                        };
                        let package = autoloop::skills::foundry::package_skill(
                            skill_id,
                            "v1",
                            &route,
                            "skills",
                            now_ms,
                        );
                        let src = source.as_deref().unwrap_or("manual://inline");
                        let md = markdown.as_deref().unwrap_or("# skill\nno content");
                        app.skill_install(&package, src, md).await?
                    },
                    "enable" => app
                        .skill_enable(skill_id, "skill enabled")
                        .await?,
                    "disable" => app
                        .skill_disable(skill_id, "skill disabled")
                        .await?,
                    "build" => {
                        let builder_id = builder.as_deref().unwrap_or("builtin:prompt-compiler");
                        app.skill_build(skill_id, builder_id).await?
                    },
                    "remove" | "retire" => app.skill_remove(skill_id).await?,
                    _ => serde_json::json!({
                        "error":"unsupported skill action",
                        "supported": [
                            "list",
                            "register|add",
                            "install",
                            "enable",
                            "disable",
                            "build",
                            "remove|retire",
                            "foundry <intake|extract|route|build|validate|package|install|enable|disable|approve_promotion>"
                        ]
                    }).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            },
            Commands::Org {
                action,
                anchor_id,
                output,
            } => {
                let session = anchor_id.unwrap_or_else(|| "cli:focus".into());
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &session,
                )
                .await?;
                let body = match action.as_str() {
                    "context" | "status" => app
                        .state_store()
                        .get_knowledge(&format!("org-context:{session}:latest"))
                        .await?
                        .map(|record| record.value)
                        .unwrap_or_else(|| "{}".to_string()),
                    "knowledge" => {
                        let lease = app.state_store().get_session_lease(&session).await?;
                        let tenant_id = lease
                            .map(|item| item.tenant_id)
                            .unwrap_or_else(|| "tenant:default".into());
                        serde_json::to_string_pretty(
                            &autoloop::rag::OrgKnowledgePublisher::snapshot(
                                &app.state_store(),
                                &tenant_id,
                            )
                            .await?,
                        )?
                    }
                    _ => serde_json::json!({"error":"unsupported plugin action"}).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Plugin {
                action,
                plugin,
                source,
                reason,
                traffic_percent,
                verify_signature,
                output,
            } => {
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &cli.session,
                )
                .await?;
                let lease = app.state_store().get_session_lease(&cli.session).await?;
                let tenant_id = lease
                    .as_ref()
                    .map(|item| item.tenant_id.as_str())
                    .unwrap_or_else(|| identity_tenant.as_deref().unwrap_or("tenant:default"));
                let operator = identity_principal
                    .as_deref()
                    .unwrap_or("principal:operator");
                let plugin_id = plugin.as_deref().unwrap_or("plugin:default");
                let body = match action.as_str() {
                    "list" => app.plugin_list().await?,
                    "status" => app.plugin_status(plugin_id).await?,
                    "install" | "add" => {
                        let plugin_source = source
                            .as_deref()
                            .unwrap_or("builtin://plugin-default#sig=0");
                        app.plugin_install(
                            plugin_id,
                            plugin_source,
                            operator,
                            tenant_id,
                            verify_signature,
                        )
                        .await?
                    }
                    "enable" => {
                        app.plugin_enable(
                            plugin_id,
                            operator,
                            reason.as_deref().unwrap_or("plugin enabled"),
                        )
                        .await?
                    }
                    "disable" | "remove" => {
                        app.plugin_disable(
                            plugin_id,
                            operator,
                            reason.as_deref().unwrap_or("plugin disabled"),
                        )
                        .await?
                    }
                    "update" => {
                        app.plugin_update(plugin_id, source.as_deref(), operator)
                            .await?
                    }
                    "rollback" => {
                        app.plugin_rollback(
                            plugin_id,
                            operator,
                            reason.as_deref().unwrap_or("plugin rollback"),
                        )
                        .await?
                    }
                    "shadow" => {
                        app.plugin_rollout(
                            plugin_id,
                            "shadow",
                            Some(0),
                            operator,
                            reason.as_deref(),
                        )
                        .await?
                    }
                    "canary" => {
                        app.plugin_rollout(
                            plugin_id,
                            "canary",
                            traffic_percent,
                            operator,
                            reason.as_deref(),
                        )
                        .await?
                    }
                    "full" => {
                        app.plugin_rollout(
                            plugin_id,
                            "full",
                            Some(100),
                            operator,
                            reason.as_deref(),
                        )
                        .await?
                    }
                    "rollback-fast" | "quick-rollback" => {
                        app.plugin_quick_rollback(plugin_id, operator).await?
                    }
                    "verify" => app.plugin_verify(plugin_id).await?,
                    "discover" | "parse" => {
                        let root = source
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("plugin discover requires --source <plugin-root-path>"))?;
                        app.plugin_discover_compat(root).await?
                    },
                    "host-status" => app.plugin_host_status().await?,
                    "host-load" => {
                        let entrypoint = source
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("plugin host-load requires --source <entrypoint> (e.g. proc://C:/path/to/plugin.exe)"))?;
                        app.plugin_host_load(plugin_id, entrypoint, operator)
                            .await?
                    }
                    "host-invoke" => {
                        let payload = match source.as_deref() {
                            Some(raw) => {
                                serde_json::from_str::<serde_json::Value>(raw).map_err(|error| {
                                    anyhow::anyhow!(
                                        "invalid --source payload json for host-invoke: {error}"
                                    )
                                })?
                            }
                            None => serde_json::json!({}),
                        };
                        let capability_id = reason.as_deref();
                        app.plugin_host_invoke(
                            plugin_id,
                            &cli.session,
                            tenant_id,
                            operator,
                            capability_id,
                            payload,
                        )
                        .await?
                    }
                    _ => serde_json::json!({"error":"unsupported plugin action"}).to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Session {
                action,
                name,
                output,
            } => {
                bind_identity_for_session(
                    &app,
                    identity_tenant.as_deref(),
                    identity_principal.as_deref(),
                    identity_policy.as_deref(),
                    identity_lease_ttl_ms,
                    &cli.session,
                )
                .await?;
                let body = match action.as_str() {
                    "new" => {
                        let target_session = name.as_deref().unwrap_or(&cli.session);
                        app.session_new(target_session).await?
                    }
                    "list" => app.session_list().await?,
                    "resume" => {
                        let target_session = name.as_deref().unwrap_or(&cli.session);
                        app.session_resume(target_session).await?
                    }
                    "named-snapshot" | "snapshot" => {
                        let snapshot_name = name
                            .as_deref()
                            .ok_or_else(|| anyhow::anyhow!("session named-snapshot requires --name <snapshot-name>"))?;
                        app.session_named_snapshot(&cli.session, snapshot_name).await?
                    }
                    "transcript-export" | "transcript" | "export-transcript" | "export" => {
                        app.session_export_transcript(&cli.session).await?
                    }
                    _ => serde_json::json!({
                        "error": "unsupported session action",
                        "supported": [
                            "new [--name <session-id>]",
                            "list",
                            "resume [--name <session-id>]",
                            "export-transcript",
                            "named-snapshot --name <name>",
                            "transcript-export"
                        ]
                    })
                    .to_string(),
                };
                write_or_print(output.as_ref(), &body)?;
            }
            Commands::Background {
                action,
                task_id,
                kind,
                command,
                prompt,
                max_restarts,
                lines,
                output,
            } => {
                command_dispatch::dispatch_background(
                    &app,
                    &command_dispatch::IdentityInput {
                        tenant: identity_tenant.clone(),
                        principal: identity_principal.clone(),
                        policy: identity_policy.clone(),
                        lease_ttl_ms: identity_lease_ttl_ms,
                    },
                    command_dispatch::BackgroundDispatchInput {
                        session_id: cli.session.clone(),
                        action,
                        task_id,
                        kind,
                        command,
                        prompt,
                        max_restarts,
                        lines,
                        output,
                    },
                )
                .await?;
            }
            Commands::Memory {
                plane,
                action,
                file,
                review_id,
                operator,
                reason,
                repo_root,
                clean,
                no_infer,
                report,
                save,
                refresh_mode,
                page,
                page_size,
                output,
            } => {
                command_dispatch::dispatch_memory(
                    &app,
                    &command_dispatch::IdentityInput {
                        tenant: identity_tenant.clone(),
                        principal: identity_principal.clone(),
                        policy: identity_policy.clone(),
                        lease_ttl_ms: identity_lease_ttl_ms,
                    },
                    command_dispatch::MemoryDispatchInput {
                        session_id: cli.session.clone(),
                        plane,
                        action,
                        file,
                        review_id,
                        operator,
                        reason,
                        repo_root,
                        clean,
                        no_infer,
                        report,
                        save,
                        refresh_mode,
                        page,
                        page_size,
                        output,
                    },
                )
                .await?;
            }
            Commands::Frontend {
                action,
                content,
                trace_id,
                request_id,
                decision,
                reason,
                jwt,
                transport_kind,
                ttl_ms,
                subject,
                tenant_id,
                format,
                limit,
                output,
            } => {
                command_dispatch::dispatch_frontend(
                    &app,
                    &command_registry,
                    &command_dispatch::IdentityInput {
                        tenant: identity_tenant.clone(),
                        principal: identity_principal.clone(),
                        policy: identity_policy.clone(),
                        lease_ttl_ms: identity_lease_ttl_ms,
                    },
                    command_dispatch::FrontendDispatchInput {
                        session_id: cli.session.clone(),
                        action,
                        content,
                        trace_id,
                        request_id,
                        decision,
                        reason,
                        jwt,
                        transport_kind,
                        ttl_ms,
                        subject,
                        tenant_id,
                        format,
                        limit,
                        output,
                    },
                )
                .await?;
            }
            Commands::Chat {} => {
                command_dispatch::dispatch_chat(
                    &app,
                    &command_registry,
                    &command_dispatch::IdentityInput {
                        tenant: identity_tenant.clone(),
                        principal: identity_principal.clone(),
                        policy: identity_policy.clone(),
                        lease_ttl_ms: identity_lease_ttl_ms,
                    },
                    &cli.session,
                )
                .await?;
            }
            Commands::System {
                action,
                subaction,
                snapshot_id,
                trace_id,
                artifact_ref,
                artifact_path,
                limit,
                profile,
                fault,
                reason,
                task_utility,
                distortion_penalty,
                attention_mismatch_penalty,
                token_cost_penalty,
                anchor_list,
                output,
                host,
                port,
            } => {
                command_dispatch::dispatch_system(
                    &app,
                    &command_registry,
                    &command_dispatch::IdentityInput {
                        tenant: identity_tenant.clone(),
                        principal: identity_principal.clone(),
                        policy: identity_policy.clone(),
                        lease_ttl_ms: identity_lease_ttl_ms,
                    },
                    command_dispatch::SystemDispatchInput {
                        session_id: cli.session.clone(),
                        action,
                        subaction,
                        snapshot_id,
                        trace_id,
                        artifact_ref,
                        artifact_path,
                        limit,
                        profile,
                        fault,
                        reason,
                        task_utility,
                        distortion_penalty,
                        attention_mismatch_penalty,
                        token_cost_penalty,
                        anchor_list,
                        output,
                        host,
                        port,
                    },
                )
                .await?;
            }
        Commands::Tui { mode } => {
            if let Some(m) = &mode {
                println!("Starting OntoLoop TUI in {} mode...", m);
            } else {
                println!("Starting OntoLoop TUI (Lite mode)...");
            }
            autoloop::tui::run_tui(tui_config.clone()).await?;
        }
        }
    } else if let Some(message) = cli.message {
        bind_identity_for_session(
            &app,
            identity_tenant.as_deref(),
            identity_principal.as_deref(),
            identity_policy.as_deref(),
            identity_lease_ttl_ms,
            &cli.session,
        )
        .await?;
        let response = if cli.swarm {
            app.process_requirement_swarm(&cli.session, &message)
                .await?
        } else {
            app.process_direct(&cli.session, &message).await?
        };
        println!("{response}");
    } else {
        // Default: launch TUI
        autoloop::tui::run_tui(tui_config.clone()).await?;
    }

    Ok(())
}

async fn run_smoke_frontend_task(task: &str) -> Result<()> {
    let smoke_profile = std::env::var("AUTOLOOP_SMOKE_PROFILE")
        .ok()
        .or_else(|| std::env::var("AUTOLOOP_PROFILE").ok());
    let smoke_config_path = std::env::var("AUTOLOOP_SMOKE_CONFIG")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let candidate = PathBuf::from("deploy/config/autoloop.prod.toml");
            candidate.exists().then_some(candidate)
        });
    let config =
        AppConfig::load_with_profile_hint(smoke_config_path.as_deref(), smoke_profile.as_deref())?;
    let mut config = config;
    let tui_config = config.clone();
apply_provider_env_overrides(&mut config);
    let app = Arc::new(AutoLoopApp::try_new(config)?);
    let session_id = "smoke:frontend";
    let trace_id = format!(
        "trace:{session_id}:{}",
        autoloop::orchestration::current_time_ms()
    );

    app.ensure_session_identity(
        session_id,
        "tenant:smoke",
        "principal:smoke",
        "policy:smoke",
        3_600_000,
    )
    .await?;
    app.frontend_attach(
        session_id,
        "cli",
        None,
        Some("bridge:smoke"),
        Some("tenant:smoke"),
        3_600_000,
    )
    .await?;

    let app_for_prompt = app.clone();
    let session_for_prompt = session_id.to_string();
    let trace_for_prompt = trace_id.clone();
    let task_for_prompt = task.to_string();
    let prompt = tokio::spawn(async move {
        app_for_prompt
            .frontend_bridge_prompt(&session_for_prompt, Some(&trace_for_prompt), &task_for_prompt)
            .await
    })
    .await??;
    let prompt_json =
        serde_json::from_str::<serde_json::Value>(&prompt).unwrap_or_else(|_| serde_json::json!({}));
    let query_view = build_unified_query_view(&app.state_store(), session_id, Some(&trace_id)).await?;

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "mode": "smoke_frontend_task",
            "session_id": session_id,
            "trace_id": trace_id,
            "task": task,
            "prompt_result": prompt_json,
            "event_count": query_view.events.as_array().map(|items| items.len()).unwrap_or_default(),
            "mismatch_count": query_view
                .replay
                .get("mismatch_explain")
                .and_then(serde_json::Value::as_array)
                .map(|items| items.len())
                .unwrap_or(0),
        }))?
    );
    Ok(())
}

fn apply_provider_env_overrides(config: &mut AppConfig) {
    let api_base = std::env::var("AUTOLOOP_API_BASE")
        .ok()
        .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
        .or_else(|| std::env::var("AUTOLOOP_SMOKE_API_BASE").ok());
    if let Some(api_base) = api_base {
        if !api_base.trim().is_empty() {
            config.providers.api_base_url = api_base;
        }
    }

    let model = std::env::var("AUTOLOOP_MODEL")
        .ok()
        .or_else(|| std::env::var("AUTOLOOP_SMOKE_MODEL").ok());
    if let Some(model) = model {
        if !model.trim().is_empty() {
            config.providers.default_model = model.clone();
            config.providers.reasoning_model = model.clone();
            config.providers.screening_model = model;
        }
    }

    let judge_model = std::env::var("AUTOLOOP_JUDGE_MODEL")
        .ok()
        .or_else(|| std::env::var("AUTOLOOP_MODEL").ok())
        .or_else(|| std::env::var("AUTOLOOP_SMOKE_MODEL").ok());
    if let Some(judge_model) = judge_model {
        if !judge_model.trim().is_empty() {
            config.providers.judge_model = judge_model;
        }
    }
}

async fn run_chat_repl(
    app: &Arc<AutoLoopApp>,
    session_id: &str,
    command_registry: &BuiltinCommandRegistry,
) -> Result<()> {
    println!("OntoLoop chat started (session={session_id}).");
    println!("Type /help for commands. Exit with /exit, /quit, exit, or quit.");

    let mut input = String::new();
    let mut last_sequence = 0u64;
    loop {
        print!("you> ");
        io::stdout().flush()?;

        input.clear();
        let read = io::stdin().read_line(&mut input)?;
        if read == 0 {
            println!("\nchat closed (EOF).");
            break;
        }

        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }

        let chat_trace_id = format!(
            "trace:{session_id}:cli:chat:{}",
            autoloop::orchestration::current_time_ms()
        );
        let _ = append_cli_query_event(
            app,
            session_id,
            &chat_trace_id,
            "cli.chat.input",
            serde_json::json!({
                "content_len": trimmed.len(),
                "is_command": trimmed.starts_with('/'),
            }),
        )
        .await;

        if matches!(trimmed, "/exit" | "/quit" | "exit" | "quit") {
            println!("bye.");
            break;
        }

        if trimmed == "/help" {
            println!("commands:");
            println!("  /help  show this help");
            println!("  /exit  exit chat");
            println!("  /quit  exit chat");
            println!("  /status | /health | /bridge | /trigger [list] | /session [status]");
            println!("  /command <status|health|bridge|trigger list|session status>");
            continue;
        }

        if let Some(command_output) =
            handle_chat_command(app, command_registry, session_id, trimmed).await?
        {
            println!("assistant(command)> {command_output}");
            let _ = append_cli_query_event(
                app,
                session_id,
                &chat_trace_id,
                "cli.chat.command.output",
                serde_json::json!({
                    "output_len": command_output.len(),
                }),
            )
            .await;
            continue;
        }

        let trace_id = format!(
            "trace:{session_id}:chat:{}",
            autoloop::orchestration::current_time_ms()
        );
        let app_for_task = app.clone();
        let session_for_task = session_id.to_string();
        let input_for_task = trimmed.to_string();
        let trace_for_task = trace_id.clone();
        let prompt_task = tokio::spawn(async move {
            app_for_task
                .frontend_bridge_prompt(&session_for_task, Some(&trace_for_task), &input_for_task)
                .await
        });

        let mut printed_prefix = false;
        let mut printed_delta = false;
        loop {
            let events = app.transport.replay_session_events_v2(session_id).await?;
            for event in events {
                if event.sequence <= last_sequence {
                    continue;
                }
                last_sequence = event.sequence;

                match event.event_type {
                    autoloop::contracts::transport::SessionEventType::AssistantDelta => {
                        if !printed_prefix {
                            print!("assistant> ");
                            io::stdout().flush()?;
                            printed_prefix = true;
                        }
                        if let Some(delta) = event.payload.get("delta").and_then(serde_json::Value::as_str)
                        {
                            print!("{delta}");
                            io::stdout().flush()?;
                            printed_delta = true;
                        }
                    }
                    autoloop::contracts::transport::SessionEventType::StateSnapshot => {
                        let is_idle = event
                            .payload
                            .get("status")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|status| status == "idle");
                        if is_idle && printed_prefix {
                            println!();
                            printed_prefix = false;
                        }
                    }
                    autoloop::contracts::transport::SessionEventType::ToolStarted => {
                        if printed_prefix {
                            println!();
                            printed_prefix = false;
                        }
                        let tool_name = event
                            .payload
                            .get("tool_name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown_tool");
                        let call_id = event
                            .payload
                            .get("call_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown_call");
                        println!("tool(started)> tool={tool_name} call_id={call_id}");
                    }
                    autoloop::contracts::transport::SessionEventType::ToolCompleted => {
                        if printed_prefix {
                            println!();
                            printed_prefix = false;
                        }
                        let tool_name = event
                            .payload
                            .get("tool_name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown_tool");
                        let call_id = event
                            .payload
                            .get("call_id")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown_call");
                        let is_error = event
                            .payload
                            .get("is_error")
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false);
                        if is_error {
                            println!("tool(error)> tool={tool_name} call_id={call_id}");
                        } else {
                            println!("tool(completed)> tool={tool_name} call_id={call_id}");
                        }
                    }
                    _ => {}
                }
            }

            if prompt_task.is_finished() {
                break;
            }
            sleep(Duration::from_millis(35)).await;
        }

        let response_raw = prompt_task.await??;
        let response_json = serde_json::from_str::<serde_json::Value>(&response_raw)
            .unwrap_or_else(|_| serde_json::json!({ "response": response_raw }));

        if response_json
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|status| status == "requires_approval")
        {
            let request_id = response_json
                .get("request_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            if request_id.is_empty() {
                eprintln!("assistant(error)> permission request missing request_id");
                continue;
            }
            print!("permission> approve request {request_id}? [y/N]: ");
            io::stdout().flush()?;
            let mut decision_input = String::new();
            io::stdin().read_line(&mut decision_input)?;
            let approved = matches!(
                decision_input.trim().to_ascii_lowercase().as_str(),
                "y" | "yes" | "approve" | "a"
            );
            let decision = if approved { "approve" } else { "reject" };
            let decision_result = app
                .frontend_permission_decide(
                    session_id,
                    &request_id,
                    decision,
                    Some("chat operator decision"),
                )
                .await?;
            let decision_json = serde_json::from_str::<serde_json::Value>(&decision_result)
                .unwrap_or_else(|_| serde_json::json!({ "raw": decision_result }));
            if approved {
                if let Some(execution) = decision_json.get("execution") {
                    if let Some(response) =
                        execution.get("response").and_then(serde_json::Value::as_str)
                    {
                        println!("assistant> {response}");
                    } else {
                        println!("assistant> {execution}");
                    }
                } else {
                    println!("assistant> permission approved");
                }
            } else {
                println!("assistant> request rejected");
            }
            let _ = append_cli_query_event(
                app,
                session_id,
                &trace_id,
                "cli.chat.permission.decision",
                serde_json::json!({
                    "approved": approved,
                    "request_id": request_id,
                }),
            )
            .await;
            continue;
        }

        if !printed_delta {
            if let Some(response) = response_json.get("response").and_then(serde_json::Value::as_str) {
                println!("assistant> {response}");
                continue;
            }
        }

        if let Some(error_message) = response_json.get("error").and_then(serde_json::Value::as_str) {
            eprintln!("assistant(error)> {error_message}");
            let _ = append_cli_query_event(
                app,
                session_id,
                &trace_id,
                "cli.chat.response.error",
                serde_json::json!({
                    "message": error_message,
                }),
            )
            .await;
        } else {
            let _ = append_cli_query_event(
                app,
                session_id,
                &trace_id,
                "cli.chat.response.ok",
                serde_json::json!({
                    "status": response_json
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("ok"),
                    "call_id": response_json.get("call_id").cloned().unwrap_or(serde_json::Value::Null),
                }),
            )
            .await;
        }
    }

    Ok(())
}

async fn handle_chat_command(
    app: &Arc<AutoLoopApp>,
    command_registry: &BuiltinCommandRegistry,
    session_id: &str,
    raw: &str,
) -> Result<Option<String>> {
    let command_line = if let Some(rest) = raw.strip_prefix("/command") {
        rest.trim()
    } else if raw.starts_with('/') {
        raw.trim_start_matches('/')
    } else {
        return Ok(None);
    };

    if command_line.is_empty() {
        return Ok(Some(
            "usage: /command <status|health|bridge|trigger list|session status>".to_string(),
        ));
    }

    let tokens = command_line.split_whitespace().collect::<Vec<_>>();
    let head = tokens[0].to_ascii_lowercase();
    let body = match head.as_str() {
        "status" => {
            let outcome = command_registry
                .dispatch(
                    "system.status",
                    app.clone(),
                    DispatchArgs {
                        session_id: session_id.to_string(),
                        action: "status".to_string(),
                        params: BTreeMap::new(),
                    },
                )
                .await?;
            outcome.body.unwrap_or(app.system_status().await?)
        }
        "health" => {
            let outcome = command_registry
                .dispatch(
                    "system.health",
                    app.clone(),
                    DispatchArgs {
                        session_id: session_id.to_string(),
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
        "bridge" => {
            let outcome = command_registry
                .dispatch(
                    "bridge.status",
                    app.clone(),
                    DispatchArgs {
                        session_id: session_id.to_string(),
                        action: "status".to_string(),
                        params: BTreeMap::new(),
                    },
                )
                .await?;
            outcome.body.unwrap_or(app.bridge_status(session_id).await?)
        }
        "trigger" => {
            let sub = tokens.get(1).copied().unwrap_or("list").to_ascii_lowercase();
            if sub != "list" {
                return Ok(Some("supported trigger command in chat: /trigger list".to_string()));
            }
            let outcome = command_registry
                .dispatch(
                    "trigger.list",
                    app.clone(),
                    DispatchArgs {
                        session_id: session_id.to_string(),
                        action: "list".to_string(),
                        params: BTreeMap::new(),
                    },
                )
                .await?;
            if let Some(body) = outcome.body {
                body
            } else {
                serde_json::to_string_pretty(&app.state_store().list_schedule_events(session_id).await?)?
            }
        }
        "session" => {
            let sub = tokens.get(1).copied().unwrap_or("status").to_ascii_lowercase();
            if sub != "status" {
                return Ok(Some("supported session command in chat: /session status".to_string()));
            }
            let lease = app.state_store().get_session_lease(session_id).await?;
            let events = app.transport.replay_session_events_v2(session_id).await?;
            serde_json::to_string_pretty(&serde_json::json!({
                "session_id": session_id,
                "lease": lease,
                "bridge": serde_json::from_str::<serde_json::Value>(&app.bridge_status(session_id).await?)
                    .unwrap_or_else(|_| serde_json::json!({})),
                "transport_event_count": events.len(),
                "latest_event": events.last(),
            }))?
        }
        _ => {
            return Ok(Some(
                "unsupported command. try: /status /health /bridge /trigger list /session status"
                    .to_string(),
            ));
        }
    };

    let _ = append_cli_query_event(
        app,
        session_id,
        &format!(
            "trace:{session_id}:cli:command:{}",
            autoloop::orchestration::current_time_ms()
        ),
        "cli.command.executed",
        serde_json::json!({
            "command_line": command_line,
            "head": head,
            "output_len": body.len(),
        }),
    )
    .await;

    Ok(Some(body))
}

async fn append_cli_query_event(
    app: &Arc<AutoLoopApp>,
    session_id: &str,
    trace_id: &str,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    let created_at_ms = autoloop::orchestration::current_time_ms();
    let cli_record = serde_json::json!({
        "kind": kind,
        "trace_id": trace_id,
        "session_id": session_id,
        "payload": payload,
        "created_at_ms": created_at_ms,
    });
    let _ = app
        .state_store()
        .upsert_json_knowledge(
            format!("observability:cli-event:{session_id}:{created_at_ms}"),
            &cli_record,
            "cli-observability",
        )
        .await;
    let _ = app
        .state_store()
        .upsert_json_knowledge(
            format!("observability:cli-event:{session_id}:latest"),
            &cli_record,
            "cli-observability",
        )
        .await;

    let _ = append_event(
        app.state_store(),
        kind,
        trace_id.to_string(),
        session_id.to_string(),
        None,
        Some("cli.frontend".to_string()),
        autoloop::contracts::version::CONTRACT_VERSION,
        payload,
    )
    .await;
    Ok(())
}

fn write_or_print(output: Option<&PathBuf>, body: &str) -> Result<()> {
    if let Some(path) = output {
        fs::write(path, body)?;
    } else {
        println!("{body}");
    }
    Ok(())
}

fn compose_anchor_request(
    anchor: &str,
    time: Option<&str>,
    region: Option<&str>,
    core_source: Option<&str>,
    update_cycle: Option<&str>,
) -> String {
    let mut parts = vec![format!("Focus anchor: {anchor}")];
    if let Some(time) = time {
        parts.push(format!("Time range: {time}"));
    }
    if let Some(region) = region {
        parts.push(format!("Region: {region}"));
    }
    if let Some(core_source) = core_source {
        parts.push(format!("Core source preference: {core_source}"));
    }
    if let Some(update_cycle) = update_cycle {
        parts.push(format!("Update cycle: {update_cycle}"));
    }
    parts.join("\n")
}

fn parse_anchor_list(path: &PathBuf) -> Result<Vec<String>> {
    let raw = fs::read_to_string(path)?;
    let mut anchors = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect::<Vec<_>>();
    anchors.sort();
    anchors.dedup();
    Ok(anchors)
}

async fn system_signal_status_view(app: &AutoLoopApp, session_id: &str) -> Result<serde_json::Value> {
    let db = app.state_store();
    let pipeline_cfg = autoloop::config::SignalPipelineConfig::default();
    let facade = autoloop::observability::SignalFacade::new(db.clone(), &pipeline_cfg);

    let latest_event = db
        .get_knowledge(&format!("signal:events:{session_id}:latest"))
        .await?
        .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let latest_explain = db
        .get_knowledge(&format!("observability:signal-explain:{session_id}:latest"))
        .await?
        .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let event_count = db
        .list_knowledge_by_prefix(&format!("signal:events:{session_id}:"))
        .await?
        .into_iter()
        .filter(|item| !item.key.ends_with(":latest"))
        .count();
    let explain_count = db
        .list_knowledge_by_prefix(&format!("observability:signal-explain:{session_id}:"))
        .await?
        .into_iter()
        .filter(|item| !item.key.ends_with(":latest"))
        .count();

    Ok(serde_json::json!({
        "session_id": session_id,
        "mode": format!("{:?}", pipeline_cfg.mode).to_ascii_lowercase(),
        "batch_size": pipeline_cfg.batch_size,
        "max_retries": pipeline_cfg.max_retries,
        "retry_backoff_ms": pipeline_cfg.retry_backoff_ms,
        "pending_in_process": facade.pending_len(),
        "persisted": {
            "event_count": event_count,
            "explain_count": explain_count,
            "latest_event": latest_event,
            "latest_explain": latest_explain,
        }
    }))
}

async fn system_signal_explain_view(
    app: &AutoLoopApp,
    session_id: &str,
    trace_id: Option<&str>,
) -> Result<serde_json::Value> {
    let mut items = app
        .state_store()
        .list_knowledge_by_prefix(&format!("observability:signal-explain:{session_id}:"))
        .await?
        .into_iter()
        .filter(|record| !record.key.ends_with(":latest"))
        .filter_map(|record| {
            serde_json::from_str::<serde_json::Value>(&record.value)
                .ok()
                .map(|value| serde_json::json!({"key": record.key, "value": value}))
        })
        .collect::<Vec<_>>();
    if let Some(trace_id) = trace_id {
        items.retain(|item| {
            item.get("value")
                .and_then(|value| value.get("trace_id"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|candidate| candidate == trace_id)
        });
    }
    items.sort_by_key(|item| {
        item.get("value")
            .and_then(|value| value.get("created_at_ms"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });

    let mut reason_counts = BTreeMap::<String, usize>::new();
    for item in &items {
        if let Some(code) = item
            .get("value")
            .and_then(|value| value.get("reason_code"))
            .and_then(serde_json::Value::as_str)
        {
            *reason_counts.entry(code.to_string()).or_insert(0) += 1;
        }
    }

    Ok(serde_json::json!({
        "session_id": session_id,
        "trace_filter": trace_id,
        "count": items.len(),
        "reason_counts": reason_counts,
        "items": items,
    }))
}

async fn system_signal_drain_view(app: &AutoLoopApp, session_id: &str) -> Result<serde_json::Value> {
    let db = app.state_store();
    let facade =
        autoloop::observability::SignalFacade::new(db.clone(), &autoloop::config::SignalPipelineConfig::default());
    let pending_before = facade.pending_len();
    let drained = facade.shutdown_flush().await?;
    let pending_after = facade.pending_len();
    Ok(serde_json::json!({
        "session_id": session_id,
        "operation": "shutdown_flush",
        "pending_before": pending_before,
        "pending_after": pending_after,
        "requested": drained.requested,
        "flushed": drained.flushed,
        "outputs": drained.outputs,
    }))
}

async fn system_relation_status_view(
    app: &AutoLoopApp,
    session_id: &str,
    limit: usize,
) -> Result<serde_json::Value> {
    let db = app.state_store();
    let edges = db.list_relation_edges_current(session_id, limit).await?;
    let events = db.list_relation_events(session_id, limit).await?;
    let hot_index = db.list_relation_hot_index(session_id, limit).await?;
    let latest_write_proof = db
        .list_knowledge_by_prefix(&format!("relation:write_proof:{session_id}:"))
        .await?
        .into_iter()
        .max_by(|left, right| left.key.cmp(&right.key))
        .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.value).ok());

    let blocked_count = events
        .iter()
        .filter(|event| event.event_type.eq_ignore_ascii_case("edge_rejected"))
        .count();
    let allowed_count = events
        .iter()
        .filter(|event| {
            event.event_type.eq_ignore_ascii_case("edge_upserted")
                || event.event_type.eq_ignore_ascii_case("node_upserted")
        })
        .count();

    Ok(serde_json::json!({
        "session_id": session_id,
        "limit": limit,
        "counts": {
            "edges": edges.len(),
            "events": events.len(),
            "hot_index": hot_index.len(),
            "blocked": blocked_count,
            "allowed": allowed_count,
        },
        "latest": {
            "event": events
                .iter()
                .max_by_key(|item| item.created_at_ms)
                .map(|item| serde_json::to_value(item).unwrap_or_else(|_| serde_json::json!({})))
                .unwrap_or_else(|| serde_json::json!({})),
            "write_proof": latest_write_proof.unwrap_or_else(|| serde_json::json!({})),
        }
    }))
}

async fn system_relation_explain_view(
    app: &AutoLoopApp,
    session_id: &str,
    trace_id: Option<&str>,
    limit: usize,
) -> Result<serde_json::Value> {
    let mut events = app.state_store().list_relation_events(session_id, limit).await?;
    if let Some(trace) = trace_id {
        events.retain(|item| item.trace_id == trace);
    }
    events.sort_by_key(|item| item.created_at_ms);

    let mut reason_counts = BTreeMap::<String, usize>::new();
    let explained = events
        .into_iter()
        .map(|event| {
            let reason_code = event
                .payload
                .get("reason")
                .and_then(|value| value.get("code"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| {
                    if event.event_type.eq_ignore_ascii_case("edge_rejected") {
                        "blocked"
                    } else {
                        "allowed"
                    }
                })
                .to_string();
            *reason_counts.entry(reason_code.clone()).or_insert(0) += 1;
            let deny_reason = event
                .payload
                .get("reason")
                .and_then(|value| value.get("deny_reason"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let evidence_ref = event
                .payload
                .get("reason")
                .and_then(|value| value.get("evidence_ref"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or(Some(event.evidence_ref.clone()));

            serde_json::json!({
                "event_id": event.event_id,
                "trace_id": event.trace_id,
                "event_type": event.event_type,
                "decision": if event.event_type.eq_ignore_ascii_case("edge_rejected") { "blocked" } else { "allowed" },
                "reason_code": reason_code,
                "deny_reason": deny_reason,
                "evidence_ref": evidence_ref,
                "payload": event.payload,
                "created_at_ms": event.created_at_ms,
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "session_id": session_id,
        "trace_filter": trace_id,
        "count": explained.len(),
        "reason_counts": reason_counts,
        "items": explained,
    }))
}

async fn system_relation_graph_view(
    app: &AutoLoopApp,
    session_id: &str,
    trace_id: Option<&str>,
    limit: usize,
) -> Result<serde_json::Value> {
    let mut edges = app
        .state_store()
        .list_relation_edges_current(session_id, limit)
        .await?;
    if let Some(trace) = trace_id {
        edges.retain(|edge| edge.trace_id == trace);
    }

    let mut nodes = BTreeSet::<String>::new();
    let edge_items = edges
        .into_iter()
        .map(|edge| {
            nodes.insert(edge.from_node.clone());
            nodes.insert(edge.to_node.clone());
            let reason = edge
                .payload
                .get("reason")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            serde_json::json!({
                "edge_id": edge.edge_id,
                "from": edge.from_node,
                "to": edge.to_node,
                "edge_type": edge.edge_type,
                "trace_id": edge.trace_id,
                "evidence_ref": edge.evidence_ref,
                "reason": reason,
                "updated_at_ms": edge.updated_at_ms,
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "session_id": session_id,
        "trace_filter": trace_id,
        "graph": {
            "node_count": nodes.len(),
            "edge_count": edge_items.len(),
            "nodes": nodes.into_iter().collect::<Vec<_>>(),
            "edges": edge_items,
        }
    }))
}

async fn system_relation_check_view(
    app: &AutoLoopApp,
    session_id: &str,
    trace_id: Option<&str>,
    limit: usize,
    enqueue_repair: bool,
    repair_reason: Option<&str>,
) -> Result<serde_json::Value> {
    let edges = app
        .state_store()
        .list_relation_edges_current(session_id, limit)
        .await?;
    let mut events = app.state_store().list_relation_events(session_id, limit).await?;
    if let Some(filter_trace_id) = trace_id {
        events.retain(|event| event.trace_id == filter_trace_id);
    }
    let edge_ids = edges
        .iter()
        .map(|item| item.edge_id.clone())
        .collect::<BTreeSet<_>>();

    let missing_evidence_edges = edges
        .iter()
        .filter(|item| item.evidence_ref.trim().is_empty())
        .map(|item| item.edge_id.clone())
        .collect::<Vec<_>>();
    let missing_evidence_events = events
        .iter()
        .filter(|item| item.evidence_ref.trim().is_empty())
        .map(|item| item.event_id.clone())
        .collect::<Vec<_>>();
    let events_with_missing_edge_ref = events
        .iter()
        .filter_map(|item| {
            let referenced = item
                .payload
                .get("edge_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            match referenced {
                Some(edge_id) if !edge_ids.contains(&edge_id) => Some(serde_json::json!({
                    "event_id": item.event_id,
                    "edge_id": edge_id,
                    "trace_id": item.trace_id,
                })),
                _ => None,
            }
        })
        .collect::<Vec<_>>();

    let orphan_edges = edges
        .iter()
        .filter_map(|item| {
            if item.from_node.trim().is_empty() || item.to_node.trim().is_empty() {
                Some(serde_json::json!({
                    "edge_id": item.edge_id,
                    "reason": "edge endpoint is empty",
                    "trace_id": item.trace_id,
                }))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut pair_types: HashMap<(String, String), HashSet<String>> = HashMap::new();
    let mut duplicate_counter: HashMap<(String, String, String), Vec<String>> = HashMap::new();
    for edge in &edges {
        let from = edge.from_node.trim().to_string();
        let to = edge.to_node.trim().to_string();
        let kind = normalize_relation_kind(&edge.edge_type);
        pair_types
            .entry((from.clone(), to.clone()))
            .or_default()
            .insert(kind.clone());
        duplicate_counter
            .entry((from, to, kind))
            .or_default()
            .push(edge.edge_id.clone());
    }

    let conflicting_relations = pair_types
        .iter()
        .filter_map(|((from, to), kinds)| {
            if kinds.contains("approvedby") && kinds.contains("blockedby") {
                Some(serde_json::json!({
                    "from_node": from,
                    "to_node": to,
                    "conflict": "approved_by_vs_blocked_by",
                    "kinds": kinds,
                }))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let duplicate_relations = duplicate_counter
        .iter()
        .filter_map(|((from, to, kind), ids)| {
            if ids.len() > 1 {
                Some(serde_json::json!({
                    "from_node": from,
                    "to_node": to,
                    "edge_type": kind,
                    "edge_ids": ids,
                }))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let cycle_paths = detect_relation_cycles(&edges);

    let healthy = missing_evidence_edges.is_empty()
        && missing_evidence_events.is_empty()
        && events_with_missing_edge_ref.is_empty()
        && orphan_edges.is_empty()
        && conflicting_relations.is_empty()
        && duplicate_relations.is_empty()
        && cycle_paths.is_empty();

    let now_ms = autoloop::orchestration::current_time_ms();
    let replay_fp = relation_replay_fingerprint(
        session_id,
        trace_id.unwrap_or("trace:any"),
        now_ms,
        edges.len(),
        events.len(),
    );
    let report = serde_json::json!({
        "session_id": session_id,
        "trace_filter": trace_id,
        "healthy": healthy,
        "replay_fp": replay_fp,
        "generated_at_ms": now_ms,
        "checks": {
            "missing_evidence_edges": missing_evidence_edges,
            "missing_evidence_events": missing_evidence_events,
            "events_with_missing_edge_ref": events_with_missing_edge_ref,
            "orphan_edges": orphan_edges,
            "conflicting_relations": conflicting_relations,
            "duplicate_relations": duplicate_relations,
            "cycle_paths": cycle_paths,
        },
        "summary": if healthy {
            "relation checks passed"
        } else {
            "relation checks found consistency gaps"
        }
    });

    app.state_store()
        .upsert_json_knowledge(
            format!("memory:relation:consistency:{session_id}:{now_ms}"),
            &report,
            "relation-consistency-check",
        )
        .await?;
    app.state_store()
        .upsert_json_knowledge(
            format!("memory:relation:consistency:{session_id}:latest"),
            &report,
            "relation-consistency-check",
        )
        .await?;

    let repair = if enqueue_repair && !healthy {
        let proposal_ref = format!("relation-consistency-proposal:{session_id}:{now_ms}");
        let patch = build_relation_repair_patch(session_id, &report, repair_reason);
        let review = PatchReviewQueue::enqueue_relation_repair_proposal(
            &app.state_store(),
            session_id,
            trace_id.unwrap_or("trace:relation:repair"),
            &patch,
            &proposal_ref,
        )
        .await?;
        Some(serde_json::json!({
            "queued": true,
            "proposal_ref": proposal_ref,
            "review_id": review.review_id,
            "status": review.status,
            "review_kind": review.review_kind,
            "approval_required": review.decision.approval_required,
            "risk_score": review.decision.risk_score,
        }))
    } else if enqueue_repair {
        Some(serde_json::json!({
            "queued": false,
            "reason": "no_consistency_issue_detected",
        }))
    } else {
        None
    };

    let mut output = report
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    if let Some(repair_info) = repair {
        output.insert("repair_proposal".into(), repair_info);
    }
    Ok(serde_json::Value::Object(output))
}

fn build_relation_repair_patch(
    session_id: &str,
    report: &serde_json::Value,
    reason: Option<&str>,
) -> PatchPlan {
    let mut ops = Vec::new();
    if let Some(orphan_edges) = report
        .get("checks")
        .and_then(|item| item.get("orphan_edges"))
        .and_then(serde_json::Value::as_array)
    {
        for orphan in orphan_edges {
            if let Some(edge_id) = orphan.get("edge_id").and_then(serde_json::Value::as_str) {
                ops.push(PatchOp {
                    kind: PatchOpKind::Update,
                    target: format!("relation:edge:{edge_id}"),
                    reason: format!(
                        "repair orphan edge endpoint ({})",
                        reason.unwrap_or("relation consistency check")
                    ),
                });
            }
        }
    }
    if let Some(missing_refs) = report
        .get("checks")
        .and_then(|item| item.get("events_with_missing_edge_ref"))
        .and_then(serde_json::Value::as_array)
    {
        for miss in missing_refs {
            if let Some(event_id) = miss.get("event_id").and_then(serde_json::Value::as_str) {
                ops.push(PatchOp {
                    kind: PatchOpKind::Update,
                    target: format!("relation:event:{event_id}"),
                    reason: format!(
                        "repair missing edge reference ({})",
                        reason.unwrap_or("relation consistency check")
                    ),
                });
            }
        }
    }
    if let Some(cycles) = report
        .get("checks")
        .and_then(|item| item.get("cycle_paths"))
        .and_then(serde_json::Value::as_array)
    {
        for (index, _) in cycles.iter().enumerate() {
            ops.push(PatchOp {
                kind: PatchOpKind::Update,
                target: format!("relation:cycle:{index}"),
                reason: format!(
                    "repair cycle in relation graph ({})",
                    reason.unwrap_or("relation consistency check")
                ),
            });
        }
    }
    if ops.is_empty() {
        ops.push(PatchOp {
            kind: PatchOpKind::None,
            target: format!("relation:session:{session_id}"),
            reason: "no relation consistency fix required".to_string(),
        });
    }

    PatchPlan {
        namespace: format!("relation:{session_id}"),
        ops,
    }
}

fn relation_replay_fingerprint(
    session_id: &str,
    trace_id: &str,
    generated_at_ms: u64,
    edge_count: usize,
    event_count: usize,
) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    trace_id.hash(&mut hasher);
    generated_at_ms.hash(&mut hasher);
    edge_count.hash(&mut hasher);
    event_count.hash(&mut hasher);
    format!("replayfp:{:016x}", hasher.finish())
}

fn normalize_relation_kind(raw: &str) -> String {
    raw.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn detect_relation_cycles(
    edges: &[autoloop_state_adapter::RelationEdgeCurrentRecord],
) -> Vec<serde_json::Value> {
    let mut graph = HashMap::<String, Vec<String>>::new();
    for edge in edges {
        if edge.from_node.trim().is_empty() || edge.to_node.trim().is_empty() {
            continue;
        }
        graph
            .entry(edge.from_node.clone())
            .or_default()
            .push(edge.to_node.clone());
    }

    let mut cycles = Vec::new();
    let mut visiting = HashSet::<String>::new();
    let mut visited = HashSet::<String>::new();
    let mut stack = Vec::<String>::new();

    fn dfs(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
        cycles: &mut Vec<serde_json::Value>,
    ) {
        if visited.contains(node) {
            return;
        }
        if !visiting.insert(node.to_string()) {
            if let Some(position) = stack.iter().position(|item| item == node) {
                let cycle = stack[position..].to_vec();
                if !cycle.is_empty() {
                    cycles.push(serde_json::json!({ "nodes": cycle }));
                }
            }
            return;
        }
        stack.push(node.to_string());
        if let Some(next_nodes) = graph.get(node) {
            for next in next_nodes {
                if visiting.contains(next) {
                    if let Some(position) = stack.iter().position(|item| item == next) {
                        let cycle = stack[position..].to_vec();
                        if !cycle.is_empty() {
                            cycles.push(serde_json::json!({ "nodes": cycle }));
                        }
                    }
                    continue;
                }
                dfs(next, graph, visiting, visited, stack, cycles);
            }
        }
        stack.pop();
        visiting.remove(node);
        visited.insert(node.to_string());
    }

    let nodes = graph.keys().cloned().collect::<Vec<_>>();
    for node in nodes {
        dfs(
            &node,
            &graph,
            &mut visiting,
            &mut visited,
            &mut stack,
            &mut cycles,
        );
    }

    cycles.sort_by_key(|item| {
        item.get("nodes")
            .and_then(serde_json::Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0)
    });
    cycles.dedup();
    cycles
}

async fn system_artifact_proof_view(
    app: &AutoLoopApp,
    session_id: &str,
    artifact_ref: Option<&str>,
    artifact_path: Option<&Path>,
    trace_id: Option<&str>,
    limit: usize,
) -> Result<serde_json::Value> {
    let db = app.state_store();
    let referenced_proof = if let Some(reference) = artifact_ref {
        db.get_knowledge(reference).await?
    } else {
        None
    };

    let relation_write_proofs = db
        .list_knowledge_by_prefix(&format!("relation:write_proof:{session_id}:"))
        .await?
        .into_iter()
        .rev()
        .take(limit)
        .map(|item| {
            let payload = serde_json::from_str::<serde_json::Value>(&item.value)
                .unwrap_or_else(|_| serde_json::json!({ "raw": item.value }));
            serde_json::json!({
                "key": item.key,
                "value": payload,
            })
        })
        .collect::<Vec<_>>();

    let mut decisions = db
        .list_knowledge_by_prefix(&format!("runtime:decision:{session_id}:"))
        .await?
        .into_iter()
        .filter_map(|item| serde_json::from_str::<serde_json::Value>(&item.value).ok())
        .collect::<Vec<_>>();
    if let Some(trace) = trace_id {
        decisions.retain(|item| {
            item.get("trace_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|candidate| candidate == trace)
        });
    }
    decisions.sort_by_key(|item| {
        item.get("decision_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
    });
    decisions.reverse();
    decisions.truncate(limit);

    let artifact_decision_hits = decisions
        .iter()
        .filter(|item| {
            item.get("reasons")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|reasons| {
                    reasons.iter().any(|reason| {
                        reason
                            .as_str()
                            .is_some_and(|text| text.contains("artifact_"))
                    })
                })
        })
        .cloned()
        .collect::<Vec<_>>();

    let local_file_proof = if let Some(path) = artifact_path {
        if path.exists() {
            let bytes = fs::read(path)?;
            let mut hasher = sha2::Sha256::new();
            use sha2::Digest;
            hasher.update(&bytes);
            let digest = format!("{:x}", hasher.finalize());
            let metadata = fs::metadata(path)?;
            Some(serde_json::json!({
                "path": path.display().to_string(),
                "exists": true,
                "size_bytes": metadata.len(),
                "sha256": digest,
                "readable": true,
            }))
        } else {
            Some(serde_json::json!({
                "path": path.display().to_string(),
                "exists": false,
                "deny_reason": "artifact_file_missing",
            }))
        }
    } else {
        None
    };

    let status = if local_file_proof
        .as_ref()
        .and_then(|item| item.get("exists"))
        .and_then(serde_json::Value::as_bool)
        == Some(false)
    {
        "blocked"
    } else if !artifact_decision_hits.is_empty() {
        "requires_attention"
    } else {
        "ok"
    };

    Ok(serde_json::json!({
        "session_id": session_id,
        "trace_filter": trace_id,
        "status": status,
        "artifact_ref": artifact_ref,
        "artifact_path": artifact_path.map(|item| item.display().to_string()),
        "referenced_proof": referenced_proof.map(|item| serde_json::json!({
            "key": item.key,
            "value": serde_json::from_str::<serde_json::Value>(&item.value).unwrap_or_else(|_| serde_json::json!({"raw": item.value})),
        })),
        "local_file_proof": local_file_proof,
        "decision_reasons": artifact_decision_hits,
        "relation_write_proofs": relation_write_proofs,
    }))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct CliDependencyGraph {
    edges: BTreeMap<String, Vec<String>>,
}

async fn compiler_status_view(
    app: &AutoLoopApp,
    session_id: &str,
    repo_root: &Path,
) -> Result<serde_json::Value> {
    let resolved = latest_compiler_run_resolved(app, session_id).await?;
    let latest_run = resolved.as_ref().map(|item| item.run.clone());
    let compile_payload = resolved.as_ref().and_then(|item| item.compile.clone());
    let hot_index_payload = resolved.as_ref().and_then(|item| item.hot_index.clone());

    let dependency_graph = load_dependency_graph_from_repo(repo_root)?;
    let projection_root = repo_root.join(".gitmemory").join("projections");
    let graph_count = count_json_files(&projection_root.join("graph"));
    let vector_count = count_json_files(&projection_root.join("vector"));
    let search_count = count_json_files(&projection_root.join("search"));

    let compile_summary = compile_payload
        .as_ref()
        .map(|compile| {
            serde_json::json!({
                "changed_files": compile
                    .get("changed_files")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or_default(),
                "expanded_targets": compile
                    .get("expanded_targets")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or_default(),
                "compiled_files": compile
                    .get("compiled_files")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or_default(),
                "failed_files": compile
                    .get("failed_files")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| items.len())
                    .unwrap_or_default(),
                "schema_registry_version": compile
                    .get("schema_registry_version")
                    .and_then(serde_json::Value::as_str),
                "dependency_graph_ref": compile
                    .get("dependency_graph_ref")
                    .and_then(serde_json::Value::as_str),
            })
        })
        .unwrap_or_else(|| serde_json::json!({}));

    let mut rule_hits = compile_payload
        .as_ref()
        .and_then(|compile| compile.get("compiled_files"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("placement")
                        .and_then(|placement| placement.get("rule_id"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    rule_hits.sort();
    rule_hits.dedup();

    Ok(serde_json::json!({
        "session_id": session_id,
        "repo_root": repo_root.display().to_string(),
        "latest_run": latest_run,
        "compile_summary": compile_summary,
        "hot_index": hot_index_payload,
        "rule_hits": rule_hits,
        "dependency_graph_edges": dependency_graph.edges.len(),
        "projection_counts": {
            "graph": graph_count,
            "vector": vector_count,
            "search": search_count,
        }
    }))
}

async fn compiler_explain_view(
    app: &AutoLoopApp,
    session_id: &str,
    repo_root: &Path,
    target_file: &str,
) -> Result<serde_json::Value> {
    let normalized = normalize_compiler_target(target_file);
    let projection_key = compiler_projection_key(&normalized);
    let projection_root = repo_root.join(".gitmemory").join("projections");
    let graph_projection_path = projection_root
        .join("graph")
        .join(format!("{projection_key}.json"));
    let vector_projection_path = projection_root
        .join("vector")
        .join(format!("{projection_key}.json"));
    let search_projection_path = projection_root
        .join("search")
        .join(format!("{projection_key}.json"));

    let graph_projection = read_json_file_opt(&graph_projection_path)?;
    let vector_projection = read_json_file_opt(&vector_projection_path)?;
    let search_projection = read_json_file_opt(&search_projection_path)?;

    let resolved = latest_compiler_run_resolved(app, session_id).await?;
    let latest_run = resolved.as_ref().map(|item| item.run.clone());
    let compile_payload = resolved.as_ref().and_then(|item| item.compile.clone());
    let hot_index_payload = resolved.as_ref().and_then(|item| item.hot_index.clone());

    let compiled_file = compile_payload
        .as_ref()
        .and_then(|compile| compile.get("compiled_files"))
        .and_then(serde_json::Value::as_array)
        .and_then(|files| {
            files
                .iter()
                .find(|entry| {
                    entry.get("source_file").and_then(serde_json::Value::as_str)
                        == Some(normalized.as_str())
                })
                .cloned()
        });

    let dependency_graph = load_dependency_graph_from_repo(repo_root)?;
    let dependencies = dependency_graph
        .edges
        .get(&normalized)
        .cloned()
        .unwrap_or_default();
    let dependents = reverse_dependents(&dependency_graph.edges, &normalized);
    let transitive = transitive_dependents(&dependency_graph.edges, &normalized);
    let changed_files = compile_payload
        .as_ref()
        .and_then(|compile| compile.get("changed_files"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let expanded_targets = compile_payload
        .as_ref()
        .and_then(|compile| compile.get("expanded_targets"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let invalidated_dependents = compiled_file
        .as_ref()
        .and_then(|entry| entry.get("invalidated_dependents"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));

    let placement_rule_id = compiled_file
        .as_ref()
        .and_then(|entry| entry.get("placement"))
        .and_then(|placement| placement.get("rule_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let schema_kinds = compiled_file
        .as_ref()
        .and_then(|entry| entry.get("schema_kinds"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let error_codes = compiled_file
        .as_ref()
        .and_then(|entry| entry.get("errors"))
        .and_then(serde_json::Value::as_array)
        .map(|errors| {
            errors
                .iter()
                .filter_map(|err| err.get("code").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let hot_index_touched = hot_index_payload
        .as_ref()
        .and_then(|hot| hot.get("touched_files"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|item| item == normalized)
        });

    let projection_rebuild = serde_json::json!({
        "rebuilt": compiled_file.is_some() || graph_projection.is_some() || vector_projection.is_some() || search_projection.is_some(),
        "projection_files": compiled_file
            .as_ref()
            .and_then(|entry| entry.get("projection_files"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
        "artifacts": {
            "graph": {
                "exists": graph_projection.is_some(),
                "path": graph_projection_path.display().to_string(),
                "projection": graph_projection,
            },
            "vector": {
                "exists": vector_projection.is_some(),
                "path": vector_projection_path.display().to_string(),
                "projection": vector_projection,
            },
            "search": {
                "exists": search_projection.is_some(),
                "path": search_projection_path.display().to_string(),
                "projection": search_projection,
            }
        },
        "hot_index": {
            "touched": hot_index_touched,
            "index_path": hot_index_payload
                .as_ref()
                .and_then(|hot| hot.get("index_path"))
                .and_then(serde_json::Value::as_str),
            "total_entries": hot_index_payload
                .as_ref()
                .and_then(|hot| hot.get("total_entries"))
                .and_then(serde_json::Value::as_u64),
        }
    });

    Ok(serde_json::json!({
        "session_id": session_id,
        "target_file": normalized.clone(),
        "projection_key": projection_key,
        "latest_run": latest_run,
        "compiled_file": compiled_file,
        "hit_rules": {
            "placement_rule_id": placement_rule_id,
            "schema_kinds": schema_kinds,
            "error_codes": error_codes,
        },
        "invalidation_chain": {
            "changed_files": changed_files,
            "expanded_targets": expanded_targets,
            "direct_dependencies": dependencies,
            "direct_dependents": dependents,
            "transitive_dependents": transitive,
            "invalidated_dependents": invalidated_dependents,
        },
        "projection_rebuild": projection_rebuild,
    }))
}

fn compiler_graph_view(repo_root: &Path, target_file: &str) -> Result<serde_json::Value> {
    let normalized = normalize_compiler_target(target_file);
    let dependency_graph = load_dependency_graph_from_repo(repo_root)?;
    let direct_dependencies = dependency_graph
        .edges
        .get(&normalized)
        .cloned()
        .unwrap_or_default();
    let direct_dependents = reverse_dependents(&dependency_graph.edges, &normalized);
    let transitive_dependents = transitive_dependents(&dependency_graph.edges, &normalized);

    let mut nodes = BTreeSet::new();
    nodes.insert(normalized.clone());
    for dep in &direct_dependencies {
        nodes.insert(dep.clone());
    }
    for dep in &direct_dependents {
        nodes.insert(dep.clone());
    }
    for dep in &transitive_dependents {
        nodes.insert(dep.clone());
    }

    let mut edges = Vec::<serde_json::Value>::new();
    for dep in &direct_dependencies {
        edges.push(serde_json::json!({
            "from": normalized.clone(),
            "to": dep,
            "relation": "depends_on",
        }));
    }
    for dep in &direct_dependents {
        edges.push(serde_json::json!({
            "from": dep,
            "to": normalized.clone(),
            "relation": "depends_on",
        }));
    }

    Ok(serde_json::json!({
        "target_file": normalized.clone(),
        "direct_dependencies": direct_dependencies,
        "direct_dependents": direct_dependents,
        "transitive_dependents": transitive_dependents,
        "edge_count": dependency_graph.edges.len(),
        "graph": {
            "nodes": nodes.into_iter().collect::<Vec<_>>(),
            "edges": edges,
        }
    }))
}

#[derive(Debug, Clone)]
struct ResolvedCompilerRun {
    run: serde_json::Value,
    compile: Option<serde_json::Value>,
    hot_index: Option<serde_json::Value>,
}

async fn latest_compiler_run_resolved(
    app: &AutoLoopApp,
    session_id: &str,
) -> Result<Option<ResolvedCompilerRun>> {
    let runs = app
        .state_store()
        .list_knowledge_by_prefix(&format!("memory:compiler:run:{session_id}:"))
        .await?;
    let Some(latest) = runs.iter().max_by(|left, right| left.key.cmp(&right.key)) else {
        return Ok(None);
    };
    let run_value = serde_json::from_str::<serde_json::Value>(&latest.value)
        .unwrap_or_else(|_| serde_json::json!({}));
    let compile = resolve_episode_payload(
        &app.state_store(),
        run_value
            .get("compile_ref")
            .and_then(serde_json::Value::as_str),
    )
    .await?;
    let hot_index = resolve_episode_payload(
        &app.state_store(),
        run_value
            .get("hot_index_ref")
            .and_then(serde_json::Value::as_str),
    )
    .await?;

    Ok(Some(ResolvedCompilerRun {
        run: run_value,
        compile,
        hot_index,
    }))
}

async fn resolve_episode_payload(
    db: &autoloop::state_store_adapter::StateStore,
    reference: Option<&str>,
) -> Result<Option<serde_json::Value>> {
    let Some(reference) = reference else {
        return Ok(None);
    };
    let Some(record) = db.get_knowledge(reference).await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&record.value)
        .unwrap_or_else(|_| serde_json::json!({}));
    Ok(value.get("payload").cloned().or(Some(value)))
}

fn load_dependency_graph_from_repo(repo_root: &Path) -> Result<CliDependencyGraph> {
    let path = repo_root.join(".gitmemory").join("dependency_graph.json");
    if !path.exists() {
        return Ok(CliDependencyGraph::default());
    }
    let raw = fs::read_to_string(path)?;
    let graph = serde_json::from_str::<CliDependencyGraph>(&raw).unwrap_or_default();
    Ok(graph)
}

fn reverse_dependents(edges: &BTreeMap<String, Vec<String>>, target: &str) -> Vec<String> {
    let mut dependents = edges
        .iter()
        .filter_map(|(source, deps)| {
            if deps.iter().any(|dep| dep == target) {
                Some(source.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    dependents.sort();
    dependents.dedup();
    dependents
}

fn transitive_dependents(edges: &BTreeMap<String, Vec<String>>, target: &str) -> Vec<String> {
    let mut visited = BTreeSet::<String>::new();
    let mut queue = VecDeque::<String>::new();
    for dep in reverse_dependents(edges, target) {
        queue.push_back(dep);
    }
    while let Some(node) = queue.pop_front() {
        if !visited.insert(node.clone()) {
            continue;
        }
        for parent in reverse_dependents(edges, &node) {
            if !visited.contains(&parent) {
                queue.push_back(parent);
            }
        }
    }
    let mut ordered = visited.into_iter().collect::<Vec<_>>();
    ordered.sort();
    ordered
}

fn count_json_files(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            })
            .count(),
        Err(_) => 0,
    }
}

fn read_json_file_opt(path: &Path) -> Result<Option<serde_json::Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    let parsed = serde_json::from_str::<serde_json::Value>(&raw)
        .unwrap_or_else(|_| serde_json::json!({"raw": raw}));
    Ok(Some(parsed))
}

fn normalize_compiler_target(value: &str) -> String {
    value.trim().replace('\\', "/")
}

fn compiler_projection_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
fn parse_refresh_plan_mode(value: Option<&str>) -> RefreshPlanMode {
    match value.unwrap_or("detect").to_ascii_lowercase().as_str() {
        "detect" => RefreshPlanMode::Detect,
        "dry-run" | "dry_run" | "dryrun" => RefreshPlanMode::DryRun,
        "force" => RefreshPlanMode::Force,
        "page" => RefreshPlanMode::Page,
        _ => RefreshPlanMode::Detect,
    }
}

fn parse_degrade_profile(value: &str) -> DegradeProfileKind {
    match value.to_ascii_lowercase().as_str() {
        "normal" => DegradeProfileKind::Normal,
        "provider_fallback" | "provider-fallback" => DegradeProfileKind::ProviderFallback,
        "mcp_conservative" | "mcp-conservative" => DegradeProfileKind::McpConservative,
        "read_only" | "read-only" => DegradeProfileKind::ReadOnly,
        "queue_throttle" | "queue-throttle" => DegradeProfileKind::QueueThrottle,
        "manual_takeover" | "manual-takeover" => DegradeProfileKind::ManualTakeover,
        _ => DegradeProfileKind::ManualTakeover,
    }
}

async fn bind_identity_for_session(
    app: &AutoLoopApp,
    tenant: Option<&str>,
    principal: Option<&str>,
    policy: Option<&str>,
    lease_ttl_ms: u64,
    session_id: &str,
) -> Result<()> {
    let tenant_id = tenant.unwrap_or("tenant:default");
    let principal_id = principal
        .map(str::to_string)
        .unwrap_or_else(|| format!("principal:{session_id}"));
    let policy_id = policy.unwrap_or("policy:default");
    app.ensure_session_identity(
        session_id,
        tenant_id,
        &principal_id,
        policy_id,
        lease_ttl_ms,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop::config::AppConfig;
    use autoloop::contracts::signal::{SignalContext, SignalDecision, SignalEvent, SignalKind};

    fn test_repo_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "autoloop-main-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".gitmemory").join("projections").join("graph"))
            .expect("create graph projection dir");
        fs::create_dir_all(root.join(".gitmemory").join("projections").join("vector"))
            .expect("create vector projection dir");
        fs::create_dir_all(root.join(".gitmemory").join("projections").join("search"))
            .expect("create search projection dir");
        root
    }

    #[tokio::test]
    async fn compiler_explain_returns_hit_rules_invalidation_chain_and_projection_rebuild() {
        let repo_root = test_repo_root("compiler-explain");
        fs::write(
            repo_root.join(".gitmemory").join("dependency_graph.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "edges": {
                    "docs/a.md": ["docs/b.md"],
                    "docs/c.md": ["docs/a.md"]
                }
            }))
            .expect("serialize graph"),
        )
        .expect("write dependency graph");

        let projection_key = compiler_projection_key("docs/a.md");
        fs::write(
            repo_root
                .join(".gitmemory")
                .join("projections")
                .join("graph")
                .join(format!("{projection_key}.json")),
            serde_json::to_string_pretty(&serde_json::json!({"source_file":"docs/a.md","ok":true}))
                .expect("serialize graph projection"),
        )
        .expect("write graph projection");
        fs::write(
            repo_root
                .join(".gitmemory")
                .join("projections")
                .join("vector")
                .join(format!("{projection_key}.json")),
            serde_json::to_string_pretty(&serde_json::json!({"source_file":"docs/a.md","ok":true}))
                .expect("serialize vector projection"),
        )
        .expect("write vector projection");
        fs::write(
            repo_root
                .join(".gitmemory")
                .join("projections")
                .join("search")
                .join(format!("{projection_key}.json")),
            serde_json::to_string_pretty(&serde_json::json!({"source_file":"docs/a.md","ok":true}))
                .expect("serialize search projection"),
        )
        .expect("write search projection");

        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "main-compiler-session";
        let trace_id = "main-compiler-trace";

        let compile_ref = format!("memory:episode:{session_id}:{trace_id}:1:compiler");
        let hot_index_ref = format!("memory:episode:{session_id}:{trace_id}:2:hotindex");
        app.state_store()
            .upsert_json_knowledge(
                compile_ref.clone(),
                &serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "stage": "compiler",
                    "payload": {
                        "changed_files": ["docs/a.md"],
                        "expanded_targets": ["docs/a.md", "docs/c.md"],
                        "compiled_files": [{
                            "source_file": "docs/a.md",
                            "placement": {"rule_id": "namespace-path-v1"},
                            "schema_kinds": ["frontmatter", "heading"],
                            "errors": [{"code": "REQUIRED_BLOCK_MISSING"}],
                            "invalidated_dependents": ["docs/c.md"],
                            "projection_files": ["graph", "vector", "search"],
                        }],
                        "failed_files": [],
                        "dependency_graph_ref": "memory:compiler:dependency-graph:test",
                        "schema_registry_version": "schema-v2"
                    }
                }),
                "episode-ledger",
            )
            .await
            .expect("seed compile episode");
        app.state_store()
            .upsert_json_knowledge(
                hot_index_ref.clone(),
                &serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "stage": "hot_index",
                    "payload": {
                        "index_path": repo_root.join(".gitmemory").join("hot_index.json").display().to_string(),
                        "touched_files": ["docs/a.md"],
                        "total_entries": 1
                    }
                }),
                "episode-ledger",
            )
            .await
            .expect("seed hot index episode");

        app.state_store()
            .upsert_json_knowledge(
                format!("memory:compiler:run:{session_id}:999"),
                &serde_json::json!({
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "changed_files": ["docs/a.md"],
                    "compile_ref": compile_ref,
                    "hot_index_ref": hot_index_ref,
                }),
                "episode-ledger",
            )
            .await
            .expect("seed compiler run");

        let explain = compiler_explain_view(&app, session_id, &repo_root, "docs/a.md")
            .await
            .expect("compiler explain");

        assert!(explain.get("hit_rules").is_some());
        assert!(explain.get("invalidation_chain").is_some());
        assert!(explain.get("projection_rebuild").is_some());
        assert_eq!(
            explain
                .get("hit_rules")
                .and_then(|v| v.get("placement_rule_id"))
                .and_then(serde_json::Value::as_str),
            Some("namespace-path-v1")
        );
        assert!(
            explain
                .get("invalidation_chain")
                .and_then(|v| v.get("expanded_targets"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item.as_str() == Some("docs/c.md")))
        );
        assert_eq!(
            explain
                .get("projection_rebuild")
                .and_then(|v| v.get("rebuilt"))
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let _ = fs::remove_dir_all(&repo_root);
    }

    #[tokio::test]
    async fn system_signal_status_and_explain_views_are_available() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "main-signal-session";
        let trace_id = "main-signal-trace";

        let facade = autoloop::observability::SignalFacade::new(
            app.state_store().clone(),
            &autoloop::config::SignalPipelineConfig::default(),
        );
        facade
            .emit(SignalEvent {
                signal_id: "signal:main:1".into(),
                kind: SignalKind::Trace,
                name: "runtime.execute.start".into(),
                context: SignalContext {
                    session_id: session_id.into(),
                    trace_id: trace_id.into(),
                    span_id: Some("span:main:1".into()),
                    task_id: Some("task:main:1".into()),
                    capability_id: Some("tool:write_file".into()),
                    tenant_id: Some("tenant:test".into()),
                    principal_id: Some("operator:test".into()),
                },
                attributes: BTreeMap::new(),
                numeric_value: None,
                body: Some("start".into()),
                decision: SignalDecision {
                    accepted: true,
                    reason: None,
                    evidence_ref: Some("evidence:main:1".into()),
                },
                emitted_at_ms: autoloop::orchestration::current_time_ms(),
            })
            .await
            .expect("emit signal");

        let status = system_signal_status_view(&app, session_id)
            .await
            .expect("signal status");
        assert_eq!(
            status
                .get("persisted")
                .and_then(|v| v.get("event_count"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );

        let explain = system_signal_explain_view(&app, session_id, Some(trace_id))
            .await
            .expect("signal explain");
        assert_eq!(
            explain
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    #[tokio::test]
    async fn system_relation_views_show_decision_reason_and_evidence_ref() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "main-relation-session";
        let trace_id = "main-relation-trace";
        let now = autoloop::orchestration::current_time_ms();

        app.state_store()
            .upsert_json_knowledge(
                format!("relation:state:{session_id}:{trace_id}:{now}:edge"),
                &serde_json::json!({
                    "kind": "relation_edge",
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "evidence_ref": "evidence:relation:edge:1",
                    "updated_at_ms": now,
                    "edge": {
                        "edge_id": "edge:main:1",
                        "from_node": "session:main",
                        "to_node": "query:main",
                        "edge_type": "depends_on",
                        "reason": {
                            "code": "route_allowed",
                            "message": "route allowed by gate",
                            "evidence_ref": "evidence:relation:edge:1"
                        }
                    }
                }),
                "relation-test",
            )
            .await
            .expect("seed relation edge");
        app.state_store()
            .upsert_json_knowledge(
                format!("relation:state:{session_id}:{trace_id}:{now}:event"),
                &serde_json::json!({
                    "kind": "relation_event",
                    "session_id": session_id,
                    "trace_id": trace_id,
                    "evidence_ref": "evidence:relation:event:1",
                    "updated_at_ms": now,
                    "event": {
                        "event_id": "event:main:1",
                        "event_type": "edge_upserted",
                        "reason": {
                            "code": "route_allowed",
                            "message": "allowed",
                            "deny_reason": null,
                            "evidence_ref": "evidence:relation:event:1"
                        }
                    }
                }),
                "relation-test",
            )
            .await
            .expect("seed relation event");

        let status = system_relation_status_view(&app, session_id, 50)
            .await
            .expect("relation status");
        assert_eq!(
            status
                .get("counts")
                .and_then(|value| value.get("edges"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );

        let explain = system_relation_explain_view(&app, session_id, Some(trace_id), 50)
            .await
            .expect("relation explain");
        assert_eq!(
            explain
                .get("count")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        let evidence_ref = explain
            .get("items")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("evidence_ref"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(evidence_ref, Some("evidence:relation:event:1"));
    }

    #[tokio::test]
    async fn system_relation_check_detects_cycle_and_queues_repair_proposal() {
        let app = AutoLoopApp::new(AppConfig::default());
        let session_id = "main-relation-check-session";
        let trace_id = "main-relation-check-trace";
        let now = autoloop::orchestration::current_time_ms();

        for (idx, (from, to)) in [("node:a", "node:b"), ("node:b", "node:c"), ("node:c", "node:a")]
            .iter()
            .enumerate()
        {
            app.state_store()
                .upsert_json_knowledge(
                    format!("relation:state:{session_id}:{trace_id}:{now}:edge:{idx}"),
                    &serde_json::json!({
                        "kind": "relation_edge",
                        "session_id": session_id,
                        "trace_id": trace_id,
                        "evidence_ref": format!("evidence:relation:edge:{idx}"),
                        "updated_at_ms": now + idx as u64,
                        "edge": {
                            "edge_id": format!("edge:cycle:{idx}"),
                            "from_node": from,
                            "to_node": to,
                            "edge_type": "depends_on",
                        }
                    }),
                    "relation-test",
                )
                .await
                .expect("seed cycle edge");
        }

        let check = system_relation_check_view(
            &app,
            session_id,
            Some(trace_id),
            100,
            true,
            Some("auto-repair for cycle"),
        )
        .await
        .expect("relation check with repair");
        assert_eq!(check.get("healthy").and_then(serde_json::Value::as_bool), Some(false));
        assert!(
            check.get("checks")
                .and_then(|item| item.get("cycle_paths"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| !items.is_empty()),
            "expected at least one detected cycle"
        );
        let review_id = check
            .get("repair_proposal")
            .and_then(|item| item.get("review_id"))
            .and_then(serde_json::Value::as_str)
            .expect("repair proposal review id");
        let review_items = PatchReviewQueue::list(&app.state_store(), session_id)
            .await
            .expect("list repair queue");
        assert!(
            review_items.iter().any(|item| item.review_id == review_id),
            "expected relation repair proposal in review queue"
        );
    }
}



















