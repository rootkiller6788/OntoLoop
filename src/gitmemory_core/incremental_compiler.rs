use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;

use super::schema_registry::{SchemaRegistry, SchemaRegistrySnapshot};
use super::semantic_edges::{
    normalize_semantic_edges, InferenceCacheEntry, InferenceCheckpointRecord, SemanticEdge,
    EDGE_TYPE_EXTRACTED, EDGE_TYPE_INFERRED,
};

const MAX_RETRY_ATTEMPTS: usize = 3;
const SEMANTIC_EDGE_CACHE_FILE: &str = "edge_cache.json";
const SEMANTIC_CHECKPOINT_FILE: &str = "checkpoint.jsonl";
const SEMANTIC_MODEL_VERSION: &str = "semantic-v1";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompileErrorSeverity {
    Transient,
    Recoverable,
    Fatal,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompileErrorRecord {
    pub severity: CompileErrorSeverity,
    pub code: String,
    pub message: String,
    pub source_file: String,
    pub line: Option<usize>,
    pub retryable: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownBlockKind {
    Frontmatter,
    Heading,
    ListItem,
    Paragraph,
    CodeFence,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MarkdownBlock {
    pub block_id: String,
    pub kind: MarkdownBlockKind,
    pub start_line: usize,
    pub end_line: usize,
    pub text_preview: String,
    pub references: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CanonicalPlacement {
    pub rule_id: String,
    pub namespace: String,
    pub relative_path: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompiledFileRecord {
    pub source_file: String,
    pub source_digest: String,
    pub bytes: usize,
    pub projection_files: Vec<String>,
    pub block_count: usize,
    pub schema_kinds: Vec<String>,
    pub dependencies: Vec<String>,
    pub invalidated_dependents: Vec<String>,
    pub placement: CanonicalPlacement,
    pub errors: Vec<CompileErrorRecord>,
    pub attempts: usize,
    #[serde(default)]
    pub semantic_edges: Vec<SemanticEdge>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IncrementalCompileReport {
    pub changed_files: Vec<String>,
    pub expanded_targets: Vec<String>,
    pub compiled_files: Vec<CompiledFileRecord>,
    pub skipped_missing_files: Vec<String>,
    pub failed_files: Vec<String>,
    pub dependency_graph_ref: String,
    pub schema_registry_version: String,
    #[serde(default)]
    pub inference_cache_entries: Vec<InferenceCacheEntry>,
    #[serde(default)]
    pub inference_checkpoint_records: Vec<InferenceCheckpointRecord>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct DependencyGraph {
    pub edges: BTreeMap<String, Vec<String>>,
}

pub struct IncrementalCompiler;

impl IncrementalCompiler {
    pub fn rebuild_changed(
        repo_root: &Path,
        changed_files: &[String],
    ) -> Result<IncrementalCompileReport> {
        let projection_root = repo_root.join(".gitmemory").join("projections");
        let graph_root = projection_root.join("graph");
        let vector_root = projection_root.join("vector");
        let search_root = projection_root.join("search");
        let control_root = repo_root.join(".gitmemory");
        fs::create_dir_all(&graph_root)?;
        fs::create_dir_all(&vector_root)?;
        fs::create_dir_all(&search_root)?;
        fs::create_dir_all(&control_root)?;
        let schema_registry = SchemaRegistry::load(repo_root)?;

        let dependency_graph_path = control_root.join("dependency_graph.json");
        let mut dependency_graph = load_dependency_graph(&dependency_graph_path)?;
        let reverse_graph = build_reverse_graph(&dependency_graph);
        let expanded_targets = expand_invalidations(changed_files, &reverse_graph);

        let mut compiled_files = Vec::new();
        let mut skipped_missing_files = Vec::new();
        let mut failed_files = Vec::new();
        for file in &expanded_targets {
            let source_path = repo_root.join(file);
            if !source_path.exists() {
                skipped_missing_files.push(file.clone());
                dependency_graph.edges.remove(file);
                continue;
            }

            let content = fs::read_to_string(&source_path)?;
            let compile_result = compile_with_retry(file, &content, &schema_registry);
            if !compile_result.fatal_errors.is_empty() {
                failed_files.push(file.clone());
                continue;
            }
            dependency_graph
                .edges
                .insert(file.clone(), compile_result.dependencies.clone());

            let digest = source_digest_from_content(file, &content);
            let basename = sanitize(file);
            let graph_path = graph_root.join(format!("{basename}.json"));
            let vector_path = vector_root.join(format!("{basename}.json"));
            let search_path = search_root.join(format!("{basename}.json"));

            write_json(
                &graph_path,
                &serde_json::json!({
                    "source_file": file,
                    "source_digest": digest,
                    "block_count": compile_result.blocks.len(),
                    "schema": compile_result.blocks,
                    "dependencies": compile_result.dependencies,
                    "placement": compile_result.placement,
                }),
            )?;
            write_json(
                &vector_path,
                &serde_json::json!({
                    "source_file": file,
                    "source_digest": digest,
                    "chunk_count_estimate": content.lines().count().max(1),
                    "embedding_profile": "default",
                    "block_count": compile_result.block_count,
                }),
            )?;
            write_json(
                &search_path,
                &serde_json::json!({
                    "source_file": file,
                    "source_digest": digest,
                    "keywords": top_keywords(&content, 8),
                    "links": compile_result.dependencies,
                }),
            )?;

            let invalidated_dependents = reverse_graph
                .get(file)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            let schema_kinds = compile_result
                .blocks
                .iter()
                .map(|block| format!("{:?}", block.kind).to_ascii_lowercase())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let mut errors = compile_result.recoverable_errors;
            errors.extend(compile_result.transient_errors);

            compiled_files.push(CompiledFileRecord {
                source_file: file.clone(),
                source_digest: digest,
                bytes: content.len(),
                projection_files: vec![
                    to_display_path(&graph_path),
                    to_display_path(&vector_path),
                    to_display_path(&search_path),
                ],
                block_count: compile_result.block_count,
                schema_kinds,
                dependencies: compile_result.dependencies,
                invalidated_dependents,
                placement: compile_result.placement,
                errors,
                attempts: compile_result.attempts,
                semantic_edges: Vec::new(),
            });
        }

        let dependency_graph_ref = format!(
            "memory:compiler:dependency-graph:{}",
            crate::observability::event_stream::digest_value(&serde_json::json!({
                "edges": dependency_graph.edges,
            }))
        );
        write_json(
            &dependency_graph_path,
            &serde_json::to_value(&dependency_graph)?,
        )?;

        let semantic_root = control_root.join("semantic");
        fs::create_dir_all(&semantic_root)?;
        let edge_cache_path = semantic_root.join(SEMANTIC_EDGE_CACHE_FILE);
        let checkpoint_path = semantic_root.join(SEMANTIC_CHECKPOINT_FILE);

        let mut edge_cache = load_semantic_edge_cache(&edge_cache_path)?;
        let mut checkpoint_latest = load_semantic_checkpoint_latest(&checkpoint_path)?;
        let mut checkpoint_records = Vec::<InferenceCheckpointRecord>::new();
        run_semantic_inference_with_resume(
            repo_root,
            &mut compiled_files,
            &mut edge_cache,
            &mut checkpoint_latest,
            &checkpoint_path,
            &mut checkpoint_records,
        )?;
        save_semantic_edge_cache(&edge_cache_path, &edge_cache)?;

        let mut inference_cache_entries = edge_cache.into_values().collect::<Vec<_>>();
        inference_cache_entries.sort_by(|left, right| left.source_file.cmp(&right.source_file));
        checkpoint_records.sort_by(|left, right| {
            left.source_file
                .cmp(&right.source_file)
                .then_with(|| left.updated_at_ms.cmp(&right.updated_at_ms))
                .then_with(|| left.status.cmp(&right.status))
        });

        Ok(IncrementalCompileReport {
            changed_files: changed_files.to_vec(),
            expanded_targets,
            compiled_files,
            skipped_missing_files,
            failed_files,
            dependency_graph_ref,
            schema_registry_version: schema_registry.version,
            inference_cache_entries,
            inference_checkpoint_records: checkpoint_records,
        })
    }
}

pub fn source_digest(repo_root: &Path, source_file: &str) -> Result<Option<String>> {
    let source_path = repo_root.join(source_file);
    if !source_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(source_path)?;
    Ok(Some(source_digest_from_content(source_file, &content)))
}

pub fn source_digest_from_content(source_file: &str, content: &str) -> String {
    crate::observability::event_stream::digest_value(&serde_json::json!({
        "path": source_file,
        "content": content,
    }))
}

fn run_semantic_inference_with_resume(
    repo_root: &Path,
    compiled_files: &mut [CompiledFileRecord],
    edge_cache: &mut BTreeMap<String, InferenceCacheEntry>,
    checkpoint_latest: &mut BTreeMap<String, InferenceCheckpointRecord>,
    checkpoint_path: &Path,
    out_records: &mut Vec<InferenceCheckpointRecord>,
) -> Result<()> {
    for compiled in compiled_files.iter_mut() {
        let source_file = compiled.source_file.clone();
        let source_digest = compiled.source_digest.clone();
        let source_path = repo_root.join(&source_file);
        let source_content = fs::read_to_string(&source_path).unwrap_or_default();

        let reused = checkpoint_latest
            .get(&source_file)
            .map(|record| {
                record.status.eq_ignore_ascii_case("completed")
                    && record.source_digest == source_digest
            })
            .unwrap_or(false);
        if reused {
            let reused_edges = edge_cache
                .get(&source_file)
                .filter(|entry| entry.source_digest == source_digest)
                .map(|entry| normalize_semantic_edges(entry.edges.clone()))
                .unwrap_or_default();
            compiled.semantic_edges = reused_edges.clone();
            let reused_record = InferenceCheckpointRecord {
                checkpoint_id: format!(
                    "semantic-ckpt:{}:{}",
                    sanitize(&source_file),
                    current_time_ms()
                ),
                source_file: source_file.clone(),
                source_digest: source_digest.clone(),
                status: "recovered".to_string(),
                created_at_ms: current_time_ms(),
                updated_at_ms: current_time_ms(),
                error: None,
                edges: reused_edges,
            };
            append_semantic_checkpoint_record(checkpoint_path, &reused_record)?;
            checkpoint_latest.insert(source_file.clone(), reused_record.clone());
            out_records.push(reused_record);
            continue;
        }

        match infer_semantic_edges(compiled, &source_content) {
            Ok(edges) => {
                compiled.semantic_edges = edges.clone();
                let now = current_time_ms();
                edge_cache.insert(
                    source_file.clone(),
                    InferenceCacheEntry {
                        source_file: source_file.clone(),
                        source_digest: source_digest.clone(),
                        model: SEMANTIC_MODEL_VERSION.to_string(),
                        inferred_at_ms: now,
                        edges: edges.clone(),
                    },
                );
                let completed = InferenceCheckpointRecord {
                    checkpoint_id: format!("semantic-ckpt:{}:{now}", sanitize(&source_file)),
                    source_file: source_file.clone(),
                    source_digest: source_digest.clone(),
                    status: "completed".to_string(),
                    created_at_ms: now,
                    updated_at_ms: now,
                    error: None,
                    edges,
                };
                append_semantic_checkpoint_record(checkpoint_path, &completed)?;
                checkpoint_latest.insert(source_file.clone(), completed.clone());
                out_records.push(completed);
            }
            Err(error) => {
                compiled.semantic_edges = Vec::new();
                let now = current_time_ms();
                let failed = InferenceCheckpointRecord {
                    checkpoint_id: format!("semantic-ckpt:{}:{now}", sanitize(&source_file)),
                    source_file: source_file.clone(),
                    source_digest: source_digest.clone(),
                    status: "failed".to_string(),
                    created_at_ms: now,
                    updated_at_ms: now,
                    error: Some(error.to_string()),
                    edges: Vec::new(),
                };
                append_semantic_checkpoint_record(checkpoint_path, &failed)?;
                checkpoint_latest.insert(source_file.clone(), failed.clone());
                out_records.push(failed);
            }
        }
    }
    Ok(())
}

fn infer_semantic_edges(
    compiled: &CompiledFileRecord,
    source_content: &str,
) -> Result<Vec<SemanticEdge>> {
    if source_content.contains("<!-- semantic:fail -->") {
        anyhow::bail!("semantic inference marker requested failure");
    }

    let mut edges = compiled
        .dependencies
        .iter()
        .filter(|dep| !dep.starts_with("http://") && !dep.starts_with("https://"))
        .map(|dep| SemanticEdge {
            from: compiled.source_file.clone(),
            to: dep.clone(),
            relation: "references".to_string(),
            confidence: if dep.ends_with(".md") { 1.0 } else { 0.75 },
            edge_type: if dep.ends_with(".md") {
                EDGE_TYPE_EXTRACTED.to_string()
            } else {
                EDGE_TYPE_INFERRED.to_string()
            },
        })
        .collect::<Vec<_>>();
    edges = normalize_semantic_edges(edges);
    Ok(edges)
}

fn load_semantic_edge_cache(path: &Path) -> Result<BTreeMap<String, InferenceCacheEntry>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(path)?;
    let entries = serde_json::from_str::<Vec<InferenceCacheEntry>>(&raw).unwrap_or_default();
    Ok(entries
        .into_iter()
        .map(|entry| (entry.source_file.clone(), entry))
        .collect::<BTreeMap<_, _>>())
}

fn save_semantic_edge_cache(
    path: &Path,
    cache: &BTreeMap<String, InferenceCacheEntry>,
) -> Result<()> {
    let mut entries = cache.values().cloned().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.source_file.cmp(&right.source_file));
    write_json(path, &serde_json::to_value(entries)?)?;
    Ok(())
}

fn load_semantic_checkpoint_latest(
    path: &Path,
) -> Result<BTreeMap<String, InferenceCheckpointRecord>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(path)?;
    let mut latest = BTreeMap::<String, InferenceCheckpointRecord>::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<InferenceCheckpointRecord>(line) else {
            continue;
        };
        match latest.get(&record.source_file) {
            Some(existing)
                if existing.updated_at_ms > record.updated_at_ms
                    || (existing.updated_at_ms == record.updated_at_ms
                        && existing.checkpoint_id >= record.checkpoint_id) => {}
            _ => {
                latest.insert(record.source_file.clone(), record);
            }
        }
    }
    Ok(latest)
}

fn append_semantic_checkpoint_record(
    path: &Path,
    record: &InferenceCheckpointRecord,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(record)?;
    let mut content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    content.push_str(&line);
    content.push('\n');
    fs::write(path, content)?;
    Ok(())
}

#[derive(Debug)]
struct CompileOutcome {
    blocks: Vec<MarkdownBlock>,
    dependencies: Vec<String>,
    placement: CanonicalPlacement,
    block_count: usize,
    attempts: usize,
    transient_errors: Vec<CompileErrorRecord>,
    recoverable_errors: Vec<CompileErrorRecord>,
    fatal_errors: Vec<CompileErrorRecord>,
}

fn compile_with_retry(
    file: &str,
    content: &str,
    registry: &SchemaRegistrySnapshot,
) -> CompileOutcome {
    let mut attempts = 0;
    let mut transient_errors = Vec::new();
    loop {
        attempts += 1;
        let pass = compile_once(file, content, attempts, registry);
        transient_errors.extend(pass.transient_errors.clone());
        if !pass.transient_errors.is_empty() && attempts < MAX_RETRY_ATTEMPTS {
            continue;
        }
        let block_count = pass.blocks.len();
        return CompileOutcome {
            blocks: pass.blocks,
            dependencies: pass.dependencies,
            placement: pass.placement,
            block_count,
            attempts,
            transient_errors,
            recoverable_errors: pass.recoverable_errors,
            fatal_errors: pass.fatal_errors,
        };
    }
}

#[derive(Debug)]
struct CompilePass {
    blocks: Vec<MarkdownBlock>,
    dependencies: Vec<String>,
    placement: CanonicalPlacement,
    transient_errors: Vec<CompileErrorRecord>,
    recoverable_errors: Vec<CompileErrorRecord>,
    fatal_errors: Vec<CompileErrorRecord>,
}

fn compile_once(
    file: &str,
    content: &str,
    attempt: usize,
    registry: &SchemaRegistrySnapshot,
) -> CompilePass {
    let mut transient_errors = Vec::new();
    let mut recoverable_errors = Vec::new();
    let mut fatal_errors = Vec::new();

    if content.contains("<!-- transient:retry -->") && attempt < 2 {
        transient_errors.push(CompileErrorRecord {
            severity: CompileErrorSeverity::Transient,
            code: "TRANSIENT_INPUT_UNSTABLE".to_string(),
            message: "transient marker detected; retry requested".to_string(),
            source_file: file.to_string(),
            line: None,
            retryable: true,
        });
    }

    let mut namespace = None::<String>;
    let mut blocks = Vec::<MarkdownBlock>::new();
    let mut dependencies = BTreeSet::<String>::new();
    let lines = content.lines().collect::<Vec<_>>();

    let mut line_idx = 0usize;
    let mut in_code_fence = false;
    let mut code_start = 0usize;
    let mut last_heading_level = 0usize;

    if lines.first().map(|line| line.trim()) == Some("---") {
        let mut end_idx = None;
        for (idx, line) in lines.iter().enumerate().skip(1) {
            if line.trim() == "---" {
                end_idx = Some(idx);
                break;
            }
        }
        if let Some(end) = end_idx {
            let frontmatter = lines[1..end].join("\n");
            for row in frontmatter.lines() {
                let trimmed = row.trim();
                if let Some(value) = trimmed.strip_prefix("namespace:") {
                    namespace = Some(value.trim().to_string());
                }
            }
            blocks.push(MarkdownBlock {
                block_id: format!("{}:frontmatter", sanitize(file)),
                kind: MarkdownBlockKind::Frontmatter,
                start_line: 1,
                end_line: end + 1,
                text_preview: preview(&frontmatter),
                references: Vec::new(),
            });
            line_idx = end + 1;
        } else {
            fatal_errors.push(CompileErrorRecord {
                severity: CompileErrorSeverity::Fatal,
                code: "FRONTMATTER_UNCLOSED".to_string(),
                message: "frontmatter opened but not closed".to_string(),
                source_file: file.to_string(),
                line: Some(1),
                retryable: false,
            });
        }
    }

    let mut paragraph_start: Option<usize> = None;
    let mut paragraph_buf = String::new();
    while line_idx < lines.len() {
        let line = lines[line_idx];
        let trimmed = line.trim();
        let line_no = line_idx + 1;

        if trimmed.starts_with("```") {
            flush_paragraph(
                file,
                &mut blocks,
                &mut dependencies,
                &mut paragraph_start,
                &mut paragraph_buf,
                line_no.saturating_sub(1),
            );
            if in_code_fence {
                in_code_fence = false;
                blocks.push(MarkdownBlock {
                    block_id: format!("{}:code:{}", sanitize(file), code_start),
                    kind: MarkdownBlockKind::CodeFence,
                    start_line: code_start,
                    end_line: line_no,
                    text_preview: "code-fence".to_string(),
                    references: Vec::new(),
                });
            } else {
                in_code_fence = true;
                code_start = line_no;
            }
            line_idx += 1;
            continue;
        }

        if in_code_fence {
            line_idx += 1;
            continue;
        }

        if trimmed.starts_with('#') {
            flush_paragraph(
                file,
                &mut blocks,
                &mut dependencies,
                &mut paragraph_start,
                &mut paragraph_buf,
                line_no.saturating_sub(1),
            );
            let level = trimmed.chars().take_while(|ch| *ch == '#').count();
            if last_heading_level > 0 && level > last_heading_level + 1 {
                recoverable_errors.push(CompileErrorRecord {
                    severity: CompileErrorSeverity::Recoverable,
                    code: "HEADING_LEVEL_JUMP".to_string(),
                    message: format!(
                        "heading level jumped from h{} to h{}",
                        last_heading_level, level
                    ),
                    source_file: file.to_string(),
                    line: Some(line_no),
                    retryable: false,
                });
            }
            last_heading_level = level;
            let refs = extract_refs(trimmed, file);
            for item in &refs {
                dependencies.insert(item.clone());
            }
            blocks.push(MarkdownBlock {
                block_id: format!("{}:h:{}:{}", sanitize(file), level, line_no),
                kind: MarkdownBlockKind::Heading,
                start_line: line_no,
                end_line: line_no,
                text_preview: preview(trimmed),
                references: refs,
            });
            line_idx += 1;
            continue;
        }

        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            flush_paragraph(
                file,
                &mut blocks,
                &mut dependencies,
                &mut paragraph_start,
                &mut paragraph_buf,
                line_no.saturating_sub(1),
            );
            let refs = extract_refs(trimmed, file);
            for item in &refs {
                dependencies.insert(item.clone());
            }
            blocks.push(MarkdownBlock {
                block_id: format!("{}:li:{}", sanitize(file), line_no),
                kind: MarkdownBlockKind::ListItem,
                start_line: line_no,
                end_line: line_no,
                text_preview: preview(trimmed),
                references: refs,
            });
            line_idx += 1;
            continue;
        }

        if trimmed.is_empty() {
            flush_paragraph(
                file,
                &mut blocks,
                &mut dependencies,
                &mut paragraph_start,
                &mut paragraph_buf,
                line_no.saturating_sub(1),
            );
            line_idx += 1;
            continue;
        }

        if paragraph_start.is_none() {
            paragraph_start = Some(line_no);
        }
        if !paragraph_buf.is_empty() {
            paragraph_buf.push('\n');
        }
        paragraph_buf.push_str(trimmed);
        line_idx += 1;
    }

    flush_paragraph(
        file,
        &mut blocks,
        &mut dependencies,
        &mut paragraph_start,
        &mut paragraph_buf,
        lines.len(),
    );

    if in_code_fence {
        fatal_errors.push(CompileErrorRecord {
            severity: CompileErrorSeverity::Fatal,
            code: "CODE_FENCE_UNCLOSED".to_string(),
            message: "code fence is not closed".to_string(),
            source_file: file.to_string(),
            line: Some(code_start),
            retryable: false,
        });
    }

    let placement = placement_for(file, namespace.as_deref(), registry);
    recoverable_errors.extend(enforce_schema_rules(file, &blocks, registry));
    CompilePass {
        blocks,
        dependencies: dependencies.into_iter().collect(),
        placement,
        transient_errors,
        recoverable_errors,
        fatal_errors,
    }
}

fn flush_paragraph(
    file: &str,
    blocks: &mut Vec<MarkdownBlock>,
    dependencies: &mut BTreeSet<String>,
    paragraph_start: &mut Option<usize>,
    paragraph_buf: &mut String,
    end_line: usize,
) {
    let Some(start) = *paragraph_start else {
        return;
    };
    let text = paragraph_buf.trim().to_string();
    if text.is_empty() {
        *paragraph_start = None;
        paragraph_buf.clear();
        return;
    }
    let refs = extract_refs(&text, file);
    for item in &refs {
        dependencies.insert(item.clone());
    }
    blocks.push(MarkdownBlock {
        block_id: format!("{}:p:{}", sanitize(file), start),
        kind: MarkdownBlockKind::Paragraph,
        start_line: start,
        end_line: end_line.max(start),
        text_preview: preview(&text),
        references: refs,
    });
    *paragraph_start = None;
    paragraph_buf.clear();
}

fn placement_for(
    source_file: &str,
    namespace: Option<&str>,
    registry: &SchemaRegistrySnapshot,
) -> CanonicalPlacement {
    let already_canonical_rule = registry
        .placement_rules
        .iter()
        .find(|rule| rule.id == "already-canonical")
        .map(|rule| rule.id.clone())
        .unwrap_or_else(|| "already_canonical".to_string());
    let default_rule = registry
        .placement_rules
        .iter()
        .find(|rule| rule.id == "namespace-path-v1")
        .map(|rule| rule.id.clone())
        .unwrap_or_else(|| "namespace_path_v1".to_string());
    if source_file.to_ascii_lowercase().starts_with("canonical/") {
        return CanonicalPlacement {
            rule_id: already_canonical_rule,
            namespace: namespace.unwrap_or("default").to_string(),
            relative_path: source_file.to_string(),
        };
    }
    let ns = namespace
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| source_file.split('/').next().unwrap_or("default"));
    let stem = source_file
        .trim_end_matches(".md")
        .trim_end_matches(".markdown")
        .to_string();
    CanonicalPlacement {
        rule_id: default_rule,
        namespace: sanitize(ns),
        relative_path: format!("canonical/{}/{}.md", sanitize(ns), sanitize(&stem)),
    }
}

fn enforce_schema_rules(
    file: &str,
    blocks: &[MarkdownBlock],
    registry: &SchemaRegistrySnapshot,
) -> Vec<CompileErrorRecord> {
    let mut errors = Vec::new();
    let mut by_kind = BTreeMap::<String, usize>::new();
    for block in blocks {
        let kind = format!("{:?}", block.kind).to_ascii_lowercase();
        *by_kind.entry(kind).or_insert(0) += 1;
        let text_len = block.text_preview.len();
        if let Some(rule) = registry.block_rules.iter().find(|rule| {
            rule.kind
                .eq_ignore_ascii_case(&format!("{:?}", block.kind).to_ascii_lowercase())
        }) {
            if text_len > rule.max_bytes {
                errors.push(CompileErrorRecord {
                    severity: CompileErrorSeverity::Recoverable,
                    code: "BLOCK_EXCEEDS_MAX_BYTES".to_string(),
                    message: format!("block '{}' exceeds max_bytes {}", rule.kind, rule.max_bytes),
                    source_file: file.to_string(),
                    line: Some(block.start_line),
                    retryable: false,
                });
            }
        }
    }

    for rule in &registry.block_rules {
        if rule.required && by_kind.get(&rule.kind).copied().unwrap_or(0) == 0 {
            errors.push(CompileErrorRecord {
                severity: CompileErrorSeverity::Recoverable,
                code: "REQUIRED_BLOCK_MISSING".to_string(),
                message: format!("required block kind '{}' is missing", rule.kind),
                source_file: file.to_string(),
                line: None,
                retryable: false,
            });
        }
    }
    errors
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

fn expand_invalidations(
    changed_files: &[String],
    reverse_graph: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let mut visited = BTreeSet::<String>::new();
    let mut queue = VecDeque::<String>::new();
    for file in changed_files {
        queue.push_back(file.clone());
    }
    while let Some(file) = queue.pop_front() {
        if !visited.insert(file.clone()) {
            continue;
        }
        if let Some(dependents) = reverse_graph.get(&file) {
            for dep in dependents {
                if !visited.contains(dep) {
                    queue.push_back(dep.clone());
                }
            }
        }
    }
    visited.into_iter().collect()
}

fn build_reverse_graph(graph: &DependencyGraph) -> BTreeMap<String, BTreeSet<String>> {
    let mut reverse = BTreeMap::<String, BTreeSet<String>>::new();
    for (source, deps) in &graph.edges {
        for dep in deps {
            reverse
                .entry(dep.clone())
                .or_default()
                .insert(source.clone());
        }
    }
    reverse
}

fn load_dependency_graph(path: &Path) -> Result<DependencyGraph> {
    if !path.exists() {
        return Ok(DependencyGraph::default());
    }
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str::<DependencyGraph>(&raw).unwrap_or_default())
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
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

fn preview(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= 120 {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..120])
}

fn to_display_path(path: &PathBuf) -> String {
    path.display().to_string()
}

fn top_keywords(content: &str, limit: usize) -> Vec<String> {
    let mut freq = std::collections::BTreeMap::<String, usize>::new();
    for token in content
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() >= 3)
    {
        *freq.entry(token).or_insert(0) += 1;
    }
    let mut ranked = freq.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(token, _)| token)
        .collect()
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{CompileErrorSeverity, IncrementalCompileReport, IncrementalCompiler};

    #[test]
    fn compiler_retries_transient_and_surfaces_schema() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-compiler-transient-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(temp.join("docs")).expect("mkdir");
        std::fs::write(
            temp.join("docs").join("a.md"),
            "---\nnamespace: product\n---\n# A\n<!-- transient:retry -->\nParagraph with [[docs/b]].\n",
        )
        .expect("write");
        let report = IncrementalCompiler::rebuild_changed(&temp, &["docs/a.md".to_string()])
            .expect("compile");
        assert_eq!(report.compiled_files.len(), 1);
        let file = &report.compiled_files[0];
        assert!(file.attempts >= 2);
        assert!(file.block_count >= 2);
        assert!(
            file.dependencies
                .iter()
                .any(|item| item.contains("docs/b.md"))
        );
        assert_eq!(file.placement.namespace, "product");
        assert!(
            file.errors
                .iter()
                .any(|err| err.severity == CompileErrorSeverity::Transient)
        );
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn compiler_invalidates_cross_file_dependents() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-compiler-deps-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(temp.join("docs")).expect("mkdir");
        std::fs::write(temp.join("docs").join("a.md"), "# A\n").expect("write a");
        std::fs::write(
            temp.join("docs").join("b.md"),
            "# B\nLinks to [[docs/a]].\n",
        )
        .expect("write b");

        let first = IncrementalCompiler::rebuild_changed(
            &temp,
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
        )
        .expect("first");
        assert_eq!(first.compiled_files.len(), 2);

        std::fs::write(temp.join("docs").join("a.md"), "# A\nUpdated.\n").expect("update a");
        let second = IncrementalCompiler::rebuild_changed(&temp, &["docs/a.md".to_string()])
            .expect("second");
        assert!(
            second
                .expanded_targets
                .iter()
                .any(|item| item == "docs/b.md"),
            "dependent file should be invalidated by reverse dependency graph"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn compile_report_deserializes_legacy_payload_without_semantic_fields() {
        let legacy = serde_json::json!({
            "changed_files": ["memory/MEMORY.md"],
            "expanded_targets": ["memory/MEMORY.md"],
            "compiled_files": [{
                "source_file": "memory/MEMORY.md",
                "source_digest": "digest",
                "bytes": 10,
                "projection_files": ["a", "b", "c"],
                "block_count": 1,
                "schema_kinds": ["heading"],
                "dependencies": [],
                "invalidated_dependents": [],
                "placement": {
                    "rule_id": "namespace-path-v1",
                    "namespace": "memory",
                    "relative_path": "canonical/memory/memory_memory_md.md"
                },
                "errors": [],
                "attempts": 1
            }],
            "skipped_missing_files": [],
            "failed_files": [],
            "dependency_graph_ref": "memory:compiler:dependency-graph:legacy",
            "schema_registry_version": "v1"
        });

        let parsed: IncrementalCompileReport =
            serde_json::from_value(legacy).expect("legacy report should deserialize");
        assert_eq!(parsed.compiled_files.len(), 1);
        assert!(parsed.compiled_files[0].semantic_edges.is_empty());
        assert!(parsed.inference_cache_entries.is_empty());
        assert!(parsed.inference_checkpoint_records.is_empty());
    }

    #[test]
    fn semantic_resume_only_backfills_unfinished_sources() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-semantic-resume-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(temp.join("docs")).expect("mkdir");
        std::fs::write(
            temp.join("docs").join("a.md"),
            "# A\n[[docs/b]]\n",
        )
        .expect("write a");
        std::fs::write(
            temp.join("docs").join("b.md"),
            "# B\n<!-- semantic:fail -->\n[[docs/a]]\n",
        )
        .expect("write b");

        let first = IncrementalCompiler::rebuild_changed(
            &temp,
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
        )
        .expect("first run");
        assert_eq!(first.compiled_files.len(), 2);
        let first_status = first
            .inference_checkpoint_records
            .iter()
            .map(|item| (item.source_file.as_str(), item.status.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(first_status.get("docs/a.md"), Some(&"completed"));
        assert_eq!(first_status.get("docs/b.md"), Some(&"failed"));

        std::fs::write(temp.join("docs").join("b.md"), "# B\n[[docs/a]]\n").expect("fix b");
        let second = IncrementalCompiler::rebuild_changed(
            &temp,
            &["docs/a.md".to_string(), "docs/b.md".to_string()],
        )
        .expect("second run");
        let second_status = second
            .inference_checkpoint_records
            .iter()
            .map(|item| (item.source_file.as_str(), item.status.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(second_status.get("docs/a.md"), Some(&"recovered"));
        assert_eq!(second_status.get("docs/b.md"), Some(&"completed"));
        assert!(
            second
                .compiled_files
                .iter()
                .find(|item| item.source_file == "docs/a.md")
                .is_some_and(|item| !item.semantic_edges.is_empty())
        );
        assert!(
            second
                .compiled_files
                .iter()
                .find(|item| item.source_file == "docs/b.md")
                .is_some_and(|item| !item.semantic_edges.is_empty())
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn semantic_edge_sort_and_dedup_is_stable() {
        let temp = std::env::temp_dir().join(format!(
            "autoloop-semantic-stability-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(temp.join("docs")).expect("mkdir");
        std::fs::write(
            temp.join("docs").join("a.md"),
            "# A\n[[docs/b]]\n[[docs/b]]\n[link](docs/c)\n",
        )
        .expect("write");
        std::fs::write(temp.join("docs").join("b.md"), "# B\n").expect("write b");
        std::fs::write(temp.join("docs").join("c.md"), "# C\n").expect("write c");

        let first =
            IncrementalCompiler::rebuild_changed(&temp, &["docs/a.md".to_string()]).expect("first");
        let second =
            IncrementalCompiler::rebuild_changed(&temp, &["docs/a.md".to_string()]).expect("second");

        let edges_first = first.compiled_files[0].semantic_edges.clone();
        let edges_second = second.compiled_files[0].semantic_edges.clone();
        assert_eq!(edges_first, edges_second, "semantic edge ordering should be stable");
        assert!(
            edges_first.windows(2).all(|pair| {
                let lhs = (
                    pair[0].from.as_str(),
                    pair[0].to.as_str(),
                    pair[0].relation.as_str(),
                    pair[0].edge_type.as_str(),
                );
                let rhs = (
                    pair[1].from.as_str(),
                    pair[1].to.as_str(),
                    pair[1].relation.as_str(),
                    pair[1].edge_type.as_str(),
                );
                lhs <= rhs
            })
        );

        let _ = std::fs::remove_dir_all(&temp);
    }
}
