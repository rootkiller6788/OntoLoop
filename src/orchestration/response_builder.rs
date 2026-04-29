use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{SwarmOutcome, current_time_ms};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseDisclosure {
    pub code: String,
    pub message: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredResponse {
    pub response: String,
    pub evidence_refs: Vec<String>,
    pub disclosure: Vec<ResponseDisclosure>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ResponseBuilderInput<'a> {
    pub session_id: &'a str,
    pub outcome: &'a SwarmOutcome,
    pub research_autonomy_score: f32,
    pub scheduled_research_tasks: usize,
    pub evolution_summary: &'a str,
    pub retired_capability_count: usize,
    pub governance_risk_tier: &'a str,
    pub policy_revised: bool,
    pub decision_kind: &'a str,
    pub decision_reasons: &'a [String],
    pub decision_evidence_ref: Option<&'a str>,
}

#[derive(Debug, Default, Clone)]
pub struct ResponseBuilder;

impl ResponseBuilder {
    pub fn build(input: ResponseBuilderInput<'_>) -> StructuredResponse {
        let response = format!(
            "Requirement brief: {}\nResearch autonomy: {:.2}\nResearch follow-ups: {}\nCEO: {}\nVerifier: {}\nSwarm tasks: {}\nValidation: {}\nEvolution: {}\nRetired capabilities: {}\nGraph update: {}",
            input.outcome.brief.clarified_goal,
            input.research_autonomy_score,
            input.scheduled_research_tasks,
            input.outcome.ceo_summary,
            input.outcome.verifier_report.summary,
            input.outcome.tasks.len(),
            input.outcome.validation.summary,
            input.evolution_summary,
            input.retired_capability_count,
            input.outcome.knowledge_update.global_context_summary
        );

        let mut evidence_refs = vec![
            format!("protocol:{}:verifier-report", input.session_id),
            format!("protocol:{}:execution-verifier-report", input.session_id),
            format!("observability:{}:audit-evidence-view", input.session_id),
            format!("protocol:{}:immutable-eval", input.session_id),
        ];
        if let Some(decision_ref) = input.decision_evidence_ref {
            evidence_refs.push(decision_ref.to_string());
        }

        let mut disclosure = vec![ResponseDisclosure {
            code: "governance_risk_tier".to_string(),
            message: format!("risk_tier={}", input.governance_risk_tier),
            severity: "info".to_string(),
        }];

        if input.policy_revised {
            disclosure.push(ResponseDisclosure {
                code: "policy_revised_request".to_string(),
                message: "request revised by policy gate before execution".to_string(),
                severity: "medium".to_string(),
            });
        }

        if !input.decision_reasons.is_empty() {
            disclosure.push(ResponseDisclosure {
                code: "runtime_decision".to_string(),
                message: format!(
                    "decision={} reasons={}",
                    input.decision_kind,
                    input.decision_reasons.join(" | ")
                ),
                severity: if input.decision_kind.eq_ignore_ascii_case("reject") {
                    "high".to_string()
                } else {
                    "info".to_string()
                },
            });
        }

        let mut metadata = BTreeMap::new();
        metadata.insert("builder".to_string(), "response_builder_v1".to_string());
        metadata.insert("session_id".to_string(), input.session_id.to_string());
        metadata.insert("generated_at_ms".to_string(), current_time_ms().to_string());
        metadata.insert(
            "verifier_verdict".to_string(),
            format!("{:?}", input.outcome.verifier_report.verdict).to_ascii_lowercase(),
        );

        StructuredResponse {
            response,
            evidence_refs,
            disclosure,
            metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        orchestration::{
            ExecutionReport, RequirementBrief, SwarmDeliberation, SwarmOutcome, SwarmTask,
            ValidationReport,
        },
        providers::OptimizationProposal,
        rag::{GraphKnowledgeUpdate, GraphRoutingSignals},
        runtime::{
            CapabilityRegressionSuite, RouteCorrectnessReport, TaskLevelJudgement, VerifierReport,
            VerifierVerdict,
        },
    };

    fn mock_outcome() -> SwarmOutcome {
        SwarmOutcome {
            brief: RequirementBrief {
                anchor_id: "anchor:test".to_string(),
                original_request: "test".to_string(),
                clarified_goal: "deliver governed response".to_string(),
                frozen_scope: "scope".to_string(),
                open_questions: vec![],
                acceptance_criteria: vec!["done".to_string()],
                clarification_turns: vec![],
                confirmation_required: false,
            },
            optimization_proposal: OptimizationProposal {
                title: "optimize".to_string(),
                change_target: "routing".to_string(),
                hypothesis: "h".to_string(),
                expected_gain: "i".to_string(),
                risk: "low".to_string(),
                patch_outline: vec!["step-1".to_string()],
                evaluation_focus: "quality".to_string(),
            },
            routing_context: crate::orchestration::RoutingContext {
                history_records: vec![],
                execution_metrics: vec![],
                graph_signals: GraphRoutingSignals::default(),
                pending_event_count: 0,
                learning_evidence: vec![],
                skill_success_rate: 0.0,
                causal_confidence: 0.0,
                forged_tool_coverage: 0,
                session_ab_stats: None,
                task_ab_stats: std::collections::HashMap::new(),
                tool_ab_stats: std::collections::HashMap::new(),
                server_ab_stats: std::collections::HashMap::new(),
                agent_reputations: std::collections::HashMap::new(),
                route_biases: vec![],
            },
            ceo_summary: "ship it".to_string(),
            deliberation: SwarmDeliberation {
                planner_notes: "".to_string(),
                critic_notes: "".to_string(),
                planner_rebuttal: "".to_string(),
                judge_notes: "".to_string(),
                arbitration_summary: "".to_string(),
                round_count: 0,
                rounds: vec![],
                final_execution_order: vec![],
                consensus_signals: vec![],
            },
            tasks: vec![SwarmTask {
                task_id: "t1".to_string(),
                agent_name: "agent".to_string(),
                role: "role".to_string(),
                objective: "obj".to_string(),
                depends_on: vec![],
            }],
            execution_reports: vec![ExecutionReport {
                task: SwarmTask {
                    task_id: "t1".to_string(),
                    agent_name: "agent".to_string(),
                    role: "role".to_string(),
                    objective: "obj".to_string(),
                    depends_on: vec![],
                },
                output: "ok".to_string(),
                tool_used: None,
                mcp_server: None,
                invocation_payload: None,
                outcome_score: 1,
                route_variant: "control".to_string(),
                control_score: 1,
                treatment_score: 0,
                guard_decision: "allow".to_string(),
            }],
            verifier_report: VerifierReport {
                verifier_name: "execution-verifier".to_string(),
                verdict: VerifierVerdict::Pass,
                overall_score: 0.9,
                summary: "verified".to_string(),
                task_judgements: vec![TaskLevelJudgement {
                    task_role: "role".to_string(),
                    satisfied: true,
                    score: 0.9,
                    summary: "ok".to_string(),
                }],
                route_reports: vec![RouteCorrectnessReport {
                    task_role: "role".to_string(),
                    tool_name: None,
                    route_variant: "control".to_string(),
                    aligned_with_catalog: true,
                    aligned_with_graph: true,
                    guard_ok: true,
                    score: 0.9,
                    summary: "aligned".to_string(),
                }],
                capability_regression: CapabilityRegressionSuite {
                    suite_name: "capability-regression".to_string(),
                    all_passed: true,
                    score: 1.0,
                    failing_tools: vec![],
                    cases: vec![],
                    summary: "all good".to_string(),
                },
                recommended_actions: vec![],
            },
            validation: ValidationReport {
                ready: true,
                summary: "ready".to_string(),
                follow_up_tasks: vec![],
                verifier_summary: "ok".to_string(),
            },
            knowledge_update: GraphKnowledgeUpdate {
                document_id: 1,
                local_context_summary: "local".to_string(),
                global_context_summary: "graph refreshed".to_string(),
                task_capability_map_summary: "map".to_string(),
                snapshot_json: "{}".to_string(),
            },
        }
    }

    #[test]
    fn response_builder_outputs_response_evidence_and_disclosure() {
        let outcome = mock_outcome();
        let reasons = vec!["repair_path_scheduled".to_string()];
        let built = ResponseBuilder::build(ResponseBuilderInput {
            session_id: "s-1",
            outcome: &outcome,
            research_autonomy_score: 0.7,
            scheduled_research_tasks: 1,
            evolution_summary: "evolved",
            retired_capability_count: 0,
            governance_risk_tier: "medium",
            policy_revised: true,
            decision_kind: "repair",
            decision_reasons: &reasons,
            decision_evidence_ref: Some("decision:s-1:latest"),
        });

        assert!(!built.response.trim().is_empty());
        assert!(
            built
                .evidence_refs
                .iter()
                .any(|item| item == "decision:s-1:latest"),
            "decision evidence should be carried"
        );
        assert!(
            built
                .disclosure
                .iter()
                .any(|item| item.code == "policy_revised_request"),
            "policy rewrite disclosure should be present"
        );
    }
}
