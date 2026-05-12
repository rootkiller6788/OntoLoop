use std::{fs, path::Path};

use autoloop::{
    observability::query_plane::persist_unified_query_view,
    plugins::gitmemory_core::{
        atomic_renderer::AtomicRenderer,
        heal_proposal::HealProposalRequest,
        patch_core::PatchOpKind,
        patch_review_queue::PatchReviewQueue,
        recall_core::RecallCore,
        GitmemoryCoreKernel,
    },
};
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct ProfileGuard(Option<String>);

impl Drop for ProfileGuard {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            unsafe {
                std::env::set_var("AUTOLOOP_PROFILE", value);
            }
        } else {
            unsafe {
                std::env::remove_var("AUTOLOOP_PROFILE");
            }
        }
    }
}

fn force_integration_profile() -> ProfileGuard {
    let previous = std::env::var("AUTOLOOP_PROFILE").ok();
    unsafe {
        std::env::set_var("AUTOLOOP_PROFILE", "integration");
    }
    ProfileGuard(previous)
}

#[tokio::test]
async fn pwiki11_end_to_end_compile_infer_health_recall_heal_recompile_query() {
    let _profile_guard = force_integration_profile();
    let db = in_memory_db();
    let kernel = GitmemoryCoreKernel::new();
    let session_id = "pwiki11-e2e";
    let tenant_id = "tenant:pwiki11";
    let trace_id = "trace:pwiki11:e2e";

    let temp = std::env::temp_dir().join(format!("pwiki11_e2e_repo_{}", now_ms()));
    fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
    fs::create_dir_all(temp.join("memory")).expect("mkdir memory");
    fs::write(temp.join("memory").join("MEMORY.md"), "# Memory\n[[docs/a]]\n")
        .expect("write memory");
    fs::write(
        temp.join("docs").join("a.md"),
        "# \u{56FE}\u{8C31}\u{5065}\u{5EB7}\n[[docs/b]]\n",
    )
    .expect("write a");
    fs::write(temp.join("docs").join("b.md"), "# B\n").expect("write b");

    let run34 = kernel
        .run_gateway_recall_patch(
            &db,
            session_id,
            tenant_id,
            "\u{67E5}\u{770B}\u{56FE}\u{8C31}\u{5065}\u{5EB7}",
            "principal:pwiki11",
        )
        .await
        .expect("day34");

    let run78 = kernel
        .run_day78_incremental(
            &db,
            &temp,
            session_id,
            trace_id,
            &[
                "memory/MEMORY.md".to_string(),
                "docs/a.md".to_string(),
                "docs/b.md".to_string(),
            ],
        )
        .await
        .expect("day78 compile");
    assert!(!run78.compile.compiled_files.is_empty());
    assert!(!run78.compile.inference_checkpoint_records.is_empty());

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

    let phase3 = kernel
        .run_phase3_source_view_plane(&db, session_id, trace_id)
        .await
        .expect("phase3");
    assert_eq!(phase3.ledger_refs.len(), 1);

    let recall = RecallCore::plan_with_graph_expansion(
        &run34.gateway,
        "plugin-recall-router",
        vec![
            "memory:graph:health".to_string(),
            "memory:graph:health:summary".to_string(),
            "memory:\u{56FE}\u{8C31}:\u{5065}\u{5EB7}".to_string(),
            "memory:notes:other".to_string(),
        ],
        vec![],
        true,
        0.6,
        3,
    );
    assert!(
        !recall.seed_hits.is_empty()
            || !recall.cjk_lexical_hits.is_empty()
            || recall
                .query_route_fallback
                .as_ref()
                .map(|item| item.applied)
                .unwrap_or(false)
    );

    let heal = kernel
        .run_heal_proposal(
            &db,
            session_id,
            trace_id,
            HealProposalRequest {
                namespace: "memory:wiki".to_string(),
                target: "docs/a.md".to_string(),
                reason: "repair graph-neighbor relation".to_string(),
                op_kind: PatchOpKind::Update,
            },
        )
        .await
        .expect("heal proposal");

    let _approved = PatchReviewQueue::approve(
        &db,
        session_id,
        &heal.proposal.review_id,
        "principal:reviewer",
        "approved in e2e",
    )
    .await
    .expect("approve proposal");

    let committed = kernel
        .execute_approved_heal_proposal(
            &db,
            &temp,
            session_id,
            trace_id,
            &heal.proposal.review_id,
            "principal:signer",
        )
        .await
        .expect("execute approved heal");

    assert!(Path::new(&temp.join(&committed.canonical_write.relative_path)).exists());

    let recompiled = kernel
        .run_day78_incremental(
            &db,
            &temp,
            session_id,
            trace_id,
            std::slice::from_ref(&committed.canonical_write.relative_path),
        )
        .await
        .expect("recompile after heal");
    assert!(!recompiled.compile.compiled_files.is_empty());

    let view = persist_unified_query_view(&db, session_id, Some(trace_id))
        .await
        .expect("query plane");
    assert!(view.replay.is_object());
    assert!(view.graph.is_object());

    let _ = fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn pwiki11_heal_proposal_requires_approval_before_write_and_supports_replay_after_approval() {
    let _profile_guard = force_integration_profile();
    let db = in_memory_db();
    let kernel = GitmemoryCoreKernel::new();
    let session_id = "pwiki11-approval-gate";
    let tenant_id = "tenant:pwiki11";
    let trace_id = "trace:pwiki11:approval-gate";

    let temp = std::env::temp_dir().join(format!("pwiki11_approval_repo_{}", now_ms()));
    fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
    fs::create_dir_all(temp.join("memory")).expect("mkdir memory");
    fs::write(temp.join("memory").join("MEMORY.md"), "# Memory\n[[docs/a]]\n")
        .expect("write memory");
    fs::write(temp.join("docs").join("a.md"), "# A\n").expect("write a");

    let _run = kernel
        .run_gateway_recall_patch(
            &db,
            session_id,
            tenant_id,
            "repair memory entry with approval gate",
            "principal:pwiki11",
        )
        .await
        .expect("day34");

    let _compiled = kernel
        .run_day78_incremental(
            &db,
            &temp,
            session_id,
            trace_id,
            &["memory/MEMORY.md".to_string(), "docs/a.md".to_string()],
        )
        .await
        .expect("day78");

    let heal = kernel
        .run_heal_proposal(
            &db,
            session_id,
            trace_id,
            HealProposalRequest {
                namespace: "memory:wiki".to_string(),
                target: "docs/a.md".to_string(),
                reason: "approval-gated repair".to_string(),
                op_kind: PatchOpKind::Update,
            },
        )
        .await
        .expect("heal proposal");

    // 未审批前：执行必须失败，且不得落盘。
    let rendered = AtomicRenderer::render(session_id, trace_id, &heal.proposal.patch);
    let pending_target = temp.join(&rendered.relative_path);
    let unapproved = kernel
        .execute_approved_heal_proposal(
            &db,
            &temp,
            session_id,
            trace_id,
            &heal.proposal.review_id,
            "principal:signer",
        )
        .await;
    assert!(unapproved.is_err(), "unapproved proposal must be rejected");
    assert!(
        unapproved
            .err()
            .expect("err")
            .to_string()
            .contains("not approved yet"),
        "deny reason should indicate approval gate"
    );
    assert!(
        !pending_target.exists(),
        "canonical file must not exist before approval"
    );

    // 审批后：执行可生效并且可回放（commit chain / query replay anchor 可见）。
    let approved = PatchReviewQueue::approve(
        &db,
        session_id,
        &heal.proposal.review_id,
        "principal:reviewer",
        "approved for d8 e2e",
    )
    .await
    .expect("approve proposal");
    assert_eq!(
        format!("{:?}", approved.status).to_ascii_lowercase(),
        "approved"
    );

    let committed = kernel
        .execute_approved_heal_proposal(
            &db,
            &temp,
            session_id,
            trace_id,
            &heal.proposal.review_id,
            "principal:signer",
        )
        .await
        .expect("execute approved heal");

    let committed_path = temp.join(&committed.canonical_write.relative_path);
    assert!(committed_path.exists(), "approved proposal should materialize file");

    let commit_chain = db
        .get_knowledge(&committed.commit_chain_ref)
        .await
        .expect("db")
        .expect("commit chain");
    assert!(
        commit_chain.value.contains("\"signature\""),
        "commit chain should contain replayable signature anchor"
    );

    let query_view = persist_unified_query_view(&db, session_id, Some(trace_id))
        .await
        .expect("query plane");
    let patch_review_visible = query_view
        .graph
        .get("patch_review")
        .or_else(|| query_view.replay.get("patch_review"))
        .is_some();
    assert!(patch_review_visible || query_view.replay.is_object());

    let _ = fs::remove_dir_all(&temp);
}




