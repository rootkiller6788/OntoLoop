use super::frozen_manifest;
use super::protocol::{CorePackageKind, CorePackageManifest};

pub struct ContextCore;

impl ContextCore {
    pub fn manifest_frozen() -> CorePackageManifest {
        frozen_manifest(
            "core.context",
            CorePackageKind::ContextCore,
            "context-builder",
        )
    }
}
