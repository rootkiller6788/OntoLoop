use autoloop::{
    AutoLoopApp,
    config::AppConfig,
    contracts::{
        ids::{CapabilityId, SessionId, TaskId, TraceId},
        types::{ConstraintSet, ExecutionIdentity, TaskEnvelope},
    },
};

fn constraints() -> ConstraintSet {
    ConstraintSet {
        max_cpu_percent: 60,
        max_memory_mb: 256,
        timeout_ms: 20_000,
        max_retries: 1,
        max_tokens: 1024,
        io_allow_paths: vec![".".into()],
        io_deny_paths: vec![],
        sandbox_profile: "standard".into(),
        requires_human_approval: false,
    }
}

#[tokio::test]
async fn layered_tool_stack_pipeline_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq-layered-stack";
    let issued = app
        .ensure_session_identity(
            session_id,
            "tenant:pq",
            "principal:pq",
            "policy:pq",
            3_600_000,
        )
        .await
        .expect("identity");

    let envelope = TaskEnvelope {
        session_id: SessionId::from(session_id),
        trace_id: TraceId::from("trace:pq-layered-stack"),
        task_id: TaskId::from("task:pq-layered-stack"),
        capability_id: CapabilityId::from("read_file"),
        identity: ExecutionIdentity {
            tenant_id: issued.tenant_id,
            principal_id: issued.principal_id,
            policy_id: issued.policy_id,
            lease_token: issued.lease_token,
        },
        payload: serde_json::json!({"path":"README.md"}),
        constraints: constraints(),
        trust_plan: None,
    };

    let _ = app
        .runtime
        .execute(
            &app.state_store(),
            &app.tools(),
            &app.providers(),
            session_id,
            &envelope,
            None,
            None,
        )
        .await
        .expect("execute");

    let records = app.state_store()
        .list_knowledge_by_prefix(&format!(
            "execution-stack:{session_id}:task:pq-layered-stack:"
        ))
        .await
        .expect("stack records");
    assert!(
        !records.is_empty(),
        "layered execution stack record missing"
    );
}




