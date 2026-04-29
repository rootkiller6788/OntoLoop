use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
};

use anyhow::Result;
use serde_json::json;

use crate::AutoLoopApp;

use super::observable_state_store::{ObservableStateStore, ObservableStateTopic};

pub type DispatchFuture = Pin<Box<dyn Future<Output = Result<String>> + Send + 'static>>;
pub type CommandHandler = Arc<
    dyn Fn(Arc<AutoLoopApp>, DispatchArgs, ObservableStateStore) -> DispatchFuture + Send + Sync,
>;

#[derive(Debug, Clone, Default)]
pub struct DispatchArgs {
    pub session_id: String,
    pub action: String,
    pub params: BTreeMap<String, String>,
}

impl DispatchArgs {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }
}

#[derive(Debug, Clone)]
pub struct DispatchOutcome {
    pub handled: bool,
    pub body: Option<String>,
}

impl DispatchOutcome {
    pub fn not_handled() -> Self {
        Self {
            handled: false,
            body: None,
        }
    }

    pub fn handled(body: String) -> Self {
        Self {
            handled: true,
            body: Some(body),
        }
    }
}

pub struct CommandRegistry {
    handlers: BTreeMap<String, CommandHandler>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, key: impl Into<String>, handler: CommandHandler) {
        self.handlers.insert(key.into(), handler);
    }

    pub fn contains(&self, key: &str) -> bool {
        self.handlers.contains_key(key)
    }

    pub fn keys(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    pub async fn dispatch(
        &self,
        key: &str,
        app: Arc<AutoLoopApp>,
        args: DispatchArgs,
        state_store: ObservableStateStore,
    ) -> Result<DispatchOutcome> {
        let Some(handler) = self.handlers.get(key) else {
            return Ok(DispatchOutcome::not_handled());
        };

        state_store
            .publish_snapshot(
                ObservableStateTopic::CommandReceived,
                format!("command/{key}"),
                json!({
                    "command": key,
                    "session_id": args.session_id,
                    "action": args.action,
                    "params": args.params,
                }),
            )
            .await;

        let result = handler(app, args.clone(), state_store.clone()).await;
        match result {
            Ok(body) => {
                state_store
                    .publish_snapshot(
                        ObservableStateTopic::CommandCompleted,
                        format!("command/{key}/last_success"),
                        json!({
                            "command": key,
                            "session_id": args.session_id,
                        }),
                    )
                    .await;
                Ok(DispatchOutcome::handled(body))
            }
            Err(error) => {
                state_store
                    .publish_snapshot(
                        ObservableStateTopic::CommandFailed,
                        format!("command/{key}/last_error"),
                        json!({
                            "command": key,
                            "session_id": args.session_id,
                            "error": error.to_string(),
                        }),
                    )
                    .await;
                Err(error)
            }
        }
    }
}

#[derive(Clone)]
pub struct BuiltinCommandRegistry {
    registry: Arc<CommandRegistry>,
    state_store: ObservableStateStore,
}

impl BuiltinCommandRegistry {
    pub fn new() -> Self {
        let mut registry = CommandRegistry::new();
        register_high_frequency_handlers(&mut registry);

        Self {
            registry: Arc::new(registry),
            state_store: ObservableStateStore::default(),
        }
    }

    pub fn state_store(&self) -> &ObservableStateStore {
        &self.state_store
    }

    pub fn keys(&self) -> Vec<String> {
        self.registry.keys()
    }

    pub async fn dispatch(
        &self,
        key: &str,
        app: Arc<AutoLoopApp>,
        args: DispatchArgs,
    ) -> Result<DispatchOutcome> {
        self.registry
            .dispatch(key, app, args, self.state_store.clone())
            .await
    }
}

fn register_high_frequency_handlers(registry: &mut CommandRegistry) {
    registry.register(
        "system.status",
        Arc::new(|app, _args, _state| Box::pin(async move { app.system_status().await })),
    );

    registry.register(
        "system.health",
        Arc::new(|app, _args, _state| {
            Box::pin(async move {
                let health = json!({
                    "research": app.research.health_report(),
                    "system": serde_json::from_str::<serde_json::Value>(&app.system_status().await?)
                        .unwrap_or_else(|_| json!({})),
                });
                Ok(serde_json::to_string_pretty(&health)?)
            })
        }),
    );

    registry.register(
        "focus.status",
        Arc::new(|app, args, _state| {
            Box::pin(async move {
                let target = if args.session_id.trim().is_empty() {
                    "cli:focus"
                } else {
                    args.session_id.as_str()
                };
                app.focus_status(target).await
            })
        }),
    );

    registry.register(
        "bridge.status",
        Arc::new(|app, args, state| {
            Box::pin(async move {
                let body = app.bridge_status(&args.session_id).await?;
                if let Ok(snapshot) = serde_json::from_str::<serde_json::Value>(&body) {
                    state
                        .publish_snapshot(
                            ObservableStateTopic::BridgeSnapshot,
                            format!("bridge/{}/status", args.session_id),
                            snapshot,
                        )
                        .await;
                }
                Ok(body)
            })
        }),
    );

    registry.register(
        "bridge.remote-status",
        Arc::new(|app, args, state| {
            Box::pin(async move {
                let body = app.bridge_remote_status(&args.session_id).await?;
                if let Ok(snapshot) = serde_json::from_str::<serde_json::Value>(&body) {
                    state
                        .publish_snapshot(
                            ObservableStateTopic::BridgeSnapshot,
                            format!("bridge/{}/remote-status", args.session_id),
                            snapshot,
                        )
                        .await;
                }
                Ok(body)
            })
        }),
    );

    registry.register(
        "trigger.list",
        Arc::new(|app, args, state| {
            Box::pin(async move {
                let list = app.state_store().list_schedule_events(&args.session_id).await?;
                let payload = serde_json::to_value(&list).unwrap_or_else(|_| json!([]));
                state
                    .publish_snapshot(
                        ObservableStateTopic::SessionSnapshot,
                        format!("trigger/{}/events", args.session_id),
                        payload,
                    )
                    .await;
                Ok(serde_json::to_string_pretty(&list)?)
            })
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registry_contains_high_frequency_commands() {
        let registry = BuiltinCommandRegistry::new();
        let keys = registry.keys();

        assert!(keys.contains(&"system.status".to_string()));
        assert!(keys.contains(&"bridge.status".to_string()));
        assert!(keys.contains(&"trigger.list".to_string()));
    }
}



