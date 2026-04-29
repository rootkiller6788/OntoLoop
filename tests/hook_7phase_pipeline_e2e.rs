use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    runtime::{
        GuardDecision,
        hook_runtime::{HookAction, HookPhase, HookRule},
    },
};

fn default_constraints() -> ConstraintSet {
    ConstraintSet {
        max_cpu_percent: 60,
        max_memory_mb: 256,
        timeout_ms: 20_000,
        max_retries: 1,
        max_tokens: 512,
        io_allow_paths: vec![".".into()],
        io_deny_paths: vec![],
        sandbox_profile: "default".into(),
        requires_human_approval: false,
    }
}

fn envelope(
    session: &str,
    identity: ExecutionIdentity,
    payload: serde_json::Value,
    capability_id: &str,
    run_id: &str,
) -> TaskEnvelope {
    TaskEnvelope {
        session_id: SessionId::from(session),
        trace_id: TraceId::from(format!("trace-{session}-{capability_id}-{run_id}")),
        task_id: TaskId::from(format!("task-{session}-{capability_id}-{run_id}")),
        capability_id: CapabilityId::from(capability_id),
        identity,
        payload,
        constraints: default_constraints(),
        trust_plan: None,
    }
}

#[tokio::test]
async fn hook_7phase_pipeline_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "hook7";

    let issued = app
        .ensure_session_identity(
            session,
            "tenant:hook7",
            "principal:hook7",
            "policy:hook7",
            3_600_000,
        )
        .await
        .expect("identity");

    let identity = ExecutionIdentity {
        tenant_id: issued.tenant_id.clone(),
        principal_id: issued.principal_id.clone(),
        policy_id: issued.policy_id.clone(),
        lease_token: issued.lease_token.clone(),
    };

    app.runtime
        .set_execution_hook_rules(vec![
            HookRule {
                id: "before-allow".into(),
                phase: HookPhase::Before,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("provider:missing".into()),
                value: None,
                reason: "before allow".into(),
            },
            HookRule {
                id: "step-allow".into(),
                phase: HookPhase::Step,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("provider:missing".into()),
                value: None,
                reason: "step allow".into(),
            },
            HookRule {
                id: "stream-mutate".into(),
                phase: HookPhase::Stream,
                action: HookAction::Mutate,
                channel: None,
                tool_contains: Some("provider:missing".into()),
                value: Some("::stream".into()),
                reason: "stream mutate".into(),
            },
            HookRule {
                id: "return-mutate".into(),
                phase: HookPhase::Return,
                action: HookAction::Mutate,
                channel: None,
                tool_contains: Some("provider:missing".into()),
                value: Some("::return".into()),
                reason: "return mutate".into(),
            },
            HookRule {
                id: "throws-allow".into(),
                phase: HookPhase::Throws,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("provider:missing".into()),
                value: None,
                reason: "throws allow".into(),
            },
            HookRule {
                id: "timeout-allow".into(),
                phase: HookPhase::Timeout,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("provider:missing".into()),
                value: None,
                reason: "timeout allow".into(),
            },
            HookRule {
                id: "kill-allow".into(),
                phase: HookPhase::Kill,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("provider:missing".into()),
                value: None,
                reason: "kill allow".into(),
            },
        ])
        .await;

    let success = app
        .runtime
        .execute(
            &app.state_store(),
            &app.tools(),
            &app.providers(),
            session,
            &envelope(
                session,
                identity.clone(),
                serde_json::json!([{"role":"user","content":"hook7-success"}]),
                "provider:missing",
                "success",
            ),
            None,
            None,
        )
        .await
        .expect("success execution");
    assert_ne!(success.guard_report.decision, GuardDecision::Blocked);
    assert!(success.content.contains("::stream"));
    assert!(success.content.contains("::return"));

    app.runtime
        .set_execution_hook_rules(vec![
            HookRule {
                id: "before-allow".into(),
                phase: HookPhase::Before,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("mcp::local-mcp::invoke".into()),
                value: None,
                reason: "before allow".into(),
            },
            HookRule {
                id: "step-allow".into(),
                phase: HookPhase::Step,
                action: HookAction::Deny,
                channel: None,
                tool_contains: Some("mcp::local-mcp::invoke".into()),
                value: None,
                reason: "step deny for throws path".into(),
            },
            HookRule {
                id: "throws-rewrite".into(),
                phase: HookPhase::Throws,
                action: HookAction::Rewrite,
                channel: None,
                tool_contains: Some("mcp::local-mcp::invoke".into()),
                value: Some("timeout forced by throws rewrite".into()),
                reason: "throws rewrite".into(),
            },
            HookRule {
                id: "timeout-allow".into(),
                phase: HookPhase::Timeout,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("mcp::local-mcp::invoke".into()),
                value: None,
                reason: "timeout allow".into(),
            },
            HookRule {
                id: "kill-allow".into(),
                phase: HookPhase::Kill,
                action: HookAction::Allow,
                channel: None,
                tool_contains: Some("mcp::local-mcp::invoke".into()),
                value: None,
                reason: "kill allow".into(),
            },
        ])
        .await;

    let degraded = app
        .runtime
        .execute(
            &app.state_store(),
            &app.tools(),
            &app.providers(),
            session,
            &envelope(
                session,
                identity,
                serde_json::json!({"task":"hook7-throws"}),
                "mcp::local-mcp::invoke",
                "degraded",
            ),
            None,
            None,
        )
        .await
        .expect("degraded execution");
    assert!(
        !degraded.content.trim().is_empty(),
        "degraded execution should still produce observable output"
    );

    let logs = app.state_store()
        .list_knowledge_by_prefix(&format!("eventlog:{session}:"))
        .await
        .expect("event logs");
    let combined = logs
        .iter()
        .filter(|item| item.value.contains("\"kind\":\"hook_runtime\""))
        .map(|item| item.value.clone())
        .collect::<Vec<_>>()
        .join("\n");

    for stage in [
        "\"stage\":\"before\"",
        "\"stage\":\"step\"",
        "\"stage\":\"stream\"",
        "\"stage\":\"return\"",
        "\"stage\":\"throws\"",
        "\"stage\":\"timeout\"",
        "\"stage\":\"kill\"",
    ] {
        assert!(
            logs.iter()
                .any(|item| item.value.contains("\"kind\":\"hook_runtime\"") && item.value.contains(stage)),
            "missing hook evidence stage: {stage}\ncombined_hook_logs:\n{combined}"
        );
    }
}



