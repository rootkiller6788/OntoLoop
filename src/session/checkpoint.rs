use std::{
    collections::{BTreeSet, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};

use crate::{contracts::context::ContextItem, providers::ChatMessage};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionHistoryCompaction {
    pub window_size: usize,
    pub compacted_history: Vec<ChatMessage>,
    pub digest: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SessionRedactionSummary {
    pub redacted_fields: u32,
    pub redaction_rules: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionCheckpoint {
    pub session_id: String,
    pub history: Vec<ChatMessage>,
    pub compaction: SessionHistoryCompaction,
    pub updated_at_ms: u64,
    pub continuation_turn_id: Option<String>,
    pub continuation_checkpoint_token: Option<String>,
    #[serde(default)]
    pub context_items: Vec<ContextItem>,
    #[serde(default)]
    pub evidence_ref: Option<String>,
    #[serde(default)]
    pub redacted_compacted_history: Vec<ChatMessage>,
    #[serde(default)]
    pub redaction_summary: SessionRedactionSummary,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SessionCheckpointEvidenceRecord {
    evidence_ref: String,
    session_id: String,
    updated_at_ms: u64,
    checkpoint_digest: String,
    history_len: usize,
    redacted_fields: u32,
    generated_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct SessionCheckpointStore {
    root: PathBuf,
}

impl SessionCheckpointStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn default() -> Self {
        let root = std::env::var("AUTOLOOP_SESSION_CHECKPOINT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("deploy/runtime/session-checkpoints"));
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load(&self, session_id: &str) -> Result<Option<SessionCheckpoint>> {
        let path = self.path_for(session_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(path)?;
        let checkpoint = serde_json::from_str::<SessionCheckpoint>(&raw)?;
        self.verify_checkpoint_evidence(&checkpoint)?;
        Ok(Some(checkpoint))
    }

    pub fn save(&self, checkpoint: &SessionCheckpoint) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let path = self.path_for(&checkpoint.session_id);
        fs::write(path, serde_json::to_string_pretty(checkpoint)?)?;
        if checkpoint.evidence_ref.is_some() {
            self.persist_evidence_record(checkpoint)?;
        }
        Ok(())
    }

    pub fn annotate_continuation(
        &self,
        session_id: &str,
        turn_id: &str,
        checkpoint_token: &str,
    ) -> Result<Option<SessionCheckpoint>> {
        let Some(mut checkpoint) = self.load(session_id)? else {
            return Ok(None);
        };
        checkpoint.continuation_turn_id = Some(turn_id.to_string());
        checkpoint.continuation_checkpoint_token = Some(checkpoint_token.to_string());
        checkpoint.updated_at_ms = current_time_ms();
        self.save(&checkpoint)?;
        Ok(Some(checkpoint))
    }

    pub fn update_history(
        &self,
        session_id: &str,
        history: &[ChatMessage],
        memory_window: usize,
        context_items: &[ContextItem],
    ) -> Result<SessionCheckpoint> {
        let compacted_history = compact_history(history, memory_window);
        let compaction = SessionHistoryCompaction {
            window_size: memory_window,
            digest: digest_messages(&compacted_history),
            compacted_history,
        };
        let previous = self.load(session_id).ok().flatten();

        let (redacted_compacted_history, redaction_summary) =
            redact_messages(&compaction.compacted_history);

        let updated_at_ms = current_time_ms();
        let evidence_ref = format!(
            "session-evidence:{}:{}:{}",
            sanitize_session_id(session_id),
            updated_at_ms,
            compaction.digest
        );

        let checkpoint = SessionCheckpoint {
            session_id: session_id.to_string(),
            history: history.to_vec(),
            compaction,
            updated_at_ms,
            continuation_turn_id: previous
                .as_ref()
                .and_then(|value| value.continuation_turn_id.clone()),
            continuation_checkpoint_token: previous
                .as_ref()
                .and_then(|value| value.continuation_checkpoint_token.clone()),
            context_items: context_items.to_vec(),
            evidence_ref: Some(evidence_ref),
            redacted_compacted_history,
            redaction_summary,
        };
        self.save(&checkpoint)?;
        Ok(checkpoint)
    }


    pub fn save_named_snapshot(
        &self,
        session_id: &str,
        snapshot_name: &str,
        checkpoint: &SessionCheckpoint,
    ) -> Result<PathBuf> {
        let named_dir = self.root.join("named").join(sanitize_session_id(session_id));
        fs::create_dir_all(&named_dir)?;
        let path = named_dir.join(format!("{}.json", sanitize_session_id(snapshot_name)));
        fs::write(&path, serde_json::to_string_pretty(checkpoint)?)?;
        Ok(path)
    }
    fn verify_checkpoint_evidence(&self, checkpoint: &SessionCheckpoint) -> Result<()> {
        let Some(ref evidence_ref) = checkpoint.evidence_ref else {
            return Ok(());
        };
        let evidence_path = self.evidence_path_for(&checkpoint.session_id);
        if !evidence_path.exists() {
            bail!(
                "checkpoint evidence missing for session '{}' (ref={})",
                checkpoint.session_id,
                evidence_ref
            );
        }
        let raw = fs::read_to_string(evidence_path)?;
        let evidence = serde_json::from_str::<SessionCheckpointEvidenceRecord>(&raw)?;
        if evidence.evidence_ref != *evidence_ref {
            bail!(
                "checkpoint evidence_ref mismatch for session '{}'",
                checkpoint.session_id
            );
        }
        if evidence.checkpoint_digest != checkpoint.compaction.digest {
            bail!(
                "checkpoint digest mismatch for session '{}'",
                checkpoint.session_id
            );
        }
        Ok(())
    }

    fn persist_evidence_record(&self, checkpoint: &SessionCheckpoint) -> Result<()> {
        let Some(evidence_ref) = checkpoint.evidence_ref.clone() else {
            return Ok(());
        };
        let evidence_dir = self.root.join("evidence");
        fs::create_dir_all(&evidence_dir)?;
        let record = SessionCheckpointEvidenceRecord {
            evidence_ref,
            session_id: checkpoint.session_id.clone(),
            updated_at_ms: checkpoint.updated_at_ms,
            checkpoint_digest: checkpoint.compaction.digest.clone(),
            history_len: checkpoint.history.len(),
            redacted_fields: checkpoint.redaction_summary.redacted_fields,
            generated_at_ms: current_time_ms(),
        };
        fs::write(
            self.evidence_path_for(&checkpoint.session_id),
            serde_json::to_string_pretty(&record)?,
        )?;
        Ok(())
    }

    fn evidence_path_for(&self, session_id: &str) -> PathBuf {
        self.root
            .join("evidence")
            .join(format!("{}.json", sanitize_session_id(session_id)))
    }

    fn path_for(&self, session_id: &str) -> PathBuf {
        self.root
            .join(format!("{}.json", sanitize_session_id(session_id)))
    }
}

pub fn compact_history(history: &[ChatMessage], memory_window: usize) -> Vec<ChatMessage> {
    let start = history.len().saturating_sub(memory_window.max(1));
    history[start..].to_vec()
}

fn sanitize_session_id(session_id: &str) -> String {
    let cleaned: String = session_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();
    if cleaned.is_empty() {
        "session".to_string()
    } else {
        cleaned
    }
}

fn digest_messages(messages: &[ChatMessage]) -> String {
    let raw = serde_json::to_string(messages).unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn redact_messages(messages: &[ChatMessage]) -> (Vec<ChatMessage>, SessionRedactionSummary) {
    let mut redacted_fields = 0_u32;
    let mut rules = BTreeSet::new();
    let mut redacted = Vec::with_capacity(messages.len());

    for message in messages {
        let (content, count, applied) = redact_sensitive_text(&message.content);
        redacted_fields = redacted_fields.saturating_add(count);
        for rule in applied {
            rules.insert(rule);
        }
        redacted.push(ChatMessage { tool_call_id: None, tool_calls: None,
            role: message.role.clone(),
            content,
        });
    }

    (
        redacted,
        SessionRedactionSummary {
            redacted_fields,
            redaction_rules: rules.into_iter().collect(),
        },
    )
}

fn redact_sensitive_text(content: &str) -> (String, u32, Vec<String>) {
    let mut text = content.to_string();
    let mut count = 0_u32;
    let mut rules = BTreeSet::new();

    let (next, changed) = replace_prefixed_token(&text, "sk-", "[REDACTED_API_KEY]");
    if changed > 0 {
        count = count.saturating_add(changed);
        rules.insert("api_key".to_string());
        text = next;
    }

    let (next, changed) = redact_bearer_tokens(&text);
    if changed > 0 {
        count = count.saturating_add(changed);
        rules.insert("bearer_token".to_string());
        text = next;
    }

    let (next, changed) = redact_email_like_tokens(&text);
    if changed > 0 {
        count = count.saturating_add(changed);
        rules.insert("email".to_string());
        text = next;
    }

    let (next, changed) = redact_private_key_markers(&text);
    if changed > 0 {
        count = count.saturating_add(changed);
        rules.insert("key_material".to_string());
        text = next;
    }

    (text, count, rules.into_iter().collect())
}

fn replace_prefixed_token(content: &str, prefix: &str, replacement: &str) -> (String, u32) {
    let mut output = String::with_capacity(content.len());
    let mut idx = 0usize;
    let mut replaced = 0_u32;

    while let Some(relative) = content[idx..].find(prefix) {
        let start = idx + relative;
        output.push_str(&content[idx..start]);

        let mut end = start + prefix.len();
        for ch in content[end..].chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                end += ch.len_utf8();
            } else {
                break;
            }
        }

        if end.saturating_sub(start) >= prefix.len() + 12 {
            output.push_str(replacement);
            replaced = replaced.saturating_add(1);
        } else {
            output.push_str(&content[start..end]);
        }
        idx = end;
    }

    output.push_str(&content[idx..]);
    (output, replaced)
}

fn redact_bearer_tokens(content: &str) -> (String, u32) {
    let marker = "Bearer ";
    let mut output = String::with_capacity(content.len());
    let mut idx = 0usize;
    let mut replaced = 0_u32;

    while let Some(relative) = content[idx..].find(marker) {
        let start = idx + relative;
        output.push_str(&content[idx..start + marker.len()]);
        let mut end = start + marker.len();
        for ch in content[end..].chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
                end += ch.len_utf8();
            } else {
                break;
            }
        }
        if end > start + marker.len() {
            output.push_str("[REDACTED_TOKEN]");
            replaced = replaced.saturating_add(1);
        }
        idx = end;
    }

    output.push_str(&content[idx..]);
    (output, replaced)
}

fn redact_email_like_tokens(content: &str) -> (String, u32) {
    let mut replaced = 0_u32;
    let redacted = content
        .split_whitespace()
        .map(|token| {
            if token.contains('@') && token.contains('.') && token.len() >= 6 {
                replaced = replaced.saturating_add(1);
                "[REDACTED_EMAIL]".to_string()
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    (redacted, replaced)
}

fn redact_private_key_markers(content: &str) -> (String, u32) {
    let mut replaced = 0_u32;
    let mut lines = Vec::new();
    for line in content.lines() {
        if line.contains("BEGIN PRIVATE KEY")
            || line.contains("END PRIVATE KEY")
            || line.contains("BEGIN RSA PRIVATE KEY")
            || line.contains("END RSA PRIVATE KEY")
        {
            lines.push("[REDACTED_KEY_MATERIAL]".to_string());
            replaced = replaced.saturating_add(1);
        } else {
            lines.push(line.to_string());
        }
    }
    (lines.join("\n"), replaced)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}


