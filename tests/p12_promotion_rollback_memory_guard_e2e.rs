use autoloop::memory::{LearningGateVerdict, LearningProposal, LearningSignal, SkillRecord};
use autoloop::{AutoLoopApp, config::AppConfig};

#[tokio::test]
async fn promotion_rollback_does_not_leave_active_skill_memory() {
    let app = AutoLoopApp::new(AppConfig::default());
    let session = "rollback-e2e";

    let proposal = LearningProposal {
        proposal_id: format!("proposal:{session}"),
        session_id: session.to_string(),
        anchor: "routing".to_string(),
        hypothesis: "rollback should demote skill safely".to_string(),
        reason: "e2e rollback guard".to_string(),
        proposed_skill_name: "skill-routing-rollback".to_string(),
        proposed_confidence: 0.77,
        created_at_ms: autoloop::orchestration::current_time_ms(),
    };
    let verdict = LearningGateVerdict {
        approved: true,
        reason: "approved for canary".to_string(),
        canary_ratio: 0.2,
        rollback_window_ms: 900_000,
        risk_tags: vec!["medium".to_string()],
        created_at_ms: autoloop::orchestration::current_time_ms(),
    };
    let candidate = SkillRecord {
        name: proposal.proposed_skill_name.clone(),
        trigger: proposal.anchor.clone(),
        procedure: "candidate-procedure".to_string(),
        confidence: proposal.proposed_confidence,
    };
    let signal = LearningSignal {
        signal_id: format!("learning-signal:{session}:e2e"),
        session_id: session.to_string(),
        trace_id: format!("trace:{session}:learning:e2e"),
        source: "tests.p12".to_string(),
        evidence_ref: format!(
            "evidence:tag:{session}:learning:{}",
            autoloop::orchestration::current_time_ms()
        ),
        metadata: std::collections::BTreeMap::new(),
    };

    let promoted = app
        .memory
        .promote_skill_with_verdict(&app.state_store(), &proposal, &verdict, &candidate, &signal)
        .await
        .expect("promote");

    let _rolled = app
        .memory
        .rollback_skill_promotion(
            &app.state_store(),
            session,
            &promoted,
            "e2e rollback",
            &signal,
        )
        .await
        .expect("rollback");

    let skills = app.state_store()
        .list_skill_library_records(session)
        .await
        .expect("skills");

    let maybe = skills
        .iter()
        .find(|s| s.name == proposal.proposed_skill_name);
    assert!(
        maybe.is_some(),
        "skill record should exist as tombstone after rollback"
    );
    let skill = maybe.unwrap();
    assert!(
        skill.confidence <= f32::EPSILON,
        "rolled-back skill must be demoted to non-active confidence"
    );
    assert_eq!(skill.trigger, "rolled-back");
}




