use anyhow::Result;
use autoloop_state_adapter::StateStore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AclEffect {
    Allow,
    Deny,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AclRule {
    pub id: String,
    pub actor_prefix: String,
    pub action: String,
    pub namespace_prefix: String,
    pub sensitivity: String,
    pub effect: AclEffect,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AclPolicyBundle {
    pub policy_version: String,
    pub tenant_id: String,
    pub rules: Vec<AclRule>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AclDecision {
    pub allowed: bool,
    pub matched_rule_id: Option<String>,
    pub policy_version: String,
    pub reason: String,
}

pub struct FineGrainedAclPolicy;

impl FineGrainedAclPolicy {
    pub async fn evaluate(
        db: &StateStore,
        tenant_id: &str,
        actor: &str,
        action: &str,
        namespace: &str,
        sensitivity: &str,
    ) -> Result<AclDecision> {
        let policy = load_bundle(db, tenant_id).await?;
        for rule in &policy.rules {
            if !actor.starts_with(&rule.actor_prefix) {
                continue;
            }
            if !action.eq_ignore_ascii_case(&rule.action) {
                continue;
            }
            if !namespace.starts_with(&rule.namespace_prefix) {
                continue;
            }
            if !matches_sensitivity(&rule.sensitivity, sensitivity) {
                continue;
            }
            return Ok(AclDecision {
                allowed: matches!(rule.effect, AclEffect::Allow),
                matched_rule_id: Some(rule.id.clone()),
                policy_version: policy.policy_version.clone(),
                reason: format!("matched acl rule {}", rule.id),
            });
        }

        Ok(AclDecision {
            allowed: false,
            matched_rule_id: None,
            policy_version: policy.policy_version,
            reason: "no acl rule matched".to_string(),
        })
    }
}

fn matches_sensitivity(rule: &str, input: &str) -> bool {
    if rule.eq_ignore_ascii_case("any") {
        return true;
    }
    rule.eq_ignore_ascii_case(input)
}

async fn load_bundle(db: &StateStore, tenant_id: &str) -> Result<AclPolicyBundle> {
    let key = format!("memory:acl:policy:{tenant_id}:latest");
    if let Some(record) = db.get_knowledge(&key).await? {
        if let Ok(bundle) = serde_json::from_str::<AclPolicyBundle>(&record.value) {
            return Ok(bundle);
        }
    }
    Ok(default_bundle(tenant_id))
}

fn default_bundle(tenant_id: &str) -> AclPolicyBundle {
    AclPolicyBundle {
        policy_version: "acl-v1".to_string(),
        tenant_id: tenant_id.to_string(),
        rules: vec![
            AclRule {
                id: "allow-operator-read".to_string(),
                actor_prefix: "principal:".to_string(),
                action: "read".to_string(),
                namespace_prefix: "memory:".to_string(),
                sensitivity: "any".to_string(),
                effect: AclEffect::Allow,
            },
            AclRule {
                id: "allow-operator-write-low".to_string(),
                actor_prefix: "principal:".to_string(),
                action: "write".to_string(),
                namespace_prefix: "memory:".to_string(),
                sensitivity: "low".to_string(),
                effect: AclEffect::Allow,
            },
            AclRule {
                id: "deny-delete-high-default".to_string(),
                actor_prefix: "principal:".to_string(),
                action: "delete".to_string(),
                namespace_prefix: "memory:".to_string(),
                sensitivity: "high".to_string(),
                effect: AclEffect::Deny,
            },
        ],
    }
}

