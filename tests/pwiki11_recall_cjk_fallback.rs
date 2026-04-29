use autoloop::plugins::gitmemory_core::{
    gateway_core::GatewayDecision,
    recall_core::RecallCore,
};

#[test]
fn pwiki11_recall_cjk_bigram_fallback_hits() {
    let decision = GatewayDecision {
        accepted: true,
        scope: "tenant:t:memory".to_string(),
        reason: "ok".to_string(),
        normalized_intent: "\u{67E5}\u{770B}\u{56FE}\u{8C31}\u{5065}\u{5EB7}".to_string(),
    };
    let sources = vec![
        "memory:graph:health".to_string(),
        "memory:\u{56FE}\u{8C31}:\u{5065}\u{5EB7}".to_string(),
        "memory:notes:other".to_string(),
    ];

    let plan = RecallCore::plan_with_graph_expansion(
        &decision,
        "plugin-recall-router",
        sources,
        Vec::new(),
        true,
        0.6,
        5,
    );

    assert!(!plan.cjk_lexical_hits.is_empty());
    assert!(!plan.seed_hits.is_empty());
}



