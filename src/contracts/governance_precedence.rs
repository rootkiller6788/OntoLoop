use crate::contracts::errors::ContractError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SovereigntyStage {
    ConstraintShield,
    ReviewGate,
    Wal,
    Harness,
}

pub const SOVEREIGNTY_PRECEDENCE_VERSION: &str = "v1";

pub const SOVEREIGNTY_PRECEDENCE_ORDER: [SovereigntyStage; 4] = [
    SovereigntyStage::ConstraintShield,
    SovereigntyStage::ReviewGate,
    SovereigntyStage::Wal,
    SovereigntyStage::Harness,
];

pub fn canonical_stage_chain() -> Vec<&'static str> {
    SOVEREIGNTY_PRECEDENCE_ORDER
        .iter()
        .map(SovereigntyStage::as_str)
        .collect()
}

pub fn validate_stage_chain(stages: &[SovereigntyStage]) -> Result<(), ContractError> {
    if stages.is_empty() {
        return Err(ContractError::InvalidTransition(
            "governance precedence chain cannot be empty".to_string(),
        ));
    }
    let mut next_index = 0usize;
    for stage in stages {
        let mut found = None;
        for (idx, canonical) in SOVEREIGNTY_PRECEDENCE_ORDER.iter().enumerate().skip(next_index) {
            if canonical == stage {
                found = Some(idx);
                break;
            }
        }
        let Some(index) = found else {
            return Err(ContractError::InvalidTransition(format!(
                "governance precedence violated at stage '{}'",
                stage.as_str()
            )));
        };
        next_index = index + 1;
    }
    Ok(())
}

impl SovereigntyStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ConstraintShield => "constraint_shield",
            Self::ReviewGate => "review_gate",
            Self::Wal => "wal",
            Self::Harness => "harness",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SovereigntyStage, canonical_stage_chain, validate_stage_chain};

    #[test]
    fn full_chain_is_valid() {
        assert!(validate_stage_chain(&[
            SovereigntyStage::ConstraintShield,
            SovereigntyStage::ReviewGate,
            SovereigntyStage::Wal,
            SovereigntyStage::Harness,
        ])
        .is_ok());
    }

    #[test]
    fn subset_chain_is_valid_when_ordered() {
        assert!(validate_stage_chain(&[
            SovereigntyStage::ConstraintShield,
            SovereigntyStage::ReviewGate,
            SovereigntyStage::Wal,
        ])
        .is_ok());
    }

    #[test]
    fn invalid_order_fails() {
        assert!(
            validate_stage_chain(&[SovereigntyStage::Wal, SovereigntyStage::ConstraintShield])
                .is_err()
        );
    }

    #[test]
    fn canonical_names_are_stable() {
        assert_eq!(
            canonical_stage_chain(),
            vec!["constraint_shield", "review_gate", "wal", "harness"]
        );
    }
}
