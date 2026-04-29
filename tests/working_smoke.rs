use autoloop::{AutoLoopApp, config::AppConfig};

#[tokio::test]
async fn bootstrap_smoke_test() {
    let app = AutoLoopApp::try_new(AppConfig::default()).expect("app should initialize");
    let report = app.bootstrap().await.expect("bootstrap should succeed");

    assert!(!report.app_name.is_empty());
    assert!(report.provider_count >= 1);
    assert!(report.tool_count >= 1);
}



