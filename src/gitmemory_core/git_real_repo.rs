use std::path::Path;
use std::process::Command;

use anyhow::{Result, anyhow, bail};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GitCommitReceipt {
    pub branch: String,
    pub commit_sha: String,
    pub parent_commit_sha: Option<String>,
    pub message: String,
}

pub struct GitRealRepo;

impl GitRealRepo {
    pub fn commit_path(repo_root: &Path, relative_path: &str, message: &str) -> Result<GitCommitReceipt> {
        ensure_repo_initialized(repo_root)?;
        ensure_identity(repo_root)?;
        let parent = current_commit_sha(repo_root)?;

        run_git(repo_root, &["add", "--", relative_path])?;
        run_git(repo_root, &["commit", "--allow-empty", "-m", message])?;

        let branch = current_branch(repo_root)?;
        let commit_sha = current_commit_sha(repo_root)?
            .ok_or_else(|| anyhow!("git commit succeeded but no commit sha found"))?;
        Ok(GitCommitReceipt {
            branch,
            commit_sha,
            parent_commit_sha: parent,
            message: message.to_string(),
        })
    }
}

fn ensure_repo_initialized(repo_root: &Path) -> Result<()> {
    let probe = run_git(repo_root, &["rev-parse", "--is-inside-work-tree"]);
    if probe.is_ok() {
        return Ok(());
    }
    run_git(repo_root, &["init"])?;
    Ok(())
}

fn ensure_identity(repo_root: &Path) -> Result<()> {
    if run_git(repo_root, &["config", "user.email"]).is_err() {
        run_git(repo_root, &["config", "user.email", "ontoloop@local"])?;
    }
    if run_git(repo_root, &["config", "user.name"]).is_err() {
        run_git(repo_root, &["config", "user.name", "OntoLoop"])?;
    }
    Ok(())
}

fn current_commit_sha(repo_root: &Path) -> Result<Option<String>> {
    let output = run_git(repo_root, &["rev-parse", "HEAD"]);
    match output {
        Ok(value) => Ok(Some(value)),
        Err(_) => Ok(None),
    }
}

fn current_branch(repo_root: &Path) -> Result<String> {
    run_git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn run_git(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "git command failed: git {} :: {}",
            args.join(" "),
            if stderr.is_empty() { "unknown error" } else { &stderr }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
