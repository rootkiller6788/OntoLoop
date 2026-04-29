use anyhow::Result;

use crate::{contracts::context::ContextItem, providers::ChatMessage};

use super::{
    checkpoint::{SessionCheckpoint, SessionCheckpointStore, compact_history},
    store::Session,
};

#[derive(Debug, Clone)]
pub struct SessionRuntime {
    checkpoints: SessionCheckpointStore,
    memory_window: usize,
}

impl SessionRuntime {
    pub fn new(checkpoints: SessionCheckpointStore, memory_window: usize) -> Self {
        Self {
            checkpoints,
            memory_window,
        }
    }

    pub fn default(memory_window: usize) -> Self {
        Self::new(SessionCheckpointStore::default(), memory_window)
    }

    pub fn checkpoint_store(&self) -> &SessionCheckpointStore {
        &self.checkpoints
    }

    pub fn checkpoint_session(
        &self,
        session: &Session,
        context_items: &[ContextItem],
    ) -> Result<SessionCheckpoint> {
        self.checkpoints.update_history(
            &session.key,
            &session.history,
            self.memory_window,
            context_items,
        )
    }

    pub fn restore_session(&self, session_id: &str) -> Result<Option<Session>> {
        let Some(checkpoint) = self.checkpoints.load(session_id)? else {
            return Ok(None);
        };
        Ok(Some(Session {
            key: checkpoint.session_id,
            history: checkpoint.history,
        }))
    }

    pub fn annotate_continuation(
        &self,
        session_id: &str,
        turn_id: &str,
        checkpoint_token: &str,
    ) -> Result<()> {
        let _ = self
            .checkpoints
            .annotate_continuation(session_id, turn_id, checkpoint_token)?;
        Ok(())
    }
    pub fn resume_compacted_history(&self, session_id: &str) -> Result<Option<Vec<ChatMessage>>> {
        let Some(checkpoint) = self.checkpoints.load(session_id)? else {
            return Ok(None);
        };
        let compacted = compact_history(&checkpoint.history, self.memory_window);
        Ok(Some(compacted))
    }
}
