CREATE TABLE IF NOT EXISTS public.kv_records (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    source TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_kv_records_key_prefix ON public.kv_records (key text_pattern_ops);
CREATE INDEX IF NOT EXISTS idx_kv_records_source ON public.kv_records (source);

CREATE TABLE IF NOT EXISTS public.identity_tenants (
    tenant_id TEXT PRIMARY KEY,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS public.identity_principals (
    tenant_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, principal_id)
);
CREATE TABLE IF NOT EXISTS public.identity_role_bindings (
    tenant_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, principal_id)
);
CREATE TABLE IF NOT EXISTS public.identity_policy_bindings (
    tenant_id TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, policy_id)
);
CREATE TABLE IF NOT EXISTS public.identity_session_leases (
    session_id TEXT PRIMARY KEY,
    lease_token TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    policy_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_identity_session_leases_tenant
    ON public.identity_session_leases(tenant_id);

CREATE TABLE IF NOT EXISTS public.billing_budget_accounts (
    tenant_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, account_id)
);
CREATE TABLE IF NOT EXISTS public.billing_spend_ledger (
    tenant_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    ledger_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, account_id, ledger_id)
);
CREATE INDEX IF NOT EXISTS idx_billing_spend_ledger_session_task
    ON public.billing_spend_ledger(session_id, task_id);
CREATE TABLE IF NOT EXISTS public.billing_quota_windows (
    tenant_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    window_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, account_id, window_id)
);
CREATE TABLE IF NOT EXISTS public.billing_cost_attribution (
    tenant_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    attribution_id TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, session_id, attribution_id)
);

CREATE TABLE IF NOT EXISTS public.schedule_events (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL,
    topic TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    payload TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    status TEXT NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_schedule_events_session_id
    ON public.schedule_events(session_id);
CREATE INDEX IF NOT EXISTS idx_schedule_events_status
    ON public.schedule_events(status);

CREATE TABLE IF NOT EXISTS public.agent_states (
    session_id TEXT PRIMARY KEY,
    last_user_message TEXT NOT NULL,
    last_assistant_message TEXT,
    evidence_ref TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS public.event_log (
    event_id BIGSERIAL PRIMARY KEY,
    stream_key TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    evidence_ref TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_event_log_stream_key ON public.event_log(stream_key);
CREATE INDEX IF NOT EXISTS idx_event_log_created_at ON public.event_log(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_event_log_evidence_ref ON public.event_log(evidence_ref);

ALTER TABLE public.schedule_events ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.agent_states ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.identity_tenants ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.identity_principals ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.identity_role_bindings ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.identity_policy_bindings ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.identity_session_leases ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.billing_budget_accounts ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.billing_spend_ledger ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.billing_quota_windows ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.billing_cost_attribution ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
ALTER TABLE public.event_log ADD COLUMN IF NOT EXISTS evidence_ref TEXT NOT NULL DEFAULT '';
