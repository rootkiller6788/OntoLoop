use anyhow::Result;
use async_trait::async_trait;
use autoloop_state_adapter::StateStore;

use crate::contracts::events::{
    OntoEvent, OntoEventBridgeInput, OntoEventSourceKind, bridge_into_onto_event,
};
use crate::contracts::org::{BranchLease, WorkPackage};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AdapterSessionHandle {
    pub adapter: String,
    pub session_id: String,
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct AdapterExecutionResult {
    pub adapter: String,
    pub session_id: String,
    pub task_node_id: String,
    pub status: String,
    pub output: serde_json::Value,
    pub onto_events: Vec<OntoEvent>,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SessionLifecycleLease {
    pub branch_lease: BranchLease,
    pub session_ttl_ms: u64,
    pub token_budget: u64,
    pub budget_micros: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SessionLifecycleState {
    pub adapter: String,
    pub session_id: String,
    pub trace_id: String,
    pub started_at_ms: u64,
    pub expires_at_ms: u64,
    pub token_budget: u64,
    pub token_used: u64,
    pub budget_micros: u64,
    pub budget_spent_micros: u64,
    pub gate_required: bool,
    pub gate_last_decision: String,
    pub gate_last_reason: String,
    pub active: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SessionLifecycleReport {
    pub session_id: String,
    pub restored: bool,
    pub active: bool,
    pub gate_required: bool,
    pub gate_decision: String,
    pub gate_reason: String,
    pub evidence_ref: String,
}

#[derive(Clone)]
pub struct PersistentSessionDaemon {
    db: StateStore,
}

impl PersistentSessionDaemon {
    pub fn new(db: StateStore) -> Self {
        Self { db }
    }

    pub async fn start_or_restore_session(
        &self,
        adapter: &str,
        session_id: &str,
        trace_id: &str,
        lease: &SessionLifecycleLease,
    ) -> Result<SessionLifecycleReport> {
        let now = current_time_ms();
        if let Some(record) = self.session_state(session_id).await? {
            if record.active && now <= record.expires_at_ms {
                let evidence_ref =
                    format!("evidence:session-daemon:{session_id}:restored:{now}");
                self.emit_lifecycle_event(
                    session_id,
                    trace_id,
                    "session_restored",
                    &evidence_ref,
                    serde_json::json!({
                        "adapter": adapter,
                        "active": record.active,
                        "expires_at_ms": record.expires_at_ms,
                    }),
                )
                .await?;
                return Ok(SessionLifecycleReport {
                    session_id: session_id.to_string(),
                    restored: true,
                    active: true,
                    gate_required: record.gate_required,
                    gate_decision: "allow".to_string(),
                    gate_reason: "session_restored".to_string(),
                    evidence_ref,
                });
            }
        }

        let requested_expires_at_ms = now.saturating_add(lease.session_ttl_ms.max(10_000));
        let expires_at_ms = requested_expires_at_ms.min(lease.branch_lease.expires_at_ms);
        let active = expires_at_ms > now;
        let state = SessionLifecycleState {
            adapter: adapter.to_string(),
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            started_at_ms: now,
            expires_at_ms,
            token_budget: lease.token_budget,
            token_used: 0,
            budget_micros: lease.budget_micros,
            budget_spent_micros: 0,
            gate_required: true,
            gate_last_decision: if active {
                "allow".to_string()
            } else {
                "block".to_string()
            },
            gate_last_reason: if active {
                "session_started".to_string()
            } else {
                "lease_expired".to_string()
            },
            active,
        };
        self.persist_state(&state).await?;
        let evidence_ref = format!("evidence:session-daemon:{session_id}:started:{now}");
        self.emit_lifecycle_event(
            session_id,
            trace_id,
            "session_started",
            &evidence_ref,
            serde_json::json!({
                "adapter": adapter,
                "token_budget": lease.token_budget,
                "budget_micros": lease.budget_micros,
                "expires_at_ms": expires_at_ms,
            }),
        )
        .await?;
        Ok(SessionLifecycleReport {
            session_id: session_id.to_string(),
            restored: false,
            active,
            gate_required: true,
            gate_decision: if active {
                "allow".to_string()
            } else {
                "block".to_string()
            },
            gate_reason: if active {
                "session_started".to_string()
            } else {
                "lease_expired".to_string()
            },
            evidence_ref,
        })
    }

    pub async fn admit_execution_or_block(
        &self,
        session_id: &str,
        trace_id: &str,
        requested_tokens: u64,
        estimated_micros: u64,
    ) -> Result<SessionLifecycleReport> {
        let now = current_time_ms();
        let mut state = self
            .session_state(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("persistent session not found: {session_id}"))?;

        let (decision, reason) = if !state.active {
            ("block".to_string(), "session_inactive".to_string())
        } else if now > state.expires_at_ms {
            ("block".to_string(), "lease_expired".to_string())
        } else if state.token_used.saturating_add(requested_tokens) > state.token_budget {
            ("block".to_string(), "token_budget_exceeded".to_string())
        } else if state.budget_spent_micros.saturating_add(estimated_micros) > state.budget_micros {
            ("block".to_string(), "budget_micros_exceeded".to_string())
        } else {
            state.token_used = state.token_used.saturating_add(requested_tokens);
            state.budget_spent_micros = state.budget_spent_micros.saturating_add(estimated_micros);
            ("allow".to_string(), "gate_admitted".to_string())
        };
        state.gate_last_decision = decision.clone();
        state.gate_last_reason = reason.clone();
        self.persist_state(&state).await?;
        let evidence_ref = format!("evidence:session-daemon:{session_id}:gate:{now}");
        self.emit_lifecycle_event(
            session_id,
            trace_id,
            if decision == "allow" {
                "gate_admitted"
            } else {
                "gate_blocked"
            },
            &evidence_ref,
            serde_json::json!({
                "decision": decision,
                "reason": reason,
                "token_used": state.token_used,
                "token_budget": state.token_budget,
                "budget_spent_micros": state.budget_spent_micros,
                "budget_micros": state.budget_micros,
                "expires_at_ms": state.expires_at_ms,
            }),
        )
        .await?;
        Ok(SessionLifecycleReport {
            session_id: session_id.to_string(),
            restored: false,
            active: state.active,
            gate_required: state.gate_required,
            gate_decision: decision,
            gate_reason: reason,
            evidence_ref,
        })
    }

    pub async fn stop_session(&self, session_id: &str, trace_id: &str) -> Result<SessionLifecycleReport> {
        let now = current_time_ms();
        let mut state = self
            .session_state(session_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("persistent session not found: {session_id}"))?;
        state.active = false;
        state.gate_last_decision = "allow".to_string();
        state.gate_last_reason = "session_stopped".to_string();
        self.persist_state(&state).await?;
        let evidence_ref = format!("evidence:session-daemon:{session_id}:stopped:{now}");
        self.emit_lifecycle_event(
            session_id,
            trace_id,
            "session_stopped",
            &evidence_ref,
            serde_json::json!({
                "active": state.active,
            }),
        )
        .await?;
        Ok(SessionLifecycleReport {
            session_id: session_id.to_string(),
            restored: false,
            active: false,
            gate_required: state.gate_required,
            gate_decision: "allow".to_string(),
            gate_reason: "session_stopped".to_string(),
            evidence_ref,
        })
    }

    pub async fn session_state(&self, session_id: &str) -> Result<Option<SessionLifecycleState>> {
        let key = format!("harness:session-daemon:{session_id}:state");
        let Some(record) = self.db.get_knowledge(&key).await? else {
            return Ok(None);
        };
        let state = serde_json::from_str::<SessionLifecycleState>(&record.value)?;
        Ok(Some(state))
    }

    async fn persist_state(&self, state: &SessionLifecycleState) -> Result<()> {
        self.db
            .upsert_json_knowledge(
                format!("harness:session-daemon:{}:state", state.session_id),
                state,
                "session-daemon",
            )
            .await?;
        Ok(())
    }

    async fn emit_lifecycle_event(
        &self,
        session_id: &str,
        trace_id: &str,
        action: &str,
        evidence_ref: &str,
        payload: serde_json::Value,
    ) -> Result<()> {
        let event = emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                task_id: None,
                agent_id: Some("session-daemon".to_string()),
                source_kind: OntoEventSourceKind::Other,
                action: action.to_string(),
                payload_ref: format!("payload:session-daemon:{session_id}:{action}"),
                evidence_ref: evidence_ref.to_string(),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "session-daemon",
        )
        .await?;
        self.db
            .upsert_json_knowledge(
                format!("harness:session-daemon:{session_id}:event:{}", event.timestamp_ms),
                &payload,
                "session-daemon",
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
pub trait ExternalCodingAgentAdapter: Send + Sync {
    async fn start_session(&self, session_id: &str) -> Result<AdapterSessionHandle>;
    async fn send_work_package(
        &self,
        session_id: &str,
        trace_id: &str,
        work_package: &WorkPackage,
    ) -> Result<Vec<OntoEvent>>;
    async fn collect_result(
        &self,
        session_id: &str,
        trace_id: &str,
        work_package: &WorkPackage,
    ) -> Result<AdapterExecutionResult>;
    async fn stop_session(&self, session_id: &str) -> Result<OntoEvent>;
}

#[derive(Clone)]
pub struct LocalBackendAdapter {
    db: StateStore,
}

impl LocalBackendAdapter {
    pub fn new(db: StateStore) -> Self {
        Self { db }
    }
}

#[derive(Clone)]
pub struct MockExternalAgentAdapter {
    db: StateStore,
}

impl MockExternalAgentAdapter {
    pub fn new(db: StateStore) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ExternalCodingAgentAdapter for LocalBackendAdapter {
    async fn start_session(&self, session_id: &str) -> Result<AdapterSessionHandle> {
        let now = current_time_ms();
        self.db
            .upsert_json_knowledge(
                format!("harness:adapter:local:{session_id}:session"),
                &serde_json::json!({
                    "adapter": "local",
                    "session_id": session_id,
                    "started_at_ms": now,
                    "status": "started",
                }),
                "external-adapter",
            )
            .await?;
        Ok(AdapterSessionHandle {
            adapter: "local".to_string(),
            session_id: session_id.to_string(),
            started_at_ms: now,
        })
    }

    async fn send_work_package(
        &self,
        session_id: &str,
        trace_id: &str,
        work_package: &WorkPackage,
    ) -> Result<Vec<OntoEvent>> {
        let started = emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                task_id: Some(work_package.task_node_id.clone()),
                agent_id: Some("local-backend".to_string()),
                source_kind: OntoEventSourceKind::Tool,
                action: "started".to_string(),
                payload_ref: format!("payload:adapter:local:{}:started", work_package.task_node_id),
                evidence_ref: format!(
                    "evidence:adapter:local:{}:{}:started",
                    session_id, work_package.task_node_id
                ),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "external-adapter",
        )
        .await?;
        let dispatched = emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                task_id: Some(work_package.task_node_id.clone()),
                agent_id: Some("local-backend".to_string()),
                source_kind: OntoEventSourceKind::Other,
                action: "work_package_dispatched".to_string(),
                payload_ref: format!(
                    "payload:adapter:local:{}:dispatch",
                    work_package.task_node_id
                ),
                evidence_ref: format!(
                    "evidence:adapter:local:{}:{}:dispatch",
                    session_id, work_package.task_node_id
                ),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "external-adapter",
        )
        .await?;
        Ok(vec![started, dispatched])
    }

    async fn collect_result(
        &self,
        session_id: &str,
        trace_id: &str,
        work_package: &WorkPackage,
    ) -> Result<AdapterExecutionResult> {
        let completed = emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                task_id: Some(work_package.task_node_id.clone()),
                agent_id: Some("local-backend".to_string()),
                source_kind: OntoEventSourceKind::Tool,
                action: "completed".to_string(),
                payload_ref: format!(
                    "payload:adapter:local:{}:completed",
                    work_package.task_node_id
                ),
                evidence_ref: format!(
                    "evidence:adapter:local:{}:{}:completed",
                    session_id, work_package.task_node_id
                ),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "external-adapter",
        )
        .await?;
        let evidence_ref = format!(
            "evidence:adapter:local:{}:{}:result",
            session_id, work_package.task_node_id
        );
        Ok(AdapterExecutionResult {
            adapter: "local".to_string(),
            session_id: session_id.to_string(),
            task_node_id: work_package.task_node_id.clone(),
            status: "ok".to_string(),
            output: serde_json::json!({
                "adapter": "local",
                "task_node_id": work_package.task_node_id,
                "summary": "local adapter simulated execution complete",
            }),
            onto_events: vec![completed],
            evidence_ref,
        })
    }

    async fn stop_session(&self, session_id: &str) -> Result<OntoEvent> {
        emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: format!("trace:{session_id}:adapter-stop"),
                task_id: None,
                agent_id: Some("local-backend".to_string()),
                source_kind: OntoEventSourceKind::Other,
                action: "session_stopped".to_string(),
                payload_ref: format!("payload:adapter:local:{session_id}:stopped"),
                evidence_ref: format!("evidence:adapter:local:{session_id}:stopped"),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "external-adapter",
        )
        .await
    }
}

#[async_trait]
impl ExternalCodingAgentAdapter for MockExternalAgentAdapter {
    async fn start_session(&self, session_id: &str) -> Result<AdapterSessionHandle> {
        let now = current_time_ms();
        self.db
            .upsert_json_knowledge(
                format!("harness:adapter:mock:{session_id}:session"),
                &serde_json::json!({
                    "adapter": "mock",
                    "session_id": session_id,
                    "started_at_ms": now,
                    "status": "started",
                }),
                "external-adapter",
            )
            .await?;
        Ok(AdapterSessionHandle {
            adapter: "mock".to_string(),
            session_id: session_id.to_string(),
            started_at_ms: now,
        })
    }

    async fn send_work_package(
        &self,
        session_id: &str,
        trace_id: &str,
        work_package: &WorkPackage,
    ) -> Result<Vec<OntoEvent>> {
        let evt = emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                task_id: Some(work_package.task_node_id.clone()),
                agent_id: Some("mock-backend".to_string()),
                source_kind: OntoEventSourceKind::Other,
                action: "work_package_buffered".to_string(),
                payload_ref: format!("payload:adapter:mock:{}:buffered", work_package.task_node_id),
                evidence_ref: format!(
                    "evidence:adapter:mock:{}:{}:buffered",
                    session_id, work_package.task_node_id
                ),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "external-adapter",
        )
        .await?;
        Ok(vec![evt])
    }

    async fn collect_result(
        &self,
        session_id: &str,
        trace_id: &str,
        work_package: &WorkPackage,
    ) -> Result<AdapterExecutionResult> {
        let evt = emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: trace_id.to_string(),
                task_id: Some(work_package.task_node_id.clone()),
                agent_id: Some("mock-backend".to_string()),
                source_kind: OntoEventSourceKind::Test,
                action: "passed".to_string(),
                payload_ref: format!("payload:adapter:mock:{}:result", work_package.task_node_id),
                evidence_ref: format!(
                    "evidence:adapter:mock:{}:{}:result",
                    session_id, work_package.task_node_id
                ),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "external-adapter",
        )
        .await?;
        Ok(AdapterExecutionResult {
            adapter: "mock".to_string(),
            session_id: session_id.to_string(),
            task_node_id: work_package.task_node_id.clone(),
            status: "ok".to_string(),
            output: serde_json::json!({
                "adapter": "mock",
                "task_node_id": work_package.task_node_id,
                "summary": "mock adapter synthetic result",
            }),
            onto_events: vec![evt],
            evidence_ref: format!(
                "evidence:adapter:mock:{}:{}:result",
                session_id, work_package.task_node_id
            ),
        })
    }

    async fn stop_session(&self, session_id: &str) -> Result<OntoEvent> {
        emit_onto_event(
            &self.db,
            OntoEventBridgeInput {
                session_id: session_id.to_string(),
                trace_id: format!("trace:{session_id}:adapter-stop"),
                task_id: None,
                agent_id: Some("mock-backend".to_string()),
                source_kind: OntoEventSourceKind::Other,
                action: "session_stopped".to_string(),
                payload_ref: format!("payload:adapter:mock:{session_id}:stopped"),
                evidence_ref: format!("evidence:adapter:mock:{session_id}:stopped"),
                wal_seq: None,
                timestamp_ms: current_time_ms(),
            },
            "external-adapter",
        )
        .await
    }
}

async fn emit_onto_event(
    db: &StateStore,
    input: OntoEventBridgeInput,
    source: &str,
) -> Result<OntoEvent> {
    let event = bridge_into_onto_event(input);
    db.upsert_json_knowledge(
        format!(
            "ontoevent:adapter:{}:{}:{}",
            event.session_id, event.event_id, event.timestamp_ms
        ),
        &event,
        source,
    )
    .await?;
    db.upsert_json_knowledge(
        format!("ontoevent:adapter:{}:latest", event.session_id),
        &event,
        source,
    )
    .await?;
    Ok(event)
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    fn in_memory_db() -> StateStore {
        StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        })
    }

    fn sample_work_package() -> WorkPackage {
        WorkPackage {
            api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
            task_node_id: "node-adapter".to_string(),
            description: "adapter e2e".to_string(),
            owned_paths: vec!["src/runtime".to_string()],
            read_only_paths: vec!["src/contracts".to_string()],
            dependencies: vec![],
            acceptance_criteria: vec!["ok".to_string()],
            local_test_command: None,
            risk_level: "low".to_string(),
        }
    }

    #[tokio::test]
    async fn local_adapter_can_start_dispatch_collect_stop_with_ontoevent_bridge() {
        let db = in_memory_db();
        let adapter = LocalBackendAdapter::new(db.clone());
        let session = "adapter-local-session";
        let trace = "trace:adapter-local";
        let wp = sample_work_package();

        let started = adapter.start_session(session).await.expect("start");
        assert_eq!(started.adapter, "local");
        let events = adapter
            .send_work_package(session, trace, &wp)
            .await
            .expect("dispatch");
        assert!(!events.is_empty());
        let result = adapter
            .collect_result(session, trace, &wp)
            .await
            .expect("collect");
        assert_eq!(result.status, "ok");
        assert!(!result.onto_events.is_empty());
        let _ = adapter.stop_session(session).await.expect("stop");

        let latest = db
            .get_knowledge(&format!("ontoevent:adapter:{session}:latest"))
            .await
            .expect("latest read")
            .expect("latest exists");
        assert!(latest.value.contains("session_stopped"));
    }

    #[tokio::test]
    async fn mock_adapter_can_run_without_external_backend() {
        let db = in_memory_db();
        let adapter = MockExternalAgentAdapter::new(db.clone());
        let session = "adapter-mock-session";
        let trace = "trace:adapter-mock";
        let wp = sample_work_package();

        adapter.start_session(session).await.expect("start");
        let events = adapter
            .send_work_package(session, trace, &wp)
            .await
            .expect("dispatch");
        assert_eq!(events.len(), 1);
        let result = adapter
            .collect_result(session, trace, &wp)
            .await
            .expect("collect");
        assert_eq!(result.adapter, "mock");
        assert_eq!(result.status, "ok");
        assert!(result.evidence_ref.contains("evidence:adapter:mock:"));
    }

    #[tokio::test]
    async fn persistent_session_daemon_supports_multi_session_restore_and_gate() {
        let db = in_memory_db();
        let daemon = PersistentSessionDaemon::new(db.clone());
        let now = current_time_ms();
        let mk_lease = |worker: &str, branch: &str| SessionLifecycleLease {
            branch_lease: BranchLease {
                api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
                branch_name: branch.to_string(),
                agent_id: worker.to_string(),
                writable_paths: vec!["src/runtime".to_string()],
                readonly_paths: vec!["src/contracts".to_string()],
                expires_at_ms: now + 120_000,
                token_budget: 1_000,
                evidence_ref: format!("evidence:lease:{worker}"),
            },
            session_ttl_ms: 120_000,
            token_budget: 1_000,
            budget_micros: 50_000,
        };

        let s1 = daemon
            .start_or_restore_session("local", "sess-a", "trace:sess-a", &mk_lease("w1", "code/agent-w1/a"))
            .await
            .expect("start s1");
        assert!(!s1.restored);
        let s2 = daemon
            .start_or_restore_session("local", "sess-b", "trace:sess-b", &mk_lease("w2", "code/agent-w2/b"))
            .await
            .expect("start s2");
        assert!(!s2.restored);

        let restored = daemon
            .start_or_restore_session("local", "sess-a", "trace:sess-a:restore", &mk_lease("w1", "code/agent-w1/a"))
            .await
            .expect("restore s1");
        assert!(restored.restored);

        let gate = daemon
            .admit_execution_or_block("sess-a", "trace:sess-a:gate", 100, 2_000)
            .await
            .expect("gate");
        assert_eq!(gate.gate_decision, "allow");
        assert_eq!(gate.gate_reason, "gate_admitted");
    }

    #[tokio::test]
    async fn persistent_session_daemon_blocks_when_lease_expired() {
        let db = in_memory_db();
        let daemon = PersistentSessionDaemon::new(db.clone());
        let now = current_time_ms();
        let lease = SessionLifecycleLease {
            branch_lease: BranchLease {
                api_version: crate::contracts::org::ORG_EXECUTION_CONTRACT_VERSION.to_string(),
                branch_name: "code/agent-wx/node-expired".to_string(),
                agent_id: "wx".to_string(),
                writable_paths: vec!["src/runtime".to_string()],
                readonly_paths: vec![],
                expires_at_ms: now + 1,
                token_budget: 100,
                evidence_ref: "evidence:lease:wx".to_string(),
            },
            session_ttl_ms: 1,
            token_budget: 100,
            budget_micros: 1_000,
        };
        daemon
            .start_or_restore_session("mock", "sess-expired", "trace:sess-expired", &lease)
            .await
            .expect("start");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let blocked = daemon
            .admit_execution_or_block("sess-expired", "trace:sess-expired:gate", 10, 100)
            .await
            .expect("gate");
        assert_eq!(blocked.gate_decision, "block");
        assert!(
            blocked.gate_reason == "lease_expired" || blocked.gate_reason == "session_inactive"
        );
    }
}
