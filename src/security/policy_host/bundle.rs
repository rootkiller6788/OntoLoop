use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tar::Archive;

use crate::contracts::policy_pdp::{PolicyMode, PolicyVersion};

use super::engine::WasmPolicyHost;
use super::verify::{
    PolicyBundleVerifier, PolicyBundleVerifyRequirements, PolicyBundleVerifyResult,
    enforce_verified,
};

const MANIFEST_FILE: &str = "manifest.json";
const DEFAULT_WASM_FILE: &str = "policy.wasm";
const DEFAULT_DATA_FILE: &str = "data.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyBundleManifest {
    pub policy_id: String,
    pub policy_version: PolicyVersion,
    #[serde(default = "default_wasm_entrypoint")]
    pub wasm_entrypoint: String,
    #[serde(default = "default_wasm_file")]
    pub wasm_file: String,
    #[serde(default = "default_data_file")]
    pub data_file: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct LoadedPolicyBundle {
    pub source_archive: PathBuf,
    pub extracted_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub wasm_path: PathBuf,
    pub data_path: PathBuf,
    pub manifest: PolicyBundleManifest,
    pub data: Value,
}

impl LoadedPolicyBundle {
    pub fn load_from_archive(
        source_archive: impl AsRef<Path>,
        staging_root: impl AsRef<Path>,
    ) -> Result<Self> {
        let source_archive = source_archive.as_ref().to_path_buf();
        let extracted_dir = extract_bundle_archive(&source_archive, staging_root.as_ref())?;
        Self::load_from_dir(source_archive, extracted_dir)
    }

    pub fn load_from_dir(source_archive: PathBuf, extracted_dir: PathBuf) -> Result<Self> {
        let manifest_path = extracted_dir.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            bail!("policy bundle missing required file: {MANIFEST_FILE}");
        }

        let manifest_raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?;
        let manifest: PolicyBundleManifest = serde_json::from_str(&manifest_raw)
            .with_context(|| format!("failed to parse {}", manifest_path.display()))?;

        let wasm_path = extracted_dir.join(&manifest.wasm_file);
        if !wasm_path.exists() {
            bail!(
                "policy bundle missing wasm file '{}' declared by manifest",
                manifest.wasm_file
            );
        }

        let data_path = extracted_dir.join(&manifest.data_file);
        if !data_path.exists() {
            bail!(
                "policy bundle missing data file '{}' declared by manifest",
                manifest.data_file
            );
        }
        let data_raw = fs::read_to_string(&data_path)
            .with_context(|| format!("failed to read {}", data_path.display()))?;
        let data: Value = serde_json::from_str(&data_raw)
            .with_context(|| format!("failed to parse {}", data_path.display()))?;

        Ok(Self {
            source_archive,
            extracted_dir,
            manifest_path,
            wasm_path,
            data_path,
            manifest,
            data,
        })
    }

    pub fn instantiate_host(&self, mode: PolicyMode) -> Result<WasmPolicyHost> {
        WasmPolicyHost::from_wasm_file(
            self.manifest.policy_id.clone(),
            self.manifest.policy_version.clone(),
            mode,
            self.manifest.wasm_entrypoint.clone(),
            &self.wasm_path,
        )
    }
}

#[derive(Debug, Clone)]
pub struct BundleActivationManager {
    root: PathBuf,
}

impl BundleActivationManager {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn staging_root(&self) -> PathBuf {
        self.root.join("staging")
    }

    pub fn candidate_dir(&self) -> PathBuf {
        self.root.join("candidate")
    }

    pub fn current_dir(&self) -> PathBuf {
        self.root.join("current")
    }

    pub fn rollback_dir(&self) -> PathBuf {
        self.root.join("rollback")
    }

    pub fn root_dir(&self) -> &Path {
        &self.root
    }

    pub fn load_bundle(&self, archive_path: impl AsRef<Path>) -> Result<LoadedPolicyBundle> {
        LoadedPolicyBundle::load_from_archive(archive_path, self.staging_root())
    }

    pub fn stage_candidate(&self, bundle: &LoadedPolicyBundle) -> Result<PathBuf> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let candidate_dir = self.candidate_dir();
        if candidate_dir.exists() {
            fs::remove_dir_all(&candidate_dir).with_context(|| {
                format!("failed to clear candidate dir {}", candidate_dir.display())
            })?;
        }
        copy_dir_all(&bundle.extracted_dir, &candidate_dir)?;
        Ok(candidate_dir)
    }

    pub fn commit_candidate(&self) -> Result<PathBuf> {
        let candidate_dir = self.candidate_dir();
        if !candidate_dir.exists() {
            bail!(
                "no staged candidate found at {}, cannot commit",
                candidate_dir.display()
            );
        }

        let current_dir = self.current_dir();
        let rollback_dir = self.rollback_dir();

        if rollback_dir.exists() {
            fs::remove_dir_all(&rollback_dir).with_context(|| {
                format!(
                    "failed to clear previous rollback dir {}",
                    rollback_dir.display()
                )
            })?;
        }

        if current_dir.exists() {
            fs::rename(&current_dir, &rollback_dir).with_context(|| {
                format!(
                    "failed to move current -> rollback ({} -> {})",
                    current_dir.display(),
                    rollback_dir.display()
                )
            })?;
        }

        if let Err(commit_error) = fs::rename(&candidate_dir, &current_dir) {
            if rollback_dir.exists() {
                let _ = fs::rename(&rollback_dir, &current_dir);
            }
            bail!(
                "failed to commit candidate bundle ({} -> {}): {}",
                candidate_dir.display(),
                current_dir.display(),
                commit_error
            );
        }

        if rollback_dir.exists() {
            let _ = fs::remove_dir_all(&rollback_dir);
        }

        Ok(current_dir)
    }

    pub fn rollback(&self) -> Result<PathBuf> {
        let current_dir = self.current_dir();
        let rollback_dir = self.rollback_dir();

        if !rollback_dir.exists() {
            bail!(
                "rollback bundle not found at {}, cannot rollback",
                rollback_dir.display()
            );
        }

        if current_dir.exists() {
            fs::remove_dir_all(&current_dir).with_context(|| {
                format!(
                    "failed to remove current bundle before rollback {}",
                    current_dir.display()
                )
            })?;
        }

        fs::rename(&rollback_dir, &current_dir).with_context(|| {
            format!(
                "failed to restore rollback bundle ({} -> {})",
                rollback_dir.display(),
                current_dir.display()
            )
        })?;
        Ok(current_dir)
    }

    pub fn activate_bundle(&self, bundle: &LoadedPolicyBundle) -> Result<PathBuf> {
        self.stage_candidate(bundle)?;
        self.commit_candidate()
    }

    pub fn verify_and_activate_bundle(
        &self,
        bundle: &LoadedPolicyBundle,
        verifier: &dyn PolicyBundleVerifier,
        requirements: &PolicyBundleVerifyRequirements,
    ) -> Result<PolicyBundleVerifyResult> {
        let result = verifier.verify(bundle, requirements)?;
        enforce_verified(&result)?;
        self.activate_bundle(bundle)?;
        Ok(result)
    }
}

fn extract_bundle_archive(archive_path: &Path, staging_root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(staging_root)
        .with_context(|| format!("failed to create staging root {}", staging_root.display()))?;

    let staging_dir = staging_root.join(format!(
        "bundle-{}-{}",
        current_time_ms(),
        std::process::id()
    ));
    fs::create_dir_all(&staging_dir)
        .with_context(|| format!("failed to create staging dir {}", staging_dir.display()))?;

    let bundle_file = fs::File::open(archive_path)
        .with_context(|| format!("failed to open bundle archive {}", archive_path.display()))?;
    let gzip_decoder = GzDecoder::new(bundle_file);
    let mut archive = Archive::new(gzip_decoder);

    let entries = archive
        .entries()
        .with_context(|| format!("failed to list entries in {}", archive_path.display()))?;
    for entry in entries {
        let mut entry = entry?;
        let entry_path = entry.path()?;
        let safe_path = sanitize_archive_path(&entry_path)?;
        let target_path = staging_dir.join(&safe_path);

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&target_path)
                .with_context(|| format!("failed to create {}", target_path.display()))?;
            continue;
        }

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut contents = Vec::new();
        entry.read_to_end(&mut contents)?;
        fs::write(&target_path, contents)
            .with_context(|| format!("failed to write {}", target_path.display()))?;
    }

    Ok(staging_dir)
}

fn sanitize_archive_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("archive entry has empty path");
    }

    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => cleaned.push(segment),
            Component::CurDir => {}
            Component::ParentDir => bail!("archive entry contains parent traversal: {path:?}"),
            Component::RootDir | Component::Prefix(_) => {
                bail!("archive entry must be relative: {path:?}")
            }
        }
    }

    if cleaned.as_os_str().is_empty() {
        bail!("archive entry resolved to empty path");
    }

    if cleaned
        .components()
        .any(|component| component.as_os_str() == OsStr::new(".."))
    {
        bail!("archive entry contains unsafe '..' component: {path:?}");
    }

    Ok(cleaned)
}

fn copy_dir_all(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to list source dir {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy file {} -> {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn default_wasm_entrypoint() -> String {
    "eval".to_string()
}

fn default_wasm_file() -> String {
    DEFAULT_WASM_FILE.to_string()
}

fn default_data_file() -> String {
    DEFAULT_DATA_FILE.to_string()
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
