use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::{
    config::SignalPipelineConfig,
    contracts::signal::SignalEvent,
    observability::signal_pipeline::{BatchFlushOutput, SignalPipeline, SinkOutput},
};

#[derive(Clone)]
pub struct SignalFacade {
    db: StateStore,
    pipeline: SignalPipeline,
}

impl SignalFacade {
    pub fn new(db: StateStore, pipeline_cfg: &SignalPipelineConfig) -> Self {
        Self {
            db,
            pipeline: SignalPipeline::from_config(pipeline_cfg),
        }
    }

    pub async fn emit(&self, event: SignalEvent) -> Result<SinkOutput> {
        self.pipeline.execute(&self.db, event).await
    }

    pub async fn emit_batch(&self, events: Vec<SignalEvent>) -> Result<BatchFlushOutput> {
        self.pipeline.execute_batch(&self.db, events).await
    }

    pub fn enqueue(&self, event: SignalEvent) -> usize {
        self.pipeline.enqueue(event)
    }

    pub async fn flush_if_needed(&self) -> Result<Option<BatchFlushOutput>> {
        self.pipeline.flush_if_needed(&self.db).await
    }

    pub async fn shutdown_flush(&self) -> Result<BatchFlushOutput> {
        self.pipeline.shutdown_flush(&self.db).await
    }

    pub fn pending_len(&self) -> usize {
        self.pipeline.pending_len()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    #[test]
    fn signal_write_path_is_no_bypass() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let allow = [
            root.join("observability").join("signal_pipeline.rs"),
            root.join("observability").join("signal_facade.rs"),
            root.join("main.rs"),
        ];

        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                    continue;
                }
                if allow.iter().any(|allowed| allowed == &path) {
                    continue;
                }
                let Ok(content) = fs::read_to_string(&path) else {
                    continue;
                };
                let has_signal_key_literal = content.contains("signal:events:")
                    || content.contains("signal-pipeline-query-explain");
                if has_signal_key_literal && content.contains("upsert_json_knowledge(") {
                    panic!(
                        "signal no-bypass violation: direct signal write in {}",
                        path.display()
                    );
                }
                if content.contains("SignalPipeline::from_config")
                    || content.contains("signal_pipeline::SignalPipeline")
                {
                    panic!(
                        "signal no-bypass violation: direct pipeline usage in {}",
                        path.display()
                    );
                }
            }
        }
    }
}
