use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryTurnState {
    Initialized,
    ProviderCall,
    ToolDispatch,
    RetryBackoff,
    Completed,
    Failed,
    MaxIterationsReached,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct TurnTransition {
    pub from: QueryTurnState,
    pub to: QueryTurnState,
    pub reason: String,
    pub iteration: usize,
    pub at_ms: u64,
    pub metadata: BTreeMap<String, String>,
}

impl TurnTransition {
    pub fn new(
        from: QueryTurnState,
        to: QueryTurnState,
        reason: impl Into<String>,
        iteration: usize,
    ) -> Self {
        Self {
            from,
            to,
            reason: reason.into(),
            iteration,
            at_ms: current_time_ms(),
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
