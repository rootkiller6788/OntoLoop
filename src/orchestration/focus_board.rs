use crate::contracts::focus_trigger::{FocusBoard, FocusItem};
use crate::orchestration::RequirementBrief;

#[derive(Debug, Clone, Default)]
pub struct FocusBoardBuilder;

impl FocusBoardBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn build(&self, session_id: &str, brief: &RequirementBrief) -> FocusBoard {
        let mut items = Vec::new();
        items.push(FocusItem {
            id: "focus-clarified-goal".to_string(),
            title: "Clarified Goal Locked".to_string(),
            status: "ready".to_string(),
            owner: "requirement-agent".to_string(),
            acceptance_hint: brief.clarified_goal.clone(),
        });

        for (idx, criterion) in brief.acceptance_criteria.iter().enumerate() {
            items.push(FocusItem {
                id: format!("focus-acceptance-{}", idx + 1),
                title: format!("Acceptance {}", idx + 1),
                status: "pending".to_string(),
                owner: "execution-agent".to_string(),
                acceptance_hint: criterion.clone(),
            });
        }

        if items.len() == 1 {
            items.push(FocusItem {
                id: "focus-deliverable".to_string(),
                title: "Deliverable Quality Check".to_string(),
                status: "pending".to_string(),
                owner: "verifier-agent".to_string(),
                acceptance_hint: "Result must pass verifier and policy checks".to_string(),
            });
        }

        FocusBoard {
            session_id: session_id.to_string(),
            goal: brief.clarified_goal.clone(),
            items,
        }
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::FocusBoardBuilderPort for FocusBoardBuilder {
    async fn build_focus_board(
        &self,
        intent: &crate::contracts::types::Intent,
    ) -> Result<FocusBoard, crate::contracts::errors::ContractError> {
        let brief = RequirementBrief {
            anchor_id: intent
                .anchor
                .clone()
                .unwrap_or_else(|| format!("anchor:{}", intent.session_id.as_ref())),
            original_request: intent.message.clone(),
            clarified_goal: intent.message.clone(),
            frozen_scope: "port-adapter-default-scope".into(),
            open_questions: vec![],
            acceptance_criteria: vec!["verifier pass with policy compliance".into()],
            clarification_turns: vec![],
            confirmation_required: false,
        };
        Ok(self.build(intent.session_id.as_ref(), &brief))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::{RequirementBrief, RequirementTurn};

    #[test]
    fn focus_board_builder_creates_focus_items() {
        let brief = RequirementBrief {
            anchor_id: "anchor:test".to_string(),
            original_request: "build feature".to_string(),
            clarified_goal: "deliver governed workflow".to_string(),
            frozen_scope: "core path".to_string(),
            open_questions: vec![],
            acceptance_criteria: vec![
                "state machine passes".to_string(),
                "audit logs present".to_string(),
            ],
            clarification_turns: vec![RequirementTurn {
                turn_index: 1,
                question: "q".to_string(),
                inferred_answer: "a".to_string(),
                resolved: true,
            }],
            confirmation_required: false,
        };
        let board = FocusBoardBuilder::new().build("session-1", &brief);
        assert_eq!(board.session_id, "session-1");
        assert!(board.items.len() >= 3);
    }
}
