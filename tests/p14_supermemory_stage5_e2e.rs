use std::collections::{BTreeMap, HashSet};

use autoloop::{AutoLoopApp, config::AppConfig, contracts::context::KnowledgeContext};
use autoloop_state_adapter::PermissionAction;

#[tokio::test]
async fn stage5_supermemory_persists_six_evidence_classes_and_injects_knowledge_context_refs() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session_id = "stage6-supermemory-e2e";

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
        "tenant:stage6",
        "principal:stage6",
        "policy:default",
        3_600_000,
    )
    .await
    .expect("seed identity");

    let seeded_content = [
        "Policy planning uses deterministic evidence tagging for runtime guard and verifier checks.",
        "Policy planning uses deterministic evidence tagging for runtime guard and verifier checks with updated routing hints.",
        "Execution results feed learning proposals and promotion gates with replay fingerprints.",
        "Execution results feed learning proposals and promotion gates with replay fingerprints and refreshed capability trust.",
        "Budget reserve consume refund records remain append only and link to trace and capability decision.",
        "Budget reserve consume refund records remain append only and link to trace and capability decision with latest reconciliation.",
        "Knowledge context bundles include private memory source evidence and profile snapshots for each session.",
        "Knowledge context bundles include private memory source evidence and profile snapshots for each session with replay scope.",
    ]
    .join(" ");

    let seeded_context = app
        .memory
        .run_supermemory_pipeline(
            &app.state_store(),
            session_id,
            "tenant:stage6",
            "supermemory-stage6-seed",
            &seeded_content,
            BTreeMap::from([
                ("source".to_string(), "stage6-seed".to_string()),
                ("tags".to_string(), "policy,memory,replay".to_string()),
            ]),
            Some("2026-03-24".to_string()),
            Some("2026-03-24".to_string()),
            "policy memory replay routing",
        )
        .await
        .expect("seed supermemory pipeline");
    assert!(
        !seeded_context.hits.is_empty(),
        "seeded supermemory context should include retrieval hits"
    );

    let response = app
        .process_requirement_swarm(
            session_id,
            "Build one governed swarm loop and persist supermemory context for audit replay.",
        )
        .await
        .expect("process requirement swarm");
    assert!(
        !response.trim().is_empty(),
        "process_requirement_swarm should return a non-empty response"
    );

    let documents = app.state_store()
        .list_knowledge_by_prefix(&format!("memory:supermemory:documents:{session_id}:"))
        .await
        .expect("list documents");
    let chunks = app.state_store()
        .list_knowledge_by_prefix(&format!("memory:supermemory:chunks:{session_id}:"))
        .await
        .expect("list chunks");
    let atomic = app.state_store()
        .list_knowledge_by_prefix(&format!("memory:supermemory:atomic:{session_id}:"))
        .await
        .expect("list atomic");
    let relations = app.state_store()
        .list_knowledge_by_prefix(&format!("memory:supermemory:relations:{session_id}:"))
        .await
        .expect("list relations");
    let profile = app.state_store()
        .list_knowledge_by_prefix(&format!("memory:supermemory:profile:{session_id}:"))
        .await
        .expect("list profile");
    let context_records = app.state_store()
        .list_knowledge_by_prefix(&format!("memory:supermemory:context:{session_id}:"))
        .await
        .expect("list context");

    let evidence_classes = [
        ("documents", &documents),
        ("chunks", &chunks),
        ("atomic", &atomic),
        ("relations", &relations),
        ("profile", &profile),
        ("context", &context_records),
    ];
    for (class_name, records) in evidence_classes {
        assert!(
            !records.is_empty(),
            "supermemory evidence class should not be empty: {class_name}"
        );
        assert!(
            records.iter().any(|record| record.key.ends_with(":latest")),
            "supermemory evidence class should include latest marker: {class_name}"
        );
    }

    assert!(
        documents
            .iter()
            .any(|record| record.value.contains("deterministic evidence tagging")),
        "documents should retain original seeded content evidence"
    );

    let chunk_ids: HashSet<String> = chunks
        .iter()
        .filter_map(|record| {
            serde_json::from_str::<serde_json::Value>(&record.value)
                .ok()
                .and_then(|value| {
                    value
                        .get("chunk_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
        })
        .collect();
    assert!(
        !chunk_ids.is_empty(),
        "chunks should expose at least one chunk_id"
    );

    let mut memory_ids = HashSet::new();
    for record in &atomic {
        let value: serde_json::Value = serde_json::from_str(&record.value).expect("parse atomic");
        let memory_id = value
            .get("memory_id")
            .and_then(serde_json::Value::as_str)
            .expect("atomic memory_id");
        let chunk_id = value
            .get("chunk_id")
            .and_then(serde_json::Value::as_str)
            .expect("atomic chunk_id");
        assert!(
            chunk_ids.contains(chunk_id),
            "atomic memory must reference an existing chunk: {chunk_id}"
        );
        memory_ids.insert(memory_id.to_string());
    }

    let mut relation_types = HashSet::new();
    for record in &relations {
        let value: serde_json::Value = serde_json::from_str(&record.value).expect("parse relation");
        let from_memory_id = value
            .get("from_memory_id")
            .and_then(serde_json::Value::as_str)
            .expect("relation from_memory_id");
        let to_memory_id = value
            .get("to_memory_id")
            .and_then(serde_json::Value::as_str)
            .expect("relation to_memory_id");
        assert!(
            memory_ids.contains(from_memory_id),
            "relation from_memory_id must exist in atomic memories"
        );
        assert!(
            memory_ids.contains(to_memory_id),
            "relation to_memory_id must exist in atomic memories"
        );
        if let Some(relation_type) = value
            .get("relation_type")
            .and_then(serde_json::Value::as_str)
        {
            relation_types.insert(relation_type.to_string());
        }
    }
    assert!(
        !relation_types.is_empty(),
        "relations should contain at least one inferred relation type"
    );

    let context_latest = context_records
        .iter()
        .find(|record| record.key.ends_with(":latest"))
        .expect("context latest record");
    let context_value: serde_json::Value =
        serde_json::from_str(&context_latest.value).expect("parse context latest");
    let hits = context_value
        .get("hits")
        .and_then(serde_json::Value::as_array)
        .expect("context hits array");
    assert!(!hits.is_empty(), "context hits should not be empty");
    for hit in hits {
        let memory_id = hit
            .get("memory_id")
            .and_then(serde_json::Value::as_str)
            .expect("hit memory_id");
        let chunk_id = hit
            .get("chunk_id")
            .and_then(serde_json::Value::as_str)
            .expect("hit chunk_id");
        assert!(
            memory_ids.contains(memory_id),
            "context hit memory_id should be traceable to atomic memory"
        );
        assert!(
            chunk_ids.contains(chunk_id),
            "context hit chunk_id should be traceable to chunk"
        );
    }

    let knowledge_context_record = app.state_store()
        .get_knowledge(&format!("knowledge-context:{session_id}:latest"))
        .await
        .expect("read knowledge context")
        .expect("knowledge context latest should exist");
    let knowledge_context: KnowledgeContext = serde_json::from_str(&knowledge_context_record.value)
        .expect("deserialize knowledge context");

    assert!(
        !knowledge_context.private_memory_refs.is_empty(),
        "knowledge context must include private memory refs"
    );
    assert!(
        knowledge_context
            .private_memory_refs
            .iter()
            .any(|key| key.contains("memory:supermemory:atomic:")),
        "private memory refs should include supermemory atomic keys"
    );
    assert!(
        !knowledge_context.source_evidence_refs.is_empty(),
        "knowledge context must include source evidence refs"
    );
    assert!(
        knowledge_context
            .source_evidence_refs
            .iter()
            .any(|key| key.contains("memory:supermemory:chunks:"))
            && knowledge_context
                .source_evidence_refs
                .iter()
                .any(|key| key.contains("memory:supermemory:documents:")),
        "source evidence refs should include both chunks and documents"
    );
    assert!(
        !knowledge_context.context_bundle_refs.is_empty(),
        "knowledge context must include context bundle refs"
    );
    assert!(
        knowledge_context
            .context_bundle_refs
            .iter()
            .any(|key| key.contains("memory:supermemory:context:"))
            && knowledge_context
                .context_bundle_refs
                .iter()
                .any(|key| key.contains("memory:supermemory:profile:")),
        "context bundle refs should include context and profile"
    );

    let expected_context_ref = format!("memory:supermemory:context:{session_id}:latest");
    let expected_profile_ref = format!("memory:supermemory:profile:{session_id}:latest");
    let expected_metrics_ref = format!("observability:{session_id}:supermemory-metrics");
    let metadata = &knowledge_context.metadata;

    assert_eq!(
        metadata
            .get("supermemory_context_latest_ref")
            .map(String::as_str),
        Some(expected_context_ref.as_str())
    );
    assert_eq!(
        metadata
            .get("supermemory_profile_latest_ref")
            .map(String::as_str),
        Some(expected_profile_ref.as_str())
    );
    assert_eq!(
        metadata.get("supermemory_metrics_ref").map(String::as_str),
        Some(expected_metrics_ref.as_str())
    );
    assert!(
        metadata
            .get("supermemory_private_ref_count")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            > 0,
        "supermemory private ref count should be positive"
    );
    assert!(
        metadata
            .get("supermemory_source_ref_count")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            > 0,
        "supermemory source ref count should be positive"
    );
    assert!(
        metadata
            .get("supermemory_context_ref_count")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            > 0,
        "supermemory context ref count should be positive"
    );
    assert!(
        metadata
            .get("supermemory_context_retrieval_hits")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            > 0,
        "knowledge context metadata should contain retrieval hits populated by read-path"
    );
    assert!(
        metadata
            .get("supermemory_metrics_retrieval_hits")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0)
            > 0,
        "knowledge context metadata should contain metrics retrieval hits populated by read-path"
    );
}




