use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};

const CONTEXT_KERNEL_SOURCE: &str = "context-runtime-kernel";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRuntimeKernelMode {
    Shadow,
    Enforce,
    Disabled,
}

impl ContextRuntimeKernelMode {
    pub fn from_env() -> Self {
        let raw = std::env::var("AUTOLOOP_CONTEXT_KERNEL_MODE")
            .unwrap_or_else(|_| "shadow".to_string())
            .trim()
            .to_ascii_lowercase();
        match raw.as_str() {
            "disabled" | "off" | "false" => Self::Disabled,
            "enforce" | "strict" | "hard" => Self::Enforce,
            _ => Self::Shadow,
        }
    }

    fn shadow_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    fn fail_closed(self) -> bool {
        matches!(self, Self::Enforce)
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::Enforce => "enforce",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRuntimeRun {
    pub run_id: String,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRuntimeKernelInput {
    pub run_id: String,
    pub session_id: String,
    pub entrypoint: String,
    pub mode: String,
    pub content_len: usize,
    pub content_preview: String,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRuntimeKernelOutput {
    pub run_id: String,
    pub session_id: String,
    pub entrypoint: String,
    pub mode: String,
    pub status: String,
    pub response_len: Option<usize>,
    pub response_preview: Option<String>,
    pub error: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub duration_ms: u64,
}

#[derive(Clone)]
pub struct ContextRuntimeKernel {
    db: StateStore,
    mode: ContextRuntimeKernelMode,
}

impl ContextRuntimeKernel {
    pub fn new(db: StateStore) -> Self {
        Self {
            db,
            mode: ContextRuntimeKernelMode::from_env(),
        }
    }

    pub fn with_mode(db: StateStore, mode: ContextRuntimeKernelMode) -> Self {
        Self { db, mode }
    }

    pub fn mode(&self) -> ContextRuntimeKernelMode {
        self.mode
    }

    pub async fn begin_turn(
        &self,
        session_id: &str,
        entrypoint: &str,
        content: &str,
    ) -> Result<ContextRuntimeRun> {
        let started_at_ms = current_time_ms();
        let run_id = format!("{entrypoint}:{session_id}:{started_at_ms}");
        let run = ContextRuntimeRun {
            run_id,
            started_at_ms,
        };
        if !self.mode.shadow_enabled() {
            return Ok(run);
        }

        let input = ContextRuntimeKernelInput {
            run_id: run.run_id.clone(),
            session_id: session_id.to_string(),
            entrypoint: entrypoint.to_string(),
            mode: self.mode.as_str().to_string(),
            content_len: content.chars().count(),
            content_preview: preview(content, 280),
            started_at_ms,
        };
        let input_key = format!(
            "context-kernel:shadow:{session_id}:run:{}:input",
            run.run_id
        );
        let latest_key = format!("context-kernel:shadow:{session_id}:latest");

        self.write_shadow(input_key.clone(), &input).await?;
        self.write_shadow(
            latest_key,
            &serde_json::json!({
                "session_id": session_id,
                "entrypoint": entrypoint,
                "run_id": run.run_id,
                "mode": self.mode.as_str(),
                "input_ref": input_key,
                "updated_at_ms": started_at_ms,
            }),
        )
        .await?;
        Ok(run)
    }

    pub async fn finish_turn(
        &self,
        session_id: &str,
        entrypoint: &str,
        run: &ContextRuntimeRun,
        outcome: Result<&str, &str>,
    ) -> Result<()> {
        if !self.mode.shadow_enabled() {
            return Ok(());
        }
        let finished_at_ms = current_time_ms();
        let duration_ms = finished_at_ms.saturating_sub(run.started_at_ms);
        let output_key = format!(
            "context-kernel:shadow:{session_id}:run:{}:output",
            run.run_id
        );
        let latest_key = format!("context-kernel:shadow:{session_id}:latest");
        let output = match outcome {
            Ok(response) => ContextRuntimeKernelOutput {
                run_id: run.run_id.clone(),
                session_id: session_id.to_string(),
                entrypoint: entrypoint.to_string(),
                mode: self.mode.as_str().to_string(),
                status: "ok".to_string(),
                response_len: Some(response.chars().count()),
                response_preview: Some(preview(response, 280)),
                error: None,
                started_at_ms: run.started_at_ms,
                finished_at_ms,
                duration_ms,
            },
            Err(error) => ContextRuntimeKernelOutput {
                run_id: run.run_id.clone(),
                session_id: session_id.to_string(),
                entrypoint: entrypoint.to_string(),
                mode: self.mode.as_str().to_string(),
                status: "error".to_string(),
                response_len: None,
                response_preview: None,
                error: Some(preview(error, 400)),
                started_at_ms: run.started_at_ms,
                finished_at_ms,
                duration_ms,
            },
        };

        self.write_shadow(output_key.clone(), &output).await?;
        self.write_shadow(
            latest_key,
            &serde_json::json!({
                "session_id": session_id,
                "entrypoint": entrypoint,
                "run_id": run.run_id,
                "mode": self.mode.as_str(),
                "status": output.status,
                "input_ref": format!("context-kernel:shadow:{session_id}:run:{}:input", run.run_id),
                "output_ref": output_key,
                "updated_at_ms": finished_at_ms,
            }),
        )
        .await?;
        Ok(())
    }

    async fn write_shadow<T: Serialize>(&self, key: String, value: &T) -> Result<()> {
        let write = self
            .db
            .upsert_json_knowledge(key, value, CONTEXT_KERNEL_SOURCE)
            .await;
        match write {
            Ok(_) => Ok(()),
            Err(error) if self.mode.fail_closed() => Err(error.into()),
            Err(_) => Ok(()),
        }
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn preview(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let clipped = input.chars().take(max_chars).collect::<String>();
    format!("{clipped}...")
}

