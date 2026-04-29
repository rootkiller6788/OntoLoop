use super::frozen_manifest;
use super::protocol::{CorePackageKind, CorePackageManifest};
use super::recall_core::RecallPlan;

pub struct PatchCore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOpKind {
    Add,
    Update,
    Delete,
    None,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PatchOp {
    pub kind: PatchOpKind,
    pub target: String,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PatchPlan {
    pub namespace: String,
    pub ops: Vec<PatchOp>,
}

impl PatchCore {
    pub fn manifest_frozen() -> CorePackageManifest {
        frozen_manifest("core.patch", CorePackageKind::PatchCore, "patch-governance")
    }

    pub fn build(recall: &RecallPlan) -> PatchPlan {
        PatchPlan {
            namespace: recall.scope.clone(),
            ops: vec![PatchOp {
                kind: if recall.query.is_empty() {
                    PatchOpKind::None
                } else {
                    PatchOpKind::Add
                },
                target: "memory:episodes".to_string(),
                reason: "derive patch candidate from recall plan".to_string(),
            }],
        }
    }
}
