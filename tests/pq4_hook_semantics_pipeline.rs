use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
    runtime::{
        GuardDecision,
        hook_runtime::{HookAction, HookRule, HookStage},
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
) -> TaskEnvelope {
    TaskEnvelope {
        session_id: SessionId::from(session),
        trace_id: TraceId::from(format!("trace-{session}")),
        task_id: TaskId::from(format!("task-{session}")),
        capability_id: CapabilityId::from("mcp::local-mcp::invoke"),
        identity,
        payload,
        constraints: default_constraints(),
        trust_plan: None,
    }
}

#[tokio::test]
async fn deny_hook_blocks_execution_and_persists_evidence() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "pq4-deny";

    let issued = app
        .ensure_session_identity(
            session,
            "tenant:pq4",
            "principal:pq4",
            "policy:pq4",
            3_600_000,
        )
        .await
        .expect("identity");

    app.runtime
        .set_execution_hook_rules(vec![HookRule {
            id: "deny-pre".into(),
            phase: HookStage::PreToolUse.to_phase(),
            action: HookAction::Deny,
            channel: None,
            tool_contains: Some("mcp::local-mcp::invoke".into()),
            value: None,
            reason: "policy denied pre tool use".into(),
        }])
        .await;

    let result = app
        .runtime
        .execute(
            &app.state_store(),
            &app.tools(),
            &app.providers(),
            session,
            &envelope(
                session,
                ExecutionIdentity {
                    tenant_id: issued.tenant_id,
                    principal_id: issued.principal_id,
                    policy_id: issued.policy_id,
                    lease_token: issued.lease_token,
                },
                serde_json::json!({"task":"deny-check"}),
            ),
            None,
            None,
        )
        .await
        .expect("runtime result");

    assert_eq!(result.guard_report.decision, GuardDecision::Blocked);
    assert!(result.guard_report.reason.contains("PreToolUse"));

    let logs = app.state_store()
        .list_knowledge_by_prefix(&format!("eventlog:{session}:"))
        .await
        .expect("event logs");
    let has_hook_evidence = logs.iter().any(|item| {
        item.value.contains("\"kind\":\"hook_runtime\"")
            && (item.value.contains("pre_tool_use") || item.value.contains("\"phase\":\"before\""))
            && item.value.contains("deny-pre")
    });
    assert!(has_hook_evidence, "hook deny evidence should be persisted");
}

#[tokio::test]
async fn rewrite_and_mutate_hooks_change_output_and_persist_evidence() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "pq4-rewrite";

    let issued = app
        .ensure_session_identity(
            session,
            "tenant:pq4",
            "principal:pq4",
            "policy:pq4",
            3_600_000,
        )
        .await
        .expect("identity");

    app.runtime
        .set_execution_hook_rules(vec![
            HookRule {
                id: "rewrite-pre".into(),
                phase: HookStage::PreToolUse.to_phase(),
                action: HookAction::Rewrite,
                channel: None,
                tool_contains: Some("mcp::local-mcp::invoke".into()),
                value: Some("{\"payload\":\"rewritten\"}".into()),
                reason: "rewrite arguments".into(),
            },
            HookRule {
                id: "mutate-result".into(),
                phase: HookStage::OnResult.to_phase(),
                action: HookAction::Mutate,
                channel: None,
                tool_contains: Some("mcp::local-mcp::invoke".into()),
                value: Some("::hooked".into()),
                reason: "tag result".into(),
            },
        ])
        .await;

    let result = app
        .runtime
        .execute(
            &app.state_store(),
            &app.tools(),
            &app.providers(),
            session,
            &envelope(
                session,
                ExecutionIdentity {
                    tenant_id: issued.tenant_id,
                    principal_id: issued.principal_id,
                    policy_id: issued.policy_id,
                    lease_token: issued.lease_token,
                },
                serde_json::json!({"task":"rewrite-check"}),
            ),
            None,
            None,
        )
        .await
        .expect("runtime result");

    assert_eq!(result.guard_report.decision, GuardDecision::Allow);
    assert!(result.content.contains("rewritten"));
    assert!(result.content.ends_with("::hooked"));

    let logs = app.state_store()
        .list_knowledge_by_prefix(&format!("eventlog:{session}:"))
        .await
        .expect("event logs");
    let has_pre = logs
        .iter()
        .any(|item| {
            (item.value.contains("pre_tool_use") || item.value.contains("\"phase\":\"before\""))
                && item.value.contains("rewrite-pre")
        });
    let has_result = logs
        .iter()
        .any(|item| {
            (item.value.contains("on_result") || item.value.contains("\"phase\":\"return\""))
                && item.value.contains("mutate-result")
        });
    assert!(
        has_pre && has_result,
        "rewrite/mutate hook evidence should be persisted"
    );
}




