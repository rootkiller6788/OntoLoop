use anyhow::Result;

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
};

use tokio::sync::RwLock;

use crate::{
    contracts::context::{ContextItem, ContextItemKind},
    providers::ChatMessage,
};

use super::{
    checkpoint::{SessionCheckpoint, SessionCheckpointStore},
    context_cache::{ContextCacheBundle, ContextCacheOrchestrator},
    resume_runner::{SessionResumeRunner, SessionResumeSnapshot},
    runtime::SessionRuntime,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionIdentity {
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub lease_token: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub key: String,
    pub history: Vec<ChatMessage>,
}

impl Session {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            history: Vec::new(),
        }
    }

    pub fn push(&mut self, role: impl Into<String>, content: impl Into<String>) {
        self.history.push(ChatMessage {
            role: role.into(),
            content: content.into(),
        });
    }

    pub fn recent_history(&self, max_messages: usize) -> Vec<ChatMessage> {
        let start = self.history.len().saturating_sub(max_messages);
        self.history[start..].to_vec()
    }
}

#[derive(Clone)]
pub struct SessionStore {
    inner: Arc<RwLock<HashMap<String, Session>>>,
    identities: Arc<RwLock<HashMap<String, SessionIdentity>>>,
    context_items: Arc<RwLock<HashMap<String, Vec<ContextItem>>>>,
    context_cache: Arc<RwLock<HashMap<String, ContextCacheBundle>>>,
    cache_orchestrator: ContextCacheOrchestrator,
    runtime: SessionRuntime,
    memory_window: usize,
}

impl SessionStore {
    pub fn new(memory_window: usize) -> Self {
        let runtime = SessionRuntime::default(memory_window);
        let cache_orchestrator = ContextCacheOrchestrator::new(runtime.checkpoint_store().clone());
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            identities: Arc::new(RwLock::new(HashMap::new())),
            context_items: Arc::new(RwLock::new(HashMap::new())),
            context_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_orchestrator,
            runtime,
            memory_window,
        }
    }

    pub fn with_checkpoint_root(memory_window: usize, root: impl Into<PathBuf>) -> Self {
        let runtime = SessionRuntime::new(SessionCheckpointStore::new(root), memory_window);
        let cache_orchestrator = ContextCacheOrchestrator::new(runtime.checkpoint_store().clone());
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            identities: Arc::new(RwLock::new(HashMap::new())),
            context_items: Arc::new(RwLock::new(HashMap::new())),
            context_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_orchestrator,
            runtime,
            memory_window,
        }
    }

    pub fn checkpoint_root(&self) -> PathBuf {
        self.runtime.checkpoint_store().root().to_path_buf()
    }

    pub async fn list_session_ids(&self) -> Vec<String> {
        let mut ids = BTreeMap::<String, ()>::new();
        for key in self.inner.read().await.keys() {
            ids.insert(key.clone(), ());
        }
        for key in self.identities.read().await.keys() {
            ids.insert(key.clone(), ());
        }

        let root = self.checkpoint_root();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let is_json = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
                if !is_json {
                    continue;
                }
                if let Ok(raw) = std::fs::read_to_string(&path) {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                        if let Some(session_id) =
                            value.get("session_id").and_then(serde_json::Value::as_str)
                        {
                            ids.insert(session_id.to_string(), ());
                            continue;
                        }
                    }
                }
                if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                    ids.insert(stem.to_string(), ());
                }
            }
        }
        ids.into_keys().collect()
    }

    pub async fn append_user_message(&self, session_id: &str, content: &str) {
        self.append_message(session_id, "user", content).await;
    }

    pub async fn append_assistant_message(&self, session_id: &str, content: &str) {
        self.append_message(session_id, "assistant", content).await;
    }

    pub async fn append_tool_message(&self, session_id: &str, tool_name: &str, content: &str) {
        self.append_message(session_id, "tool", &format!("{tool_name}: {content}"))
            .await;
    }

    pub async fn history(&self, session_id: &str) -> Vec<ChatMessage> {
        if let Some(history) = self.history_from_memory(session_id).await {
            return history;
        }

        if let Ok(Some(restored)) = self.runtime.restore_session(session_id) {
            let history = restored.recent_history(self.memory_window);
            self.inner
                .write()
                .await
                .insert(session_id.to_string(), restored);
            self.hydrate_context_items_from_checkpoint(session_id).await;
            return history;
        }

        Vec::new()
    }

    pub async fn load_from_checkpoint(&self, session_id: &str) -> bool {
        match self.runtime.restore_session(session_id) {
            Ok(Some(restored)) => {
                self.inner
                    .write()
                    .await
                    .insert(session_id.to_string(), restored);
                self.hydrate_context_items_from_checkpoint(session_id).await;
                true
            }
            _ => false,
        }
    }

    pub async fn checkpoint(&self, session_id: &str) -> Option<SessionCheckpoint> {
        self.runtime
            .checkpoint_store()
            .load(session_id)
            .ok()
            .flatten()
    }

    pub async fn resume_snapshot(&self, session_id: &str) -> Option<SessionResumeSnapshot> {
        let runner = SessionResumeRunner::new(self.runtime.clone());
        runner.resume(session_id).ok().flatten()
    }

    pub async fn bind_continuation(&self, session_id: &str, turn_id: &str, checkpoint_token: &str) {
        let _ = self
            .runtime
            .annotate_continuation(session_id, turn_id, checkpoint_token);
    }
    pub async fn compacted_history(&self, session_id: &str) -> Vec<ChatMessage> {
        self.runtime
            .resume_compacted_history(session_id)
            .ok()
            .flatten()
            .unwrap_or_default()
    }


    pub async fn named_snapshot(&self, session_id: &str, snapshot_name: &str) -> Result<Option<PathBuf>> {
        if snapshot_name.trim().is_empty() {
            return Ok(None);
        }

        let checkpoint = if let Some(existing) = self.runtime.checkpoint_store().load(session_id)? {
            existing
        } else {
            let maybe_session = { self.inner.read().await.get(session_id).cloned() };
            let Some(session) = maybe_session else {
                return Ok(None);
            };
            let checkpoint_items = self
                .context_items
                .read()
                .await
                .get(session_id)
                .cloned()
                .unwrap_or_default();
            self.runtime.checkpoint_session(&session, &checkpoint_items)?
        };

        let path = self
            .runtime
            .checkpoint_store()
            .save_named_snapshot(session_id, snapshot_name, &checkpoint)?;
        Ok(Some(path))
    }

    pub async fn export_transcript_markdown(&self, session_id: &str) -> Result<String> {
        let history = self.history(session_id).await;
        let mut transcript = String::new();
        transcript.push_str("# Session Transcript\n\n");
        transcript.push_str(&format!("- session_id: {}\n", session_id));
        transcript.push_str(&format!("- message_count: {}\n\n", history.len()));

        if history.is_empty() {
            transcript.push_str("_No messages captured for this session yet._\n");
            return Ok(transcript);
        }

        for (index, message) in history.iter().enumerate() {
            transcript.push_str(&format!("## {}. {}\n\n", index + 1, message.role));
            transcript.push_str(&message.content);
            transcript.push_str("\n\n");
        }

        Ok(transcript)
    }    pub async fn bind_identity(&self, session_id: &str, identity: SessionIdentity) {
        self.identities
            .write()
            .await
            .insert(session_id.to_string(), identity);
    }

    pub async fn identity(&self, session_id: &str) -> Option<SessionIdentity> {
        self.identities.read().await.get(session_id).cloned()
    }

    pub async fn context_items(&self, session_id: &str) -> Vec<ContextItem> {
        if let Some(items) = self.context_items.read().await.get(session_id).cloned() {
            return items;
        }
        self.runtime
            .checkpoint_store()
            .load(session_id)
            .ok()
            .flatten()
            .map(|checkpoint| checkpoint.context_items)
            .unwrap_or_default()
    }

    pub async fn context_cache_bundle(&self, session_id: &str) -> Option<ContextCacheBundle> {
        if let Some(bundle) = self.context_cache.read().await.get(session_id).cloned() {
            return Some(bundle);
        }
        let bundle = self.cache_orchestrator.load(session_id).ok().flatten()?;
        self.context_cache
            .write()
            .await
            .insert(session_id.to_string(), bundle.clone());
        Some(bundle)
    }
    pub async fn context_summary_cache(
        &self,
        session_id: &str,
    ) -> Option<super::context_cache::ContextSummaryCache> {
        self.context_cache_bundle(session_id)
            .await
            .map(|bundle| bundle.summary_cache)
    }

    pub async fn context_retrieval_index(
        &self,
        session_id: &str,
    ) -> Option<super::context_cache::ContextRetrievalIndex> {
        self.context_cache_bundle(session_id)
            .await
            .map(|bundle| bundle.retrieval_index)
    }

    pub async fn context_state_snapshot(
        &self,
        session_id: &str,
    ) -> Option<super::context_cache::ContextStateSnapshot> {
        self.context_cache_bundle(session_id)
            .await
            .map(|bundle| bundle.state_snapshot)
    }

    async fn append_message(&self, session_id: &str, role: &str, content: &str) {
        let (snapshot, message_index) = {
            let mut sessions = self.inner.write().await;
            let session = sessions
                .entry(session_id.to_string())
                .or_insert_with(|| Session::new(session_id));
            session.push(role, content);
            (session.clone(), session.history.len().saturating_sub(1))
        };

        let identity = self.identities.read().await.get(session_id).cloned();
        let item = build_context_item(session_id, role, content, message_index, identity.as_ref());

        let checkpoint_items = {
            let mut context_items = self.context_items.write().await;
            let items = context_items
                .entry(session_id.to_string())
                .or_insert_with(Vec::new);
            items.push(item);
            let max_items = self.memory_window.saturating_mul(12).max(64);
            if items.len() > max_items {
                let overflow = items.len().saturating_sub(max_items);
                items.drain(0..overflow);
            }
            items.clone()
        };

        if let Ok(checkpoint) = self
            .runtime
            .checkpoint_session(&snapshot, &checkpoint_items)
        {
            let bundle = ContextCacheOrchestrator::from_checkpoint(&checkpoint);
            self.context_cache
                .write()
                .await
                .insert(session_id.to_string(), bundle);
        }
    }

    async fn history_from_memory(&self, session_id: &str) -> Option<Vec<ChatMessage>> {
        let sessions = self.inner.read().await;
        sessions
            .get(session_id)
            .map(|session| session.recent_history(self.memory_window))
    }

    async fn hydrate_context_items_from_checkpoint(&self, session_id: &str) {
        if let Ok(Some(checkpoint)) = self.runtime.checkpoint_store().load(session_id) {
            let bundle = ContextCacheOrchestrator::from_checkpoint(&checkpoint);
            self.context_items
                .write()
                .await
                .insert(session_id.to_string(), bundle.context_items.clone());
            self.context_cache
                .write()
                .await
                .insert(session_id.to_string(), bundle);
        }
    }
}

fn build_context_item(
    session_id: &str,
    role: &str,
    content: &str,
    message_index: usize,
    identity: Option<&SessionIdentity>,
) -> ContextItem {
    let is_tool = role.eq_ignore_ascii_case("tool");
    let kind = if is_tool {
        ContextItemKind::ToolState
    } else {
        ContextItemKind::Session
    };
    let priority = if is_tool {
        0.95
    } else if role.eq_ignore_ascii_case("user") {
        0.9
    } else if role.eq_ignore_ascii_case("assistant") {
        0.78
    } else {
        0.7
    };
    let budget_micros = ((content.chars().count() as u64).saturating_mul(120))
        .saturating_add(1_000)
        .min(2_000_000);
    let source_ref = if is_tool {
        format!("tool-state:{session_id}:message:{message_index}")
    } else {
        format!("session:{session_id}:message:{message_index}")
    };
    let permission_scope = derive_permission_scope(identity, is_tool);

    let mut metadata = BTreeMap::new();
    metadata.insert("role".to_string(), role.to_string());
    metadata.insert("source_layer".to_string(), "session-store".to_string());
    metadata.insert("message_index".to_string(), message_index.to_string());
    metadata.insert(
        "content_chars".to_string(),
        content.chars().count().to_string(),
    );
    if let Some(identity) = identity {
        metadata.insert("tenant_id".to_string(), identity.tenant_id.clone());
        metadata.insert("principal_id".to_string(), identity.principal_id.clone());
        metadata.insert("policy_id".to_string(), identity.policy_id.clone());
    }

    ContextItem::new(
        session_id.to_string(),
        format!("{session_id}:{role}:{message_index}"),
        kind,
        source_ref,
        permission_scope,
        priority,
        budget_micros,
        metadata,
    )
}

fn derive_permission_scope(identity: Option<&SessionIdentity>, is_tool: bool) -> String {
    let lane = if is_tool { "tool_state" } else { "session" };
    match identity {
        Some(identity) => format!(
            "tenant:{}:principal:{}:policy:{}:{}",
            identity.tenant_id, identity.principal_id, identity.policy_id, lane
        ),
        None => format!("tenant:default:principal:unknown:policy:default:{lane}"),
    }
}


