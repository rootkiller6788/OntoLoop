use std::collections::BTreeMap;

pub const CODE_HARNESS_CONTRACT_VERSION: &str = "code-harness/v1";
pub const CODE_EXECUTION_LOOP_CONTRACT_VERSION: &str = "code-execution-loop/v2";
pub const REPO_CONTEXT_BUNDLE_CONTRACT_VERSION: &str = "repo-context/v2";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodeExecutionFailureClass {
    Compile,
    Lint,
    Test,
    Tool,
    Permission,
    Budget,
    Timeout,
    Runtime,
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodeExecutionTarget {
    pub objective: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default, alias = "targets")]
    pub target_paths: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub requires_artifact: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodeExecutionSuccessDefinition {
    #[serde(default)]
    pub require_build_pass: bool,
    #[serde(default)]
    pub require_lint_pass: bool,
    #[serde(default)]
    pub require_test_pass: bool,
    #[serde(default)]
    pub require_artifact_proof: bool,
    #[serde(default)]
    pub require_write_proof: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodeExecutionRetryPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_max_replans")]
    pub max_replans: u32,
    #[serde(default = "default_max_same_failure_retries")]
    pub max_same_failure_retries: u32,
    #[serde(default = "default_max_budget_retries")]
    pub max_budget_retries: u32,
    #[serde(default, alias = "retryable_failures")]
    pub retryable_classes: Vec<CodeExecutionFailureClass>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodeExecutionEvidenceFields {
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
    #[serde(default)]
    pub decision_hash: Option<String>,
    #[serde(default)]
    pub policy_version: Option<String>,
    #[serde(default)]
    pub relation_ref: Option<String>,
    #[serde(default)]
    pub write_proof_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodeExecutionLoopContract {
    #[serde(default = "default_loop_api_version")]
    pub api_version: String,
    pub session_id: String,
    pub trace_id: String,
    #[serde(alias = "goal")]
    pub target: CodeExecutionTarget,
    #[serde(default, alias = "success_criteria")]
    pub success_definition: CodeExecutionSuccessDefinition,
    #[serde(default, alias = "retry_limits")]
    pub retry_policy: CodeExecutionRetryPolicy,
    #[serde(default, alias = "failure_classes")]
    pub failure_classification: Vec<CodeExecutionFailureClass>,
    #[serde(default, alias = "evidence")]
    pub evidence_fields: CodeExecutionEvidenceFields,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

fn default_loop_api_version() -> String {
    CODE_EXECUTION_LOOP_CONTRACT_VERSION.to_string()
}

fn default_max_attempts() -> u32 {
    3
}

fn default_max_replans() -> u32 {
    2
}

fn default_max_same_failure_retries() -> u32 {
    2
}

fn default_max_budget_retries() -> u32 {
    1
}

impl Default for CodeExecutionSuccessDefinition {
    fn default() -> Self {
        Self {
            require_build_pass: true,
            require_lint_pass: true,
            require_test_pass: true,
            require_artifact_proof: false,
            require_write_proof: false,
        }
    }
}

impl Default for CodeExecutionRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            max_replans: default_max_replans(),
            max_same_failure_retries: default_max_same_failure_retries(),
            max_budget_retries: default_max_budget_retries(),
            retryable_classes: vec![
                CodeExecutionFailureClass::Compile,
                CodeExecutionFailureClass::Lint,
                CodeExecutionFailureClass::Test,
                CodeExecutionFailureClass::Tool,
                CodeExecutionFailureClass::Budget,
                CodeExecutionFailureClass::Timeout,
                CodeExecutionFailureClass::Runtime,
                CodeExecutionFailureClass::Unknown,
            ],
        }
    }
}

impl Default for CodeExecutionEvidenceFields {
    fn default() -> Self {
        Self {
            evidence_ref: None,
            replay_fp: None,
            decision_hash: None,
            policy_version: None,
            relation_ref: None,
            write_proof_ref: None,
        }
    }
}

pub fn code_execution_loop_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == CODE_EXECUTION_LOOP_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("code-execution-loop/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(2))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoNodeKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RepoTreeNode {
    pub path: String,
    pub kind: RepoNodeKind,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub language_hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct FileImportanceScore {
    pub path: String,
    pub score: f32,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DependencyEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RecentDiffEntry {
    pub path: String,
    pub change_kind: DiffChangeKind,
    #[serde(default)]
    pub old_path: Option<String>,
    #[serde(default)]
    pub added_lines: Option<u32>,
    #[serde(default)]
    pub removed_lines: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RepoContextBundle {
    #[serde(default = "default_api_version", alias = "contract_version")]
    pub api_version: String,
    pub session_id: String,
    pub trace_id: String,
    pub repo_root: String,
    #[serde(default)]
    pub repo_tree: Vec<RepoTreeNode>,
    #[serde(default, alias = "file_scores")]
    pub file_importance_ranking: Vec<FileImportanceScore>,
    #[serde(default)]
    pub dependency_graph: Vec<DependencyEdge>,
    #[serde(default, alias = "recent_diffs")]
    pub recent_diff: Vec<RecentDiffEntry>,
    pub generated_at_ms: u64,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

fn default_api_version() -> String {
    REPO_CONTEXT_BUNDLE_CONTRACT_VERSION.to_string()
}

pub fn repo_context_bundle_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == REPO_CONTEXT_BUNDLE_CONTRACT_VERSION
        || normalized == CODE_HARNESS_CONTRACT_VERSION
    {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("repo-context/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(2))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatchOpKind {
    CreateFile,
    UpdateFile,
    DeleteFile,
    MoveFile,
    Revert,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PatchOp {
    pub op_id: String,
    pub kind: PatchOpKind,
    pub path: String,
    #[serde(default)]
    pub from_path: Option<String>,
    #[serde(default)]
    pub patch: Option<String>,
    #[serde(default)]
    pub expected_hash: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStepStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
    Timeout,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ExecutionStep {
    pub step_id: String,
    pub session_id: String,
    pub trace_id: String,
    #[serde(default)]
    pub attempt: u32,
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub status: ExecutionStepStatus,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub stdout_tail: Option<String>,
    #[serde(default)]
    pub stderr_tail: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestVerdictStatus {
    Pass,
    Fail,
    Error,
    Skipped,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TestVerdict {
    pub verdict_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub runner: String,
    pub target: String,
    pub status: TestVerdictStatus,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub failed_cases: Vec<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IterationDecision {
    Continue,
    Retry,
    Replan,
    StopSolved,
    StopFailed,
    StopBudgetExceeded,
    StopAttemptLimit,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct IterationState {
    pub session_id: String,
    pub trace_id: String,
    pub objective: String,
    pub attempt: u32,
    pub max_attempts: u32,
    #[serde(default)]
    pub error_fingerprint: Option<String>,
    pub decision: IterationDecision,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub last_execution_step_id: Option<String>,
    #[serde(default)]
    pub last_test_verdict_id: Option<String>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitCheckpointAction {
    Branch,
    Commit,
    Tag,
    Rollback,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct GitCheckpoint {
    pub checkpoint_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub action: GitCheckpointAction,
    pub branch: String,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub parent_commit_sha: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub dirty: bool,
    pub created_at_ms: u64,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

pub fn code_harness_contract_compatible(version: &str) -> bool {
    let normalized = version.trim().to_ascii_lowercase();
    if normalized == CODE_HARNESS_CONTRACT_VERSION {
        return true;
    }

    let Some(stripped) = normalized.strip_prefix("code-harness/v") else {
        return false;
    };

    let major = stripped
        .split(['.', '-', '+'])
        .next()
        .unwrap_or_default()
        .parse::<u32>()
        .ok();

    matches!(major, Some(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_execution_loop_v2_roundtrip() {
        let contract = CodeExecutionLoopContract {
            api_version: CODE_EXECUTION_LOOP_CONTRACT_VERSION.to_string(),
            session_id: "session-a".into(),
            trace_id: "trace-a".into(),
            target: CodeExecutionTarget {
                objective: "implement billing page".into(),
                task_id: Some("task-1".into()),
                target_paths: vec!["D:/AutoLoop/output/billing.html".into()],
                working_directory: Some("D:/AutoLoop/autoloop-app".into()),
                requires_artifact: true,
            },
            success_definition: CodeExecutionSuccessDefinition {
                require_build_pass: true,
                require_lint_pass: true,
                require_test_pass: true,
                require_artifact_proof: true,
                require_write_proof: true,
            },
            retry_policy: CodeExecutionRetryPolicy {
                max_attempts: 4,
                max_replans: 2,
                max_same_failure_retries: 2,
                max_budget_retries: 1,
                retryable_classes: vec![
                    CodeExecutionFailureClass::Compile,
                    CodeExecutionFailureClass::Test,
                    CodeExecutionFailureClass::Budget,
                ],
            },
            failure_classification: vec![
                CodeExecutionFailureClass::Compile,
                CodeExecutionFailureClass::Test,
                CodeExecutionFailureClass::Permission,
            ],
            evidence_fields: CodeExecutionEvidenceFields {
                evidence_ref: Some("evidence:exec-loop:1".into()),
                replay_fp: Some("replay:fp:exec-loop:1".into()),
                decision_hash: Some("decision:hash:1".into()),
                policy_version: Some("policy/v3".into()),
                relation_ref: Some("relation:event:1".into()),
                write_proof_ref: Some("artifact:proof:1".into()),
            },
            metadata: BTreeMap::from([("mode".into(), "enforced".into())]),
        };

        let raw = serde_json::to_string(&contract).expect("serialize loop contract");
        let decoded: CodeExecutionLoopContract =
            serde_json::from_str(&raw).expect("deserialize loop contract");
        assert_eq!(decoded, contract);
    }

    #[test]
    fn code_execution_loop_v2_accepts_aliases_and_defaults() {
        let raw = serde_json::json!({
            "api_version": "code-execution-loop/v2",
            "session_id": "session-a",
            "trace_id": "trace-a",
            "goal": {
                "objective": "fix build errors",
                "targets": ["src/lib.rs"]
            },
            "success_criteria": {
                "require_build_pass": true
            },
            "retry_limits": {
                "max_attempts": 5,
                "retryable_failures": ["compile", "test"]
            },
            "failure_classes": ["compile", "tool"],
            "evidence": {
                "evidence_ref": "evidence:loop:2"
            }
        });

        let decoded: CodeExecutionLoopContract =
            serde_json::from_value(raw).expect("deserialize loop aliases");
        assert_eq!(decoded.target.target_paths, vec!["src/lib.rs".to_string()]);
        assert_eq!(decoded.retry_policy.max_attempts, 5);
        assert_eq!(decoded.retry_policy.max_replans, 2);
        assert_eq!(
            decoded.evidence_fields.evidence_ref,
            Some("evidence:loop:2".to_string())
        );
    }

    #[test]
    fn code_execution_loop_compat_accepts_v2_series() {
        assert!(code_execution_loop_contract_compatible(
            "code-execution-loop/v2"
        ));
        assert!(code_execution_loop_contract_compatible(
            "code-execution-loop/v2.1"
        ));
        assert!(code_execution_loop_contract_compatible(
            "CODE-EXECUTION-LOOP/V2-beta"
        ));
    }

    #[test]
    fn code_execution_loop_compat_rejects_other_majors() {
        assert!(!code_execution_loop_contract_compatible(
            "code-execution-loop/v1"
        ));
        assert!(!code_execution_loop_contract_compatible(
            "code-execution-loop/v3"
        ));
        assert!(!code_execution_loop_contract_compatible("v2"));
    }

    #[test]
    fn repo_context_bundle_roundtrip() {
        let bundle = RepoContextBundle {
            api_version: REPO_CONTEXT_BUNDLE_CONTRACT_VERSION.to_string(),
            session_id: "session-a".into(),
            trace_id: "trace-a".into(),
            repo_root: "D:/AutoLoop/autoloop-app".into(),
            repo_tree: vec![RepoTreeNode {
                path: "src/lib.rs".into(),
                kind: RepoNodeKind::File,
                size_bytes: Some(1024),
                language_hint: Some("rust".into()),
            }],
            file_importance_ranking: vec![FileImportanceScore {
                path: "src/lib.rs".into(),
                score: 0.92,
                reasons: vec!["entrypoint".into()],
            }],
            dependency_graph: vec![DependencyEdge {
                from: "src/lib.rs".into(),
                to: "src/runtime/mod.rs".into(),
                kind: Some("module_import".into()),
            }],
            recent_diff: vec![RecentDiffEntry {
                path: "src/lib.rs".into(),
                change_kind: DiffChangeKind::Modified,
                old_path: None,
                added_lines: Some(12),
                removed_lines: Some(2),
            }],
            generated_at_ms: 1_717_000_000_000,
            evidence_ref: Some("evidence:repo-context:1".into()),
            replay_fp: Some("replay:fp:repo-context".into()),
            metadata: BTreeMap::from([("source".to_string(), "repo-compiler".to_string())]),
        };

        let raw = serde_json::to_string(&bundle).expect("serialize repo context bundle");
        let decoded: RepoContextBundle =
            serde_json::from_str(&raw).expect("deserialize repo context bundle");
        assert_eq!(decoded, bundle);
    }

    #[test]
    fn repo_context_accepts_legacy_aliases() {
        let raw = serde_json::json!({
            "api_version": "code-harness/v1",
            "session_id": "session-a",
            "trace_id": "trace-a",
            "repo_root": "D:/AutoLoop/autoloop-app",
            "file_scores": [{"path":"src/main.rs","score":0.8}],
            "recent_diffs": [{"path":"src/main.rs","change_kind":"modified"}],
            "generated_at_ms": 1_717_000_000_000u64
        });
        let decoded: RepoContextBundle =
            serde_json::from_value(raw).expect("deserialize legacy alias payload");
        assert_eq!(decoded.file_importance_ranking.len(), 1);
        assert_eq!(decoded.recent_diff.len(), 1);
    }

    #[test]
    fn repo_context_contract_compat_accepts_v2_and_legacy_v1() {
        assert!(repo_context_bundle_contract_compatible("repo-context/v2"));
        assert!(repo_context_bundle_contract_compatible("repo-context/v2.1"));
        assert!(repo_context_bundle_contract_compatible("REPO-CONTEXT/V2-beta"));
        assert!(repo_context_bundle_contract_compatible("code-harness/v1"));
    }

    #[test]
    fn repo_context_contract_compat_rejects_other_majors() {
        assert!(!repo_context_bundle_contract_compatible("repo-context/v1"));
        assert!(!repo_context_bundle_contract_compatible("repo-context/v3"));
        assert!(!repo_context_bundle_contract_compatible("v2"));
    }

    #[test]
    fn code_harness_compat_accepts_v1_series() {
        assert!(code_harness_contract_compatible("code-harness/v1"));
        assert!(code_harness_contract_compatible("code-harness/v1.1"));
        assert!(code_harness_contract_compatible("CODE-HARNESS/V1-beta"));
    }

    #[test]
    fn code_harness_compat_rejects_other_majors() {
        assert!(!code_harness_contract_compatible("code-harness/v2"));
        assert!(!code_harness_contract_compatible("v1"));
    }
}
