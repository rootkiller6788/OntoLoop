use autoloop::{AutoLoopApp, config::AppConfig};

#[tokio::test]
async fn background_task_shell_lifecycle_restart_and_log_tail_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "bg-shell-e2e";

    let start = app
        .background_task_start_shell(
            session_id,
            "shell-ok",
            "Write-Output 'background-shell-ok'",
            0,
        )
        .await
        .expect("start shell");
    let start_json: serde_json::Value = serde_json::from_str(&start).expect("start json");
    assert_eq!(start_json["status"], "started");

    for _ in 0..30 {
        let polled = app
            .background_task_status(session_id, Some("shell-ok"))
            .await
            .expect("status poll");
        let value: serde_json::Value = serde_json::from_str(&polled).expect("poll json");
        let status = value["tasks"][0]["status"].as_str().unwrap_or("running");
        if status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let status = app
        .background_task_status(session_id, Some("shell-ok"))
        .await
        .expect("status");
    let status_json: serde_json::Value = serde_json::from_str(&status).expect("status json");
    assert_eq!(status_json["count"], 1);

    let logs = app
        .background_task_logs(session_id, "shell-ok", 20)
        .await
        .expect("logs");
    let logs_json: serde_json::Value = serde_json::from_str(&logs).expect("logs json");
    let joined = logs_json["logs"]
        .as_array()
        .expect("logs array")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("background-shell-ok"));

    let start_fail = app
        .background_task_start_shell(
            session_id,
            "shell-fail",
            "Write-Error 'background-shell-fail'; exit 1",
            1,
        )
        .await
        .expect("start fail shell");
    let start_fail_json: serde_json::Value =
        serde_json::from_str(&start_fail).expect("start fail json");
    assert_eq!(start_fail_json["status"], "started");

    for _ in 0..30 {
        let polled = app
            .background_task_status(session_id, Some("shell-fail"))
            .await
            .expect("failed poll");
        let value: serde_json::Value = serde_json::from_str(&polled).expect("failed poll json");
        let status = value["tasks"][0]["status"].as_str().unwrap_or("running");
        if status != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let failed_status = app
        .background_task_status(session_id, Some("shell-fail"))
        .await
        .expect("failed status");
    let failed_status_json: serde_json::Value =
        serde_json::from_str(&failed_status).expect("failed status json");
    let restart_count = failed_status_json["tasks"][0]["restart_count"]
        .as_u64()
        .unwrap_or(0);
    assert!(
        restart_count >= 1,
        "expected restart count >= 1, got {}",
        restart_count
    );

    let restarted = app
        .background_task_restart(session_id, "shell-ok")
        .await
        .expect("restart");
    let restarted_json: serde_json::Value = serde_json::from_str(&restarted).expect("restart json");
    assert_eq!(restarted_json["status"], "restarted");

    let stopping = app
        .background_task_stop(session_id, "shell-fail")
        .await
        .expect("stop");
    let stopping_json: serde_json::Value = serde_json::from_str(&stopping).expect("stop json");
    assert_eq!(stopping_json["status"], "stopping");
}

#[tokio::test]
async fn background_task_agent_entrypoint_is_available_e2e() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "bg-agent-e2e";

    let started = app
        .background_task_start_agent(
            session_id,
            "agent-one",
            "Respond with one short line saying background agent ok.",
            0,
        )
        .await
        .expect("start agent");
    let started_json: serde_json::Value = serde_json::from_str(&started).expect("start json");
    assert_eq!(started_json["status"], "started");

    tokio::time::sleep(std::time::Duration::from_millis(220)).await;
    let status = app
        .background_task_status(session_id, Some("agent-one"))
        .await
        .expect("status");
    let status_json: serde_json::Value = serde_json::from_str(&status).expect("status json");
    assert_eq!(status_json["count"], 1);
    let state = status_json["tasks"][0]["status"]
        .as_str()
        .unwrap_or("unknown");
    assert!(
        ["running", "completed", "failed", "stopped", "stopping"].contains(&state),
        "unexpected status {}",
        state
    );
}



