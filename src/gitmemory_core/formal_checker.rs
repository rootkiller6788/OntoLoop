use anyhow::Result;
use autoloop_state_adapter::StateStore;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FormalCheckMode {
    Admission,
    Verify,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FormalCheckRuleResult {
    pub rule_id: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FormalCheckReport {
    pub session_id: String,
    pub trace_id: String,
    #[serde(default = "default_formal_check_mode")]
    pub mode: FormalCheckMode,
    pub passed: bool,
    pub rule_results: Vec<FormalCheckRuleResult>,
}

pub struct FormalChecker;

impl FormalChecker {
    pub async fn check(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<FormalCheckReport> {
        Self::check_with_mode(db, session_id, trace_id, FormalCheckMode::Verify).await
    }

    pub async fn check_with_mode(
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        mode: FormalCheckMode,
    ) -> Result<FormalCheckReport> {
        let mut rules = Vec::<FormalCheckRuleResult>::new();

        let recall_key = format!("memory:recall:route:{session_id}:latest");
        let recall_exists = db.get_knowledge(&recall_key).await?.is_some();
        let admission_exists = !db
            .list_knowledge_by_prefix(&format!("capability-admission:{session_id}:"))
            .await?
            .is_empty();
        let recall_or_admission_exists = recall_exists || admission_exists;
        rules.push(FormalCheckRuleResult {
            rule_id: "inv-recall-or-admission-exists".to_string(),
            passed: recall_or_admission_exists,
            detail: if recall_or_admission_exists {
                if recall_exists {
                    "recall route exists".to_string()
                } else {
                    "capability admission evidence exists".to_string()
                }
            } else {
                "missing recall route and capability admission evidence".to_string()
            },
        });

        let patch_key = format!("memory:patch:review:{session_id}:latest");
        let patch_exists = db.get_knowledge(&patch_key).await?.is_some();
        let verifier_report_exists = db
            .get_knowledge(&format!("protocol:{session_id}:verifier-report"))
            .await?
            .is_some();
        let review_anchor_exists = patch_exists || verifier_report_exists;
        let review_required = matches!(mode, FormalCheckMode::Verify);
        let review_passed = if review_required {
            review_anchor_exists
        } else {
            true
        };
        rules.push(FormalCheckRuleResult {
            rule_id: "inv-review-anchor-exists".to_string(),
            passed: review_passed,
            detail: if review_anchor_exists {
                if patch_exists {
                    "patch review exists".to_string()
                } else {
                    "verifier report exists".to_string()
                }
            } else if review_required {
                "missing patch review and verifier report".to_string()
            } else {
                "review anchor deferred for admission phase".to_string()
            },
        });

        let provenance_key = format!("memory:provenance:{session_id}:{trace_id}:latest");
        let provenance_exists = db.get_knowledge(&provenance_key).await?.is_some();
        let stage_chain_exists = !db
            .list_knowledge_by_prefix(&format!("evidence:stage:{session_id}:"))
            .await?
            .is_empty();
        let lineage_anchor_exists =
            provenance_exists || stage_chain_exists || verifier_report_exists;
        let lineage_required = matches!(mode, FormalCheckMode::Verify);
        let lineage_passed = if lineage_required {
            lineage_anchor_exists
        } else {
            true
        };
        rules.push(FormalCheckRuleResult {
            rule_id: "inv-lineage-anchor-exists".to_string(),
            passed: lineage_passed,
            detail: if lineage_anchor_exists {
                if provenance_exists {
                    "provenance lineage exists".to_string()
                } else if verifier_report_exists {
                    "verifier report exists".to_string()
                } else {
                    "evidence stage chain exists".to_string()
                }
            } else if lineage_required {
                "missing provenance lineage and evidence stage chain".to_string()
            } else {
                "lineage anchor deferred for admission phase".to_string()
            },
        });

        let passed = rules.iter().all(|item| item.passed);
        let report = FormalCheckReport {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            mode,
            passed,
            rule_results: rules,
        };
        db.upsert_json_knowledge(
            format!("memory:formal-check:{session_id}:{trace_id}:latest"),
            &report,
            "formal-checker",
        )
        .await?;
        Ok(report)
    }
}

fn default_formal_check_mode() -> FormalCheckMode {
    FormalCheckMode::Verify
}

