pub mod collector;
pub mod event_stream;
pub mod policy_signal;
pub mod query_plane;
pub mod signal_facade;
mod signal_pipeline;

pub use signal_facade::SignalFacade;

use anyhow::Result;
use autoloop_state_adapter::StateStore;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    config::{DeploymentConfig, ObservabilityConfig},
    contracts::version::CONTRACT_VERSION,
    observability::event_stream::append_event,
    orchestration::governance_telemetry_scope::GovernanceTelemetryScope,
    orchestration::{ExecutionReport, SwarmOutcome},
    runtime::FailoverRecord,
    tools::CapabilityLifecycleReport,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEnvelope {
    pub session_id: String,
    pub span_name: String,
    pub level: String,
    pub detail: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteAnalyticsRecord {
    pub session_id: String,
    pub total_reports: usize,
    pub treatment_share: f32,
    pub guarded_reports: usize,
    pub top_tools: Vec<String>,
    pub top_servers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureForensicsRecord {
    pub session_id: String,
    pub failing_tasks: Vec<String>,
    pub blocked_tools: Vec<String>,
    pub approval_gated_tools: Vec<String>,
    pub primary_failure_mode: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub session_id: String,
    pub route_analytics: RouteAnalyticsRecord,
    pub failure_forensics: FailureForensicsRecord,
    pub cost_report: CostReportRecord,
    pub validation_ready: bool,
    pub verifier_score: f32,
    pub capability_failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanFacingObservability {
    pub session_id: String,
    pub dashboard: DashboardSnapshot,
    pub operations_report: OperationsReport,
    pub resilience_report: ResilienceReport,
    pub report_channels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemTelemetrySnapshot {
    pub session_id: String,
    pub route_quality_score: f32,
    pub verifier_failure_rate: f32,
    pub capability_trust_decay: f32,
    pub promotion_success_rate: f32,
    pub open_circuit_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupermemoryMetricsRecord {
    pub session_id: String,
    pub ingestion_count: usize,
    pub chunk_count: usize,
    pub atomic_count: usize,
    pub relation_count: usize,
    pub retrieval_hits: usize,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationsReport {
    pub session_id: String,
    pub session_summary: String,
    pub task_summary: Vec<String>,
    pub capability_summary: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductOpsSnapshot {
    pub session_id: String,
    pub dashboard_endpoint_hint: String,
    pub capability_lifecycle: CapabilityLifecycleReport,
    pub verifier_queue_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostReportRecord {
    pub session_id: String,
    pub task_count: usize,
    pub token_cost_micros: u64,
    pub tool_cost_micros: u64,
    pub duration_cost_micros: u64,
    pub total_cost_micros: u64,
    pub reconciliation_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResilienceReport {
    pub session_id: String,
    pub mttr_ms: Option<u64>,
    pub degrade_success_rate: f32,
    pub key_task_availability: f32,
    pub failover_events: usize,
    pub recovered_events: usize,
    pub manual_takeover_events: usize,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceTier {
    Basic,
    Standard,
    Premium,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkOrderStatus {
    Accepted,
    Delivered,
    Rejected,
    SlaBreached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkOrder {
    pub work_order_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub task_role: String,
    pub capability_id: Option<String>,
    pub service_tier: ServiceTier,
    pub status: WorkOrderStatus,
    pub accepted_at_ms: u64,
    pub delivered_at_ms: Option<u64>,
    pub acceptance_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueEvent {
    pub revenue_event_id: String,
    pub work_order_id: String,
    pub session_id: String,
    pub task_id: String,
    pub service_tier: ServiceTier,
    pub revenue_micros: u64,
    pub cost_micros: u64,
    pub profit_micros: i64,
    pub recognized_at_ms: u64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarginReport {
    pub session_id: String,
    pub recognized_revenue_micros: u64,
    pub allocated_cost_micros: u64,
    pub gross_profit_micros: i64,
    pub gross_margin_ratio: f32,
    pub negative_margin_tasks: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SLAReport {
    pub session_id: String,
    pub delivered_orders: usize,
    pub breached_orders: usize,
    pub sla_success_ratio: f32,
    pub breach_tasks: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessReport {
    pub session_id: String,
    pub work_orders: Vec<WorkOrder>,
    pub revenue_events: Vec<RevenueEvent>,
    pub margin: MarginReport,
    pub sla: SLAReport,
    pub risk_summary: String,
}

#[derive(Debug, Clone)]
pub struct ObservabilityKernel {
    config: ObservabilityConfig,
    deployment: DeploymentConfig,
}

impl ObservabilityKernel {
    pub fn from_config(config: &ObservabilityConfig, deployment: &DeploymentConfig) -> Self {
        Self {
            config: config.clone(),
            deployment: deployment.clone(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.config.enabled && self.config.report_top_k == 0 {
            anyhow::bail!("observability.report_top_k must be greater than 0");
        }
        Ok(())
    }

    pub async fn persist_supermemory_metrics(
        &self,
        db: &StateStore,
        session_id: &str,
        retrieval_hits: usize,
    ) -> Result<SupermemoryMetricsRecord> {
        let ingestion_count = count_non_latest_records(
            db.list_knowledge_by_prefix(&format!("memory:supermemory:queue:{session_id}:"))
                .await?
                .iter()
                .map(|record| record.key.clone())
                .collect::<Vec<_>>(),
        );
        let chunk_count = count_non_latest_records(
            db.list_knowledge_by_prefix(&format!("memory:supermemory:chunks:{session_id}:"))
                .await?
                .iter()
                .map(|record| record.key.clone())
                .collect::<Vec<_>>(),
        );
        let atomic_count = count_non_latest_records(
            db.list_knowledge_by_prefix(&format!("memory:supermemory:atomic:{session_id}:"))
                .await?
                .iter()
                .map(|record| record.key.clone())
                .collect::<Vec<_>>(),
        );
        let relation_count = count_non_latest_records(
            db.list_knowledge_by_prefix(&format!("memory:supermemory:relations:{session_id}:"))
                .await?
                .iter()
                .map(|record| record.key.clone())
                .collect::<Vec<_>>(),
        );

        let metrics = SupermemoryMetricsRecord {
            session_id: session_id.to_string(),
            ingestion_count,
            chunk_count,
            atomic_count,
            relation_count,
            retrieval_hits,
            generated_at_ms: crate::orchestration::current_time_ms(),
        };

        db.upsert_json_knowledge(
            format!("observability:{session_id}:supermemory-metrics"),
            &metrics,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("metrics:supermemory:{session_id}:latest"),
            &metrics,
            "observability",
        )
        .await?;

        Ok(metrics)
    }
    pub async fn persist_swarm_observability(
        &self,
        db: &StateStore,
        session_id: &str,
        governance_scope: &GovernanceTelemetryScope,
        outcome: &SwarmOutcome,
        capability_lifecycle: &CapabilityLifecycleReport,
        verifier_queue_depth: usize,
    ) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let route_analytics = self.route_analytics(session_id, &outcome.execution_reports);
        let failure_forensics = self.failure_forensics(session_id, &outcome.execution_reports);
        let dashboard = DashboardSnapshot {
            session_id: session_id.to_string(),
            route_analytics: route_analytics.clone(),
            failure_forensics: failure_forensics.clone(),
            cost_report: self.cost_report(db, session_id).await?,
            validation_ready: outcome.validation.ready,
            verifier_score: outcome.verifier_report.overall_score,
            capability_failures: outcome
                .verifier_report
                .capability_regression
                .failing_tools
                .clone(),
        };
        let operations_report = self.operations_report(session_id, outcome);
        let resilience_report = self
            .resilience_report(db, session_id, &outcome.execution_reports)
            .await?;
        let business_report = self
            .business_report(
                db,
                session_id,
                &outcome.execution_reports,
                outcome.validation.ready,
            )
            .await?;
        let promotion_events = db
            .list_knowledge_by_prefix(&format!("eventlog:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| {
                serde_json::from_str::<crate::observability::event_stream::EventEnvelope>(
                    &record.value,
                )
                .ok()
            })
            .collect::<Vec<_>>();
        let promotion_total = promotion_events
            .iter()
            .filter(|event| event.kind == "verifier.promotion_decision")
            .count();
        let promotion_passed = promotion_events
            .iter()
            .filter(|event| {
                event.kind == "verifier.promotion_decision"
                    && event
                        .payload
                        .get("approved")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
            })
            .count();
        let human_view = HumanFacingObservability {
            session_id: session_id.to_string(),
            dashboard: dashboard.clone(),
            operations_report: operations_report.clone(),
            resilience_report: resilience_report.clone(),
            report_channels: vec!["dashboard".into(), "reports".into(), "replay".into()],
        };
        let total_reports = route_analytics.total_reports.max(1) as f32;
        let verifier_failure_rate =
            (route_analytics.guarded_reports as f32 / total_reports).clamp(0.0, 1.0);
        let avg_capability_health = if capability_lifecycle.entries.is_empty() {
            1.0
        } else {
            capability_lifecycle
                .entries
                .iter()
                .map(|entry| entry.average_health)
                .sum::<f32>()
                / capability_lifecycle.entries.len() as f32
        };
        let system_telemetry = SystemTelemetrySnapshot {
            session_id: session_id.to_string(),
            route_quality_score: outcome.verifier_report.overall_score.clamp(0.0, 1.0),
            verifier_failure_rate,
            capability_trust_decay: (1.0 - avg_capability_health.clamp(0.0, 1.0)).clamp(0.0, 1.0),
            promotion_success_rate: if promotion_total == 0 {
                1.0
            } else {
                promotion_passed as f32 / promotion_total as f32
            },
            open_circuit_count: failure_forensics.blocked_tools.len()
                + failure_forensics.approval_gated_tools.len(),
        };
        let telemetry_snapshot = crate::observability::collector::collect_and_persist(
            db,
            session_id,
            governance_scope,
            &outcome.execution_reports,
        )
        .await?;
        crate::observability::query_plane::persist_unified_query_view(db, session_id, None).await?;
        let product_ops = ProductOpsSnapshot {
            session_id: session_id.to_string(),
            dashboard_endpoint_hint: format!("state_store://observability/{session_id}/dashboard"),
            capability_lifecycle: capability_lifecycle.clone(),
            verifier_queue_depth,
        };

        db.upsert_json_knowledge(
            format!("observability:{session_id}:route-analytics"),
            &route_analytics,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:failure-forensics"),
            &failure_forensics,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:dashboard"),
            &dashboard,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:cost-report"),
            &dashboard.cost_report,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:operations-report"),
            &operations_report,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:resilience"),
            &resilience_report,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:business-report"),
            &business_report,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:work-orders"),
            &business_report.work_orders,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:revenue-events"),
            &business_report.revenue_events,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:margin-report"),
            &business_report.margin,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:sla-report"),
            &business_report.sla,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:product-ops"),
            &product_ops,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:human-view"),
            &human_view,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:system-telemetry"),
            &system_telemetry,
            "observability",
        )
        .await?;
        db.upsert_json_knowledge(
            format!("observability:{session_id}:telemetry-collector"),
            &telemetry_snapshot,
            "observability",
        )
        .await?;

        let trace = TraceEnvelope {
            session_id: session_id.to_string(),
            span_name: "swarm.completed".into(),
            level: if outcome.validation.ready {
                "info".into()
            } else {
                "warn".into()
            },
            detail: format!(
                "validation_ready={} verifier_score={:.2} deployment_profile={}",
                outcome.validation.ready,
                outcome.verifier_report.overall_score,
                self.deployment.profile
            ),
            created_at_ms: crate::orchestration::current_time_ms(),
        };
        db.upsert_json_knowledge(
            format!("observability:{session_id}:trace:{}", trace.created_at_ms),
            &trace,
            "observability",
        )
        .await?;

        info!(
            session_id = session_id,
            verifier_score = outcome.verifier_report.overall_score,
            validation_ready = outcome.validation.ready,
            "persisted swarm observability snapshot"
        );
        if !failure_forensics.failing_tasks.is_empty() {
            warn!(
                session_id = session_id,
                failure_mode = failure_forensics.primary_failure_mode,
                "failure forensics detected"
            );
        }

        Ok(())
    }

    async fn business_report(
        &self,
        db: &StateStore,
        session_id: &str,
        reports: &[ExecutionReport],
        validation_ready: bool,
    ) -> Result<BusinessReport> {
        let now = crate::orchestration::current_time_ms();
        let lease = db.get_session_lease(session_id).await?;
        let tenant_id = lease
            .as_ref()
            .map(|item| item.tenant_id.clone())
            .unwrap_or_else(|| "tenant:default".into());
        let costs = db
            .list_cost_attribution_by_session(&tenant_id, session_id)
            .await
            .unwrap_or_default();
        let mut cost_by_task = std::collections::HashMap::<String, u64>::new();
        for cost in costs {
            *cost_by_task.entry(cost.task_id).or_insert(0) += cost.total_cost_micros;
        }

        let mut work_orders = Vec::new();
        let mut revenue_events = Vec::new();
        let mut breach_tasks = Vec::new();
        let mut negative_margin_tasks = Vec::new();

        for report in reports {
            let tier = service_tier_for_report(report);
            let cost_micros = *cost_by_task.get(&report.task.task_id).unwrap_or(&0);
            let revenue_micros = estimate_revenue_micros(report, &tier, validation_ready);
            let profit_micros = revenue_micros as i64 - cost_micros as i64;
            let trace_id = format!("trace:{session_id}:{}", report.task.task_id);
            let capability_id = report
                .tool_used
                .clone()
                .unwrap_or_else(|| "provider".into());
            let status = if report.guard_decision.eq_ignore_ascii_case("blocked") {
                breach_tasks.push(report.task.task_id.clone());
                WorkOrderStatus::Rejected
            } else if is_sla_breach(report, &tier) {
                breach_tasks.push(report.task.task_id.clone());
                WorkOrderStatus::SlaBreached
            } else {
                WorkOrderStatus::Delivered
            };
            if profit_micros < 0 {
                negative_margin_tasks.push(report.task.task_id.clone());
            }
            let work_order = WorkOrder {
                work_order_id: format!("wo:{session_id}:{}", report.task.task_id),
                session_id: session_id.to_string(),
                trace_id: trace_id.clone(),
                task_id: report.task.task_id.clone(),
                task_role: report.task.role.clone(),
                capability_id: report.tool_used.clone(),
                service_tier: tier.clone(),
                status: status.clone(),
                accepted_at_ms: now,
                delivered_at_ms: Some(now),
                acceptance_note: report.guard_decision.clone(),
            };
            let revenue_event = RevenueEvent {
                revenue_event_id: format!("rev:{session_id}:{}", report.task.task_id),
                work_order_id: work_order.work_order_id.clone(),
                session_id: session_id.to_string(),
                task_id: report.task.task_id.clone(),
                service_tier: tier,
                revenue_micros,
                cost_micros,
                profit_micros,
                recognized_at_ms: now,
                source: report
                    .tool_used
                    .clone()
                    .unwrap_or_else(|| "provider".into()),
            };
            let _ = append_event(
                db,
                "workorder.accepted",
                trace_id.clone(),
                session_id.to_string(),
                Some(report.task.task_id.clone()),
                Some(capability_id.clone()),
                CONTRACT_VERSION,
                serde_json::json!({
                    "work_order_id": work_order.work_order_id.clone(),
                    "task_role": report.task.role.clone(),
                    "service_tier": work_order.service_tier.clone(),
                    "status": "accepted",
                    "accepted_at_ms": work_order.accepted_at_ms,
                }),
            )
            .await;
            let _ = append_event(
                db,
                "workorder.delivered",
                trace_id.clone(),
                session_id.to_string(),
                Some(report.task.task_id.clone()),
                Some(capability_id.clone()),
                CONTRACT_VERSION,
                serde_json::json!({
                    "work_order_id": work_order.work_order_id.clone(),
                    "status": work_order.status.clone(),
                    "delivered_at_ms": work_order.delivered_at_ms,
                    "acceptance_note": work_order.acceptance_note.clone(),
                }),
            )
            .await;
            let _ = append_event(
                db,
                "revenue.recognized",
                trace_id.clone(),
                session_id.to_string(),
                Some(report.task.task_id.clone()),
                Some(capability_id),
                CONTRACT_VERSION,
                serde_json::json!({
                    "revenue_event_id": revenue_event.revenue_event_id.clone(),
                    "work_order_id": revenue_event.work_order_id.clone(),
                    "revenue_micros": revenue_event.revenue_micros,
                    "cost_micros": revenue_event.cost_micros,
                    "profit_micros": revenue_event.profit_micros,
                }),
            )
            .await;

            db.upsert_json_knowledge(
                format!("business:work-order:{session_id}:{}", report.task.task_id),
                &work_order,
                "business",
            )
            .await?;
            db.upsert_json_knowledge(
                format!(
                    "business:revenue-event:{session_id}:{}",
                    report.task.task_id
                ),
                &revenue_event,
                "business",
            )
            .await?;
            work_orders.push(work_order);
            revenue_events.push(revenue_event);
        }

        let recognized_revenue_micros = revenue_events
            .iter()
            .map(|event| event.revenue_micros)
            .sum::<u64>();
        let allocated_cost_micros = revenue_events
            .iter()
            .map(|event| event.cost_micros)
            .sum::<u64>();
        let gross_profit_micros = recognized_revenue_micros as i64 - allocated_cost_micros as i64;
        let gross_margin_ratio = if recognized_revenue_micros == 0 {
            0.0
        } else {
            gross_profit_micros as f32 / recognized_revenue_micros as f32
        };
        let delivered_orders = work_orders
            .iter()
            .filter(|order| matches!(order.status, WorkOrderStatus::Delivered))
            .count();
        let breached_orders = work_orders
            .iter()
            .filter(|order| {
                matches!(
                    order.status,
                    WorkOrderStatus::SlaBreached | WorkOrderStatus::Rejected
                )
            })
            .count();
        let sla_success_ratio = if work_orders.is_empty() {
            1.0
        } else {
            delivered_orders as f32 / work_orders.len() as f32
        };
        let margin = MarginReport {
            session_id: session_id.to_string(),
            recognized_revenue_micros,
            allocated_cost_micros,
            gross_profit_micros,
            gross_margin_ratio,
            negative_margin_tasks: negative_margin_tasks.clone(),
            summary: format!(
                "revenue={} cost={} profit={} margin={:.2}",
                recognized_revenue_micros,
                allocated_cost_micros,
                gross_profit_micros,
                gross_margin_ratio
            ),
        };
        let sla = SLAReport {
            session_id: session_id.to_string(),
            delivered_orders,
            breached_orders,
            sla_success_ratio,
            breach_tasks: breach_tasks.clone(),
            summary: format!(
                "delivered={} breached={} sla_success={:.2}",
                delivered_orders, breached_orders, sla_success_ratio
            ),
        };
        let report_trace_id = format!("trace:{session_id}:business-summary");
        let _ = append_event(
            db,
            "margin.reported",
            report_trace_id.clone(),
            session_id.to_string(),
            None,
            None,
            CONTRACT_VERSION,
            serde_json::json!({
                "recognized_revenue_micros": margin.recognized_revenue_micros,
                "allocated_cost_micros": margin.allocated_cost_micros,
                "gross_profit_micros": margin.gross_profit_micros,
                "gross_margin_ratio": margin.gross_margin_ratio,
                "negative_margin_tasks": margin.negative_margin_tasks.clone(),
            }),
        )
        .await;
        let _ = append_event(
            db,
            "sla.reported",
            report_trace_id,
            session_id.to_string(),
            None,
            None,
            CONTRACT_VERSION,
            serde_json::json!({
                "delivered_orders": sla.delivered_orders,
                "breached_orders": sla.breached_orders,
                "sla_success_ratio": sla.sla_success_ratio,
                "breach_tasks": sla.breach_tasks.clone(),
            }),
        )
        .await;
        if breached_orders > 0 || gross_margin_ratio < 0.0 {
            db.upsert_json_knowledge(
                format!("business:automation:{session_id}:{now}"),
                &serde_json::json!({
                    "session_id": session_id,
                    "action": "operator_attention",
                    "reason": if breached_orders > 0 {
                        "sla_breach"
                    } else {
                        "negative_margin"
                    },
                    "breach_tasks": breach_tasks,
                    "negative_margin_tasks": negative_margin_tasks,
                }),
                "business",
            )
            .await?;
        }
        let risk_summary = if breached_orders > 0 {
            format!("SLA breaches detected on {} tasks", breached_orders)
        } else if gross_margin_micros_below_zero(gross_profit_micros) {
            "Negative gross profit detected".into()
        } else {
            "business risk within target bounds".into()
        };
        Ok(BusinessReport {
            session_id: session_id.to_string(),
            work_orders,
            revenue_events,
            margin,
            sla,
            risk_summary,
        })
    }

    async fn resilience_report(
        &self,
        db: &StateStore,
        session_id: &str,
        reports: &[ExecutionReport],
    ) -> Result<ResilienceReport> {
        let mut failovers = db
            .list_knowledge_by_prefix(&format!("runtime:failover:{session_id}:"))
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<FailoverRecord>(&record.value).ok())
            .collect::<Vec<_>>();
        failovers.sort_by_key(|record| record.started_at_ms);
        let failover_events = failovers.len();
        let recovered = failovers.iter().filter(|record| record.recovered).count();
        let manual_takeover_events = failovers
            .iter()
            .filter(|record| record.profile == crate::runtime::DegradeProfileKind::ManualTakeover)
            .count();
        let mttr_ms = {
            let samples = failovers
                .iter()
                .filter_map(|record| record.mttr_ms)
                .collect::<Vec<_>>();
            if samples.is_empty() {
                None
            } else {
                Some(samples.iter().sum::<u64>() / samples.len() as u64)
            }
        };
        let degrade_success_rate = if failover_events == 0 {
            1.0
        } else {
            recovered as f32 / failover_events as f32
        };
        let key_total = reports
            .iter()
            .filter(|report| {
                report.task.role.eq_ignore_ascii_case("execution")
                    || report.task.role.eq_ignore_ascii_case("security")
            })
            .count();
        let key_success = reports
            .iter()
            .filter(|report| {
                (report.task.role.eq_ignore_ascii_case("execution")
                    || report.task.role.eq_ignore_ascii_case("security"))
                    && report.outcome_score > 0
            })
            .count();
        let key_task_availability = if key_total == 0 {
            1.0
        } else {
            key_success as f32 / key_total as f32
        };

        Ok(ResilienceReport {
            session_id: session_id.to_string(),
            mttr_ms,
            degrade_success_rate,
            key_task_availability,
            failover_events,
            recovered_events: recovered,
            manual_takeover_events,
            summary: format!(
                "failovers={} recovered={} mttr_ms={:?} key_availability={:.2}",
                failover_events, recovered, mttr_ms, key_task_availability
            ),
        })
    }

    async fn cost_report(&self, db: &StateStore, session_id: &str) -> Result<CostReportRecord> {
        let lease = db.get_session_lease(session_id).await?;
        let (tenant_id, account_id) = if let Some(lease) = lease {
            let account_id = format!(
                "{}:{}:{}",
                lease.tenant_id, lease.principal_id, lease.policy_id
            );
            (lease.tenant_id, account_id)
        } else {
            (
                "tenant:default".into(),
                "tenant:default:principal:unknown:policy:default".into(),
            )
        };
        let costs = db
            .list_cost_attribution_by_session(&tenant_id, session_id)
            .await
            .unwrap_or_default();
        let (token_cost_micros, tool_cost_micros, duration_cost_micros, total_cost_micros) =
            costs.iter().fold((0u64, 0u64, 0u64, 0u64), |acc, item| {
                (
                    acc.0.saturating_add(item.token_cost_micros),
                    acc.1.saturating_add(item.tool_cost_micros),
                    acc.2.saturating_add(item.duration_cost_micros),
                    acc.3.saturating_add(item.total_cost_micros),
                )
            });
        let ledger_entries = db
            .list_spend_ledger(&tenant_id, &account_id)
            .await
            .unwrap_or_default();
        let ledger_settled = ledger_entries
            .iter()
            .filter(|entry| entry.kind == autoloop_state_adapter::SpendLedgerKind::Settle)
            .map(|entry| entry.amount_micros.max(0) as u64)
            .sum::<u64>();
        let reconciliation_ok = ledger_settled >= total_cost_micros;
        Ok(CostReportRecord {
            session_id: session_id.to_string(),
            task_count: costs.len(),
            token_cost_micros,
            tool_cost_micros,
            duration_cost_micros,
            total_cost_micros,
            reconciliation_ok,
        })
    }

    fn route_analytics(
        &self,
        session_id: &str,
        reports: &[ExecutionReport],
    ) -> RouteAnalyticsRecord {
        let total_reports = reports.len();
        let treatment_count = reports
            .iter()
            .filter(|report| report.route_variant == "treatment")
            .count();
        let guarded_reports = reports
            .iter()
            .filter(|report| !report.guard_decision.eq_ignore_ascii_case("allow"))
            .count();
        let mut top_tools = reports
            .iter()
            .filter_map(|report| report.tool_used.clone())
            .collect::<Vec<_>>();
        top_tools.sort();
        top_tools.dedup();
        top_tools.truncate(self.config.report_top_k);
        let mut top_servers = reports
            .iter()
            .filter_map(|report| report.mcp_server.clone())
            .collect::<Vec<_>>();
        top_servers.sort();
        top_servers.dedup();
        top_servers.truncate(self.config.report_top_k);

        RouteAnalyticsRecord {
            session_id: session_id.to_string(),
            total_reports,
            treatment_share: if total_reports == 0 {
                0.0
            } else {
                treatment_count as f32 / total_reports as f32
            },
            guarded_reports,
            top_tools,
            top_servers,
        }
    }

    fn failure_forensics(
        &self,
        session_id: &str,
        reports: &[ExecutionReport],
    ) -> FailureForensicsRecord {
        let failing_tasks = reports
            .iter()
            .filter(|report| report.outcome_score <= 0)
            .map(|report| format!("{}:{}", report.task.role, report.task.objective))
            .collect::<Vec<_>>();
        let blocked_tools = reports
            .iter()
            .filter(|report| report.guard_decision.eq_ignore_ascii_case("blocked"))
            .filter_map(|report| report.tool_used.clone())
            .collect::<Vec<_>>();
        let approval_gated_tools = reports
            .iter()
            .filter(|report| {
                report
                    .guard_decision
                    .eq_ignore_ascii_case("requiresapproval")
            })
            .filter_map(|report| report.tool_used.clone())
            .collect::<Vec<_>>();
        let primary_failure_mode = if !blocked_tools.is_empty() {
            "runtime-guard-blocked"
        } else if !approval_gated_tools.is_empty() {
            "approval-gated"
        } else if !failing_tasks.is_empty() {
            "execution-regression"
        } else {
            "none"
        };

        FailureForensicsRecord {
            session_id: session_id.to_string(),
            summary: format!(
                "failure_mode={} failing_tasks={} blocked_tools={} approval_gated={}",
                primary_failure_mode,
                failing_tasks.len(),
                blocked_tools.len(),
                approval_gated_tools.len()
            ),
            failing_tasks,
            blocked_tools,
            approval_gated_tools,
            primary_failure_mode: primary_failure_mode.into(),
        }
    }

    fn operations_report(&self, session_id: &str, outcome: &SwarmOutcome) -> OperationsReport {
        OperationsReport {
            session_id: session_id.to_string(),
            session_summary: format!(
                "validation_ready={} verifier={:?} tasks={} profile={}",
                outcome.validation.ready,
                outcome.verifier_report.verdict,
                outcome.tasks.len(),
                self.deployment.profile
            ),
            task_summary: outcome
                .tasks
                .iter()
                .map(|task| format!("{} -> {}", task.role, task.objective))
                .collect(),
            capability_summary: outcome
                .verifier_report
                .capability_regression
                .cases
                .iter()
                .map(|case| {
                    format!(
                        "{} v{} status={} approval={} health={:.2}",
                        case.tool_name,
                        case.version,
                        case.status,
                        case.approval_status,
                        case.health_score
                    )
                })
                .take(self.config.report_top_k)
                .collect(),
        }
    }
}

fn count_non_latest_records(keys: Vec<String>) -> usize {
    keys.into_iter()
        .filter(|key| !key.ends_with(":latest"))
        .count()
}
fn service_tier_for_report(report: &ExecutionReport) -> ServiceTier {
    if report.task.role.eq_ignore_ascii_case("security") {
        ServiceTier::Premium
    } else if report.task.role.eq_ignore_ascii_case("execution") {
        ServiceTier::Standard
    } else {
        ServiceTier::Basic
    }
}

fn estimate_revenue_micros(
    report: &ExecutionReport,
    tier: &ServiceTier,
    validation_ready: bool,
) -> u64 {
    let base = match tier {
        ServiceTier::Basic => 6_000,
        ServiceTier::Standard => 12_000,
        ServiceTier::Premium => 20_000,
    };
    let outcome_bonus = report.outcome_score.max(0) as u64 * 800;
    let validation_bonus = if validation_ready { 2_000 } else { 0 };
    base + outcome_bonus + validation_bonus
}

fn is_sla_breach(report: &ExecutionReport, tier: &ServiceTier) -> bool {
    if report.guard_decision.eq_ignore_ascii_case("blocked")
        || report
            .guard_decision
            .eq_ignore_ascii_case("requiresapproval")
    {
        return true;
    }
    let threshold = match tier {
        ServiceTier::Basic => -1,
        ServiceTier::Standard => 0,
        ServiceTier::Premium => 1,
    };
    report.outcome_score <= threshold
}

fn gross_margin_micros_below_zero(value: i64) -> bool {
    value < 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::{SwarmTask, ValidationReport};

    fn report(task_id: &str, role: &str, outcome_score: i32, guard: &str) -> ExecutionReport {
        ExecutionReport {
            task: SwarmTask {
                task_id: task_id.into(),
                agent_name: format!("{role}-agent"),
                role: role.into(),
                objective: format!("{role} objective"),
                depends_on: vec![],
            },
            output: "ok".into(),
            tool_used: Some("mcp::local-mcp::invoke".into()),
            mcp_server: Some("local-mcp".into()),
            invocation_payload: Some("{}".into()),
            outcome_score,
            route_variant: "control".into(),
            control_score: outcome_score,
            treatment_score: outcome_score,
            guard_decision: guard.into(),
        }
    }

    #[tokio::test]
    async fn p13_order_lifecycle_and_revenue_mapping_is_consistent() {
        let kernel = ObservabilityKernel::from_config(
            &crate::config::AppConfig::default().observability,
            &crate::config::AppConfig::default().deployment,
        );
        let db = autoloop_state_adapter::StateStore::from_config(
            &autoloop_state_adapter::StateStoreConfig {
                enabled: true,
                backend: autoloop_state_adapter::StateStoreBackend::InMemory,
                uri: "http://state_store:3000".into(),
                module_name: "autoloop_core".into(),
                namespace: "autoloop".into(),
                pool_size: 4,
            },
        );
        let reports = vec![
            report("task-1", "Execution", 3, "Allow"),
            report("task-2", "Security", 2, "Allow"),
        ];
        let business = kernel
            .business_report(&db, "session-p13-orders", &reports, true)
            .await
            .expect("business");
        assert_eq!(business.work_orders.len(), 2);
        assert_eq!(business.revenue_events.len(), 2);
        assert!(business.margin.recognized_revenue_micros > 0);
    }

    #[tokio::test]
    async fn p13_sla_breach_is_detected_and_reported() {
        let kernel = ObservabilityKernel::from_config(
            &crate::config::AppConfig::default().observability,
            &crate::config::AppConfig::default().deployment,
        );
        let db = autoloop_state_adapter::StateStore::from_config(
            &autoloop_state_adapter::StateStoreConfig {
                enabled: true,
                backend: autoloop_state_adapter::StateStoreBackend::InMemory,
                uri: "http://state_store:3000".into(),
                module_name: "autoloop_core".into(),
                namespace: "autoloop".into(),
                pool_size: 4,
            },
        );
        let reports = vec![
            report("task-breach", "Security", -3, "Blocked"),
            report("task-ok", "Execution", 2, "Allow"),
        ];
        let business = kernel
            .business_report(&db, "session-p13-sla", &reports, false)
            .await
            .expect("business");
        assert!(business.sla.breached_orders >= 1);
        assert!(
            business
                .sla
                .breach_tasks
                .iter()
                .any(|task| task == "task-breach")
        );
    }

    #[tokio::test]
    async fn p13_revenue_attribution_matches_cost_and_profit_breakdown() {
        let kernel = ObservabilityKernel::from_config(
            &crate::config::AppConfig::default().observability,
            &crate::config::AppConfig::default().deployment,
        );
        let db = autoloop_state_adapter::StateStore::from_config(
            &autoloop_state_adapter::StateStoreConfig {
                enabled: true,
                backend: autoloop_state_adapter::StateStoreBackend::InMemory,
                uri: "http://state_store:3000".into(),
                module_name: "autoloop_core".into(),
                namespace: "autoloop".into(),
                pool_size: 4,
            },
        );
        let now = crate::orchestration::current_time_ms();
        db.upsert_cost_attribution(autoloop_state_adapter::CostAttribution {
            attribution_id: "attr-task-cost".into(),
            tenant_id: "tenant:default".into(),
            principal_id: "principal:test".into(),
            policy_id: "policy:test".into(),
            session_id: "session-p13-revenue".into(),
            trace_id: "trace:session-p13-revenue:task-cost".into(),
            task_id: "task-cost".into(),
            capability_id: "mcp::local-mcp::invoke".into(),
            provider_tokens: 120,
            tool_invocations: 1,
            duration_ms: 95,
            token_cost_micros: 4_000,
            tool_cost_micros: 2_000,
            duration_cost_micros: 1_000,
            total_cost_micros: 7_000,
            settled_at_ms: now,
        })
        .await
        .expect("seed attribution");

        let reports = vec![report("task-cost", "Execution", 2, "Allow")];
        let business = kernel
            .business_report(&db, "session-p13-revenue", &reports, true)
            .await
            .expect("business");
        let event = business.revenue_events.first().expect("revenue event");
        assert_eq!(event.cost_micros, 7_000);
        assert_eq!(business.margin.allocated_cost_micros, 7_000);
        assert_eq!(
            business.margin.gross_profit_micros,
            business.margin.recognized_revenue_micros as i64 - 7_000
        );
    }

    #[tokio::test]
    async fn p13_persist_swarm_observability_writes_business_reports() {
        let config = crate::config::AppConfig::default();
        let kernel = ObservabilityKernel::from_config(&config.observability, &config.deployment);
        let db = autoloop_state_adapter::StateStore::from_config(
            &autoloop_state_adapter::StateStoreConfig {
                enabled: true,
                backend: autoloop_state_adapter::StateStoreBackend::InMemory,
                uri: "http://state_store:3000".into(),
                module_name: "autoloop_core".into(),
                namespace: "autoloop".into(),
                pool_size: 4,
            },
        );
        let outcome = SwarmOutcome {
            brief: crate::orchestration::RequirementBrief {
                anchor_id: "anchor:session-p13".into(),
                original_request: "build".into(),
                clarified_goal: "build".into(),
                frozen_scope: "scope".into(),
                open_questions: vec![],
                acceptance_criteria: vec![],
                clarification_turns: vec![],
                confirmation_required: false,
            },
            optimization_proposal: crate::providers::OptimizationProposal {
                title: "t".into(),
                change_target: "c".into(),
                hypothesis: "h".into(),
                expected_gain: "g".into(),
                risk: "r".into(),
                patch_outline: vec![],
                evaluation_focus: "e".into(),
            },
            routing_context: crate::orchestration::RoutingContext {
                history_records: vec![],
                execution_metrics: vec![],
                graph_signals: Default::default(),
                pending_event_count: 0,
                learning_evidence: vec![],
                skill_success_rate: 0.0,
                causal_confidence: 0.0,
                forged_tool_coverage: 0,
                session_ab_stats: None,
                task_ab_stats: Default::default(),
                tool_ab_stats: Default::default(),
                server_ab_stats: Default::default(),
                agent_reputations: Default::default(),
                route_biases: vec![],
            },
            ceo_summary: "summary".into(),
            deliberation: crate::orchestration::SwarmDeliberation {
                planner_notes: "p".into(),
                critic_notes: "c".into(),
                planner_rebuttal: "r".into(),
                judge_notes: "j".into(),
                arbitration_summary: "a".into(),
                round_count: 1,
                rounds: vec![],
                final_execution_order: vec![],
                consensus_signals: vec![],
            },
            tasks: vec![],
            execution_reports: vec![report("task-1", "Execution", 2, "Allow")],
            verifier_report: crate::runtime::VerifierReport {
                verifier_name: "v".into(),
                verdict: crate::runtime::VerifierVerdict::Pass,
                overall_score: 0.9,
                summary: "ok".into(),
                task_judgements: vec![],
                route_reports: vec![],
                capability_regression: crate::runtime::CapabilityRegressionSuite {
                    suite_name: "suite".into(),
                    all_passed: true,
                    score: 1.0,
                    failing_tools: vec![],
                    cases: vec![],
                    summary: "ok".into(),
                },
                recommended_actions: vec![],
            },
            validation: ValidationReport {
                ready: true,
                summary: "ok".into(),
                follow_up_tasks: vec![],
                verifier_summary: "ok".into(),
            },
            knowledge_update: crate::rag::GraphKnowledgeUpdate {
                document_id: 1,
                local_context_summary: "l".into(),
                global_context_summary: "g".into(),
                task_capability_map_summary: "t".into(),
                snapshot_json: "{}".into(),
            },
        };

        kernel
            .persist_swarm_observability(
                &db,
                "session-p13",
                &crate::orchestration::governance_telemetry_scope::GovernanceTelemetryScope {
                    scope_id: "gov-scope:session-p13".into(),
                    session_id: "session-p13".into(),
                    tenant_scope: "tenant-default".into(),
                    risk_tier: "low".into(),
                    privacy_level: "internal".into(),
                    approval_required: false,
                    retention_hours: 168,
                    redaction_fields: vec![],
                },
                &outcome,
                &crate::tools::CapabilityLifecycleReport {
                    total_lineages: 0,
                    active_capabilities: 0,
                    deprecated_capabilities: 0,
                    rollback_ready_capabilities: 0,
                    entries: vec![],
                },
                0,
            )
            .await
            .expect("persist");

        assert!(
            db.get_knowledge("observability:session-p13:business-report")
                .await
                .expect("read")
                .is_some()
        );
        assert!(
            db.get_knowledge("observability:session-p13:margin-report")
                .await
                .expect("read")
                .is_some()
        );
        assert!(
            db.get_knowledge("observability:session-p13:sla-report")
                .await
                .expect("read")
                .is_some()
        );
        assert!(
            db.get_knowledge("observability:session-p13:work-orders")
                .await
                .expect("read")
                .is_some()
        );
        assert!(
            db.get_knowledge("observability:session-p13:revenue-events")
                .await
                .expect("read")
                .is_some()
        );
    }
}

