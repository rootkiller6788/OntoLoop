use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use autoloop_state_adapter::StateStore;
use tokio::process::Command;

use crate::{
    contracts::code_harness::{GitCheckpoint, GitCheckpointAction},
    evolution_os::replay,
    runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
};

const GIT_CHECKPOINT_API_VERSION: &str = "git_checkpoint/v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitCheckpointOperationInput {
    pub action: GitCheckpointAction,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub rollback_ref: Option<String>,
    #[serde(default)]
    pub checkpoint_message: Option<String>,
    #[serde(default)]
    pub allow_dirty: bool,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitCheckpointRequest {
    pub api_version: String,
    #[serde(default = "default_true")]
    pub local_safe_mode: bool,
    #[serde(default = "default_true")]
    pub stop_on_error: bool,
    #[serde(default)]
    pub operations: Vec<GitCheckpointOperationInput>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitCheckpointStepRecord {
    pub op_index: usize,
    pub action: GitCheckpointAction,
    pub success: bool,
    #[serde(default)]
    pub checkpoint_ref: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitCheckpointReport {
    pub session_id: String,
    pub trace_id: String,
    pub success: bool,
    pub local_safe_mode: bool,
    pub halted_reason: String,
    pub step_count: usize,
    pub steps: Vec<GitCheckpointStepRecord>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
}

pub struct GitCheckpointLayer {
    workspace_root: PathBuf,
}

impl GitCheckpointLayer {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub async fn run(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        request: &GitCheckpointRequest,
    ) -> Result<GitCheckpointReport> {
        if request.api_version.trim() != GIT_CHECKPOINT_API_VERSION {
            bail!(
                "git checkpoint request api_version mismatch: expected {}, got {}",
                GIT_CHECKPOINT_API_VERSION,
                request.api_version
            );
        }
        if request.operations.is_empty() {
            bail!("git checkpoint operations cannot be empty");
        }
        self.ensure_git_repo().await?;

        let mut steps = Vec::with_capacity(request.operations.len());
        let mut success = true;
        let mut halted_reason = "completed".to_string();

        for (index, op) in request.operations.iter().enumerate() {
            let applied = self
                .apply_single_operation(session_id, trace_id, index, request.local_safe_mode, op)
                .await;
            match applied {
                Ok(checkpoint) => {
                    let key = format!(
                        "harness:git-checkpoint:step:{session_id}:{trace_id}:{}:{index}",
                        checkpoint.created_at_ms
                    );
                    db.upsert_json_knowledge(key.clone(), &checkpoint, "git-checkpoint-layer")
                        .await?;
                    steps.push(GitCheckpointStepRecord {
                        op_index: index,
                        action: checkpoint.action,
                        success: true,
                        checkpoint_ref: Some(key),
                        error: None,
                    });
                }
                Err(error) => {
                    success = false;
                    halted_reason = "operation_failed".to_string();
                    steps.push(GitCheckpointStepRecord {
                        op_index: index,
                        action: op.action.clone(),
                        success: false,
                        checkpoint_ref: None,
                        error: Some(error.to_string()),
                    });
                    if request.stop_on_error {
                        break;
                    }
                }
            }
        }

        let replay_fp = replay::build_fingerprint(
            "gitcheckpoint",
            "git-checkpoint/schema/v1",
            "git-checkpoint/seed/v1",
            "git-checkpoint/replay/v1",
            &serde_json::json!({
                "trace_id": trace_id,
                "local_safe_mode": request.local_safe_mode,
                "success": success,
                "halted_reason": halted_reason,
                "steps": steps,
            }),
        );
        let evidence_ref = EvidenceLedgerWriter::append_stage(
            db,
            session_id,
            trace_id,
            EvidenceStage::Execution,
            serde_json::json!({
                "stage": "git_checkpoint_layer",
                "success": success,
                "local_safe_mode": request.local_safe_mode,
                "step_count": steps.len(),
                "halted_reason": halted_reason,
                "replay_fp": replay_fp,
            }),
            None,
        )
        .await
        .ok();

        Ok(GitCheckpointReport {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            success,
            local_safe_mode: request.local_safe_mode,
            halted_reason,
            step_count: steps.len(),
            steps,
            evidence_ref,
            replay_fp: Some(replay_fp),
        })
    }

    async fn ensure_git_repo(&self) -> Result<()> {
        let output = self
            .run_git(vec!["rev-parse".to_string(), "--is-inside-work-tree".to_string()])
            .await?;
        if output.stdout.trim() != "true" {
            bail!(
                "workspace is not a git repository: {}",
                self.workspace_root.to_string_lossy()
            );
        }
        Ok(())
    }

    async fn apply_single_operation(
        &self,
        session_id: &str,
        trace_id: &str,
        op_index: usize,
        local_safe_mode: bool,
        op: &GitCheckpointOperationInput,
    ) -> Result<GitCheckpoint> {
        match op.action {
            GitCheckpointAction::Branch => {
                let branch = op
                    .branch
                    .as_deref()
                    .ok_or_else(|| anyhow!("git branch action requires branch"))?;
                validate_git_ref_name(branch, "branch")?;
                if let Some(base) = op.base_branch.as_deref() {
                    validate_git_ref_name(base, "base_branch")?;
                }
                let dirty = self.is_dirty().await?;
                let parent_commit_sha = Some(self.current_commit_sha().await?);
                if self.branch_exists(branch).await? {
                    self.run_git(vec!["checkout".to_string(), branch.to_string()])
                        .await?;
                } else if let Some(base) = op.base_branch.as_deref() {
                    self.run_git(vec![
                        "checkout".to_string(),
                        "-b".to_string(),
                        branch.to_string(),
                        base.to_string(),
                    ])
                    .await?;
                } else {
                    self.run_git(vec![
                        "checkout".to_string(),
                        "-b".to_string(),
                        branch.to_string(),
                    ])
                    .await?;
                }
                let commit_sha = Some(self.current_commit_sha().await?);
                Ok(GitCheckpoint {
                    checkpoint_id: format!("git-checkpoint:{}:{}:{op_index}", session_id, trace_id),
                    session_id: session_id.to_string(),
                    trace_id: trace_id.to_string(),
                    action: GitCheckpointAction::Branch,
                    branch: branch.to_string(),
                    base_branch: op.base_branch.clone(),
                    commit_sha,
                    parent_commit_sha,
                    tag: None,
                    dirty,
                    created_at_ms: now_ms(),
                    evidence_ref: None,
                    metadata: op.metadata.clone(),
                })
            }
            GitCheckpointAction::Commit => {
                let dirty_before = self.is_dirty().await?;
                if !op.allow_dirty && !dirty_before {
                    bail!("git commit checkpoint requires dirty workspace unless allow_dirty=true");
                }
                let branch = self.current_branch().await?;
                let parent_commit_sha = Some(self.current_commit_sha().await?);
                self.run_git(vec!["add".to_string(), "-A".to_string()]).await?;
                let message = op
                    .checkpoint_message
                    .clone()
                    .unwrap_or_else(|| format!("OntoLoop checkpoint {trace_id}"));
                self.run_git(vec![
                    "commit".to_string(),
                    "--allow-empty".to_string(),
                    "-m".to_string(),
                    message.clone(),
                ])
                .await?;
                let mut metadata = op.metadata.clone();
                metadata.insert("checkpoint_message".to_string(), message);
                Ok(GitCheckpoint {
                    checkpoint_id: format!("git-checkpoint:{}:{}:{op_index}", session_id, trace_id),
                    session_id: session_id.to_string(),
                    trace_id: trace_id.to_string(),
                    action: GitCheckpointAction::Commit,
                    branch,
                    base_branch: op.base_branch.clone(),
                    commit_sha: Some(self.current_commit_sha().await?),
                    parent_commit_sha,
                    tag: None,
                    dirty: dirty_before,
                    created_at_ms: now_ms(),
                    evidence_ref: None,
                    metadata,
                })
            }
            GitCheckpointAction::Tag => {
                let tag = op
                    .tag
                    .clone()
                    .unwrap_or_else(|| format!("checkpoint-{session_id}-{}", now_ms()));
                validate_git_ref_name(&tag, "tag")?;
                self.run_git(vec!["tag".to_string(), "-f".to_string(), tag.clone()])
                    .await?;
                Ok(GitCheckpoint {
                    checkpoint_id: format!("git-checkpoint:{}:{}:{op_index}", session_id, trace_id),
                    session_id: session_id.to_string(),
                    trace_id: trace_id.to_string(),
                    action: GitCheckpointAction::Tag,
                    branch: self.current_branch().await?,
                    base_branch: op.base_branch.clone(),
                    commit_sha: Some(self.current_commit_sha().await?),
                    parent_commit_sha: None,
                    tag: Some(tag),
                    dirty: self.is_dirty().await?,
                    created_at_ms: now_ms(),
                    evidence_ref: None,
                    metadata: op.metadata.clone(),
                })
            }
            GitCheckpointAction::Rollback => {
                let rollback_ref = op
                    .rollback_ref
                    .as_deref()
                    .or(op.branch.as_deref())
                    .or(op.tag.as_deref())
                    .or(op.base_branch.as_deref())
                    .ok_or_else(|| anyhow!("git rollback action requires rollback_ref"))?;
                validate_git_ref_name(rollback_ref, "rollback_ref")?;
                if local_safe_mode && rollback_ref.to_ascii_lowercase().starts_with("origin/") {
                    bail!(
                        "local_safe_mode blocks remote rollback refs: {}",
                        rollback_ref
                    );
                }
                let parent_commit_sha = Some(self.current_commit_sha().await?);
                self.run_git(vec![
                    "reset".to_string(),
                    "--hard".to_string(),
                    rollback_ref.to_string(),
                ])
                .await?;
                Ok(GitCheckpoint {
                    checkpoint_id: format!("git-checkpoint:{}:{}:{op_index}", session_id, trace_id),
                    session_id: session_id.to_string(),
                    trace_id: trace_id.to_string(),
                    action: GitCheckpointAction::Rollback,
                    branch: self.current_branch().await?,
                    base_branch: op.base_branch.clone(),
                    commit_sha: Some(self.current_commit_sha().await?),
                    parent_commit_sha,
                    tag: None,
                    dirty: self.is_dirty().await?,
                    created_at_ms: now_ms(),
                    evidence_ref: None,
                    metadata: op.metadata.clone(),
                })
            }
        }
    }

    async fn branch_exists(&self, branch: &str) -> Result<bool> {
        let output = self
            .run_git(vec![
                "rev-parse".to_string(),
                "--verify".to_string(),
                format!("refs/heads/{branch}"),
            ])
            .await;
        Ok(output.is_ok())
    }

    async fn current_commit_sha(&self) -> Result<String> {
        Ok(self
            .run_git(vec!["rev-parse".to_string(), "HEAD".to_string()])
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn current_branch(&self) -> Result<String> {
        Ok(self
            .run_git(vec![
                "rev-parse".to_string(),
                "--abbrev-ref".to_string(),
                "HEAD".to_string(),
            ])
            .await?
            .stdout
            .trim()
            .to_string())
    }

    async fn is_dirty(&self) -> Result<bool> {
        let output = self
            .run_git(vec![
                "status".to_string(),
                "--porcelain".to_string(),
                "--untracked-files=normal".to_string(),
            ])
            .await?;
        Ok(!output.stdout.trim().is_empty())
    }

    async fn run_git(&self, args: Vec<String>) -> Result<GitCommandOutput> {
        let output = Command::new("git")
            .args(args.iter())
            .current_dir(&self.workspace_root)
            .output()
            .await
            .map_err(|error| anyhow!("git execution failed: {}", error))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.status.success() {
            bail!(
                "git command failed exit={:?} stderr={} stdout={}",
                output.status.code(),
                trim_tail(stderr.trim()),
                trim_tail(stdout.trim())
            );
        }
        Ok(GitCommandOutput { stdout })
    }
}

#[derive(Debug, Clone)]
struct GitCommandOutput {
    stdout: String,
}

fn default_true() -> bool {
    true
}

fn validate_git_ref_name(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{field} cannot be empty");
    }
    if value.contains("..")
        || value.contains('~')
        || value.contains('^')
        || value.contains(':')
        || value.contains('?')
        || value.contains('*')
        || value.contains('\\')
        || value.ends_with('.')
        || value.starts_with('-')
    {
        bail!("{field} contains unsupported git ref characters: {value}");
    }
    Ok(())
}

fn trim_tail(input: &str) -> String {
    if input.chars().count() <= 500 {
        return input.to_string();
    }
    input
        .chars()
        .rev()
        .take(500)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_git_ref_rejects_unsafe_chars() {
        assert!(validate_git_ref_name("feature/new", "branch").is_ok());
        assert!(validate_git_ref_name("bad:ref", "branch").is_err());
        assert!(validate_git_ref_name("bad..ref", "branch").is_err());
        assert!(validate_git_ref_name("-bad", "branch").is_err());
    }

    #[test]
    fn request_defaults_to_local_safe_mode_and_stop_on_error() {
        let request: GitCheckpointRequest = serde_json::from_value(serde_json::json!({
            "api_version": "git_checkpoint/v1",
            "operations": [{ "action": "branch", "branch": "feature/test" }]
        }))
        .expect("request parse");
        assert!(request.local_safe_mode);
        assert!(request.stop_on_error);
    }
}
