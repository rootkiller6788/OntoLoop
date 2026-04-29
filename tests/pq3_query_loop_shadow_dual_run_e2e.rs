use autoloop::{AutoLoopApp, config::AppConfig};

#[tokio::test]
async fn pq3_query_loop_shadow_dual_run_emits_shadow_artifact() {
    unsafe {
        std::env::set_var("AUTOLOOP_QUERY_LOOP_SHADOW_MODE", "enabled");
        std::env::set_var("AUTOLOOP_QUERY_LOOP_SHADOW_MAX_INPUT_TOKENS", "2048");
        std::env::set_var("AUTOLOOP_QUERY_LOOP_SHADOW_MAX_OUTPUT_TOKENS", "512");
    }

    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pq3-shadow-dual-run";

    app.ensure_session_identity(
        session_id,
        "tenant:pq3",
        "principal:pq3",
        "policy:pq3",
        3_600_000,
    )
    .await
    .expect("bind identity");

    for idx in 0..3 {
        let response = app
            .process_direct(
                session_id,
                &format!(
                    "shadow warmup #{idx}: build a governed plan with tools, telemetry and verifier outputs"
                ),
            )
            .await
            .expect("process direct");
        assert!(!response.trim().is_empty(), "response should not be empty");
    }

    let shadow_records = app.state_store()
        .list_knowledge_by_prefix(&format!("query-loop:shadow:{session_id}:"))
        .await
        .expect("list shadow artifacts");

    assert!(
        !shadow_records.is_empty(),
        "expected query-loop shadow artifacts to be written"
    );

    let has_dual_run = shadow_records.iter().any(|record| {
        record.value.contains("dual_run_shadow")
            && record.value.contains("\"primary\"")
            && record.value.contains("\"shadow\"")
    });
    assert!(has_dual_run, "expected at least one dual-run shadow payload");

    unsafe {
        std::env::remove_var("AUTOLOOP_QUERY_LOOP_SHADOW_MODE");
        std::env::remove_var("AUTOLOOP_QUERY_LOOP_SHADOW_MAX_INPUT_TOKENS");
        std::env::remove_var("AUTOLOOP_QUERY_LOOP_SHADOW_MAX_OUTPUT_TOKENS");
    }
}




