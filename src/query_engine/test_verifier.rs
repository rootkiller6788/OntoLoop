use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use autoloop_state_adapter::StateStore;
use tokio::{
    process::Command,
    time::{Duration, timeout},
};

use crate::{
    contracts::code_harness::{TestVerdict, TestVerdictStatus},
    evolution_os::replay,
    runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestRunnerKind {
    Build,
    Lint,
    Test,
    Custom,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StageVerdict {
    pub stage: TestRunnerKind,
    pub covered: bool,
    pub passed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestRunnerSpec {
    pub runner_id: String,
    pub kind: TestRunnerKind,
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_required")]
    pub required: bool,
}

fn default_required() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestVerifierRequest {
    pub api_version: String,
    #[serde(default = "default_fail_fast")]
    pub fail_fast: bool,
    pub runners: Vec<TestRunnerSpec>,
}

fn default_fail_fast() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestRunnerResult {
    pub runner_id: String,
    pub kind: TestRunnerKind,
    pub verdict: TestVerdictStatus,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout_tail: Option<String>,
    #[serde(default)]
    pub stderr_tail: Option<String>,
    pub duration_ms: u64,
    pub required: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TestVerifierReport {
    pub session_id: String,
    pub trace_id: String,
    pub hard_pass: bool,
    pub hard_fail: bool,
    pub results: Vec<TestRunnerResult>,
    #[serde(default)]
    pub stage_verdicts: Vec<StageVerdict>,
    pub summary: String,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
}

pub struct TestVerifierEngine {
    workspace_root: PathBuf,
}

impl TestVerifierEngine {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub async fn verify(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        request: &TestVerifierRequest,
    ) -> Result<TestVerifierReport> {
        if request.runners.is_empty() {
            bail!("test verifier requires at least one runner");
        }
        let mut results = Vec::new();

        for runner in &request.runners {
            let start = now_ms();
            let execution = run_command(
                &runner.command,
                self.resolve_cwd(runner.cwd.as_deref())?.as_path(),
                runner.timeout_ms,
            )
            .await;
            let duration_ms = now_ms().saturating_sub(start);
            let result = match execution {
                Ok(output) => TestRunnerResult {
                    runner_id: runner.runner_id.clone(),
                    kind: runner.kind.clone(),
                    verdict: if output.exit_code == Some(0) {
                        TestVerdictStatus::Pass
                    } else {
                        TestVerdictStatus::Fail
                    },
                    exit_code: output.exit_code,
                    stdout_tail: Some(trim_tail(&output.stdout)),
                    stderr_tail: Some(trim_tail(&output.stderr)),
                    duration_ms,
                    required: runner.required,
                },
                Err(error) => {
                    let message = error.to_string();
                    TestRunnerResult {
                        runner_id: runner.runner_id.clone(),
                        kind: runner.kind.clone(),
                        verdict: if message.to_ascii_lowercase().contains("timeout") {
                            TestVerdictStatus::Error
                        } else {
                            TestVerdictStatus::Fail
                        },
                        exit_code: None,
                        stdout_tail: None,
                        stderr_tail: Some(trim_tail(&message)),
                        duration_ms,
                        required: runner.required,
                    }
                }
            };
            let verdict_key = format!(
                "harness:test-verdict:{session_id}:{trace_id}:{}:{}",
                now_ms(),
                sanitize_key_component(&runner.runner_id)
            );
            let verdict = TestVerdict {
                verdict_id: verdict_key.clone(),
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                runner: format!("{:?}", runner.kind).to_ascii_lowercase(),
                target: runner.command.clone(),
                status: result.verdict.clone(),
                summary: Some(format!(
                    "runner={} required={} verdict={:?}",
                    runner.runner_id, runner.required, result.verdict
                )),
                failed_cases: Vec::new(),
                duration_ms: Some(duration_ms),
                evidence_ref: None,
            };
            db.upsert_json_knowledge(verdict_key, &verdict, "test-verifier")
                .await?;

            let required_failed = runner.required
                && matches!(result.verdict, TestVerdictStatus::Fail | TestVerdictStatus::Error);
            results.push(result);
            if required_failed && request.fail_fast {
                break;
            }
        }

        let stage_verdicts = compute_stage_verdicts(&results);
        let missing_required_stage = stage_verdicts.iter().any(|stage| !stage.covered);
        let failed_required_stage = stage_verdicts.iter().any(|stage| stage.covered && !stage.passed);
        let hard_fail = results.iter().any(|item| {
            item.required
                && matches!(item.verdict, TestVerdictStatus::Fail | TestVerdictStatus::Error)
        }) || missing_required_stage
            || failed_required_stage;
        let hard_pass = !hard_fail
            && stage_verdicts.iter().all(|stage| stage.covered && stage.passed)
            && results.iter().any(|item| matches!(item.verdict, TestVerdictStatus::Pass));
        let summary = if missing_required_stage {
            "hard_fail: missing required verifier stage(s) build/lint/test".to_string()
        } else if failed_required_stage {
            "hard_fail: required verifier stage failed".to_string()
        } else if hard_fail {
            "hard_fail: required runner failed".to_string()
        } else {
            "hard_pass: build/lint/test all passed".to_string()
        };
        let replay_fp = replay::build_fingerprint(
            "testverifier",
            "test-verifier/schema/v1",
            "test-verifier/seed/v1",
            "test-verifier/replay/v1",
            &serde_json::json!({
                "trace_id": trace_id,
                "workspace_root": self.workspace_root.to_string_lossy().replace('\\', "/"),
                "summary": summary,
                "results": results,
            }),
        );
        let evidence_ref = EvidenceLedgerWriter::append_stage(
            db,
            session_id,
            trace_id,
            EvidenceStage::Verify,
            serde_json::json!({
                "stage": "test_verifier",
                "hard_pass": hard_pass,
                "hard_fail": hard_fail,
                "result_count": results.len(),
                "stage_verdicts": stage_verdicts,
                "replay_fp": replay_fp,
            }),
            None,
        )
        .await
        .ok();

        Ok(TestVerifierReport {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            hard_pass,
            hard_fail,
            results,
            stage_verdicts,
            summary,
            evidence_ref,
            replay_fp: Some(replay_fp),
        })
    }

    fn resolve_cwd(&self, relative: Option<&str>) -> Result<PathBuf> {
        let candidate = match relative {
            Some(value) if !value.trim().is_empty() => {
                let path = PathBuf::from(value);
                if path.is_absolute() {
                    path
                } else {
                    self.workspace_root.join(path)
                }
            }
            _ => self.workspace_root.clone(),
        };
        let normalized = normalize_without_resolving(&candidate);
        let root = normalize_without_resolving(&self.workspace_root);
        if !normalized.starts_with(&root) {
            bail!(
                "test verifier cwd escapes workspace root (target={}, root={})",
                normalized.display(),
                root.display()
            );
        }
        Ok(normalized)
    }
}

#[derive(Debug, Clone)]
struct CommandOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn run_command(command: &str, cwd: &Path, timeout_ms: Option<u64>) -> Result<CommandOutput> {
    if is_high_risk_command(command) {
        bail!("command rejected by test verifier guard");
    }
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("powershell");
        c.arg("-Command").arg(command).current_dir(cwd);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-lc").arg(command).current_dir(cwd);
        c
    };
    let budget = Duration::from_millis(timeout_ms.unwrap_or(90_000).clamp(500, 300_000));
    let output = timeout(budget, cmd.output())
        .await
        .map_err(|_| anyhow!("command timeout"))??;
    Ok(CommandOutput {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn is_high_risk_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let banned = [
        "rm -rf",
        "remove-item -recurse -force",
        "del /f /s /q",
        "format ",
        "shutdown ",
        "reboot",
        "mkfs",
        "dd if=",
        "diskpart",
    ];
    banned.iter().any(|item| lower.contains(item))
}

fn trim_tail(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() > 800 {
        trimmed
            .chars()
            .rev()
            .take(800)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    } else {
        trimmed.to_string()
    }
}

fn compute_stage_verdicts(results: &[TestRunnerResult]) -> Vec<StageVerdict> {
    let core_stages = [TestRunnerKind::Build, TestRunnerKind::Lint, TestRunnerKind::Test];
    core_stages
        .iter()
        .map(|stage| {
            let stage_results = results
                .iter()
                .filter(|item| item.required && item.kind == *stage)
                .collect::<Vec<_>>();
            if stage_results.is_empty() {
                return StageVerdict {
                    stage: stage.clone(),
                    covered: false,
                    passed: false,
                };
            }
            let passed = stage_results
                .iter()
                .all(|item| matches!(item.verdict, TestVerdictStatus::Pass));
            StageVerdict {
                stage: stage.clone(),
                covered: true,
                passed,
            }
        })
        .collect()
}

fn sanitize_key_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn normalize_without_resolving(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use autoloop_state_adapter::{StateStore, StateStoreBackend, StateStoreConfig};

    use super::*;

    fn db() -> StateStore {
        StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        })
    }

    #[tokio::test]
    async fn verifier_outputs_hard_pass_when_all_required_runners_pass() {
        let engine = TestVerifierEngine::new(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let request = TestVerifierRequest {
            api_version: "test_verifier/v1".to_string(),
            fail_fast: true,
            runners: vec![
                TestRunnerSpec {
                    runner_id: "build".to_string(),
                    kind: TestRunnerKind::Build,
                    command: "Write-Output 'ok-build'".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
                TestRunnerSpec {
                    runner_id: "lint".to_string(),
                    kind: TestRunnerKind::Lint,
                    command: "Write-Output 'ok-lint'".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
                TestRunnerSpec {
                    runner_id: "test".to_string(),
                    kind: TestRunnerKind::Test,
                    command: "Write-Output 'ok-test'".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
            ],
        };
        let report = engine
            .verify(&db(), "session-tv-pass", "trace-tv-pass", &request)
            .await
            .expect("verify");
        assert!(report.hard_pass);
        assert!(!report.hard_fail);
        assert_eq!(report.results.len(), 3);
        assert!(report.stage_verdicts.iter().all(|item| item.covered && item.passed));
    }

    #[tokio::test]
    async fn verifier_outputs_hard_fail_when_required_runner_fails() {
        let engine = TestVerifierEngine::new(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let request = TestVerifierRequest {
            api_version: "test_verifier/v1".to_string(),
            fail_fast: true,
            runners: vec![
                TestRunnerSpec {
                    runner_id: "build".to_string(),
                    kind: TestRunnerKind::Build,
                    command: "Write-Output 'ok-build'".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
                TestRunnerSpec {
                    runner_id: "lint".to_string(),
                    kind: TestRunnerKind::Lint,
                    command: "Write-Error 'fail'; exit 1".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
                TestRunnerSpec {
                    runner_id: "test".to_string(),
                    kind: TestRunnerKind::Test,
                    command: "Write-Output 'ok-test'".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
            ],
        };
        let report = engine
            .verify(&db(), "session-tv-fail", "trace-tv-fail", &request)
            .await
            .expect("verify");
        assert!(report.hard_fail);
        assert!(!report.hard_pass);
    }

    #[tokio::test]
    async fn verifier_hard_fails_when_build_lint_test_stage_missing() {
        let engine = TestVerifierEngine::new(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let request = TestVerifierRequest {
            api_version: "test_verifier/v1".to_string(),
            fail_fast: false,
            runners: vec![
                TestRunnerSpec {
                    runner_id: "build".to_string(),
                    kind: TestRunnerKind::Build,
                    command: "Write-Output 'ok-build'".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
                TestRunnerSpec {
                    runner_id: "lint".to_string(),
                    kind: TestRunnerKind::Lint,
                    command: "Write-Output 'ok-lint'".to_string(),
                    cwd: None,
                    timeout_ms: Some(5_000),
                    required: true,
                },
            ],
        };
        let report = engine
            .verify(&db(), "session-tv-missing-stage", "trace-tv-missing-stage", &request)
            .await
            .expect("verify");
        assert!(report.hard_fail);
        assert!(
            report
                .summary
                .to_ascii_lowercase()
                .contains("missing required verifier stage"),
            "summary should mention missing stage"
        );
        assert!(
            report
                .stage_verdicts
                .iter()
                .any(|stage| stage.stage == TestRunnerKind::Test && !stage.covered)
        );
    }
}
