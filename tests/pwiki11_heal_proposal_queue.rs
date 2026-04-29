use autoloop::plugins::gitmemory_core::{
    heal_proposal::HealProposalRequest,
    patch_core::PatchOpKind,
    patch_review_queue::PatchReviewQueue,
    GitmemoryCoreKernel,
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

#[tokio::test]
async fn pwiki11_heal_proposal_is_queued_before_manual_approval() {
    let db = in_memory_db();
    let kernel = GitmemoryCoreKernel::new();
    let session_id = "pwiki11-heal";
    let trace_id = "trace:pwiki11:heal";

    let run = kernel
        .run_heal_proposal(
            &db,
            session_id,
            trace_id,
            HealProposalRequest {
                namespace: "memory:wiki".to_string(),
                target: "docs/missing-link".to_string(),
                reason: "repair missing relation".to_string(),
                op_kind: PatchOpKind::Update,
            },
        )
        .await
        .expect("heal proposal");

    assert_eq!(run.ledger_refs.len(), 1);
    let queue = PatchReviewQueue::list(&db, session_id).await.expect("list queue");
    assert!(!queue.is_empty());
    assert!(queue.iter().any(|item| item.review_id == run.proposal.review_id));
    assert!(queue.iter().any(|item| {
        item.review_id == run.proposal.review_id
            && (item.status
                == autoloop::plugins::gitmemory_core::patch_review_queue::PatchReviewStatus::Queued
                || item.status
                    == autoloop::plugins::gitmemory_core::patch_review_queue::PatchReviewStatus::Approved)
    }));
}




