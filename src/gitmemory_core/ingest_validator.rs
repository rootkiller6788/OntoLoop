use std::{
    collections::BTreeSet,
    path::Path,
};

use anyhow::Result;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IngestValidationMode {
    Enforced,
    ValidateOnly,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct IngestValidationReport {
    pub mode: IngestValidationMode,
    pub relative_path: String,
    pub checked_at_ms: u64,
    pub broken_links: Vec<String>,
    pub unindexed: Vec<String>,
    pub passed: bool,
}

pub struct IngestValidator;

impl IngestValidator {
    pub fn validate(
        repo_root: &Path,
        relative_path: &str,
        content: &str,
        mode: IngestValidationMode,
    ) -> Result<IngestValidationReport> {
        let refs = extract_refs(content, relative_path);
        let mut broken_links = BTreeSet::<String>::new();
        let mut unindexed = BTreeSet::<String>::new();

        for target in refs {
            let target_path = repo_root.join(&target);
            if !target_path.exists() {
                broken_links.insert(target.clone());
                continue;
            }
            if !has_projection_index(repo_root, &target) {
                unindexed.insert(target);
            }
        }

        let broken_links = broken_links.into_iter().collect::<Vec<_>>();
        let unindexed = unindexed.into_iter().collect::<Vec<_>>();
        let passed = broken_links.is_empty() && unindexed.is_empty();

        Ok(IngestValidationReport {
            mode,
            relative_path: relative_path.to_string(),
            checked_at_ms: current_time_ms(),
            broken_links,
            unindexed,
            passed,
        })
    }
}

fn has_projection_index(repo_root: &Path, source_file: &str) -> bool {
    let projection_root = repo_root.join(".gitmemory").join("projections");
    let projection_key = sanitize(source_file);
    ["graph", "vector", "search"].iter().all(|plane| {
        projection_root
            .join(plane)
            .join(format!("{projection_key}.json"))
            .exists()
    })
}

fn extract_refs(input: &str, source_file: &str) -> Vec<String> {
    let mut refs = BTreeSet::new();
    refs.extend(extract_wiki_refs(input, source_file));
    refs.extend(extract_markdown_refs(input, source_file));
    refs.into_iter().collect()
}

fn extract_wiki_refs(input: &str, source_file: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut cursor = input;
    while let Some(start) = cursor.find("[[") {
        let rest = &cursor[start + 2..];
        if let Some(end) = rest.find("]]") {
            let target = rest[..end].trim();
            if !target.is_empty() {
                refs.push(normalize_dependency(source_file, target));
            }
            cursor = &rest[end + 2..];
        } else {
            break;
        }
    }
    refs
}

fn extract_markdown_refs(input: &str, source_file: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let mut cursor = input;
    while let Some(open) = cursor.find("](") {
        let rest = &cursor[open + 2..];
        if let Some(close) = rest.find(')') {
            let target = rest[..close].trim();
            if !target.is_empty() {
                refs.push(normalize_dependency(source_file, target));
            }
            cursor = &rest[close + 1..];
        } else {
            break;
        }
    }
    refs
}

fn normalize_dependency(source_file: &str, target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        return target.to_string();
    }
    let mut local = target
        .split('#')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if local.is_empty() {
        return source_file.to_string();
    }
    if local.starts_with("./") {
        local = local.trim_start_matches("./").to_string();
    }
    if !local.ends_with(".md") && !local.ends_with(".markdown") {
        local.push_str(".md");
    }
    local.replace('\\', "/")
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{IngestValidationMode, IngestValidator};
    use std::fs;

    #[test]
    fn validate_only_reports_broken_links_and_unindexed_without_writing() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-ingest-validator-{}",
            super::current_time_ms()
        ));
        fs::create_dir_all(temp.join("docs")).expect("mkdir docs");
        fs::write(temp.join("docs").join("indexed.md"), "# Indexed\n").expect("write indexed");
        fs::write(temp.join("docs").join("unindexed.md"), "# Unindexed\n").expect("write unindexed");

        let key = "docs_indexed_md";
        fs::create_dir_all(temp.join(".gitmemory").join("projections").join("graph"))
            .expect("mkdir graph");
        fs::create_dir_all(temp.join(".gitmemory").join("projections").join("vector"))
            .expect("mkdir vector");
        fs::create_dir_all(temp.join(".gitmemory").join("projections").join("search"))
            .expect("mkdir search");
        fs::write(
            temp.join(".gitmemory").join("projections").join("graph").join(format!("{key}.json")),
            "{}",
        )
        .expect("write graph");
        fs::write(
            temp.join(".gitmemory").join("projections").join("vector").join(format!("{key}.json")),
            "{}",
        )
        .expect("write vector");
        fs::write(
            temp.join(".gitmemory").join("projections").join("search").join(format!("{key}.json")),
            "{}",
        )
        .expect("write search");

        let content = "Links: [[docs/indexed]] [[docs/unindexed]] [[docs/missing]]";
        let report = IngestValidator::validate(
            &temp,
            "canonical/memory/new.md",
            content,
            IngestValidationMode::ValidateOnly,
        )
        .expect("validate");

        assert!(!report.passed);
        assert!(report.broken_links.iter().any(|item| item == "docs/missing.md"));
        assert!(report.unindexed.iter().any(|item| item == "docs/unindexed.md"));
        assert!(report.unindexed.iter().all(|item| item != "docs/indexed.md"));
        let _ = fs::remove_dir_all(&temp);
    }
}
