use autoloop::plugins::gitmemory_core::{
    gateway_core::GatewayDecision,
    recall_core::RecallCore,
};

#[test]
fn pwiki11_recall_neighbor_expansion_respects_threshold_and_limit() {
    let decision = GatewayDecision {
        accepted: true,
        scope: "tenant:t:memory".to_string(),
        reason: "ok".to_string(),
        normalized_intent: "review memory graph health".to_string(),
    };

    let plan = RecallCore::plan_with_graph_expansion(
        &decision,
        "plugin-recall-router",
        vec!["memory:graph:health".to_string()],
        vec!["memory:graph:health".to_string()],
        true,
        0.7,
        1,
    );

    assert!(plan.graph_enabled);
    assert_eq!(plan.neighbor_expansion.len(), 1);
    assert!(plan.neighbor_expansion[0].confidence >= 0.7);
}



