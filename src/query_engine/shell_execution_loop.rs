use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, bail};
use autoloop_state_adapter::StateStore;
use tokio::{
    process::Command,
    time::{Duration, timeout},
};

use crate::{
    contracts::code_harness::{ExecutionStep, ExecutionStepStatus},
    evolution_os::replay,
    query_engine::iteration_controller::{FailureCategory, classify_failure},
    runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage},
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellLoopPhase {
    Run,
    Capture,
    Classify,
    Patch,
    Verify,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellLoopPhaseStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellLoopPhaseRecord {
    pub phase: ShellLoopPhase,
    pub status: ShellLoopPhaseStatus,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellLoopStepInput {
    pub step_id: String,
    pub command: String,
    #[serde(default)]
    pub continue_on_error: bool,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellLoopRequest {
    pub api_version: String,
    #[serde(default)]
    pub max_iterations: Option<usize>,
    #[serde(default)]
    pub stop_on_success: bool,
    #[serde(default)]
    pub steps: Vec<ShellLoopStepInput>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellLoopIterationRecord {
    pub step_id: String,
    pub command: String,
    pub status: ExecutionStepStatus,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout_tail: Option<String>,
    #[serde(default)]
    pub stderr_tail: Option<String>,
    pub duration_ms: u64,
    #[serde(default)]
    pub context_feedback: Option<String>,
    #[serde(default)]
    pub phase_trace: Vec<ShellLoopPhaseRecord>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShellLoopReport {
    pub session_id: String,
    pub trace_id: String,
    pub success: bool,
    pub iterations: Vec<ShellLoopIterationRecord>,
    pub context_feedback: Vec<String>,
    pub halted_reason: String,
    #[serde(default = "default_state_machine_version")]
    pub state_machine_version: String,
    #[serde(default = "default_fixed_phase_order")]
    pub fixed_phase_order: Vec<ShellLoopPhase>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
}

fn default_state_machine_version() -> String {
    "shell-loop-state-machine/v1".to_string()
}

fn default_fixed_phase_order() -> Vec<ShellLoopPhase> {
    vec![
        ShellLoopPhase::Run,
        ShellLoopPhase::Capture,
        ShellLoopPhase::Classify,
        ShellLoopPhase::Patch,
        ShellLoopPhase::Verify,
    ]
}

pub struct ShellExecutionLoopEngine {
    workspace_root: PathBuf,
}

impl ShellExecutionLoopEngine {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub async fn run(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        request: &ShellLoopRequest,
    ) -> Result<ShellLoopReport> {
        let mut iterations = Vec::new();
        let mut context_feedback = Vec::new();
        let mut success = true;
        let mut halted_reason = "completed".to_string();
        let fixed_phase_order = default_fixed_phase_order();
        let max = request
            .max_iterations
            .unwrap_or(request.steps.len())
            .min(request.steps.len())
            .max(1);

        for (idx, step) in request.steps.iter().take(max).enumerate() {
            let start_ms = now_ms();
            let exec = run_single_command(&step.command, &self.workspace_root, step.timeout_ms).await;
            let (status, exit_code, stdout_tail, stderr_tail): (
                ExecutionStepStatus,
                Option<i32>,
                Option<String>,
                Option<String>,
            ) = match exec {
                Ok(output) => {
                    let status = if output.exit_code == Some(0) {
                        ExecutionStepStatus::Passed
                    } else {
                        ExecutionStepStatus::Failed
                    };
                    (
                        status,
                        output.exit_code,
                        Some(trim_tail(&output.stdout)),
                        Some(trim_tail(&output.stderr)),
                    )
                }
                Err(error) => {
                    let msg = error.to_string();
                    let status = if msg.to_ascii_lowercase().contains("timeout") {
                        ExecutionStepStatus::Timeout
                    } else {
                        ExecutionStepStatus::Failed
                    };
                    (status, None, None, Some(trim_tail(msg.as_str())))
                }
            };
            let duration_ms = now_ms().saturating_sub(start_ms);
            let failure_input = build_failure_input(exit_code, stdout_tail.as_deref(), stderr_tail.as_deref());
            let failure_category = classify_failure(&failure_input);
            let patch_hint = build_patch_hint(&failure_category, step, stderr_tail.as_deref());
            let phase_trace = build_phase_trace(
                &status,
                &failure_category,
                exit_code,
                stdout_tail.as_deref(),
                stderr_tail.as_deref(),
                patch_hint.as_deref(),
            );
            let feedback = build_feedback(
                idx,
                step,
                exit_code,
                stdout_tail.as_deref(),
                stderr_tail.as_deref(),
                &failure_category,
                patch_hint.as_deref(),
            );
            context_feedback.push(feedback.clone());
            iterations.push(ShellLoopIterationRecord {
                step_id: step.step_id.clone(),
                command: step.command.clone(),
                status: status.clone(),
                exit_code,
                stdout_tail: stdout_tail.clone(),
                stderr_tail: stderr_tail.clone(),
                duration_ms,
                context_feedback: Some(feedback),
                phase_trace,
            });

            let exec_step = ExecutionStep {
                step_id: step.step_id.clone(),
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                attempt: (idx + 1) as u32,
                command: step.command.clone(),
                cwd: Some(self.workspace_root.to_string_lossy().replace('\\', "/")),
                env: BTreeMap::new(),
                status: match status {
                    ExecutionStepStatus::Passed => ExecutionStepStatus::Passed,
                    ExecutionStepStatus::Failed => ExecutionStepStatus::Failed,
                    ExecutionStepStatus::Timeout => ExecutionStepStatus::Timeout,
                    _ => status.clone(),
                },
                exit_code,
                stdout_tail: stdout_tail.clone(),
                stderr_tail: stderr_tail.clone(),
                duration_ms: Some(duration_ms),
                evidence_ref: None,
            };
            let step_key = format!("harness:shell-loop:step:{session_id}:{trace_id}:{start_ms}:{idx}");
            db.upsert_json_knowledge(step_key, &exec_step, "shell-execution-loop")
                .await?;

            match status {
                ExecutionStepStatus::Passed if request.stop_on_success => {
                    halted_reason = "stop_on_success".to_string();
                    break;
                }
                ExecutionStepStatus::Passed => {}
                _ if step.continue_on_error => {}
                _ => {
                    success = false;
                    halted_reason = "step_failed".to_string();
                    break;
                }
            }
        }

        let replay_fp = replay::build_fingerprint(
            "shellloop",
            "shell-loop/schema/v1",
            "shell-loop/seed/v1",
            "shell-loop/replay/v1",
            &serde_json::json!({
                "trace_id": trace_id,
                "workspace_root": self.workspace_root.to_string_lossy().replace('\\', "/"),
                "success": success,
                "halted_reason": halted_reason,
                "iterations": iterations,
            }),
        );
        let evidence_ref = EvidenceLedgerWriter::append_stage(
            db,
            session_id,
            trace_id,
            EvidenceStage::Execution,
            serde_json::json!({
                "stage": "shell_execution_loop",
                "success": success,
                "iteration_count": iterations.len(),
                "halted_reason": halted_reason,
                "state_machine_version": "shell-loop-state-machine/v1",
                "fixed_phase_order": fixed_phase_order,
                "replay_fp": replay_fp,
            }),
            None,
        )
        .await
        .ok();

        Ok(ShellLoopReport {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            success,
            iterations,
            context_feedback,
            halted_reason,
            state_machine_version: "shell-loop-state-machine/v1".to_string(),
            fixed_phase_order,
            evidence_ref,
            replay_fp: Some(replay_fp),
        })
    }
}

struct ShellOutput {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

async fn run_single_command(command: &str, cwd: &PathBuf, timeout_ms: Option<u64>) -> Result<ShellOutput> {
    if is_high_risk_command(command) {
        bail!("command rejected by shell loop guard");
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
    let timeout_budget = Duration::from_millis(timeout_ms.unwrap_or(15_000).clamp(250, 120_000));
    let output = timeout(timeout_budget, cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("command timeout"))??;

    Ok(ShellOutput {
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
        "bcdedit",
        "reg delete",
    ];
    banned.iter().any(|item| lower.contains(item))
}

fn build_feedback(
    idx: usize,
    step: &ShellLoopStepInput,
    exit_code: Option<i32>,
    stdout: Option<&str>,
    stderr: Option<&str>,
    category: &FailureCategory,
    patch_hint: Option<&str>,
) -> String {
    format!(
        "iter={} step={} exit={:?} category={:?} patch_hint={} stdout={} stderr={}",
        idx + 1,
        step.step_id,
        exit_code,
        category,
        patch_hint.unwrap_or("none"),
        stdout.unwrap_or(""),
        stderr.unwrap_or("")
    )
}

fn build_failure_input(exit_code: Option<i32>, stdout: Option<&str>, stderr: Option<&str>) -> String {
    if exit_code == Some(0) {
        return "step_passed".to_string();
    }
    format!(
        "step_failed exit_code={:?} stdout={} stderr={}",
        exit_code,
        stdout.unwrap_or(""),
        stderr.unwrap_or("")
    )
}

fn build_patch_hint(
    category: &FailureCategory,
    step: &ShellLoopStepInput,
    stderr: Option<&str>,
) -> Option<String> {
    if matches!(category, FailureCategory::Unknown) {
        return None;
    }
    let reason = stderr.unwrap_or("").to_ascii_lowercase();
    let hint = match category {
        FailureCategory::Compile => {
            format!("recompile: inspect compile errors and patch sources for step {}", step.step_id)
        }
        FailureCategory::Test => {
            format!("retest: patch failing assertions and rerun tests for step {}", step.step_id)
        }
        FailureCategory::Tool => {
            format!("retry_tool_chain: validate command args and tool chain for step {}", step.step_id)
        }
        FailureCategory::Budget => {
            format!("compact_and_replan: shrink payload before rerun step {}", step.step_id)
        }
        FailureCategory::Permission => {
            format!("escalate_permission: approval required for step {}", step.step_id)
        }
        FailureCategory::Policy => {
            format!("escalate_policy: policy denied execution for step {}", step.step_id)
        }
        FailureCategory::Timeout => {
            format!("retry_after_timeout: tune timeout/retry for step {}", step.step_id)
        }
        FailureCategory::Unknown => return None,
    };
    if !reason.is_empty() {
        Some(format!("{hint}; reason={}", trim_tail(&reason)))
    } else {
        Some(hint)
    }
}

fn build_phase_trace(
    status: &ExecutionStepStatus,
    category: &FailureCategory,
    exit_code: Option<i32>,
    stdout_tail: Option<&str>,
    stderr_tail: Option<&str>,
    patch_hint: Option<&str>,
) -> Vec<ShellLoopPhaseRecord> {
    let run_ok = matches!(status, ExecutionStepStatus::Passed | ExecutionStepStatus::Failed);
    let capture_ok = stdout_tail.is_some() || stderr_tail.is_some();
    let verify_ok = matches!(status, ExecutionStepStatus::Passed);
    vec![
        ShellLoopPhaseRecord {
            phase: ShellLoopPhase::Run,
            status: if run_ok {
                ShellLoopPhaseStatus::Passed
            } else {
                ShellLoopPhaseStatus::Failed
            },
            detail: Some(format!("exit_code={exit_code:?}")),
        },
        ShellLoopPhaseRecord {
            phase: ShellLoopPhase::Capture,
            status: if capture_ok {
                ShellLoopPhaseStatus::Passed
            } else {
                ShellLoopPhaseStatus::Failed
            },
            detail: Some(format!(
                "stdout={} stderr={}",
                stdout_tail.unwrap_or(""),
                stderr_tail.unwrap_or("")
            )),
        },
        ShellLoopPhaseRecord {
            phase: ShellLoopPhase::Classify,
            status: ShellLoopPhaseStatus::Passed,
            detail: Some(format!("{category:?}").to_ascii_lowercase()),
        },
        ShellLoopPhaseRecord {
            phase: ShellLoopPhase::Patch,
            status: if patch_hint.is_some() {
                ShellLoopPhaseStatus::Passed
            } else if verify_ok {
                ShellLoopPhaseStatus::Skipped
            } else {
                ShellLoopPhaseStatus::Failed
            },
            detail: patch_hint.map(|value| value.to_string()),
        },
        ShellLoopPhaseRecord {
            phase: ShellLoopPhase::Verify,
            status: if verify_ok {
                ShellLoopPhaseStatus::Passed
            } else {
                ShellLoopPhaseStatus::Failed
            },
            detail: Some(format!("step_status={:?}", status).to_ascii_lowercase()),
        },
    ]
}

fn trim_tail(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.chars().count() > 600 {
        trimmed.chars().rev().take(600).collect::<Vec<_>>().into_iter().rev().collect()
    } else {
        trimmed.to_string()
    }
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
    async fn shell_loop_executes_and_captures_feedback() {
        let request = ShellLoopRequest {
            api_version: "shell_loop/v1".to_string(),
            max_iterations: Some(2),
            stop_on_success: false,
            steps: vec![
                ShellLoopStepInput {
                    step_id: "s1".to_string(),
                    command: "Write-Output 'loop-one'".to_string(),
                    continue_on_error: false,
                    timeout_ms: Some(5_000),
                },
                ShellLoopStepInput {
                    step_id: "s2".to_string(),
                    command: "Write-Output 'loop-two'".to_string(),
                    continue_on_error: false,
                    timeout_ms: Some(5_000),
                },
            ],
        };
        let engine = ShellExecutionLoopEngine::new(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let report = engine
            .run(&db(), "session-shell", "trace-shell", &request)
            .await
            .expect("shell loop");
        assert!(report.success);
        assert_eq!(report.iterations.len(), 2);
        assert_eq!(report.context_feedback.len(), 2);
        assert_eq!(report.state_machine_version, "shell-loop-state-machine/v1");
    }

    #[tokio::test]
    async fn shell_loop_records_fixed_phase_order_per_iteration() {
        let request = ShellLoopRequest {
            api_version: "shell_loop/v1".to_string(),
            max_iterations: Some(1),
            stop_on_success: false,
            steps: vec![ShellLoopStepInput {
                step_id: "s-order".to_string(),
                command: "Write-Output 'phase-order'".to_string(),
                continue_on_error: false,
                timeout_ms: Some(5_000),
            }],
        };
        let engine = ShellExecutionLoopEngine::new(
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
        let report = engine
            .run(&db(), "session-shell-order", "trace-shell-order", &request)
            .await
            .expect("shell loop");
        assert_eq!(
            report.fixed_phase_order,
            vec![
                ShellLoopPhase::Run,
                ShellLoopPhase::Capture,
                ShellLoopPhase::Classify,
                ShellLoopPhase::Patch,
                ShellLoopPhase::Verify,
            ]
        );
        let first = report.iterations.first().expect("first iteration");
        let phases = first
            .phase_trace
            .iter()
            .map(|item| item.phase.clone())
            .collect::<Vec<_>>();
        assert_eq!(phases, report.fixed_phase_order);
        assert_eq!(first.phase_trace.len(), 5);
    }

    #[test]
    fn high_risk_command_is_blocked() {
        assert!(is_high_risk_command("Remove-Item -Recurse -Force C:\\danger"));
    }
}
