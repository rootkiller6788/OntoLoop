use anyhow::Result;
use autoloop_state_adapter::StateStore;

use crate::contracts::org::{OrganizationContext, QuotaSnapshot};

#[derive(Clone)]
pub struct OrganizationContextResolver {
    state_store: StateStore,
}

impl OrganizationContextResolver {
    pub fn new(state_store: StateStore) -> Self {
        Self { state_store }
    }

    pub async fn resolve(&self, session_id: &str) -> Result<OrganizationContext> {
        let lease = self.state_store.get_session_lease(session_id).await?;
        let (tenant_id, principal_id, policy_id) = match lease {
            Some(lease) => (lease.tenant_id, lease.principal_id, lease.policy_id),
            None => (
                "tenant-default".to_string(),
                format!("principal:{session_id}"),
                "policy-default".to_string(),
            ),
        };

        let role = self
            .state_store
            .get_role_binding(&tenant_id, &principal_id)
            .await?
            .map(|binding| binding.role)
            .unwrap_or_else(|| "operator".to_string());

        let policy = self
            .state_store
            .get_policy_binding(&tenant_id, &policy_id)
            .await?;

        let account = self
            .state_store
            .get_budget_account(&tenant_id, &principal_id)
            .await?;

        let quotas = match (policy.clone(), account) {
            (Some(policy), Some(account)) => QuotaSnapshot {
                account_id: Some(account.account_id),
                total_budget_micros: account.total_budget_micros,
                reserved_micros: account.reserved_micros,
                spent_micros: account.spent_micros,
                blocked_count: account.blocked_count,
                max_tokens: policy.max_tokens,
                max_memory_mb: policy.max_memory_mb,
            },
            (Some(policy), None) => QuotaSnapshot {
                account_id: None,
                total_budget_micros: 0,
                reserved_micros: 0,
                spent_micros: 0,
                blocked_count: 0,
                max_tokens: policy.max_tokens,
                max_memory_mb: policy.max_memory_mb,
            },
            (None, Some(account)) => QuotaSnapshot {
                account_id: Some(account.account_id),
                total_budget_micros: account.total_budget_micros,
                reserved_micros: account.reserved_micros,
                spent_micros: account.spent_micros,
                blocked_count: account.blocked_count,
                max_tokens: 0,
                max_memory_mb: 0,
            },
            (None, None) => QuotaSnapshot::default(),
        };

        let kb_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("kb:{tenant_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(16)
            .collect::<Vec<_>>();

        let plaza_refs = self
            .state_store
            .list_knowledge_by_prefix(&format!("plaza:{tenant_id}:"))
            .await?
            .into_iter()
            .map(|record| record.key)
            .take(16)
            .collect::<Vec<_>>();

        let mut metadata = std::collections::BTreeMap::new();
        metadata.insert(
            "context_source".to_string(),
            "organization-context-resolver".to_string(),
        );
        metadata.insert("tenant_scope".to_string(), tenant_id.clone());
        metadata.insert("role".to_string(), role.clone());

        Ok(OrganizationContext {
            session_id: session_id.to_string(),
            tenant_id,
            principal_id,
            policy_id,
            role,
            approval_policy: policy
                .as_ref()
                .map(|p| {
                    if p.allowed_actions.iter().any(|action| {
                        action == &autoloop_state_adapter::PermissionAction::Admin
                    }) {
                        "approval_required_for_high_risk".to_string()
                    } else {
                        "auto_approve_low_risk".to_string()
                    }
                })
                .unwrap_or_else(|| "policy-missing-default-deny".to_string()),
            kb_refs,
            plaza_refs,
            quotas,
            metadata,
        })
    }
}

#[async_trait::async_trait]
impl crate::contracts::ports::OrganizationContextInjector for OrganizationContextResolver {
    async fn inject_context(
        &self,
        session_id: &crate::contracts::ids::SessionId,
    ) -> Result<OrganizationContext, crate::contracts::errors::ContractError> {
        self.resolve(session_id.as_ref())
            .await
            .map_err(|error| crate::contracts::errors::ContractError::Storage(error.to_string()))
    }
}

