use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonicalWriteReceipt {
    pub repo_root: String,
    pub relative_path: String,
    pub absolute_path: String,
    pub bytes: usize,
}

pub struct CanonicalRepo;

impl CanonicalRepo {
    pub fn ensure(repo_root: &Path) -> Result<()> {
        let canonical_root = repo_root.join("canonical");
        let control_root = repo_root.join(".gitmemory");
        fs::create_dir_all(&canonical_root)?;
        fs::create_dir_all(&control_root)?;
        Ok(())
    }

    pub fn write_atomic(
        repo_root: &Path,
        relative_path: &str,
        content: &str,
    ) -> Result<CanonicalWriteReceipt> {
        Self::ensure(repo_root)?;
        let target = repo_root.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        // Atomic replace: write temp then rename.
        let mut temp = PathBuf::from(&target);
        temp.set_extension("tmp");
        fs::write(&temp, content.as_bytes())?;
        if target.exists() {
            fs::remove_file(&target)?;
        }
        fs::rename(&temp, &target)?;

        Ok(CanonicalWriteReceipt {
            repo_root: repo_root.display().to_string(),
            relative_path: relative_path.to_string(),
            absolute_path: target.display().to_string(),
            bytes: content.len(),
        })
    }
}
