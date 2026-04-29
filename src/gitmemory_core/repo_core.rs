use super::frozen_manifest;
use super::protocol::{CorePackageKind, CorePackageManifest};

pub struct RepoCore;

impl RepoCore {
    pub fn manifest_frozen() -> CorePackageManifest {
        frozen_manifest(
            "core.repo",
            CorePackageKind::RepoCore,
            "canonical-repository",
        )
    }
}
