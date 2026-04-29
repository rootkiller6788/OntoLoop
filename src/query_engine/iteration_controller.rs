#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
    Timeout,
    Budget,
    Permission,
    Policy,
    #[serde(alias = "validation")]
    Test,
    #[serde(alias = "tooling")]
    Tool,
    Compile,
    Unknown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepairStrategy {
    Recompile,
    Retest,
    RetryToolChain,
    CompactAndReplan,
    EscalatePermission,
    EscalatePolicy,
    RetryAfterTimeout,
    ReplanUnknown,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminationReason {
    Success,
    AttemptLimitExceeded,
    BudgetLimitExceeded,
    TimeLimitExceeded,
    NonRetryableFailure,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IterationControllerConfig {
    pub max_attempts: u32,
    pub budget_tokens: Option<u32>,
    pub max_runtime_ms: u64,
    #[serde(default)]
    pub retry_on: Vec<String>,
}

impl Default for IterationControllerConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            budget_tokens: None,
            max_runtime_ms: 120_000,
            retry_on: vec![
                "compile".to_string(),
                "test".to_string(),
                "tool".to_string(),
                "budget".to_string(),
                "timeout".to_string(),
                "structured_patch".to_string(),
                "shell_loop".to_string(),
                "test_verifier".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IterationAttemptRecord {
    pub attempt: u32,
    pub stage: String,
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub failure_category: Option<FailureCategory>,
    #[serde(default)]
    pub repair_strategy: Option<RepairStrategy>,
    #[serde(default)]
    pub retry_allowed: Option<bool>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IterationControllerReport {
    pub session_id: String,
    pub trace_id: String,
    pub success: bool,
    pub attempts_used: u32,
    pub max_attempts: u32,
    #[serde(default)]
    pub budget_tokens: Option<u32>,
    pub estimated_tokens: u32,
    pub elapsed_ms: u64,
    pub termination_reason: TerminationReason,
    pub attempts: Vec<IterationAttemptRecord>,
    #[serde(default)]
    pub routing_version: Option<String>,
    #[serde(default)]
    pub strategy_summary: serde_json::Value,
    #[serde(default)]
    pub stage_refs: serde_json::Value,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
}

pub fn classify_failure(error: &str) -> FailureCategory {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("timeout") {
        FailureCategory::Timeout
    } else if lowered.contains("budget") || lowered.contains("token") {
        FailureCategory::Budget
    } else if lowered.contains("permission") || lowered.contains("denied") {
        FailureCategory::Permission
    } else if lowered.contains("policy") {
        FailureCategory::Policy
    } else if lowered.contains("stage=test_verifier")
        || lowered.contains("test verifier")
        || lowered.contains("hard failed")
        || lowered.contains("assertion failed")
        || lowered.contains("test failed")
    {
        FailureCategory::Test
    } else if lowered.contains("compile")
        || lowered.contains("compilation")
        || lowered.contains("cargo check")
        || lowered.contains("build failed")
        || lowered.contains("rustc")
    {
        FailureCategory::Compile
    } else if lowered.contains("patch")
        || lowered.contains("shell")
        || lowered.contains("tool")
        || lowered.contains("git checkpoint")
        || lowered.contains("stage=git_checkpoint")
        || lowered.contains("stage=shell_loop")
        || lowered.contains("stage=structured_patch")
    {
        FailureCategory::Tool
    } else {
        FailureCategory::Unknown
    }
}

pub fn stage_from_error(error: &str) -> String {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("git checkpoint") || lowered.contains("stage=git_checkpoint") {
        "git_checkpoint".to_string()
    } else if lowered.contains("structured patch") || lowered.contains("patch") {
        "structured_patch".to_string()
    } else if lowered.contains("shell execution loop") || lowered.contains("shell loop") {
        "shell_loop".to_string()
    } else if lowered.contains("test verifier") {
        "test_verifier".to_string()
    } else {
        "unknown".to_string()
    }
}

pub fn should_retry(stage: &str, config: &IterationControllerConfig) -> bool {
    config
        .retry_on
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(stage))
}

pub fn select_repair_strategy(stage: &str, category: &FailureCategory) -> RepairStrategy {
    match category {
        FailureCategory::Compile => RepairStrategy::Recompile,
        FailureCategory::Test => RepairStrategy::Retest,
        FailureCategory::Tool => RepairStrategy::RetryToolChain,
        FailureCategory::Budget => RepairStrategy::CompactAndReplan,
        FailureCategory::Permission => RepairStrategy::EscalatePermission,
        FailureCategory::Policy => RepairStrategy::EscalatePolicy,
        FailureCategory::Timeout => RepairStrategy::RetryAfterTimeout,
        FailureCategory::Unknown => {
            if stage.eq_ignore_ascii_case("test_verifier") {
                RepairStrategy::Retest
            } else if stage.eq_ignore_ascii_case("structured_patch")
                || stage.eq_ignore_ascii_case("shell_loop")
                || stage.eq_ignore_ascii_case("git_checkpoint")
            {
                RepairStrategy::RetryToolChain
            } else {
                RepairStrategy::ReplanUnknown
            }
        }
    }
}

pub fn should_retry_for_failure(
    stage: &str,
    category: &FailureCategory,
    strategy: &RepairStrategy,
    config: &IterationControllerConfig,
) -> bool {
    let retry_stage_allowed = should_retry(stage, config);
    let retry_category_allowed = config.retry_on.iter().any(|candidate| {
        candidate.eq_ignore_ascii_case(match category {
            FailureCategory::Timeout => "timeout",
            FailureCategory::Budget => "budget",
            FailureCategory::Permission => "permission",
            FailureCategory::Policy => "policy",
            FailureCategory::Test => "test",
            FailureCategory::Tool => "tool",
            FailureCategory::Compile => "compile",
            FailureCategory::Unknown => "unknown",
        })
    });
    let retry_strategy_allowed = config.retry_on.iter().any(|candidate| {
        candidate.eq_ignore_ascii_case(match strategy {
            RepairStrategy::Recompile => "recompile",
            RepairStrategy::Retest => "retest",
            RepairStrategy::RetryToolChain => "retry_tool_chain",
            RepairStrategy::CompactAndReplan => "compact_and_replan",
            RepairStrategy::EscalatePermission => "escalate_permission",
            RepairStrategy::EscalatePolicy => "escalate_policy",
            RepairStrategy::RetryAfterTimeout => "retry_after_timeout",
            RepairStrategy::ReplanUnknown => "replan_unknown",
        })
    });

    if matches!(
        strategy,
        RepairStrategy::EscalatePermission | RepairStrategy::EscalatePolicy
    ) {
        return false;
    }

    retry_stage_allowed || retry_category_allowed || retry_strategy_allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_failure_maps_compile_test_tool_budget_permission() {
        assert_eq!(
            classify_failure("stage=structured_patch; cargo check compile failed"),
            FailureCategory::Compile
        );
        assert_eq!(
            classify_failure("stage=test_verifier; hard failed"),
            FailureCategory::Test
        );
        assert_eq!(
            classify_failure("stage=shell_loop; tool execution failed"),
            FailureCategory::Tool
        );
        assert_eq!(classify_failure("budget limit exceeded"), FailureCategory::Budget);
        assert_eq!(
            classify_failure("permission denied for write"),
            FailureCategory::Permission
        );
    }

    #[test]
    fn strategy_routes_follow_failure_category() {
        assert_eq!(
            select_repair_strategy("structured_patch", &FailureCategory::Compile),
            RepairStrategy::Recompile
        );
        assert_eq!(
            select_repair_strategy("test_verifier", &FailureCategory::Test),
            RepairStrategy::Retest
        );
        assert_eq!(
            select_repair_strategy("shell_loop", &FailureCategory::Tool),
            RepairStrategy::RetryToolChain
        );
        assert_eq!(
            select_repair_strategy("controller", &FailureCategory::Budget),
            RepairStrategy::CompactAndReplan
        );
    }

    #[test]
    fn permission_strategy_is_not_retryable() {
        let cfg = IterationControllerConfig::default();
        let strategy = select_repair_strategy("unknown", &FailureCategory::Permission);
        assert!(!should_retry_for_failure(
            "unknown",
            &FailureCategory::Permission,
            &strategy,
            &cfg
        ));
    }
}
