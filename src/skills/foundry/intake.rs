use crate::contracts::skill_foundry::FoundryIntake;

pub fn normalize_intake(mut intake: FoundryIntake) -> FoundryIntake {
    intake.task_name = intake.task_name.trim().to_string();
    intake.expected_output = intake.expected_output.trim().to_string();
    intake
}
