use serde::{Deserialize, Serialize};
use state_store::{DbContext, ReducerContext, Table};

#[derive(Clone, Debug, Serialize, Deserialize, state_store::SpacetimeType)]
pub enum PermissionAction {
    Read,
    Write,
    Dispatch,
    Admin,
}

#[derive(Clone, Debug, Serialize, Deserialize, state_store::SpacetimeType)]
pub enum LearningEventKind {
    Failure,
    Success,
    ToolCall,
    RouteDecision,
    Audit,
}

#[state_store::table(accessor = permission_grant, public)]
pub struct PermissionGrant {
    #[primary_key]
    pub actor_id: String,
    pub permissions: Vec<PermissionAction>,
}

#[state_store::table(accessor = schedule_event, public)]
pub struct ScheduleEvent {
    #[primary_key]
    pub id: u64,
    pub session_id: String,
    pub topic: String,
    pub tool_name: String,
    pub payload: String,
    pub actor_id: String,
    pub status: String,
}

#[state_store::table(accessor = agent_state, public)]
pub struct AgentState {
    #[primary_key]
    pub session_id: String,
    pub last_user_message: String,
    pub last_assistant_message: Option<String>,
}

#[state_store::table(accessor = knowledge_record, public)]
pub struct KnowledgeRecord {
    #[primary_key]
    pub key: String,
    pub value: String,
    pub source: String,
}

#[state_store::table(accessor = reflexion_episode, public)]
pub struct ReflexionEpisode {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub hypothesis: String,
    pub outcome: String,
    pub lesson: String,
    pub status: String,
    pub score: f32,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = skill_library_record, public)]
pub struct SkillLibraryRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub name: String,
    pub trigger: String,
    pub procedure: String,
    pub confidence: f32,
    pub success_rate: f32,
    pub evidence_count: u32,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[state_store::table(accessor = causal_edge_record, public)]
pub struct CausalEdgeRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub cause: String,
    pub effect: String,
    pub evidence: String,
    pub strength: f32,
    pub confidence: f32,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = learning_session_record, public)]
pub struct LearningSessionRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub objective: String,
    pub status: String,
    pub priority: f32,
    pub summary: String,
    pub started_at_ms: u64,
    pub completed_at_ms: Option<u64>,
}

#[state_store::table(accessor = witness_log_record, public)]
pub struct WitnessLogRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub event_type: LearningEventKind,
    pub source: String,
    pub detail: String,
    pub score: f32,
    pub created_at_ms: u64,
    pub metadata_json: String,
}

#[state_store::table(accessor = session_record, public)]
pub struct SessionRecord {
    #[primary_key]
    pub session_id: String,
    pub trace_id: String,
    pub status: String,
    pub updated_at_ms: u64,
}

#[state_store::table(accessor = state_transition_record, public)]
pub struct StateTransitionRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub from_state: String,
    pub signal: String,
    pub to_state: String,
    pub reason: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = policy_decision_record, public)]
pub struct PolicyDecisionRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub decision: String,
    pub reason: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = plan_record, public)]
pub struct PlanRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub objective: String,
    pub payload: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = task_run_record, public)]
pub struct TaskRunRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub status: String,
    pub output: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = runtime_block_record, public)]
pub struct RuntimeBlockRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub reason: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = verifier_verdict_record, public)]
pub struct VerifierVerdictRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub verdict: String,
    pub score: f32,
    pub summary: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = learning_delta_record, public)]
pub struct LearningDeltaRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub delta_json: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = report_record, public)]
pub struct ReportRecord {
    #[primary_key]
    pub id: String,
    pub session_id: String,
    pub trace_id: String,
    pub report_type: String,
    pub payload: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = budget_account_record, public)]
pub struct BudgetAccountRecord {
    #[primary_key]
    pub account_key: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub total_budget_micros: u64,
    pub reserved_micros: u64,
    pub spent_micros: u64,
    pub blocked_count: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, state_store::SpacetimeType)]
pub enum SpendLedgerKind {
    Reserve,
    Settle,
    Refund,
    Blocked,
}

#[state_store::table(accessor = spend_ledger_record, public)]
pub struct SpendLedgerRecord {
    #[primary_key]
    pub id: String,
    pub tenant_id: String,
    pub account_key: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub kind: SpendLedgerKind,
    pub amount_micros: i64,
    pub token_cost_micros: u64,
    pub tool_cost_micros: u64,
    pub duration_cost_micros: u64,
    pub reason: String,
    pub created_at_ms: u64,
}

#[state_store::table(accessor = quota_window_record, public)]
pub struct QuotaWindowRecord {
    #[primary_key]
    pub window_key: String,
    pub tenant_id: String,
    pub account_key: String,
    pub window_start_ms: u64,
    pub window_end_ms: u64,
    pub window_budget_micros: u64,
    pub consumed_micros: u64,
    pub blocked_count: u64,
    pub updated_at_ms: u64,
}

#[state_store::table(accessor = cost_attribution_record, public)]
pub struct CostAttributionRecord {
    #[primary_key]
    pub id: String,
    pub tenant_id: String,
    pub principal_id: String,
    pub policy_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub task_id: String,
    pub capability_id: String,
    pub provider_tokens: u32,
    pub tool_invocations: u32,
    pub duration_ms: u64,
    pub token_cost_micros: u64,
    pub tool_cost_micros: u64,
    pub duration_cost_micros: u64,
    pub total_cost_micros: u64,
    pub settled_at_ms: u64,
}

#[state_store::reducer(init)]
pub fn init(_ctx: &ReducerContext) {}

#[state_store::reducer]
pub fn grant_permissions(
    ctx: &ReducerContext,
    actor_id: String,
    permissions: Vec<PermissionAction>,
) -> Result<(), String> {
    let row = PermissionGrant {
        actor_id: actor_id.clone(),
        permissions,
    };

    if ctx.db().permission_grant().actor_id().find(&actor_id).is_some() {
        ctx.db().permission_grant().actor_id().update(row);
    } else {
        ctx.db().permission_grant().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn create_schedule_event(
    ctx: &ReducerContext,
    id: u64,
    session_id: String,
    topic: String,
    tool_name: String,
    payload: String,
    actor_id: String,
    status: String,
) -> Result<(), String> {
    if ctx.db().schedule_event().id().find(&id).is_some() {
        return Err(format!("schedule event {id} already exists"));
    }

    ctx.db().schedule_event().insert(ScheduleEvent {
        id,
        session_id,
        topic,
        tool_name,
        payload,
        actor_id,
        status,
    });

    Ok(())
}

#[state_store::reducer]
pub fn update_schedule_status(
    ctx: &ReducerContext,
    id: u64,
    status: String,
) -> Result<(), String> {
    let mut row = ctx
        .db()
        .schedule_event()
        .id()
        .find(&id)
        .ok_or_else(|| format!("schedule event {id} not found"))?;

    row.status = status;
    ctx.db().schedule_event().id().update(row);

    Ok(())
}

#[state_store::reducer]
pub fn upsert_agent_state(
    ctx: &ReducerContext,
    session_id: String,
    last_user_message: String,
    last_assistant_message: Option<String>,
) -> Result<(), String> {
    let row = AgentState {
        session_id: session_id.clone(),
        last_user_message,
        last_assistant_message,
    };

    if ctx.db().agent_state().session_id().find(&session_id).is_some() {
        ctx.db().agent_state().session_id().update(row);
    } else {
        ctx.db().agent_state().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn upsert_knowledge(
    ctx: &ReducerContext,
    key: String,
    value: String,
    source: String,
) -> Result<(), String> {
    let row = KnowledgeRecord {
        key: key.clone(),
        value,
        source,
    };

    if ctx.db().knowledge_record().key().find(&key).is_some() {
        ctx.db().knowledge_record().key().update(row);
    } else {
        ctx.db().knowledge_record().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn upsert_reflexion_episode(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    objective: String,
    hypothesis: String,
    outcome: String,
    lesson: String,
    status: String,
    score: f32,
    created_at_ms: u64,
) -> Result<(), String> {
    let row = ReflexionEpisode {
        id: id.clone(),
        session_id,
        objective,
        hypothesis,
        outcome,
        lesson,
        status,
        score,
        created_at_ms,
    };

    if ctx.db().reflexion_episode().id().find(&id).is_some() {
        ctx.db().reflexion_episode().id().update(row);
    } else {
        ctx.db().reflexion_episode().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn upsert_skill_library_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    name: String,
    trigger: String,
    procedure: String,
    confidence: f32,
    success_rate: f32,
    evidence_count: u32,
    created_at_ms: u64,
    updated_at_ms: u64,
) -> Result<(), String> {
    let row = SkillLibraryRecord {
        id: id.clone(),
        session_id,
        name,
        trigger,
        procedure,
        confidence,
        success_rate,
        evidence_count,
        created_at_ms,
        updated_at_ms,
    };

    if ctx.db().skill_library_record().id().find(&id).is_some() {
        ctx.db().skill_library_record().id().update(row);
    } else {
        ctx.db().skill_library_record().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn upsert_causal_edge_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    cause: String,
    effect: String,
    evidence: String,
    strength: f32,
    confidence: f32,
    created_at_ms: u64,
) -> Result<(), String> {
    let row = CausalEdgeRecord {
        id: id.clone(),
        session_id,
        cause,
        effect,
        evidence,
        strength,
        confidence,
        created_at_ms,
    };

    if ctx.db().causal_edge_record().id().find(&id).is_some() {
        ctx.db().causal_edge_record().id().update(row);
    } else {
        ctx.db().causal_edge_record().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn upsert_learning_session_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    objective: String,
    status: String,
    priority: f32,
    summary: String,
    started_at_ms: u64,
    completed_at_ms: Option<u64>,
) -> Result<(), String> {
    let row = LearningSessionRecord {
        id: id.clone(),
        session_id,
        objective,
        status,
        priority,
        summary,
        started_at_ms,
        completed_at_ms,
    };

    if ctx.db().learning_session_record().id().find(&id).is_some() {
        ctx.db().learning_session_record().id().update(row);
    } else {
        ctx.db().learning_session_record().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn append_witness_log_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    event_type: LearningEventKind,
    source: String,
    detail: String,
    score: f32,
    created_at_ms: u64,
    metadata_json: String,
) -> Result<(), String> {
    let row = WitnessLogRecord {
        id: id.clone(),
        session_id,
        event_type,
        source,
        detail,
        score,
        created_at_ms,
        metadata_json,
    };

    if ctx.db().witness_log_record().id().find(&id).is_some() {
        ctx.db().witness_log_record().id().update(row);
    } else {
        ctx.db().witness_log_record().insert(row);
    }

    Ok(())
}

#[state_store::reducer]
pub fn upsert_session_record(
    ctx: &ReducerContext,
    session_id: String,
    trace_id: String,
    status: String,
    updated_at_ms: u64,
) -> Result<(), String> {
    let row = SessionRecord {
        session_id: session_id.clone(),
        trace_id,
        status,
        updated_at_ms,
    };
    if ctx.db().session_record().session_id().find(&session_id).is_some() {
        ctx.db().session_record().session_id().update(row);
    } else {
        ctx.db().session_record().insert(row);
    }
    Ok(())
}

#[state_store::reducer]
pub fn append_state_transition_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    from_state: String,
    signal: String,
    to_state: String,
    reason: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().state_transition_record().id().find(&id).is_some() {
        return Err(format!("state transition record {id} already exists"));
    }
    ctx.db().state_transition_record().insert(StateTransitionRecord {
        id,
        session_id,
        trace_id,
        from_state,
        signal,
        to_state,
        reason,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn append_policy_decision_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    decision: String,
    reason: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().policy_decision_record().id().find(&id).is_some() {
        return Err(format!("policy decision record {id} already exists"));
    }
    ctx.db().policy_decision_record().insert(PolicyDecisionRecord {
        id,
        session_id,
        trace_id,
        decision,
        reason,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn append_plan_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    objective: String,
    payload: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().plan_record().id().find(&id).is_some() {
        return Err(format!("plan record {id} already exists"));
    }
    ctx.db().plan_record().insert(PlanRecord {
        id,
        session_id,
        trace_id,
        objective,
        payload,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn append_task_run_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    task_id: String,
    capability_id: String,
    status: String,
    output: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().task_run_record().id().find(&id).is_some() {
        return Err(format!("task run record {id} already exists"));
    }
    ctx.db().task_run_record().insert(TaskRunRecord {
        id,
        session_id,
        trace_id,
        task_id,
        capability_id,
        status,
        output,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn append_runtime_block_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    task_id: String,
    capability_id: String,
    reason: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().runtime_block_record().id().find(&id).is_some() {
        return Err(format!("runtime block record {id} already exists"));
    }
    ctx.db().runtime_block_record().insert(RuntimeBlockRecord {
        id,
        session_id,
        trace_id,
        task_id,
        capability_id,
        reason,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn append_verifier_verdict_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    verdict: String,
    score: f32,
    summary: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().verifier_verdict_record().id().find(&id).is_some() {
        return Err(format!("verifier verdict record {id} already exists"));
    }
    ctx.db().verifier_verdict_record().insert(VerifierVerdictRecord {
        id,
        session_id,
        trace_id,
        verdict,
        score,
        summary,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn append_learning_delta_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    delta_json: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().learning_delta_record().id().find(&id).is_some() {
        return Err(format!("learning delta record {id} already exists"));
    }
    ctx.db().learning_delta_record().insert(LearningDeltaRecord {
        id,
        session_id,
        trace_id,
        delta_json,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn append_report_record(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    trace_id: String,
    report_type: String,
    payload: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().report_record().id().find(&id).is_some() {
        return Err(format!("report record {id} already exists"));
    }
    ctx.db().report_record().insert(ReportRecord {
        id,
        session_id,
        trace_id,
        report_type,
        payload,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn upsert_budget_account_record(
    ctx: &ReducerContext,
    account_key: String,
    tenant_id: String,
    principal_id: String,
    policy_id: String,
    total_budget_micros: u64,
    reserved_micros: u64,
    spent_micros: u64,
    blocked_count: u64,
    updated_at_ms: u64,
) -> Result<(), String> {
    let row = BudgetAccountRecord {
        account_key: account_key.clone(),
        tenant_id,
        principal_id,
        policy_id,
        total_budget_micros,
        reserved_micros,
        spent_micros,
        blocked_count,
        updated_at_ms,
    };
    if ctx.db().budget_account_record().account_key().find(&account_key).is_some() {
        ctx.db().budget_account_record().account_key().update(row);
    } else {
        ctx.db().budget_account_record().insert(row);
    }
    Ok(())
}

#[state_store::reducer]
pub fn append_spend_ledger_record(
    ctx: &ReducerContext,
    id: String,
    tenant_id: String,
    account_key: String,
    session_id: String,
    trace_id: String,
    task_id: String,
    capability_id: String,
    kind: SpendLedgerKind,
    amount_micros: i64,
    token_cost_micros: u64,
    tool_cost_micros: u64,
    duration_cost_micros: u64,
    reason: String,
    created_at_ms: u64,
) -> Result<(), String> {
    if ctx.db().spend_ledger_record().id().find(&id).is_some() {
        return Err(format!("spend ledger record {id} already exists"));
    }
    ctx.db().spend_ledger_record().insert(SpendLedgerRecord {
        id,
        tenant_id,
        account_key,
        session_id,
        trace_id,
        task_id,
        capability_id,
        kind,
        amount_micros,
        token_cost_micros,
        tool_cost_micros,
        duration_cost_micros,
        reason,
        created_at_ms,
    });
    Ok(())
}

#[state_store::reducer]
pub fn upsert_quota_window_record(
    ctx: &ReducerContext,
    window_key: String,
    tenant_id: String,
    account_key: String,
    window_start_ms: u64,
    window_end_ms: u64,
    window_budget_micros: u64,
    consumed_micros: u64,
    blocked_count: u64,
    updated_at_ms: u64,
) -> Result<(), String> {
    let row = QuotaWindowRecord {
        window_key: window_key.clone(),
        tenant_id,
        account_key,
        window_start_ms,
        window_end_ms,
        window_budget_micros,
        consumed_micros,
        blocked_count,
        updated_at_ms,
    };
    if ctx.db().quota_window_record().window_key().find(&window_key).is_some() {
        ctx.db().quota_window_record().window_key().update(row);
    } else {
        ctx.db().quota_window_record().insert(row);
    }
    Ok(())
}

#[state_store::reducer]
pub fn upsert_cost_attribution_record(
    ctx: &ReducerContext,
    id: String,
    tenant_id: String,
    principal_id: String,
    policy_id: String,
    session_id: String,
    trace_id: String,
    task_id: String,
    capability_id: String,
    provider_tokens: u32,
    tool_invocations: u32,
    duration_ms: u64,
    token_cost_micros: u64,
    tool_cost_micros: u64,
    duration_cost_micros: u64,
    total_cost_micros: u64,
    settled_at_ms: u64,
) -> Result<(), String> {
    let row = CostAttributionRecord {
        id: id.clone(),
        tenant_id,
        principal_id,
        policy_id,
        session_id,
        trace_id,
        task_id,
        capability_id,
        provider_tokens,
        tool_invocations,
        duration_ms,
        token_cost_micros,
        tool_cost_micros,
        duration_cost_micros,
        total_cost_micros,
        settled_at_ms,
    };
    if ctx.db().cost_attribution_record().id().find(&id).is_some() {
        ctx.db().cost_attribution_record().id().update(row);
    } else {
        ctx.db().cost_attribution_record().insert(row);
    }
    Ok(())
}

