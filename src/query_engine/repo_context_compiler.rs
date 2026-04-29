use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::contracts::code_harness::{
    DependencyEdge, DiffChangeKind, FileImportanceScore, RecentDiffEntry, RepoContextBundle,
    RepoNodeKind, RepoTreeNode, REPO_CONTEXT_BUNDLE_CONTRACT_VERSION,
};

const TREE_SCAN_LIMIT: usize = 600;
const RANK_LIMIT: usize = 64;
const DEP_LIMIT: usize = 512;
const READ_FILE_LIMIT_BYTES: u64 = 512 * 1024;

pub struct RepoContextCompiler {
    repo_root: PathBuf,
}

impl RepoContextCompiler {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }

    pub fn compile(
        &self,
        session_id: &str,
        trace_id: &str,
        request_hint: &str,
    ) -> Result<RepoContextBundle> {
        let repo_tree = scan_repo_tree(&self.repo_root, TREE_SCAN_LIMIT)?;
        let dependency_graph = build_dependency_graph(&self.repo_root, &repo_tree, DEP_LIMIT);
        let recent_diff = collect_recent_diff(&self.repo_root);
        let file_importance_ranking =
            rank_file_importance(&repo_tree, &recent_diff, request_hint, RANK_LIMIT);

        let mut metadata = BTreeMap::new();
        metadata.insert("compiler".to_string(), "repo_context_compiler".to_string());
        metadata.insert(
            "bundle_contract".to_string(),
            REPO_CONTEXT_BUNDLE_CONTRACT_VERSION.to_string(),
        );
        metadata.insert(
            "bundle_sections".to_string(),
            "repo_tree,file_importance_ranking,dependency_graph,recent_diff".to_string(),
        );
        metadata.insert(
            "bundle_unified".to_string(),
            "true".to_string(),
        );
        metadata.insert(
            "tree_count".to_string(),
            repo_tree.len().to_string(),
        );
        metadata.insert(
            "dep_edge_count".to_string(),
            dependency_graph.len().to_string(),
        );
        metadata.insert(
            "recent_diff_count".to_string(),
            recent_diff.len().to_string(),
        );
        metadata.insert(
            "request_hint_len".to_string(),
            request_hint.chars().count().to_string(),
        );
        metadata.insert(
            "bundle_complete".to_string(),
            (!repo_tree.is_empty()
                && !file_importance_ranking.is_empty()
                && !dependency_graph.is_empty()
                && !recent_diff.is_empty())
            .to_string(),
        );

        Ok(RepoContextBundle {
            api_version: REPO_CONTEXT_BUNDLE_CONTRACT_VERSION.to_string(),
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            repo_root: self.repo_root.to_string_lossy().replace('\\', "/"),
            repo_tree,
            file_importance_ranking,
            dependency_graph,
            recent_diff,
            generated_at_ms: now_ms(),
            evidence_ref: None,
            replay_fp: None,
            metadata,
        })
    }
}

fn scan_repo_tree(root: &Path, limit: usize) -> Result<Vec<RepoTreeNode>> {
    let mut out = Vec::new();
    walk_dir(root, root, &mut out, limit)?;
    Ok(out)
}

fn walk_dir(root: &Path, current: &Path, out: &mut Vec<RepoTreeNode>, limit: usize) -> Result<()> {
    if out.len() >= limit {
        return Ok(());
    }
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        if out.len() >= limit {
            break;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() || should_skip_path(&rel) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            out.push(RepoTreeNode {
                path: rel.clone(),
                kind: RepoNodeKind::Directory,
                size_bytes: None,
                language_hint: None,
            });
            walk_dir(root, &path, out, limit)?;
        } else if metadata.is_file() {
            out.push(RepoTreeNode {
                path: rel,
                kind: RepoNodeKind::File,
                size_bytes: Some(metadata.len()),
                language_hint: infer_language_hint(&path),
            });
        } else if metadata.file_type().is_symlink() {
            out.push(RepoTreeNode {
                path: rel,
                kind: RepoNodeKind::Symlink,
                size_bytes: None,
                language_hint: None,
            });
        } else {
            out.push(RepoTreeNode {
                path: rel,
                kind: RepoNodeKind::Other,
                size_bytes: None,
                language_hint: None,
            });
        }
    }
    Ok(())
}

fn should_skip_path(rel: &str) -> bool {
    rel.starts_with(".git/")
        || rel.starts_with("target/")
        || rel.starts_with("node_modules/")
        || rel.starts_with("dist/")
        || rel.starts_with("build/")
}

fn infer_language_hint(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    let hint = match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" => "javascript",
        "jsx" => "jsx",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "html" => "html",
        "css" => "css",
        "md" => "markdown",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => return None,
    };
    Some(hint.to_string())
}

fn collect_recent_diff(repo_root: &Path) -> Vec<RecentDiffEntry> {
    let mut out = Vec::new();
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("status")
        .arg("--porcelain=v1")
        .output();

    let Ok(output) = output else {
        return out;
    };
    if !output.status.success() {
        return out;
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    for line in raw.lines() {
        if line.len() < 3 {
            continue;
        }
        let status = &line[0..2];
        let mut path = line[3..].trim().to_string();
        let mut old_path = None;
        let change_kind = if status.contains('R') {
            if let Some((from, to)) = path.split_once(" -> ") {
                old_path = Some(from.trim().replace('\\', "/"));
                path = to.trim().replace('\\', "/");
            }
            DiffChangeKind::Renamed
        } else if status.contains('A') || status == "??" {
            DiffChangeKind::Added
        } else if status.contains('D') {
            DiffChangeKind::Deleted
        } else if status.contains('C') {
            DiffChangeKind::Copied
        } else {
            DiffChangeKind::Modified
        };

        out.push(RecentDiffEntry {
            path: path.replace('\\', "/"),
            change_kind,
            old_path,
            added_lines: None,
            removed_lines: None,
        });
    }
    out
}

fn rank_file_importance(
    tree: &[RepoTreeNode],
    recent_diff: &[RecentDiffEntry],
    request_hint: &str,
    limit: usize,
) -> Vec<FileImportanceScore> {
    let diff_paths = recent_diff
        .iter()
        .map(|item| item.path.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let hint_tokens = tokenize_hint(request_hint);

    let mut scored = Vec::new();
    for node in tree.iter().filter(|item| matches!(item.kind, RepoNodeKind::File)) {
        let path_lower = node.path.to_ascii_lowercase();
        let mut score = 0.05_f32;
        let mut reasons = Vec::new();

        if diff_paths.contains(&path_lower) {
            score += 0.55;
            reasons.push("recent_diff".to_string());
        }
        if path_lower.starts_with("src/lib.")
            || path_lower.starts_with("src/main.")
            || path_lower.contains("/runtime/")
        {
            score += 0.25;
            reasons.push("runtime_or_entrypoint".to_string());
        }
        if path_lower.ends_with(".rs")
            || path_lower.ends_with(".py")
            || path_lower.ends_with(".ts")
            || path_lower.ends_with(".tsx")
            || path_lower.ends_with(".js")
        {
            score += 0.10;
            reasons.push("code_file".to_string());
        }
        if hint_tokens.iter().any(|token| path_lower.contains(token)) {
            score += 0.20;
            reasons.push("request_hint_match".to_string());
        }

        if score > 0.08 {
            scored.push(FileImportanceScore {
                path: node.path.clone(),
                score,
                reasons,
            });
        }
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    scored.truncate(limit);
    scored
}

fn tokenize_hint(hint: &str) -> Vec<String> {
    hint.to_ascii_lowercase()
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.'))
        .filter(|token| token.len() >= 3)
        .map(|token| token.to_string())
        .collect()
}

fn build_dependency_graph(root: &Path, tree: &[RepoTreeNode], limit: usize) -> Vec<DependencyEdge> {
    let file_paths = tree
        .iter()
        .filter(|item| matches!(item.kind, RepoNodeKind::File))
        .map(|item| item.path.clone())
        .collect::<BTreeSet<_>>();
    let mut edges = BTreeSet::<(String, String, Option<String>)>::new();

    for node in tree.iter().filter(|item| matches!(item.kind, RepoNodeKind::File)) {
        if edges.len() >= limit {
            break;
        }
        let lower = node.path.to_ascii_lowercase();
        if !(lower.ends_with(".rs")
            || lower.ends_with(".py")
            || lower.ends_with(".ts")
            || lower.ends_with(".tsx")
            || lower.ends_with(".js")
            || lower.ends_with(".jsx"))
        {
            continue;
        }
        let full_path = root.join(node.path.replace('/', "\\"));
        let size = fs::metadata(&full_path).map(|meta| meta.len()).unwrap_or(0);
        if size == 0 || size > READ_FILE_LIMIT_BYTES {
            continue;
        }
        let Ok(content) = fs::read_to_string(&full_path) else {
            continue;
        };

        for dep in extract_dependencies(node.path.as_str(), &content) {
            if !file_paths.contains(&dep.target) {
                continue;
            }
            edges.insert((node.path.clone(), dep.target, Some(dep.kind)));
            if edges.len() >= limit {
                break;
            }
        }
    }

    edges
        .into_iter()
        .map(|(from, to, kind)| DependencyEdge { from, to, kind })
        .collect()
}

struct RawDep {
    target: String,
    kind: String,
}

fn extract_dependencies(path: &str, content: &str) -> Vec<RawDep> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        extract_rust_dependencies(path, content)
    } else if lower.ends_with(".py") {
        extract_python_dependencies(content)
    } else {
        extract_js_ts_dependencies(path, content)
    }
}

fn extract_rust_dependencies(path: &str, content: &str) -> Vec<RawDep> {
    let mut out = Vec::new();
    let path_dir = Path::new(path).parent().map(|p| p.to_path_buf());
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("use crate::") {
            let candidate = rest
                .split(';')
                .next()
                .unwrap_or_default()
                .split(" as ")
                .next()
                .unwrap_or_default()
                .trim()
                .replace("::", "/");
            for suffix in [".rs", "/mod.rs"] {
                let target = format!("src/{}{}", candidate, suffix).replace("//", "/");
                out.push(RawDep {
                    target,
                    kind: "rust_use".to_string(),
                });
            }
        } else if let Some(rest) = trimmed.strip_prefix("mod ") {
            let module = rest
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            if let Some(dir) = &path_dir {
                let target = dir.join(format!("{module}.rs")).to_string_lossy().replace('\\', "/");
                out.push(RawDep {
                    target,
                    kind: "rust_mod".to_string(),
                });
            }
        }
    }
    out
}

fn extract_python_dependencies(content: &str) -> Vec<RawDep> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("from ") {
            if let Some((module, _)) = rest.split_once(" import ") {
                let target = format!("{}.py", module.replace('.', "/"));
                out.push(RawDep {
                    target,
                    kind: "python_from".to_string(),
                });
            }
        } else if let Some(module) = trimmed.strip_prefix("import ") {
            let top = module.split(',').next().unwrap_or_default().trim();
            if !top.is_empty() {
                let target = format!("{}.py", top.replace('.', "/"));
                out.push(RawDep {
                    target,
                    kind: "python_import".to_string(),
                });
            }
        }
    }
    out
}

fn extract_js_ts_dependencies(path: &str, content: &str) -> Vec<RawDep> {
    let mut out = Vec::new();
    let parent = Path::new(path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    for line in content.lines() {
        let trimmed = line.trim();
        let mut spec = None;
        if trimmed.starts_with("import ") && trimmed.contains(" from ") {
            spec = trimmed.split(" from ").nth(1);
        } else if let Some(rest) = trimmed.strip_prefix("import ") {
            spec = Some(rest);
        } else if trimmed.contains("require(") {
            spec = trimmed.split("require(").nth(1);
        }
        let Some(raw_spec) = spec else {
            continue;
        };
        let normalized = raw_spec
            .trim()
            .trim_matches(';')
            .trim_matches('"')
            .trim_matches('\'')
            .trim_matches(')')
            .trim();
        if !normalized.starts_with("./") && !normalized.starts_with("../") {
            continue;
        }
        let base = parent.join(normalized);
        for suffix in ["", ".ts", ".tsx", ".js", ".jsx", "/index.ts", "/index.js"] {
            let candidate = format!("{}{}", base.to_string_lossy(), suffix).replace('\\', "/");
            out.push(RawDep {
                target: candidate,
                kind: "js_import".to_string(),
            });
        }
    }
    out
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn repo_context_compiler_emits_tree_ranking_and_dependencies() {
        let root = env::temp_dir().join(format!("repo-context-compiler-{}", now_ms()));
        fs::create_dir_all(root.join("src/runtime")).expect("create dirs");
        fs::write(
            root.join("src/lib.rs"),
            "mod runtime;\nuse crate::runtime::modu;\n",
        )
        .expect("write lib");
        fs::write(root.join("src/runtime/mod.rs"), "pub mod modu;\n").expect("write runtime mod");
        fs::write(root.join("src/runtime/modu.rs"), "pub fn run() {}\n").expect("write modu");
        fs::write(root.join("README.md"), "# test\n").expect("write readme");

        let compiler = RepoContextCompiler::new(root.clone());
        let bundle = compiler
            .compile(
                "session:test",
                "trace:test",
                "implement runtime modu and update src lib",
            )
            .expect("compile bundle");

        assert!(!bundle.repo_tree.is_empty());
        assert!(!bundle.file_importance_ranking.is_empty());
        assert!(
            bundle
                .dependency_graph
                .iter()
                .any(|edge| edge.from == "src/lib.rs" && edge.to.contains("src/runtime"))
        );
        assert_eq!(bundle.api_version, REPO_CONTEXT_BUNDLE_CONTRACT_VERSION);
        assert_eq!(
            bundle.metadata.get("bundle_unified").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            bundle.metadata.get("bundle_sections").map(String::as_str),
            Some("repo_tree,file_importance_ranking,dependency_graph,recent_diff")
        );
        assert_eq!(bundle.repo_root.replace('\\', "/"), root.to_string_lossy().replace('\\', "/"));

        let _ = fs::remove_dir_all(root);
    }
}
