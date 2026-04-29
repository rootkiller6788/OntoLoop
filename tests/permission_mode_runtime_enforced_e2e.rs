use autoloop::{
    config::AppConfig,
    runtime::{GuardDecision, RuntimeKernel},
    tools::{CapabilityRisk, ForgedMcpToolManifest},
};

#[test]
fn permission_mode_runtime_enforced_e2e() {
    let mut cfg = AppConfig::default();
    cfg.runtime.permission_mode = "prompt".into();

    let runtime = RuntimeKernel::from_config(&cfg.runtime);
    let mut manifest = ForgedMcpToolManifest::default();
    manifest.risk = CapabilityRisk::High;

    let report =
        runtime.guard_tool_execution("actor:pq", &manifest.registered_tool_name, Some(&manifest));
    assert_eq!(report.decision, GuardDecision::Blocked);
    assert!(report.reason.contains("permission mode blocked"));
}



