use super::frozen_manifest;
use super::protocol::{CorePackageKind, CorePackageManifest};

pub struct CompilerCore;

impl CompilerCore {
    pub fn manifest_frozen() -> CorePackageManifest {
        frozen_manifest(
            "core.compiler",
            CorePackageKind::CompilerCore,
            "incremental-compiler",
        )
    }
}
