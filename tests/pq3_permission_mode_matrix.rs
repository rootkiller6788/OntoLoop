use autoloop::{
    config::AppConfig,
    runtime::{GuardDecision, RuntimeKernel},
    tools::{CapabilityRisk, ForgedMcpToolManifest},
};

fn runtime_with_mode(mode: &str) -> RuntimeKernel {
    let mut config = AppConfig::default();
    config.runtime.permission_mode = mode.to_string();
    RuntimeKernel::from_config(&config.runtime)
}

fn high_risk_manifest() -> ForgedMcpToolManifest {
    let mut manifest = ForgedMcpToolManifest::default();
    manifest.risk = CapabilityRisk::High;
    manifest
}

#[test]
fn high_risk_capability_is_blocked_in_prompt_mode() {
    let runtime = runtime_with_mode("prompt");
    let manifest = high_risk_manifest();

    let report = runtime.guard_tool_execution(
        "actor:test",
        &manifest.registered_tool_name,
        Some(&manifest),
    );

    assert_eq!(report.decision, GuardDecision::Blocked);
    assert!(report.reason.contains("permission mode blocked"));
}

#[test]
fn high_risk_capability_is_blocked_in_auto_mode() {
    let runtime = runtime_with_mode("auto");
    let manifest = high_risk_manifest();

    let report = runtime.guard_tool_execution(
        "actor:test",
        &manifest.registered_tool_name,
        Some(&manifest),
    );

    assert_eq!(report.decision, GuardDecision::Blocked);
    assert!(report.reason.contains("permission mode blocked"));
}

#[test]
fn high_risk_capability_is_blocked_in_restricted_bypass_mode() {
    let runtime = runtime_with_mode("bypass");
    let manifest = high_risk_manifest();

    let report = runtime.guard_tool_execution(
        "actor:test",
        &manifest.registered_tool_name,
        Some(&manifest),
    );

    assert_eq!(report.decision, GuardDecision::Blocked);
    assert!(report.reason.contains("permission mode blocked"));
}

#[test]
fn high_risk_capability_in_strict_mode_requires_approval_not_block() {
    let runtime = runtime_with_mode("strict");
    let manifest = high_risk_manifest();

    let report = runtime.guard_tool_execution(
        "actor:test",
        &manifest.registered_tool_name,
        Some(&manifest),
    );

    assert_eq!(report.decision, GuardDecision::RequiresApproval);
}



