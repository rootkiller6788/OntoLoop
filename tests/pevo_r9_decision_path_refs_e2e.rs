use autoloop::{AutoLoopApp, config::AppConfig};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn pevo_r9_decision_path_refs_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "pevo-r9-path-refs";

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
        "principal:pevo-r9",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");

    let _ = app
        .process_direct(session_id, "trigger evolution path execution for r9 ref checks")
        .await
        .expect("process direct");

    let records = app.state_store()
        .list_knowledge_by_prefix(&format!("evo:shadow:{session_id}:process_direct:"))
        .await
        .expect("list records");
    let path_record = records
        .iter()
        .filter(|r| r.key.ends_with(":path-execution"))
        .max_by_key(|r| r.key.clone())
        .expect("path execution record");
    let value: serde_json::Value = serde_json::from_str(&path_record.value).expect("json");

    assert_eq!(
        value["decision_path_consistent"].as_bool(),
        Some(true),
        "decision -> path must stay consistent"
    );

    let path = value["path"].as_str().unwrap_or_default();
    let status = value["status"].as_str().unwrap_or_default();
    if status == "blocked" {
        assert_eq!(
            value["reason"].as_str(),
            Some("production_write_gate_denied"),
            "blocked path must come from production write gate"
        );
        assert!(
            value.get("gate").is_some(),
            "blocked payload must carry unified gate metadata"
        );
        return;
    }

    match path {
        "9B" => {
            assert!(value.get("apply_ready_ref").is_some(), "9B needs apply_ready_ref");
            assert!(value.get("apply_ready_history_ref").is_some(), "9B needs apply_ready_history_ref");
        }
        "9C" => {
            assert!(value.get("version_ref").is_some(), "9C needs version_ref");
            assert!(value.get("index_ref").is_some(), "9C needs index_ref");
        }
        "9D" => {
            assert!(value.get("rollback_ref").is_some(), "9D needs rollback_ref");
            assert!(value.get("rollback_history_ref").is_some(), "9D needs rollback_history_ref");
        }
        "9A" => {
            // Runtime update path is valid; R9-specific refs are for 9B/9C/9D.
            assert!(value.get("runtime_gate").is_some() || value.get("status").is_some());
        }
        other => panic!("unexpected path code: {other}"),
    }
}




