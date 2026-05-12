use autoloop::{AutoLoopApp, config::AppConfig};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn pevo_r10_promote_canary_fail_rollback_e2e() {
    let mut config = AppConfig::default();
    config.storage.backend = autoloop::config::StorageBackend::Postgres;
    config.storage.postgres.enabled = true;
    config.storage.postgres.uri = std::env::var("ONTOLOOP_TEST_POSTGRES_URI")
        .unwrap_or_else(|_| "postgres://postgres:123456@localhost:5432/postgres".to_string());
    config.storage.shadow_read_preference = "postgres".to_string();
    let app = AutoLoopApp::new(config);
    let session_id = "pevo-r10-canary-rollback";

    app.state_store()
        .grant_permissions(
            session_id,
            vec![
                PermissionAction::Read,
                PermissionAction::Write,
                PermissionAction::Dispatch,
            ],
        )
        .await
        .expect("grant permissions");
    app.ensure_session_identity(
        session_id,
        "tenant:pevo",
        "principal:pevo-r10",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");

    let mut canary_seen = false;
    for _ in 0..8 {
        let _ = app
            .process_direct(
                session_id,
                "run evolution shadow probe and produce runtime promotion decision",
            )
            .await
            .expect("process direct");

        let next_gen = app.state_store()
            .get_knowledge(&format!("evolution:next-gen:{session_id}:latest"))
            .await
            .expect("db next-gen")
            .expect("next-gen record should exist");
        let next_gen_json: serde_json::Value =
            serde_json::from_str(&next_gen.value).expect("next-gen json");

        if next_gen_json["execution_mode"].as_str() == Some("canary") {
            canary_seen = true;
            assert_eq!(next_gen_json["final_status"].as_str(), Some("rolled_back"));
            assert_eq!(next_gen_json["auto_rollback"].as_bool(), Some(true));
            assert_eq!(next_gen_json["runtime_gate_pass"].as_bool(), Some(false));
            let rollback_ref = next_gen_json["rollback_ref"]
                .as_str()
                .expect("rollback ref");
            let rollback = app.state_store()
                .get_knowledge(rollback_ref)
                .await
                .expect("db rollback")
                .expect("rollback should exist");
            let rollback_json: serde_json::Value =
                serde_json::from_str(&rollback.value).expect("rollback json");
            assert_eq!(
                rollback_json["reason"].as_str(),
                Some("canary_failed_auto_rollback")
            );
            break;
        }
    }

    assert!(
        canary_seen,
        "expected at least one canary execution in retry window"
    );
}





