use autoloop::plugins::gitmemory_core::GitmemoryCoreKernel;
use autoloop_state_adapter::{StateStoreBackend, StateStore, StateStoreConfig};

fn in_memory_db() -> StateStore {
    StateStore::from_config(&StateStoreConfig {
        enabled: true,
        backend: StateStoreBackend::InMemory,
        uri: "http://state_store:3000".into(),
        module_name: "autoloop_core".into(),
        namespace: "autoloop".into(),
        pool_size: 4,
    })
}

#[tokio::test]
async fn pwiki11_view_plane_persist_writes_graph_health_latest_ref() {
    let db = in_memory_db();
    let kernel = GitmemoryCoreKernel::new();
    let session_id = "pwiki11-view";
    let trace_id = "trace:pwiki11:view";

    db.upsert_json_knowledge(
        format!("graph:{session_id}:snapshot"),
        &serde_json::json!({
            "nodes": ["memory:graph:health", "memory:graph:health:summary"],
            "edges": [{"from":"memory:graph:health", "to":"memory:graph:health:summary"}]
        }),
        "graph",
    )
    .await
    .expect("seed graph snapshot");

    let run = kernel
        .run_phase3_source_view_plane(&db, session_id, trace_id)
        .await
        .expect("phase3 view");

    assert_eq!(run.ledger_refs.len(), 1);
    let latest = db
        .get_knowledge(&format!("memory:graph:health:{session_id}:latest"))
        .await
        .expect("db")
        .expect("latest");
    assert!(latest.value.contains("memory:graph:health"));
}




