use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Result, bail};
use tokio::{
    process::Command,
    sync::{RwLock, watch},
    task::JoinHandle,
    time::{Duration, timeout},
};

use crate::agent::AgentRuntime;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskKind {
    Shell,
    Agent,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskStatus {
    Running,
    Completed,
    Failed,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackgroundTaskRecord {
    pub session_id: String,
    pub task_id: String,
    pub kind: BackgroundTaskKind,
    pub spec: String,
    pub status: BackgroundTaskStatus,
    pub restart_count: u32,
    pub max_restarts: u32,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub last_error: Option<String>,
    pub last_exit_code: Option<i32>,
    pub log_path: String,
}

#[derive(Clone)]
pub struct BackgroundTaskManager {
    root: PathBuf,
    tasks: Arc<RwLock<HashMap<String, TaskHandle>>>,
}

struct TaskHandle {
    record: Arc<RwLock<BackgroundTaskRecord>>,
    stop_tx: watch::Sender<bool>,
    join: Arc<RwLock<Option<JoinHandle<()>>>>,
}

impl BackgroundTaskManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let _ = std::fs::create_dir_all(&root);
        Self {
            root,
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn default() -> Self {
        Self::new("deploy/runtime/background-tasks")
    }

    pub async fn start_shell_task(
        &self,
        session_id: &str,
        task_id: &str,
        command: &str,
        max_restarts: u32,
    ) -> Result<BackgroundTaskRecord> {
        let key = task_key(session_id, task_id);
        self.ensure_not_running(&key).await?;
        let log_path = self.log_path_for(session_id, task_id);
        let record = Arc::new(RwLock::new(BackgroundTaskRecord {
            session_id: session_id.to_string(),
            task_id: task_id.to_string(),
            kind: BackgroundTaskKind::Shell,
            spec: command.to_string(),
            status: BackgroundTaskStatus::Running,
            restart_count: 0,
            max_restarts,
            started_at_ms: current_time_ms(),
            finished_at_ms: None,
            last_error: None,
            last_exit_code: None,
            log_path: log_path.display().to_string(),
        }));
        append_log_line(&log_path, &format!("[start] shell task {task_id}")).await;

        let (stop_tx, stop_rx) = watch::channel(false);
        let record_ref = Arc::clone(&record);
        let command_spec = command.to_string();
        let task_name = task_id.to_string();
        let join = tokio::spawn(async move {
            let mut attempts: u32 = 0;
            loop {
                if *stop_rx.borrow() {
                    let mut guard = record_ref.write().await;
                    guard.status = BackgroundTaskStatus::Stopped;
                    guard.finished_at_ms = Some(current_time_ms());
                    append_log_line(&PathBuf::from(&guard.log_path), "[stop] stop signal observed")
                        .await;
                    break;
                }

                let output = run_shell_command(&command_spec).await;
                match output {
                    Ok(result) if result.exit_code == Some(0) => {
                        let mut guard = record_ref.write().await;
                        guard.status = BackgroundTaskStatus::Completed;
                        guard.finished_at_ms = Some(current_time_ms());
                        guard.last_exit_code = result.exit_code;
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            &format!(
                                "[completed] exit_code={:?}\n{}{}",
                                result.exit_code, result.stdout, result.stderr
                            ),
                        )
                        .await;
                        break;
                    }
                    Ok(result) => {
                        attempts = attempts.saturating_add(1);
                        let mut guard = record_ref.write().await;
                        guard.restart_count = attempts;
                        guard.last_exit_code = result.exit_code;
                        guard.last_error = Some(format!(
                            "shell task failed with exit_code={:?}",
                            result.exit_code
                        ));
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            &format!(
                                "[failed] attempt={attempts} exit_code={:?}\n{}{}",
                                result.exit_code, result.stdout, result.stderr
                            ),
                        )
                        .await;
                        if attempts > guard.max_restarts {
                            guard.status = BackgroundTaskStatus::Failed;
                            guard.finished_at_ms = Some(current_time_ms());
                            break;
                        }
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            &format!("[restart] retrying shell task {}", task_name),
                        )
                        .await;
                    }
                    Err(error) => {
                        attempts = attempts.saturating_add(1);
                        let mut guard = record_ref.write().await;
                        guard.restart_count = attempts;
                        guard.last_error = Some(error.to_string());
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            &format!("[error] attempt={attempts} {}", error),
                        )
                        .await;
                        if attempts > guard.max_restarts {
                            guard.status = BackgroundTaskStatus::Failed;
                            guard.finished_at_ms = Some(current_time_ms());
                            break;
                        }
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            &format!("[restart] retrying shell task {}", task_name),
                        )
                        .await;
                    }
                }
            }
        });
        let join_ref = Arc::new(RwLock::new(Some(join)));

        self.tasks.write().await.insert(
            key,
            TaskHandle {
                record: Arc::clone(&record),
                stop_tx,
                join: join_ref,
            },
        );
        Ok(record.read().await.clone())
    }

    pub(crate) async fn start_agent_task(
        &self,
        session_id: &str,
        task_id: &str,
        prompt: &str,
        max_restarts: u32,
        agent: AgentRuntime,
    ) -> Result<BackgroundTaskRecord> {
        if prompt_requires_harness_facade(prompt) {
            bail!("code task requires harness façade execution");
        }
        let key = task_key(session_id, task_id);
        self.ensure_not_running(&key).await?;
        let log_path = self.log_path_for(session_id, task_id);
        let record = Arc::new(RwLock::new(BackgroundTaskRecord {
            session_id: session_id.to_string(),
            task_id: task_id.to_string(),
            kind: BackgroundTaskKind::Agent,
            spec: prompt.to_string(),
            status: BackgroundTaskStatus::Running,
            restart_count: 0,
            max_restarts,
            started_at_ms: current_time_ms(),
            finished_at_ms: None,
            last_error: None,
            last_exit_code: None,
            log_path: log_path.display().to_string(),
        }));
        append_log_line(&log_path, &format!("[start] agent task {task_id}")).await;

        let (stop_tx, stop_rx) = watch::channel(false);
        let record_ref = Arc::clone(&record);
        let prompt_text = prompt.to_string();
        let session = session_id.to_string();
        let join = tokio::spawn(async move {
            let mut attempts: u32 = 0;
            loop {
                if *stop_rx.borrow() {
                    let mut guard = record_ref.write().await;
                    guard.status = BackgroundTaskStatus::Stopped;
                    guard.finished_at_ms = Some(current_time_ms());
                    append_log_line(&PathBuf::from(&guard.log_path), "[stop] stop signal observed")
                        .await;
                    break;
                }

                match agent.process_message(&session, &prompt_text).await {
                    Ok(reply) => {
                        let mut guard = record_ref.write().await;
                        guard.status = BackgroundTaskStatus::Completed;
                        guard.finished_at_ms = Some(current_time_ms());
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            &format!("[completed] reply:\n{reply}"),
                        )
                        .await;
                        break;
                    }
                    Err(error) => {
                        attempts = attempts.saturating_add(1);
                        let mut guard = record_ref.write().await;
                        guard.restart_count = attempts;
                        guard.last_error = Some(error.to_string());
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            &format!("[error] attempt={attempts} {}", error),
                        )
                        .await;
                        if attempts > guard.max_restarts {
                            guard.status = BackgroundTaskStatus::Failed;
                            guard.finished_at_ms = Some(current_time_ms());
                            break;
                        }
                        append_log_line(
                            &PathBuf::from(&guard.log_path),
                            "[restart] retrying agent task",
                        )
                        .await;
                    }
                }
            }
        });
        let join_ref = Arc::new(RwLock::new(Some(join)));

        self.tasks.write().await.insert(
            key,
            TaskHandle {
                record: Arc::clone(&record),
                stop_tx,
                join: join_ref,
            },
        );
        Ok(record.read().await.clone())
    }

    pub async fn stop_task(&self, session_id: &str, task_id: &str) -> Result<BackgroundTaskRecord> {
        let key = task_key(session_id, task_id);
        let tasks = self.tasks.read().await;
        let handle = tasks
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("background task not found"))?;
        let _ = handle.stop_tx.send(true);
        let join_ref = Arc::clone(&handle.join);
        let record_ref = Arc::clone(&handle.record);
        drop(tasks);

        {
            let mut record = record_ref.write().await;
            record.status = BackgroundTaskStatus::Stopping;
        }
        if let Some(join) = join_ref.write().await.take() {
            let _ = timeout(Duration::from_secs(2), join).await;
        }
        {
            let mut record = record_ref.write().await;
            if record.status == BackgroundTaskStatus::Stopping {
                record.status = BackgroundTaskStatus::Stopped;
                record.finished_at_ms = Some(current_time_ms());
            }
        }
        Ok(record_ref.read().await.clone())
    }

    pub async fn restart_task(&self, session_id: &str, task_id: &str, agent: AgentRuntime) -> Result<BackgroundTaskRecord> {
        let key = task_key(session_id, task_id);
        let (kind, spec, max_restarts) = {
            let tasks = self.tasks.read().await;
            let handle = tasks
                .get(&key)
                .ok_or_else(|| anyhow::anyhow!("background task not found"))?;
            let record = handle.record.read().await;
            (record.kind.clone(), record.spec.clone(), record.max_restarts)
        };

        let _ = self.stop_task(session_id, task_id).await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        match kind {
            BackgroundTaskKind::Shell => {
                self.start_shell_task(session_id, task_id, &spec, max_restarts)
                    .await
            }
            BackgroundTaskKind::Agent => {
                self.start_agent_task(session_id, task_id, &spec, max_restarts, agent)
                    .await
            }
        }
    }

    pub async fn status(
        &self,
        session_id: &str,
        task_id: Option<&str>,
    ) -> Result<Vec<BackgroundTaskRecord>> {
        let tasks = self.tasks.read().await;
        let mut rows = Vec::new();
        for handle in tasks.values() {
            let record = handle.record.read().await.clone();
            if record.session_id != session_id {
                continue;
            }
            if let Some(target) = task_id {
                if record.task_id != target {
                    continue;
                }
            }
            rows.push(record);
        }
        rows.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        Ok(rows)
    }

    pub async fn tail_logs(&self, session_id: &str, task_id: &str, lines: usize) -> Result<Vec<String>> {
        let key = task_key(session_id, task_id);
        let tasks = self.tasks.read().await;
        let handle = tasks
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!("background task not found"))?;
        let path = PathBuf::from(handle.record.read().await.log_path.clone());
        drop(tasks);

        let raw = std::fs::read_to_string(path).unwrap_or_default();
        let mut rows = raw.lines().map(|line| line.to_string()).collect::<Vec<_>>();
        if rows.len() > lines.max(1) {
            let start = rows.len().saturating_sub(lines.max(1));
            rows = rows[start..].to_vec();
        }
        Ok(rows)
    }

    async fn ensure_not_running(&self, key: &str) -> Result<()> {
        let tasks = self.tasks.read().await;
        if let Some(existing) = tasks.get(key) {
            let record = existing.record.read().await;
            if record.status == BackgroundTaskStatus::Running
                || record.status == BackgroundTaskStatus::Stopping
            {
                bail!("background task is already running: {}", record.task_id);
            }
        }
        Ok(())
    }

    fn log_path_for(&self, session_id: &str, task_id: &str) -> PathBuf {
        self.root
            .join(sanitize(session_id))
            .join(format!("{}.log", sanitize(task_id)))
    }
}

#[derive(Debug, Clone)]
struct ShellOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn run_shell_command(command: &str) -> Result<ShellOutput> {
    #[cfg(target_os = "windows")]
    let output = Command::new("powershell")
        .arg("-Command")
        .arg(command)
        .output()
        .await?;

    #[cfg(not(target_os = "windows"))]
    let output = Command::new("sh").arg("-lc").arg(command).output().await?;

    Ok(ShellOutput {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

async fn append_log_line(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut content = String::new();
    content.push_str(&format!("[{}] ", current_time_ms()));
    content.push_str(line);
    if !line.ends_with('\n') {
        content.push('\n');
    }
    if let Ok(mut file) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
    {
        use tokio::io::AsyncWriteExt;
        let _ = file.write_all(content.as_bytes()).await;
    }
}

fn task_key(session_id: &str, task_id: &str) -> String {
    format!("{}::{}", sanitize(session_id), sanitize(task_id))
}

fn sanitize(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if cleaned.is_empty() {
        "default".to_string()
    } else {
        cleaned
    }
}

fn prompt_requires_harness_facade(prompt: &str) -> bool {
    let lowered = prompt.to_ascii_lowercase();
    if lowered.contains("requires_artifact")
        || lowered.contains("artifact_delivery/v1")
        || lowered.contains("target_path")
        || lowered.contains("```")
    {
        return true;
    }
    let code_task_markers = [
        ".rs",
        ".py",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        ".html",
        ".css",
        "cargo ",
        "pytest",
        "npm run",
        "编译",
        "写代码",
        "代码",
        "修复bug",
        "debug",
        "implement",
        "refactor",
        "clone",
        "build frontend",
    ];
    code_task_markers
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shell_task_runs_and_logs_tail() {
        let manager = BackgroundTaskManager::new(std::env::temp_dir().join(format!(
            "autoloop-bg-task-tests-{}",
            current_time_ms()
        )));
        let record = manager
            .start_shell_task(
                "session-bg",
                "task-echo",
                "Write-Output 'background-task-ok'",
                0,
            )
            .await
            .expect("start");

        for _ in 0..30 {
            let status = manager
                .status("session-bg", Some("task-echo"))
                .await
                .expect("status poll");
            if let Some(row) = status.first() {
                if row.status != BackgroundTaskStatus::Running {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let rows = manager
            .tail_logs("session-bg", "task-echo", 20)
            .await
            .expect("tail");

        assert!(rows.iter().any(|line| line.contains("background-task-ok")));
        let status = manager
            .status("session-bg", Some("task-echo"))
            .await
            .expect("status");
        assert_eq!(status.len(), 1);
        assert_eq!(record.task_id, "task-echo");
    }

    #[test]
    fn prompt_classifier_requires_harness_for_code_tasks() {
        assert!(prompt_requires_harness_facade(
            "Implement frontend billing page in html/css and run cargo test"
        ));
        assert!(prompt_requires_harness_facade(
            r#"{"api_version":"artifact_delivery/v1","requires_artifact":true}"#
        ));
        assert!(!prompt_requires_harness_facade(
            "Summarize latest policy signals and replay mismatch."
        ));
    }
}
