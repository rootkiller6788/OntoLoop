use std::{fs, path::Path};

use anyhow::Result;

use super::incremental_compiler::{IncrementalCompileReport, source_digest};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HotIndexEntry {
    pub source_file: String,
    pub source_digest: String,
    pub summary: String,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HotIndexUpdateReport {
    pub index_path: String,
    pub touched_files: Vec<String>,
    pub total_entries: usize,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RefreshPlanMode {
    Detect,
    DryRun,
    Force,
    Page,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SourceRefreshPlan {
    pub mode: RefreshPlanMode,
    pub requested_files: Vec<String>,
    pub stale_files: Vec<String>,
    pub unchanged_files: Vec<String>,
    pub missing_files: Vec<String>,
    pub effective_changed_files: Vec<String>,
    #[serde(default)]
    pub page: Option<usize>,
    #[serde(default)]
    pub page_size: Option<usize>,
    #[serde(default)]
    pub total_candidates: usize,
    #[serde(default)]
    pub paged_files: Vec<String>,
}

pub struct HotIndexUpdater;

impl HotIndexUpdater {
    pub fn plan_refresh(
        repo_root: &Path,
        requested_files: &[String],
        mode: RefreshPlanMode,
    ) -> Result<SourceRefreshPlan> {
        Self::plan_refresh_with_options(repo_root, requested_files, mode, None, None)
    }

    pub fn plan_refresh_with_options(
        repo_root: &Path,
        requested_files: &[String],
        mode: RefreshPlanMode,
        page: Option<usize>,
        page_size: Option<usize>,
    ) -> Result<SourceRefreshPlan> {
        let index_path = repo_root.join(".gitmemory").join("hot_index.json");
        let map = load_index_map(&index_path)?;
        let requested = requested_files
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<std::collections::BTreeSet<_>>();

        let mut stale = std::collections::BTreeSet::<String>::new();
        let mut unchanged = std::collections::BTreeSet::<String>::new();
        let mut missing = std::collections::BTreeSet::<String>::new();

        for (source_file, entry) in &map {
            let digest = source_digest(repo_root, source_file)?;
            match digest {
                None => {
                    missing.insert(source_file.clone());
                    stale.insert(source_file.clone());
                }
                Some(actual) => {
                    if mode == RefreshPlanMode::Force || actual != entry.source_digest {
                        stale.insert(source_file.clone());
                    } else {
                        unchanged.insert(source_file.clone());
                    }
                }
            }
        }

        for requested_file in requested.clone() {
            let digest = source_digest(repo_root, &requested_file)?;
            if digest.is_none() {
                missing.insert(requested_file);
            }
        }

        let mut effective = std::collections::BTreeSet::<String>::new();
        for requested_file in &requested {
            effective.insert(requested_file.clone());
        }
        for stale_file in &stale {
            effective.insert(stale_file.clone());
        }

        let mut paged_files = Vec::<String>::new();
        let mut total_candidates = 0usize;
        let normalized_page = page.unwrap_or(1).max(1);
        let normalized_page_size = page_size.unwrap_or(50).max(1);
        if mode == RefreshPlanMode::Page {
            let mut candidates = stale
                .iter()
                .cloned()
                .chain(requested.iter().cloned())
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if candidates.is_empty() {
                candidates = map.keys().cloned().collect::<Vec<_>>();
                candidates.sort();
            }
            total_candidates = candidates.len();
            let start = normalized_page
                .saturating_sub(1)
                .saturating_mul(normalized_page_size);
            if start < candidates.len() {
                let end = (start + normalized_page_size).min(candidates.len());
                paged_files = candidates[start..end].to_vec();
            }
            effective = paged_files
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
        }

        Ok(SourceRefreshPlan {
            mode,
            requested_files: requested.iter().cloned().collect(),
            stale_files: stale.iter().cloned().collect(),
            unchanged_files: unchanged.iter().cloned().collect(),
            missing_files: missing.iter().cloned().collect(),
            effective_changed_files: effective.into_iter().collect(),
            page: (mode == RefreshPlanMode::Page).then_some(normalized_page),
            page_size: (mode == RefreshPlanMode::Page).then_some(normalized_page_size),
            total_candidates,
            paged_files,
        })
    }

    pub fn update(
        repo_root: &Path,
        compile: &IncrementalCompileReport,
    ) -> Result<HotIndexUpdateReport> {
        let index_path = repo_root.join(".gitmemory").join("hot_index.json");
        if let Some(parent) = index_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut map = load_index_map(&index_path)?;
        let now = current_time_ms();
        let mut touched = Vec::new();
        for compiled in &compile.compiled_files {
            touched.push(compiled.source_file.clone());
            map.insert(
                compiled.source_file.clone(),
                HotIndexEntry {
                    source_file: compiled.source_file.clone(),
                    source_digest: compiled.source_digest.clone(),
                    summary: format!(
                        "bytes={} blocks={} deps={} errors={} projections={}",
                        compiled.bytes,
                        compiled.block_count,
                        compiled.dependencies.len(),
                        compiled.errors.len(),
                        compiled.projection_files.len()
                    ),
                    updated_at_ms: now,
                },
            );
        }

        let mut entries = map.values().cloned().collect::<Vec<_>>();
        entries.sort_by(|a, b| a.source_file.cmp(&b.source_file));
        fs::write(&index_path, serde_json::to_string_pretty(&entries)?)?;

        Ok(HotIndexUpdateReport {
            index_path: index_path.display().to_string(),
            touched_files: touched,
            total_entries: entries.len(),
        })
    }
}

fn load_index_map(path: &Path) -> Result<std::collections::BTreeMap<String, HotIndexEntry>> {
    if !path.exists() {
        return Ok(std::collections::BTreeMap::new());
    }
    let raw = fs::read_to_string(path)?;
    let entries = serde_json::from_str::<Vec<HotIndexEntry>>(&raw).unwrap_or_default();
    Ok(entries
        .into_iter()
        .map(|entry| (entry.source_file.clone(), entry))
        .collect())
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{HotIndexEntry, HotIndexUpdater, RefreshPlanMode};
    use std::fs;

    #[test]
    fn refresh_plan_detect_marks_stale_and_keeps_unchanged() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-refresh-plan-detect-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ));
        fs::create_dir_all(temp.join(".gitmemory")).expect("mkdir");
        fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
        fs::write(temp.join("docs").join("a.md"), "# A\n").expect("write a");
        fs::write(temp.join("docs").join("b.md"), "# B\n").expect("write b");

        let digest_a = super::source_digest(&temp, "docs/a.md")
            .expect("digest a")
            .expect("digest a exists");
        let stale_digest = "stale-digest".to_string();
        let index = vec![
            HotIndexEntry {
                source_file: "docs/a.md".to_string(),
                source_digest: digest_a,
                summary: "ok".to_string(),
                updated_at_ms: 1,
            },
            HotIndexEntry {
                source_file: "docs/b.md".to_string(),
                source_digest: stale_digest,
                summary: "stale".to_string(),
                updated_at_ms: 1,
            },
        ];
        fs::write(
            temp.join(".gitmemory").join("hot_index.json"),
            serde_json::to_string_pretty(&index).expect("serialize index"),
        )
        .expect("write index");

        let plan = HotIndexUpdater::plan_refresh(
            &temp,
            &["docs/a.md".to_string()],
            RefreshPlanMode::Detect,
        )
        .expect("plan");
        assert_eq!(plan.requested_files, vec!["docs/a.md".to_string()]);
        assert_eq!(plan.stale_files, vec!["docs/b.md".to_string()]);
        assert_eq!(plan.unchanged_files, vec!["docs/a.md".to_string()]);
        assert_eq!(
            plan.effective_changed_files,
            vec!["docs/a.md".to_string(), "docs/b.md".to_string()]
        );
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn refresh_plan_force_includes_indexed_sources() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-refresh-plan-force-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ));
        fs::create_dir_all(temp.join(".gitmemory")).expect("mkdir");
        fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
        fs::write(temp.join("docs").join("a.md"), "# A\n").expect("write a");
        let digest_a = super::source_digest(&temp, "docs/a.md")
            .expect("digest a")
            .expect("digest a exists");
        let index = vec![HotIndexEntry {
            source_file: "docs/a.md".to_string(),
            source_digest: digest_a,
            summary: "ok".to_string(),
            updated_at_ms: 1,
        }];
        fs::write(
            temp.join(".gitmemory").join("hot_index.json"),
            serde_json::to_string_pretty(&index).expect("serialize index"),
        )
        .expect("write index");

        let plan =
            HotIndexUpdater::plan_refresh(&temp, &[], RefreshPlanMode::Force).expect("plan");
        assert_eq!(plan.stale_files, vec!["docs/a.md".to_string()]);
        assert_eq!(plan.effective_changed_files, vec!["docs/a.md".to_string()]);
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn refresh_plan_page_returns_stable_slice() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-refresh-plan-page-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ));
        fs::create_dir_all(temp.join(".gitmemory")).expect("mkdir");
        fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
        for name in ["a", "b", "c", "d"] {
            fs::write(temp.join("docs").join(format!("{name}.md")), format!("# {name}\n"))
                .expect("write");
        }
        let index = vec![
            HotIndexEntry {
                source_file: "docs/a.md".to_string(),
                source_digest: "digest:a".to_string(),
                summary: "stale".to_string(),
                updated_at_ms: 1,
            },
            HotIndexEntry {
                source_file: "docs/b.md".to_string(),
                source_digest: "digest:b".to_string(),
                summary: "stale".to_string(),
                updated_at_ms: 1,
            },
            HotIndexEntry {
                source_file: "docs/c.md".to_string(),
                source_digest: "digest:c".to_string(),
                summary: "stale".to_string(),
                updated_at_ms: 1,
            },
            HotIndexEntry {
                source_file: "docs/d.md".to_string(),
                source_digest: "digest:d".to_string(),
                summary: "stale".to_string(),
                updated_at_ms: 1,
            },
        ];
        fs::write(
            temp.join(".gitmemory").join("hot_index.json"),
            serde_json::to_string_pretty(&index).expect("serialize index"),
        )
        .expect("write index");

        let plan = HotIndexUpdater::plan_refresh_with_options(
            &temp,
            &[],
            RefreshPlanMode::Page,
            Some(2),
            Some(2),
        )
        .expect("plan");

        assert_eq!(plan.page, Some(2));
        assert_eq!(plan.page_size, Some(2));
        assert_eq!(plan.total_candidates, 4);
        assert_eq!(
            plan.effective_changed_files,
            vec!["docs/c.md".to_string(), "docs/d.md".to_string()]
        );
        assert_eq!(
            plan.paged_files,
            vec!["docs/c.md".to_string(), "docs/d.md".to_string()]
        );
        let _ = fs::remove_dir_all(&temp);
    }
}
