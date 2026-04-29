use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow, bail};
use autoloop_state_adapter::StateStore;

use crate::contracts::code_harness::{PatchOp, PatchOpKind};
use crate::evolution_os::replay;
use crate::runtime::evidence_ledger::{EvidenceLedgerWriter, EvidenceStage};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchStepStatus {
    Applied,
    Reverted,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PatchStepRecord {
    pub op_id: String,
    pub kind: String,
    pub path: String,
    pub status: PatchStepStatus,
    #[serde(default)]
    pub before_hash: Option<String>,
    #[serde(default)]
    pub after_hash: Option<String>,
    #[serde(default)]
    pub diff_ref: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PatchExecutionReport {
    pub session_id: String,
    pub trace_id: String,
    pub success: bool,
    #[serde(default = "default_false")]
    pub transaction_committed: bool,
    pub rollback_performed: bool,
    #[serde(default = "default_true")]
    pub rollback_successful: bool,
    pub steps: Vec<PatchStepRecord>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub replay_fp: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

fn default_false() -> bool {
    false
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PatchSnapshotRecord {
    pub op_id: String,
    pub path: String,
    pub existed_before: bool,
    #[serde(default)]
    pub content_before: Option<Vec<u8>>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone)]
enum UndoAction {
    DeleteCreated { path: PathBuf },
    RestoreFile { path: PathBuf, content: Vec<u8> },
    RestoreDeleted { path: PathBuf, content: Vec<u8> },
    MoveBack { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PatchConflict {
    op_id: String,
    path: String,
    reason: String,
}

pub struct DiffPatchEngine {
    workspace_root: PathBuf,
}

impl DiffPatchEngine {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub async fn apply_with_rollback(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        ops: &[PatchOp],
    ) -> Result<PatchExecutionReport> {
        let conflicts = self.detect_conflicts(ops)?;
        if !conflicts.is_empty() {
            let steps = conflicts
                .iter()
                .enumerate()
                .map(|(idx, conflict)| PatchStepRecord {
                    op_id: if conflict.op_id.is_empty() {
                        format!("preflight-conflict-{idx}")
                    } else {
                        conflict.op_id.clone()
                    },
                    kind: "preflight_conflict".to_string(),
                    path: conflict.path.clone(),
                    status: PatchStepStatus::Failed,
                    before_hash: None,
                    after_hash: None,
                    diff_ref: None,
                    error: Some(conflict.reason.clone()),
                })
                .collect::<Vec<_>>();
            let replay_fp = self.build_report_replay_fp(trace_id, &steps, false);
            let evidence_ref = EvidenceLedgerWriter::append_stage(
                db,
                session_id,
                trace_id,
                EvidenceStage::Execution,
                serde_json::json!({
                    "stage": "diff_patch_engine",
                    "status": "preflight_conflict",
                    "conflict_count": steps.len(),
                    "replay_fp": replay_fp,
                    "conflicts": conflicts,
                }),
                None,
            )
            .await
            .ok();
            return Ok(PatchExecutionReport {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                success: false,
                transaction_committed: false,
                rollback_performed: false,
                rollback_successful: true,
                steps,
                evidence_ref,
                replay_fp: Some(replay_fp),
                message: Some("patch preflight conflict detected".to_string()),
            });
        }

        let mut steps = Vec::new();
        let mut undo = Vec::<UndoAction>::new();
        let mut rollback_performed = false;
        let mut rollback_successful = true;

        for (idx, op) in ops.iter().enumerate() {
            let step = self.apply_one(db, session_id, trace_id, idx, op).await;
            match step {
                Ok((record, undo_action)) => {
                    if let Some(action) = undo_action {
                        undo.push(action);
                    }
                    steps.push(record);
                }
                Err(error) => {
                    steps.push(PatchStepRecord {
                        op_id: normalize_op_id(op, idx),
                        kind: format!("{:?}", op.kind).to_ascii_lowercase(),
                        path: op.path.clone(),
                        status: PatchStepStatus::Failed,
                        before_hash: None,
                        after_hash: None,
                        diff_ref: None,
                        error: Some(error.to_string()),
                    });
                    rollback_performed = true;
                    for (rollback_idx, action) in undo.into_iter().rev().enumerate() {
                        match self.rollback_action(action) {
                            Ok(rollback_step) => steps.push(rollback_step),
                            Err(rollback_err) => {
                                rollback_successful = false;
                                steps.push(PatchStepRecord {
                                    op_id: format!("rollback-{rollback_idx}"),
                                    kind: "rollback".to_string(),
                                    path: "<unknown>".to_string(),
                                    status: PatchStepStatus::Failed,
                                    before_hash: None,
                                    after_hash: None,
                                    diff_ref: None,
                                    error: Some(rollback_err.to_string()),
                                });
                            }
                        }
                    }
                    let replay_fp = self.build_report_replay_fp(trace_id, &steps, true);
                    let evidence_ref = EvidenceLedgerWriter::append_stage(
                        db,
                        session_id,
                        trace_id,
                        EvidenceStage::Execution,
                        serde_json::json!({
                            "stage": "diff_patch_engine",
                            "status": "failed_rolled_back",
                            "step_count": steps.len(),
                            "rollback_successful": rollback_successful,
                            "replay_fp": replay_fp,
                        }),
                        None,
                    )
                    .await
                    .ok();
                    return Ok(PatchExecutionReport {
                        session_id: session_id.to_string(),
                        trace_id: trace_id.to_string(),
                        success: false,
                        transaction_committed: false,
                        rollback_performed,
                        rollback_successful,
                        steps,
                        evidence_ref,
                        replay_fp: Some(replay_fp),
                        message: Some(if rollback_successful {
                            "patch apply failed and rollback executed".to_string()
                        } else {
                            "patch apply failed and rollback partially failed".to_string()
                        }),
                    });
                }
            }
        }

        let replay_fp = self.build_report_replay_fp(trace_id, &steps, false);
        let evidence_ref = EvidenceLedgerWriter::append_stage(
            db,
            session_id,
            trace_id,
            EvidenceStage::Execution,
            serde_json::json!({
                "stage": "diff_patch_engine",
                "status": "applied",
                "step_count": steps.len(),
                "replay_fp": replay_fp,
            }),
            None,
        )
        .await
        .ok();
        Ok(PatchExecutionReport {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            success: true,
            transaction_committed: true,
            rollback_performed,
            rollback_successful,
            steps,
            evidence_ref,
            replay_fp: Some(replay_fp),
            message: Some("patch applied".to_string()),
        })
    }

    pub async fn revert_from_trace(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
    ) -> Result<PatchExecutionReport> {
        let prefix = format!("harness:patch:snapshot:{session_id}:{trace_id}:");
        let mut snapshots = db
            .list_knowledge_by_prefix(&prefix)
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<PatchSnapshotRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        snapshots.sort_by_key(|item| item.created_at_ms);

        let mut steps = Vec::new();
        for snapshot in snapshots.into_iter().rev() {
            let target = self.resolve_path(&snapshot.path)?;
            let result = if snapshot.existed_before {
                let bytes = snapshot.content_before.unwrap_or_default();
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("create parent directories for revert {}", parent.display())
                    })?;
                }
                fs::write(&target, &bytes)
                    .with_context(|| format!("restore file during revert {}", target.display()))
            } else if target.exists() {
                fs::remove_file(&target)
                    .with_context(|| format!("remove created file during revert {}", target.display()))
            } else {
                Ok(())
            };

            let status = if result.is_ok() {
                PatchStepStatus::Reverted
            } else {
                PatchStepStatus::Failed
            };
            steps.push(PatchStepRecord {
                op_id: snapshot.op_id.clone(),
                kind: "revert".to_string(),
                path: snapshot.path.clone(),
                status,
                before_hash: None,
                after_hash: hash_file_if_exists(&target),
                diff_ref: None,
                error: result.err().map(|err| err.to_string()),
            });
        }

        let success = steps.iter().all(|step| !matches!(step.status, PatchStepStatus::Failed));
        let replay_fp = self.build_report_replay_fp(trace_id, &steps, !success);
        let evidence_ref = EvidenceLedgerWriter::append_stage(
            db,
            session_id,
            trace_id,
            EvidenceStage::Execution,
            serde_json::json!({
                "stage": "diff_patch_engine_revert",
                "status": if success { "reverted" } else { "revert_failed" },
                "step_count": steps.len(),
                "replay_fp": replay_fp,
            }),
            None,
        )
        .await
        .ok();
        Ok(PatchExecutionReport {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            success,
            transaction_committed: false,
            rollback_performed: true,
            rollback_successful: success,
            steps,
            evidence_ref,
            replay_fp: Some(replay_fp),
            message: Some("revert executed from trace snapshots".to_string()),
        })
    }

    async fn apply_one(
        &self,
        db: &StateStore,
        session_id: &str,
        trace_id: &str,
        idx: usize,
        op: &PatchOp,
    ) -> Result<(PatchStepRecord, Option<UndoAction>)> {
        let op_id = normalize_op_id(op, idx);
        let target = self.resolve_path(&op.path)?;
        let before_hash = hash_file_if_exists(&target);
        let existed_before = target.exists();
        let previous_bytes = if existed_before {
            Some(fs::read(&target).with_context(|| format!("read previous file {}", target.display()))?)
        } else {
            None
        };

        let undo = match op.kind {
            PatchOpKind::CreateFile => UndoAction::DeleteCreated {
                path: target.clone(),
            },
            PatchOpKind::UpdateFile => UndoAction::RestoreFile {
                path: target.clone(),
                content: previous_bytes.clone().unwrap_or_default(),
            },
            PatchOpKind::DeleteFile => UndoAction::RestoreDeleted {
                path: target.clone(),
                content: previous_bytes.clone().unwrap_or_default(),
            },
            PatchOpKind::MoveFile => {
                let from = op
                    .from_path
                    .as_deref()
                    .ok_or_else(|| anyhow!("move_file requires from_path"))?;
                let src = self.resolve_path(from)?;
                UndoAction::MoveBack {
                    from: target.clone(),
                    to: src,
                }
            }
            PatchOpKind::Revert => {
                return Err(anyhow!(
                    "PatchOpKind::Revert should be performed via revert_from_trace()"
                ))
            }
        };

        self.apply_patch_op(op, &target)?;
        let after_hash = hash_file_if_exists(&target);
        if let Some(expected) = op.expected_hash.as_ref() {
            if let Some(actual) = after_hash.as_ref() {
                if actual != expected {
                    bail!(
                        "expected hash mismatch for {} expected={} actual={}",
                        target.display(),
                        expected,
                        actual
                    );
                }
            } else {
                bail!("expected hash provided but target missing after apply");
            }
        }

        let now = now_ms();
        let diff_ref = format!("harness:diff:{session_id}:{trace_id}:{now}:{idx}");
        let diff_payload = serde_json::json!({
            "op_id": op_id,
            "kind": format!("{:?}", op.kind).to_ascii_lowercase(),
            "path": op.path,
            "before_hash": before_hash,
            "after_hash": after_hash,
            "expected_hash": op.expected_hash,
            "from_path": op.from_path,
            "metadata": op.metadata.clone(),
            "timestamp_ms": now,
        });
        db.upsert_json_knowledge(diff_ref.clone(), &diff_payload, "diff-patch-engine")
            .await?;
        let snapshot_ref = format!("harness:patch:snapshot:{session_id}:{trace_id}:{now}:{idx}");
        let snapshot = PatchSnapshotRecord {
            op_id: normalize_op_id(op, idx),
            path: op.path.clone(),
            existed_before,
            content_before: previous_bytes,
            created_at_ms: now,
        };
        db.upsert_json_knowledge(snapshot_ref, &snapshot, "diff-patch-engine")
            .await?;

        Ok((
            PatchStepRecord {
                op_id: normalize_op_id(op, idx),
                kind: format!("{:?}", op.kind).to_ascii_lowercase(),
                path: op.path.clone(),
                status: PatchStepStatus::Applied,
                before_hash,
                after_hash,
                diff_ref: Some(diff_ref),
                error: None,
            },
            Some(undo),
        ))
    }

    fn apply_patch_op(&self, op: &PatchOp, target: &Path) -> Result<()> {
        match op.kind {
            PatchOpKind::CreateFile => {
                if target.exists() {
                    bail!("create_file target already exists: {}", target.display());
                }
                let patch = op
                    .patch
                    .as_ref()
                    .ok_or_else(|| anyhow!("create_file requires patch content"))?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("create parent directory {}", parent.display()))?;
                }
                fs::write(target, patch.as_bytes())
                    .with_context(|| format!("write created file {}", target.display()))?;
            }
            PatchOpKind::UpdateFile => {
                if !target.exists() {
                    bail!("update_file target missing: {}", target.display());
                }
                let patch = op
                    .patch
                    .as_ref()
                    .ok_or_else(|| anyhow!("update_file requires patch content"))?;
                fs::write(target, patch.as_bytes())
                    .with_context(|| format!("update file {}", target.display()))?;
            }
            PatchOpKind::DeleteFile => {
                if !target.exists() {
                    bail!("delete_file target missing: {}", target.display());
                }
                fs::remove_file(target).with_context(|| format!("delete file {}", target.display()))?;
            }
            PatchOpKind::MoveFile => {
                let from = op
                    .from_path
                    .as_deref()
                    .ok_or_else(|| anyhow!("move_file requires from_path"))?;
                let src = self.resolve_path(from)?;
                if !src.exists() {
                    bail!("move_file source missing: {}", src.display());
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("create parent directory {}", parent.display()))?;
                }
                fs::rename(&src, target)
                    .with_context(|| format!("move file {} -> {}", src.display(), target.display()))?;
            }
            PatchOpKind::Revert => bail!(
                "revert op should use revert_from_trace(), not apply_patch_op()"
            ),
        }
        Ok(())
    }

    fn rollback_action(&self, action: UndoAction) -> Result<PatchStepRecord> {
        match action {
            UndoAction::DeleteCreated { path } => {
                if path.exists() {
                    fs::remove_file(&path)
                        .with_context(|| format!("rollback delete created file {}", path.display()))?;
                }
                Ok(PatchStepRecord {
                    op_id: format!("rollback:delete-created:{}", path.display()),
                    kind: "rollback".to_string(),
                    path: path.to_string_lossy().replace('\\', "/"),
                    status: PatchStepStatus::Reverted,
                    before_hash: None,
                    after_hash: hash_file_if_exists(&path),
                    diff_ref: None,
                    error: None,
                })
            }
            UndoAction::RestoreFile { path, content } => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("rollback create parent directory {}", parent.display())
                    })?;
                }
                fs::write(&path, content)
                    .with_context(|| format!("rollback restore file {}", path.display()))?;
                Ok(PatchStepRecord {
                    op_id: format!("rollback:restore-file:{}", path.display()),
                    kind: "rollback".to_string(),
                    path: path.to_string_lossy().replace('\\', "/"),
                    status: PatchStepStatus::Reverted,
                    before_hash: None,
                    after_hash: hash_file_if_exists(&path),
                    diff_ref: None,
                    error: None,
                })
            }
            UndoAction::RestoreDeleted { path, content } => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("rollback create parent directory {}", parent.display())
                    })?;
                }
                fs::write(&path, content)
                    .with_context(|| format!("rollback restore deleted file {}", path.display()))?;
                Ok(PatchStepRecord {
                    op_id: format!("rollback:restore-deleted:{}", path.display()),
                    kind: "rollback".to_string(),
                    path: path.to_string_lossy().replace('\\', "/"),
                    status: PatchStepStatus::Reverted,
                    before_hash: None,
                    after_hash: hash_file_if_exists(&path),
                    diff_ref: None,
                    error: None,
                })
            }
            UndoAction::MoveBack { from, to } => {
                if from.exists() {
                    if let Some(parent) = to.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("rollback create parent directory {}", parent.display())
                        })?;
                    }
                    fs::rename(&from, &to).with_context(|| {
                        format!("rollback move file {} -> {}", from.display(), to.display())
                    })?;
                }
                Ok(PatchStepRecord {
                    op_id: format!("rollback:move-back:{}->{}", from.display(), to.display()),
                    kind: "rollback".to_string(),
                    path: to.to_string_lossy().replace('\\', "/"),
                    status: PatchStepStatus::Reverted,
                    before_hash: None,
                    after_hash: hash_file_if_exists(&to),
                    diff_ref: None,
                    error: None,
                })
            }
        }
    }

    fn detect_conflicts(&self, ops: &[PatchOp]) -> Result<Vec<PatchConflict>> {
        let mut conflicts = Vec::new();
        let mut seen_ids = BTreeSet::<String>::new();
        let mut target_writers: BTreeMap<PathBuf, String> = BTreeMap::new();
        let mut move_sources: BTreeMap<PathBuf, String> = BTreeMap::new();

        for (idx, op) in ops.iter().enumerate() {
            let op_id = normalize_op_id(op, idx);
            if !seen_ids.insert(op_id.clone()) {
                conflicts.push(PatchConflict {
                    op_id: op_id.clone(),
                    path: op.path.clone(),
                    reason: "duplicate_op_id".to_string(),
                });
            }

            let target = self.resolve_path(&op.path)?;
            if let Some(prev) = target_writers.insert(target.clone(), op_id.clone()) {
                conflicts.push(PatchConflict {
                    op_id: op_id.clone(),
                    path: op.path.clone(),
                    reason: format!("write_write_conflict_with_op={prev}"),
                });
            }

            if let PatchOpKind::MoveFile = op.kind {
                let Some(from) = op.from_path.as_deref() else {
                    continue;
                };
                let src = self.resolve_path(from)?;
                if let Some(prev) = move_sources.insert(src.clone(), op_id.clone()) {
                    conflicts.push(PatchConflict {
                        op_id: op_id.clone(),
                        path: from.to_string(),
                        reason: format!("move_source_conflict_with_op={prev}"),
                    });
                }
                if src == target {
                    conflicts.push(PatchConflict {
                        op_id,
                        path: op.path.clone(),
                        reason: "move_source_equals_target".to_string(),
                    });
                }
            }
        }

        for (source, source_op) in &move_sources {
            if let Some(target_op) = target_writers.get(source) {
                conflicts.push(PatchConflict {
                    op_id: source_op.clone(),
                    path: source.to_string_lossy().replace('\\', "/"),
                    reason: format!("move_source_written_by_op={target_op}"),
                });
            }
        }

        Ok(conflicts)
    }

    fn resolve_path(&self, input: &str) -> Result<PathBuf> {
        let raw = PathBuf::from(input);
        let joined = if raw.is_absolute() {
            raw
        } else {
            self.workspace_root.join(raw)
        };
        let normalized = normalize_without_resolving(&joined);
        let root = normalize_without_resolving(&self.workspace_root);
        if !normalized.starts_with(&root) {
            bail!(
                "path escapes workspace root (target={}, root={})",
                normalized.display(),
                root.display()
            );
        }
        Ok(normalized)
    }

    fn build_report_replay_fp(
        &self,
        trace_id: &str,
        steps: &[PatchStepRecord],
        rollback: bool,
    ) -> String {
        replay::build_fingerprint(
            "diffpatch",
            "diffpatch/schema/v1",
            "diffpatch/seed/v1",
            "diffpatch/replay/v1",
            &serde_json::json!({
                "trace_id": trace_id,
                "workspace_root": self.workspace_root.to_string_lossy().replace('\\', "/"),
                "rollback": rollback,
                "steps": steps,
            }),
        )
    }
}

fn normalize_op_id(op: &PatchOp, idx: usize) -> String {
    if op.op_id.trim().is_empty() {
        format!("op-{idx}")
    } else {
        op.op_id.clone()
    }
}

fn hash_file_if_exists(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_without_resolving(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use autoloop_state_adapter::{StateStore, StateStoreBackend, StateStoreConfig};

    use super::*;

    fn db() -> StateStore {
        StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 4,
        })
    }

    #[tokio::test]
    async fn structured_patch_apply_and_revert_work() {
        let root = std::env::temp_dir().join(format!("diff-patch-engine-{}", now_ms()));
        fs::create_dir_all(&root).expect("create root");
        let seed = root.join("demo.txt");
        fs::write(&seed, "v1").expect("seed");
        let engine = DiffPatchEngine::new(root.clone());
        let db = db();
        let trace = format!("trace-{}", now_ms());

        let ops = vec![PatchOp {
            op_id: "update-1".to_string(),
            kind: PatchOpKind::UpdateFile,
            path: "demo.txt".to_string(),
            from_path: None,
            patch: Some("v2".to_string()),
            expected_hash: None,
            metadata: BTreeMap::new(),
        }];
        let report = engine
            .apply_with_rollback(&db, "session-a", &trace, &ops)
            .await
            .expect("apply");
        assert!(report.success);
        assert_eq!(fs::read_to_string(&seed).expect("read"), "v2");

        let reverted = engine
            .revert_from_trace(&db, "session-a", &trace)
            .await
            .expect("revert");
        assert!(reverted.success);
        assert_eq!(fs::read_to_string(&seed).expect("read"), "v1");

        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn failure_triggers_rollback_and_leaves_original_state() {
        let root = std::env::temp_dir().join(format!("diff-patch-engine-rollback-{}", now_ms()));
        fs::create_dir_all(&root).expect("create root");
        let seed = root.join("demo.txt");
        fs::write(&seed, "original").expect("seed");
        let engine = DiffPatchEngine::new(root.clone());
        let db = db();

        let ops = vec![
            PatchOp {
                op_id: "update-ok".to_string(),
                kind: PatchOpKind::UpdateFile,
                path: "demo.txt".to_string(),
                from_path: None,
                patch: Some("changed".to_string()),
                expected_hash: None,
                metadata: BTreeMap::new(),
            },
            PatchOp {
                op_id: "delete-missing".to_string(),
                kind: PatchOpKind::DeleteFile,
                path: "missing.txt".to_string(),
                from_path: None,
                patch: None,
                expected_hash: None,
                metadata: BTreeMap::new(),
            },
        ];

        let report = engine
            .apply_with_rollback(&db, "session-b", "trace-b", &ops)
            .await
            .expect("report");
        assert!(!report.success);
        assert!(!report.transaction_committed);
        assert!(report.rollback_performed);
        assert!(report.rollback_successful);
        assert!(
            report
                .steps
                .iter()
                .any(|step| matches!(step.status, PatchStepStatus::Reverted)),
            "rollback actions should be visible in report steps"
        );
        assert_eq!(fs::read_to_string(&seed).expect("read"), "original");

        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn preflight_conflict_rejects_duplicate_writes_without_touching_files() {
        let root = std::env::temp_dir().join(format!("diff-patch-engine-conflict-{}", now_ms()));
        fs::create_dir_all(&root).expect("create root");
        let seed = root.join("demo.txt");
        fs::write(&seed, "original").expect("seed");
        let engine = DiffPatchEngine::new(root.clone());
        let db = db();

        let ops = vec![
            PatchOp {
                op_id: "dup-write-1".to_string(),
                kind: PatchOpKind::UpdateFile,
                path: "demo.txt".to_string(),
                from_path: None,
                patch: Some("changed-a".to_string()),
                expected_hash: None,
                metadata: BTreeMap::new(),
            },
            PatchOp {
                op_id: "dup-write-2".to_string(),
                kind: PatchOpKind::UpdateFile,
                path: "demo.txt".to_string(),
                from_path: None,
                patch: Some("changed-b".to_string()),
                expected_hash: None,
                metadata: BTreeMap::new(),
            },
        ];

        let report = engine
            .apply_with_rollback(&db, "session-c", "trace-c", &ops)
            .await
            .expect("report");
        assert!(!report.success);
        assert!(!report.transaction_committed);
        assert!(!report.rollback_performed);
        assert!(
            report
                .message
                .as_deref()
                .is_some_and(|m| m.contains("preflight conflict")),
            "preflight conflict reason should be surfaced"
        );
        assert!(
            report
                .steps
                .iter()
                .all(|step| matches!(step.status, PatchStepStatus::Failed)),
            "conflict report should surface failed preflight steps"
        );
        assert_eq!(fs::read_to_string(&seed).expect("read"), "original");

        let _ = fs::remove_dir_all(&root);
    }
}
