use anyhow::Result;
use autoloop_state_adapter::{ScheduleEvent, StateStore};
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

use crate::contracts::focus_trigger::{FocusBoard, TriggerKind, TriggerRef, TriggerSpec};
use crate::plugins::gitmemory_core::hot_index_updater::{HotIndexUpdater, RefreshPlanMode};

#[derive(Clone)]
pub struct TriggerRuntimeEngine {
    state_store: StateStore,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct TriggerWorkerReport {
    pub session_id: String,
    pub scanned: usize,
    pub executed: usize,
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub executed_by_kind: BTreeMap<String, usize>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerDispatchOutcome {
    Skipped,
    Completed,
    Failed,
}

impl TriggerRuntimeEngine {
    pub fn new(state_store: StateStore) -> Self {
        Self { state_store }
    }

    pub async fn ingest_webhook_event(
        &self,
        session_id: &str,
        topic: &str,
        payload: Option<String>,
        actor_id: &str,
    ) -> Result<ScheduleEvent> {
        let normalized_topic = normalize_webhook_topic(topic);
        let payload = payload
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty())
            .unwrap_or_else(|| {
                serde_json::json!({
                    "source": "webhook",
                    "topic": topic,
                    "normalized_topic": normalized_topic,
                })
                .to_string()
            });
        let actor = if actor_id.trim().is_empty() {
            "webhook"
        } else {
            actor_id
        };

        self.state_store
            .create_schedule_event(
                session_id.to_string(),
                normalized_topic,
                "focus-trigger".to_string(),
                payload,
                actor.to_string(),
            )
            .await
    }
    pub async fn register_focus_triggers(
        &self,
        session_id: &str,
        board: &FocusBoard,
        actor_id: &str,
    ) -> Result<Vec<TriggerRef>> {
        let mut refs = Vec::new();
        for item in &board.items {
            let spec = TriggerSpec {
                trigger_id: format!("trigger:{}:{}", session_id, item.id),
                kind: TriggerKind::OnMessage,
                config: serde_json::json!({
                    "focus_item_id": item.id,
                    "status": item.status,
                }),
                reason: format!("wake/remind focus item: {}", item.title),
                focus_ref: Some(item.id.clone()),
            };

            let event = self
                .state_store
                .create_schedule_event(
                    session_id.to_string(),
                    format!("focus:trigger:{}", item.id),
                    "focus-trigger".to_string(),
                    serde_json::to_string(&spec)?,
                    actor_id.to_string(),
                )
                .await?;

            refs.push(TriggerRef {
                trigger_id: spec.trigger_id,
                session_id: session_id.to_string(),
                status: event.status,
            });
        }
        Ok(refs)
    }

    pub async fn list_pending(&self, session_id: &str) -> Result<Vec<ScheduleEvent>> {
        Ok(self
            .state_store
            .list_schedule_events(session_id)
            .await?
            .into_iter()
            .filter(|event| {
                let status = event.status.to_ascii_lowercase();
                status != "completed" && status != "cancelled" && status != "failed"
            })
            .collect())
    }

    pub async fn run_worker_once<F, Fut>(
        &self,
        session_id: &str,
        mut executor: F,
    ) -> Result<TriggerWorkerReport>
    where
        F: FnMut(&ScheduleEvent) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let pending = self.list_pending(session_id).await?;
        let mut report = TriggerWorkerReport {
            session_id: session_id.to_string(),
            scanned: pending.len(),
            ..TriggerWorkerReport::default()
        };

        for event in pending {
            let kind = classify_trigger_kind(&event.topic);
            if !event.topic.starts_with("focus:trigger:") && !event.topic.starts_with("trigger:") {
                report.skipped += 1;
                continue;
            }
            let outcome = match kind {
                TriggerRouteKind::Cron => self.execute_cron_event(&event, &mut executor).await?,
                TriggerRouteKind::Interval => {
                    self.execute_interval_event(&event, &mut executor).await?
                }
                TriggerRouteKind::Poll => self.execute_poll_event(&event, &mut executor).await?,
                TriggerRouteKind::OnMessage => {
                    self.execute_on_message_event(&event, &mut executor).await?
                }
                TriggerRouteKind::Webhook
                | TriggerRouteKind::Focus
                | TriggerRouteKind::Once
                | TriggerRouteKind::Unknown => {
                    self.execute_single_fire_event(&event, &mut executor)
                        .await?
                }
            };

            match outcome {
                TriggerDispatchOutcome::Skipped => {
                    report.skipped += 1;
                }
                TriggerDispatchOutcome::Completed => {
                    report.executed += 1;
                    report.completed += 1;
                    let key = kind.as_str().to_string();
                    let current = report.executed_by_kind.get(&key).copied().unwrap_or(0);
                    report.executed_by_kind.insert(key, current + 1);
                }
                TriggerDispatchOutcome::Failed => {
                    report.executed += 1;
                    report.failed += 1;
                    let key = kind.as_str().to_string();
                    let current = report.executed_by_kind.get(&key).copied().unwrap_or(0);
                    report.executed_by_kind.insert(key, current + 1);
                }
            }
        }

        Ok(report)
    }

    async fn execute_cron_event<F, Fut>(
        &self,
        event: &ScheduleEvent,
        executor: &mut F,
    ) -> Result<TriggerDispatchOutcome>
    where
        F: FnMut(&ScheduleEvent) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        if !should_fire_event(event) {
            return Ok(TriggerDispatchOutcome::Skipped);
        }
        match executor(event).await {
            Ok(_) => {
                self.state_store
                    .update_schedule_status(event.id, "completed")
                    .await?;
                let interval_secs = event_payload_u64(event, "cron_interval_secs")
                    .or_else(|| event_payload_u64(event, "interval_secs"))
                    .unwrap_or(300);
                self.schedule_next_event(event, current_time_ms() + interval_secs * 1000)
                    .await?;
                Ok(TriggerDispatchOutcome::Completed)
            }
            Err(_) => {
                self.state_store
                    .update_schedule_status(event.id, "failed")
                    .await?;
                Ok(TriggerDispatchOutcome::Failed)
            }
        }
    }

    async fn execute_interval_event<F, Fut>(
        &self,
        event: &ScheduleEvent,
        executor: &mut F,
    ) -> Result<TriggerDispatchOutcome>
    where
        F: FnMut(&ScheduleEvent) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        if !should_fire_event(event) {
            return Ok(TriggerDispatchOutcome::Skipped);
        }
        match executor(event).await {
            Ok(_) => {
                self.state_store
                    .update_schedule_status(event.id, "completed")
                    .await?;
                let interval_secs = event_payload_u64(event, "interval_secs").unwrap_or(120);
                self.schedule_next_event(event, current_time_ms() + interval_secs * 1000)
                    .await?;
                Ok(TriggerDispatchOutcome::Completed)
            }
            Err(_) => {
                self.state_store
                    .update_schedule_status(event.id, "failed")
                    .await?;
                Ok(TriggerDispatchOutcome::Failed)
            }
        }
    }

    async fn execute_poll_event<F, Fut>(
        &self,
        event: &ScheduleEvent,
        executor: &mut F,
    ) -> Result<TriggerDispatchOutcome>
    where
        F: FnMut(&ScheduleEvent) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        if !should_fire_event(event) {
            return Ok(TriggerDispatchOutcome::Skipped);
        }
        let poll_ready = event_payload_bool(event, "poll_ready").unwrap_or(true);
        if !poll_ready {
            return Ok(TriggerDispatchOutcome::Skipped);
        }
        match executor(event).await {
            Ok(_) => {
                self.state_store
                    .update_schedule_status(event.id, "completed")
                    .await?;
                let interval_secs = event_payload_u64(event, "poll_interval_secs").unwrap_or(60);
                self.schedule_next_event(event, current_time_ms() + interval_secs * 1000)
                    .await?;
                Ok(TriggerDispatchOutcome::Completed)
            }
            Err(_) => {
                self.state_store
                    .update_schedule_status(event.id, "failed")
                    .await?;
                Ok(TriggerDispatchOutcome::Failed)
            }
        }
    }

    async fn execute_on_message_event<F, Fut>(
        &self,
        event: &ScheduleEvent,
        executor: &mut F,
    ) -> Result<TriggerDispatchOutcome>
    where
        F: FnMut(&ScheduleEvent) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        self.execute_single_fire_event(event, executor).await
    }

    async fn execute_single_fire_event<F, Fut>(
        &self,
        event: &ScheduleEvent,
        executor: &mut F,
    ) -> Result<TriggerDispatchOutcome>
    where
        F: FnMut(&ScheduleEvent) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        if !should_fire_event(event) {
            return Ok(TriggerDispatchOutcome::Skipped);
        }
        if is_sync_topic(&event.topic) {
            match self.handle_sync_event(event).await {
                Ok(_) => {
                    self.state_store
                        .update_schedule_status(event.id, "completed")
                        .await?;
                    return Ok(TriggerDispatchOutcome::Completed);
                }
                Err(_) => {
                    self.state_store
                        .update_schedule_status(event.id, "failed")
                        .await?;
                    return Ok(TriggerDispatchOutcome::Failed);
                }
            }
        }
        if is_refresh_topic(&event.topic) {
            match self.handle_refresh_event(event).await {
                Ok(_) => {
                    self.state_store
                        .update_schedule_status(event.id, "completed")
                        .await?;
                    return Ok(TriggerDispatchOutcome::Completed);
                }
                Err(_) => {
                    self.state_store
                        .update_schedule_status(event.id, "failed")
                        .await?;
                    return Ok(TriggerDispatchOutcome::Failed);
                }
            }
        }
        match executor(event).await {
            Ok(_) => {
                self.state_store
                    .update_schedule_status(event.id, "completed")
                    .await?;
                Ok(TriggerDispatchOutcome::Completed)
            }
            Err(_) => {
                self.state_store
                    .update_schedule_status(event.id, "failed")
                    .await?;
                Ok(TriggerDispatchOutcome::Failed)
            }
        }
    }

    async fn handle_refresh_event(&self, event: &ScheduleEvent) -> Result<()> {
        let payload = parse_refresh_payload(&event.payload);
        let mode = parse_refresh_mode(payload.mode.as_deref());
        let repo_root = payload.repo_root.unwrap_or_else(default_repo_root);
        let plan = HotIndexUpdater::plan_refresh_with_options(
            std::path::Path::new(&repo_root),
            &payload.requested_files,
            mode,
            payload.page,
            payload.page_size,
        )?;

        let replay_key = format!(
            "memory:refresh:plan:{}:{}",
            event.session_id,
            current_time_ms()
        );
        self.state_store
            .upsert_json_knowledge(
                replay_key.clone(),
                &serde_json::json!({
                    "session_id": event.session_id,
                    "event_id": event.id,
                    "topic": event.topic,
                    "repo_root": repo_root,
                    "mode": mode,
                    "requested_files": payload.requested_files,
                    "page": payload.page,
                    "page_size": payload.page_size,
                    "plan": plan,
                    "triggered_at_ms": current_time_ms(),
                }),
                "trigger-refresh",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("memory:refresh:plan:{}:latest", event.session_id),
                &serde_json::json!({
                    "ref": replay_key,
                    "mode": mode,
                    "event_id": event.id,
                    "updated_at_ms": current_time_ms(),
                }),
                "trigger-refresh",
            )
            .await?;
        Ok(())
    }
    async fn handle_sync_event(&self, event: &ScheduleEvent) -> Result<()> {
        let payload = parse_sync_payload(&event.payload);
        let repo_root = payload.repo_root.unwrap_or_else(default_repo_root);
        let mode = payload.mode.unwrap_or_else(|| "dry-run".to_string());
        let batch_size = payload.batch_size.unwrap_or(10).max(1);
        let dry_run = payload.dry_run || mode.eq_ignore_ascii_case("dry-run");
        let targets = if payload.targets.is_empty() {
            discover_markdown_targets(&repo_root)?
        } else {
            payload.targets.clone()
        };
        let batches = build_batches(&targets, batch_size);

        let script_execution = if dry_run {
            serde_json::json!({
                "executed": false,
                "reason": "dry_run"
            })
        } else {
            let script_path = payload.script_path.unwrap_or_else(default_sync_script_path);
            let mut runs = Vec::<serde_json::Value>::new();
            for (idx, batch) in batches.iter().enumerate() {
                let report = run_sync_script(
                    &script_path,
                    &repo_root,
                    &event.session_id,
                    idx + 1,
                    &mode,
                    batch,
                )?;
                runs.push(report);
            }
            serde_json::json!({
                "executed": true,
                "script_path": script_path,
                "runs": runs
            })
        };

        let replay_key = format!(
            "memory:sync:orchestration:{}:{}",
            event.session_id,
            current_time_ms()
        );
        self.state_store
            .upsert_json_knowledge(
                replay_key.clone(),
                &serde_json::json!({
                    "session_id": event.session_id,
                    "event_id": event.id,
                    "topic": event.topic,
                    "repo_root": repo_root,
                    "mode": mode,
                    "dry_run": dry_run,
                    "batch_size": batch_size,
                    "target_count": targets.len(),
                    "targets": targets,
                    "batches": batches,
                    "script_execution": script_execution,
                    "triggered_at_ms": current_time_ms(),
                }),
                "trigger-sync",
            )
            .await?;
        self.state_store
            .upsert_json_knowledge(
                format!("memory:sync:orchestration:{}:latest", event.session_id),
                &serde_json::json!({
                    "ref": replay_key,
                    "event_id": event.id,
                    "updated_at_ms": current_time_ms(),
                }),
                "trigger-sync",
            )
            .await?;
        Ok(())
    }
    async fn schedule_next_event(&self, event: &ScheduleEvent, next_fire_at_ms: u64) -> Result<()> {
        let payload = with_next_fire_payload(&event.payload, next_fire_at_ms);
        self.state_store
            .create_schedule_event(
                event.session_id.clone(),
                event.topic.clone(),
                event.tool_name.clone(),
                payload,
                event.actor_id.clone(),
            )
            .await?;
        Ok(())
    }
}

pub(crate) fn normalize_webhook_topic(topic: &str) -> String {
    let topic = topic.trim();
    if topic.is_empty() {
        return "trigger:webhook:external".to_string();
    }
    if topic.starts_with("trigger:") || topic.starts_with("focus:trigger:") {
        return topic.to_string();
    }

    let normalized = topic
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == ':' || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if normalized.is_empty() {
        "trigger:webhook:external".to_string()
    } else {
        format!("trigger:webhook:{normalized}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerRouteKind {
    Cron,
    Once,
    Interval,
    Poll,
    OnMessage,
    Webhook,
    Focus,
    Unknown,
}

impl TriggerRouteKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Cron => "cron",
            Self::Once => "once",
            Self::Interval => "interval",
            Self::Poll => "poll",
            Self::OnMessage => "on_message",
            Self::Webhook => "webhook",
            Self::Focus => "focus",
            Self::Unknown => "unknown",
        }
    }
}

fn classify_trigger_kind(topic: &str) -> TriggerRouteKind {
    let lowered = topic.to_ascii_lowercase();
    if lowered.starts_with("focus:trigger:") {
        TriggerRouteKind::Focus
    } else if lowered.starts_with("trigger:webhook:") {
        TriggerRouteKind::Webhook
    } else if lowered.contains(":cron:") {
        TriggerRouteKind::Cron
    } else if lowered.contains(":interval:") {
        TriggerRouteKind::Interval
    } else if lowered.contains(":poll:") {
        TriggerRouteKind::Poll
    } else if lowered.contains(":on_message:") || lowered.contains(":message:") {
        TriggerRouteKind::OnMessage
    } else if lowered.contains(":once:") {
        TriggerRouteKind::Once
    } else {
        TriggerRouteKind::Unknown
    }
}

fn should_fire_event(event: &ScheduleEvent) -> bool {
    match classify_trigger_kind(&event.topic) {
        TriggerRouteKind::Cron | TriggerRouteKind::Interval | TriggerRouteKind::Poll => {
            let now = current_time_ms();
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&event.payload) {
                let next = value
                    .get("next_fire_at_ms")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                next == 0 || now >= next
            } else {
                true
            }
        }
        _ => true,
    }
}

fn event_payload_u64(event: &ScheduleEvent, key: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(&event.payload)
        .ok()
        .and_then(|value| value.get(key).and_then(serde_json::Value::as_u64))
}

fn event_payload_bool(event: &ScheduleEvent, key: &str) -> Option<bool> {
    serde_json::from_str::<serde_json::Value>(&event.payload)
        .ok()
        .and_then(|value| value.get(key).and_then(serde_json::Value::as_bool))
}

fn with_next_fire_payload(payload: &str, next_fire_at_ms: u64) -> String {
    let mut value = serde_json::from_str::<serde_json::Value>(payload)
        .ok()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(map) = value.as_object_mut() {
        map.insert(
            "next_fire_at_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(next_fire_at_ms)),
        );
    }
    value.to_string()
}
#[derive(Debug, Clone, serde::Deserialize, Default)]
struct RefreshTriggerPayload {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    requested_files: Vec<String>,
    #[serde(default)]
    page: Option<usize>,
    #[serde(default)]
    page_size: Option<usize>,
}

fn parse_refresh_payload(raw: &str) -> RefreshTriggerPayload {
    serde_json::from_str::<RefreshTriggerPayload>(raw).unwrap_or_default()
}

fn parse_refresh_mode(raw: Option<&str>) -> RefreshPlanMode {
    match raw
        .map(|item| item.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "dry-run".to_string())
        .as_str()
    {
        "force" => RefreshPlanMode::Force,
        "page" => RefreshPlanMode::Page,
        "detect" => RefreshPlanMode::Detect,
        "dryrun" | "dry_run" | "dry-run" => RefreshPlanMode::DryRun,
        _ => RefreshPlanMode::DryRun,
    }
}

fn default_repo_root() -> String {
    std::env::var("AUTOLOOP_REPO_ROOT")
        .unwrap_or_else(|_| "D:\\AutoLoop\\autoloop-app".to_string())
}

fn is_refresh_topic(topic: &str) -> bool {
    topic.to_ascii_lowercase().starts_with("trigger:refresh:")
}
#[derive(Debug, Clone, serde::Deserialize, Default)]
struct SyncTriggerPayload {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    repo_root: Option<String>,
    #[serde(default)]
    script_path: Option<String>,
    #[serde(default)]
    targets: Vec<String>,
    #[serde(default)]
    batch_size: Option<usize>,
    #[serde(default)]
    dry_run: bool,
}

fn parse_sync_payload(raw: &str) -> SyncTriggerPayload {
    serde_json::from_str::<SyncTriggerPayload>(raw).unwrap_or_default()
}

fn is_sync_topic(topic: &str) -> bool {
    topic.to_ascii_lowercase().starts_with("trigger:sync:")
}

fn discover_markdown_targets(repo_root: &str) -> Result<Vec<String>> {
    let root = PathBuf::from(repo_root);
    let candidates = vec![root.join("memory"), root.join("canonical")];
    let mut files = Vec::<String>::new();
    for dir in candidates {
        if !dir.exists() {
            continue;
        }
        collect_markdown_files(&dir, &root, &mut files)?;
    }
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_markdown_files(dir: &PathBuf, root: &PathBuf, out: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, root, out)?;
            continue;
        }
        let Some(ext) = path.extension().and_then(|item| item.to_str()) else {
            continue;
        };
        if !ext.eq_ignore_ascii_case("md") && !ext.eq_ignore_ascii_case("markdown") {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

fn build_batches(items: &[String], batch_size: usize) -> Vec<Vec<String>> {
    if items.is_empty() {
        return vec![Vec::new()];
    }
    items
        .chunks(batch_size.max(1))
        .map(|chunk| chunk.to_vec())
        .collect()
}

fn default_sync_script_path() -> String {
    #[cfg(target_os = "windows")]
    {
        return "deploy/scripts/pwiki_sync.ps1".to_string();
    }
    #[cfg(not(target_os = "windows"))]
    {
        "deploy/scripts/pwiki_sync.sh".to_string()
    }
}

fn run_sync_script(
    script_path: &str,
    repo_root: &str,
    session_id: &str,
    batch_no: usize,
    mode: &str,
    targets: &[String],
) -> Result<serde_json::Value> {
    let targets_json = serde_json::to_string(targets).unwrap_or_else(|_| "[]".to_string());
    let mut command;
    #[cfg(target_os = "windows")]
    {
        command = if script_path.to_ascii_lowercase().ends_with(".ps1") {
            let mut cmd = Command::new("powershell");
            cmd.arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(script_path);
            cmd
        } else {
            let cmd = Command::new(script_path);
            cmd
        };
    }
    #[cfg(not(target_os = "windows"))]
    {
        command = if script_path.ends_with(".sh") {
            let mut cmd = Command::new("bash");
            cmd.arg(script_path);
            cmd
        } else {
            let mut cmd = Command::new(script_path);
            cmd
        };
    }

    let output = command
        .arg("--repo-root")
        .arg(repo_root)
        .arg("--session")
        .arg(session_id)
        .arg("--mode")
        .arg(mode)
        .arg("--batch-no")
        .arg(batch_no.to_string())
        .arg("--targets-json")
        .arg(targets_json)
        .output()?;

    Ok(serde_json::json!({
        "batch_no": batch_no,
        "status": output.status.code().unwrap_or(-1),
        "success": output.status.success(),
        "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
        "stderr": String::from_utf8_lossy(&output.stderr).to_string(),
        "target_count": targets.len(),
    }))
}
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[async_trait::async_trait]
impl crate::contracts::ports::TriggerRuntimePort for TriggerRuntimeEngine {
    async fn register_trigger(
        &self,
        session_id: &crate::contracts::ids::SessionId,
        trigger: &crate::contracts::focus_trigger::TriggerSpec,
    ) -> Result<(), crate::contracts::errors::ContractError> {
        self.state_store
            .create_schedule_event(
                session_id.to_string(),
                format!("trigger:{}", trigger.trigger_id),
                "focus-trigger".into(),
                serde_json::to_string(trigger).map_err(|e| {
                    crate::contracts::errors::ContractError::Internal(e.to_string())
                })?,
                "trigger-runtime".into(),
            )
            .await
            .map_err(|e| crate::contracts::errors::ContractError::Storage(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn trigger_runtime_registers_focus_events() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let engine = TriggerRuntimeEngine::new(db.clone());
        let board = FocusBoard {
            session_id: "session-rt".to_string(),
            goal: "g".to_string(),
            items: vec![crate::contracts::focus_trigger::FocusItem {
                id: "item-1".to_string(),
                title: "title".to_string(),
                status: "pending".to_string(),
                owner: "agent".to_string(),
                acceptance_hint: "hint".to_string(),
            }],
        };

        let refs = engine
            .register_focus_triggers("session-rt", &board, "planner")
            .await
            .expect("register");
        assert_eq!(refs.len(), 1);

        let events = db.list_schedule_events("session-rt").await.expect("events");
        assert_eq!(events.len(), 1);
        assert!(events[0].topic.starts_with("focus:trigger:"));
    }
    #[test]
    fn normalize_webhook_topic_enforces_trigger_prefix() {
        assert_eq!(
            normalize_webhook_topic(""),
            "trigger:webhook:external".to_string()
        );
        assert_eq!(
            normalize_webhook_topic("trigger:ops:daily"),
            "trigger:ops:daily".to_string()
        );
        assert_eq!(
            normalize_webhook_topic("focus:trigger:item-1"),
            "focus:trigger:item-1".to_string()
        );
        assert_eq!(
            normalize_webhook_topic("order.created"),
            "trigger:webhook:order-created".to_string()
        );
    }

    #[test]
    fn classify_trigger_kind_detects_runtime_modes() {
        assert_eq!(
            classify_trigger_kind("focus:trigger:item-1").as_str(),
            "focus"
        );
        assert_eq!(
            classify_trigger_kind("trigger:webhook:order-created").as_str(),
            "webhook"
        );
        assert_eq!(
            classify_trigger_kind("trigger:cron:nightly").as_str(),
            "cron"
        );
        assert_eq!(
            classify_trigger_kind("trigger:interval:sync").as_str(),
            "interval"
        );
        assert_eq!(classify_trigger_kind("trigger:poll:queue").as_str(), "poll");
    }

    #[tokio::test]
    async fn webhook_ingress_normalizes_topic_and_executes_in_worker() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let engine = TriggerRuntimeEngine::new(db.clone());
        let event = engine
            .ingest_webhook_event("session-webhook", "order.created", None, "webhook-agent")
            .await
            .expect("ingest webhook");
        assert_eq!(event.session_id, "session-webhook");
        assert!(event.topic.starts_with("trigger:webhook:"));

        let executed_topics = Arc::new(Mutex::new(Vec::<String>::new()));
        let topics = Arc::clone(&executed_topics);
        let report = engine
            .run_worker_once("session-webhook", move |event| {
                let topics = Arc::clone(&topics);
                let topic = event.topic.clone();
                async move {
                    topics.lock().await.push(topic);
                    Ok(())
                }
            })
            .await
            .expect("worker report");

        assert_eq!(report.scanned, 1);
        assert_eq!(report.executed, 1);
        assert_eq!(report.completed, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.skipped, 0);

        let captured = executed_topics.lock().await.clone();
        assert_eq!(captured.len(), 1);
        assert!(captured[0].starts_with("trigger:webhook:"));
    }
    #[tokio::test]
    async fn cron_event_reschedules_next_cycle() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        db.create_schedule_event(
            "session-cron".into(),
            "trigger:cron:nightly".into(),
            "focus-trigger".into(),
            serde_json::json!({
                "next_fire_at_ms": 0u64,
                "cron_interval_secs": 1u64
            })
            .to_string(),
            "scheduler".into(),
        )
        .await
        .expect("seed cron event");

        let engine = TriggerRuntimeEngine::new(db.clone());
        let report = engine
            .run_worker_once("session-cron", |_| async { Ok(()) })
            .await
            .expect("run worker");

        assert_eq!(report.executed, 1);
        assert_eq!(report.completed, 1);
        assert_eq!(report.failed, 0);
        assert_eq!(report.executed_by_kind.get("cron").copied().unwrap_or(0), 1);

        let events = db
            .list_schedule_events("session-cron")
            .await
            .expect("events after run");
        assert!(
            events
                .iter()
                .any(|event| event.status.eq_ignore_ascii_case("completed"))
        );
        assert!(events.iter().any(|event| {
            event.status.eq_ignore_ascii_case("pending")
                || event.status.eq_ignore_ascii_case("queued")
        }));
    }

    #[tokio::test]
    async fn refresh_trigger_writes_replayable_plan() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let temp = std::env::temp_dir().join(format!(
            "autoloop-trigger-refresh-{}",
            current_time_ms()
        ));
        std::fs::create_dir_all(temp.join(".gitmemory")).expect("mkdir");
        std::fs::write(
            temp.join(".gitmemory").join("hot_index.json"),
            serde_json::to_string_pretty(&serde_json::json!([
                {"source_file":"docs/a.md","source_digest":"x","summary":"x","updated_at_ms":1},
                {"source_file":"docs/b.md","source_digest":"y","summary":"y","updated_at_ms":1}
            ]))
            .expect("serialize"),
        )
        .expect("write index");

        let engine = TriggerRuntimeEngine::new(db.clone());
        db.create_schedule_event(
            "session-refresh".into(),
            "trigger:refresh:hot-index".into(),
            "focus-trigger".into(),
            serde_json::json!({
                "mode": "page",
                "page": 1,
                "page_size": 1,
                "repo_root": temp.display().to_string()
            })
            .to_string(),
            "scheduler".into(),
        )
        .await
        .expect("seed refresh event");

        let report = engine
            .run_worker_once("session-refresh", |_| async { Ok(()) })
            .await
            .expect("run worker");
        assert_eq!(report.completed, 1);

        let latest = db
            .get_knowledge("memory:refresh:plan:session-refresh:latest")
            .await
            .expect("db latest")
            .expect("latest exists");
        assert!(latest.value.contains("\"mode\":\"page\""));
        assert!(latest.value.contains("memory:refresh:plan:session-refresh:"));

        let _ = std::fs::remove_dir_all(&temp);
    }
    #[tokio::test]
    async fn sync_trigger_writes_replayable_orchestration_record() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        });
        let temp = std::env::temp_dir().join(format!(
            "autoloop-trigger-sync-{}",
            current_time_ms()
        ));
        std::fs::create_dir_all(temp.join("memory")).expect("mkdir");
        std::fs::write(temp.join("memory").join("a.md"), "# A\n").expect("write a");
        std::fs::write(temp.join("memory").join("b.md"), "# B\n").expect("write b");

        let engine = TriggerRuntimeEngine::new(db.clone());
        db.create_schedule_event(
            "session-sync".into(),
            "trigger:sync:pwiki".into(),
            "focus-trigger".into(),
            serde_json::json!({
                "mode": "dry-run",
                "dry_run": true,
                "batch_size": 1,
                "repo_root": temp.display().to_string()
            })
            .to_string(),
            "scheduler".into(),
        )
        .await
        .expect("seed sync event");

        let report = engine
            .run_worker_once("session-sync", |_| async { Ok(()) })
            .await
            .expect("run worker");
        assert_eq!(report.completed, 1);

        let latest = db
            .get_knowledge("memory:sync:orchestration:session-sync:latest")
            .await
            .expect("db latest")
            .expect("latest exists");
        assert!(latest.value.contains("memory:sync:orchestration:session-sync:"));

        let _ = std::fs::remove_dir_all(&temp);
    }
}
