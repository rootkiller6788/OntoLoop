pub mod builder;
pub mod extractor;
pub mod feedback;
pub mod intake;
pub mod packager;
pub mod router;
pub mod validator;

pub use builder::build_skill_skeleton;
pub use extractor::extract_first_principles;
pub use feedback::{
    FoundryFeedbackEvent, FoundryFeedbackKind, classify_feedback_kind, default_promotion_policy,
    evaluate_promotion_gate, load_skill_layer_state, persist_feedback_event, persist_skill_layer_state,
    record_promotion_feedback_event,
};
pub use intake::normalize_intake;
pub use packager::{build_package_meta, disable_skill, enable_skill, install_skill, package_skill};
pub use router::{promotion_suggestion, route_layer};
pub use validator::validate_skill_contract;

