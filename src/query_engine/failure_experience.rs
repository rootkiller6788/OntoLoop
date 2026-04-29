use std::collections::{HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use anyhow::Result;
use autoloop_state_adapter::StateStore;

use super::{FailureCategory, RepairStrategy};

const HINT_BLOCK_START: &str = "[FailureExperienceHints]";
const HINT_BLOCK_END: &str = "[EndFailureExperienceHints]";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FailureExperienceEntry {
    pub entry_id: String,
    pub session_id: String,
    pub trace_id: String,
    pub signature: String,
    pub stage: String,
    pub failure_category: FailureCategory,
    pub repair_strategy: RepairStrategy,
    pub recipe_hint: String,
    pub requires_manual_approval: bool,
    pub retry_recommended: bool,
    pub evidence_ref: Option<String>,
    pub created_at_ms: u64,
}

pub async fn record_failure_experience(
    db: &StateStore,
    session_id: &str,
    trace_id: &str,
    stage: &str,
    category: &FailureCategory,
    strategy: &RepairStrategy,
    error_message: &str,
    evidence_ref: Option<&str>,
) -> Result<String> {
    let created_at_ms = current_time_ms();
    let signature = build_error_signature(stage, category, error_message);
    let (recipe_hint, requires_manual_approval, retry_recommended) =
        build_recipe(stage, category, strategy);
    let entry_id = format!("failure-exp:{session_id}:{signature}:{created_at_ms}");
    let entry = FailureExperienceEntry {
        entry_id: entry_id.clone(),
        session_id: session_id.to_string(),
        trace_id: trace_id.to_string(),
        signature: signature.clone(),
        stage: stage.to_string(),
        failure_category: category.clone(),
        repair_strategy: strategy.clone(),
        recipe_hint,
        requires_manual_approval,
        retry_recommended,
        evidence_ref: evidence_ref.map(str::to_string),
        created_at_ms,
    };
    let entry_key = format!("failure-experience:entry:{session_id}:{created_at_ms}:{signature}");
    let latest_key = format!("failure-experience:latest:{session_id}:{signature}");
    db.upsert_json_knowledge(entry_key, &entry, "failure-experience-library")
        .await?;
    db.upsert_json_knowledge(latest_key, &entry, "failure-experience-library")
        .await?;
    Ok(entry_id)
}

pub async fn load_failure_experience_hints(
    db: &StateStore,
    session_id: &str,
    limit: usize,
) -> Result<Vec<FailureExperienceEntry>> {
    let mut entries = db
        .list_knowledge_by_prefix(&format!("failure-experience:entry:{session_id}:"))
        .await?
        .into_iter()
        .filter_map(|record| serde_json::from_str::<FailureExperienceEntry>(&record.value).ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.created_at_ms);
    entries.reverse();

    let mut dedup = HashSet::<String>::new();
    let mut hints = Vec::<FailureExperienceEntry>::new();
    for entry in entries {
        if dedup.insert(entry.signature.clone()) {
            hints.push(entry);
        }
        if hints.len() >= limit.max(1) {
            break;
        }
    }
    hints.sort_by_key(|entry| entry.created_at_ms);
    Ok(hints)
}

pub fn merge_failure_experience_hints(
    content: &str,
    hints: &[FailureExperienceEntry],
) -> String {
    if hints.is_empty() {
        return strip_hint_block(content).trim().to_string();
    }
    let base = strip_hint_block(content).trim().to_string();
    let mut block = String::new();
    block.push_str(HINT_BLOCK_START);
    block.push('\n');
    for hint in hints {
        let privilege = if hint.requires_manual_approval {
            "manual_approval_required"
        } else {
            "no_extra_privilege"
        };
        block.push_str(
            format!(
                "- signature={}\n  stage={}\n  category={:?}\n  strategy={:?}\n  retry_recommended={}\n  privilege={}\n  recipe={}\n",
                hint.signature,
                hint.stage,
                hint.failure_category,
                hint.repair_strategy,
                hint.retry_recommended,
                privilege,
                hint.recipe_hint
            )
            .as_str(),
        );
    }
    block.push_str(HINT_BLOCK_END);
    format!("{block}\n\n{base}").trim().to_string()
}

fn build_error_signature(
    stage: &str,
    category: &FailureCategory,
    error_message: &str,
) -> String {
    let normalized = normalize_error(error_message);
    let core = format!(
        "{}|{:?}|{}",
        stage.to_ascii_lowercase(),
        category,
        normalized
    );
    let mut hasher = DefaultHasher::new();
    core.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn build_recipe(
    stage: &str,
    category: &FailureCategory,
    strategy: &RepairStrategy,
) -> (String, bool, bool) {
    match category {
        FailureCategory::Compile => (
            format!(
                "Run build-only pass for `{stage}`, patch the first compiler error, then re-run build before any test stage."
            ),
            false,
            true,
        ),
        FailureCategory::Test => (
            format!(
                "Keep code unchanged where possible; isolate failing tests in `{stage}`, fix assertions/contracts, then run required test suite again."
            ),
            false,
            true,
        ),
        FailureCategory::Tool => (
            format!(
                "Narrow tool scope in `{stage}`, validate input arguments, and retry once with bounded side effects."
            ),
            false,
            true,
        ),
        FailureCategory::Budget => (
            "Apply compact/replan first, preserve artifact delivery constraints, and execute only the highest-value path."
                .to_string(),
            false,
            true,
        ),
        FailureCategory::Timeout => (
            "Reduce payload/parallelism, keep deterministic steps, then retry with bounded timeout.".to_string(),
            false,
            true,
        ),
        FailureCategory::Permission | FailureCategory::Policy => (
            "Do not execute privileged operations. Prepare approval request with evidence refs and wait for explicit authorization."
                .to_string(),
            true,
            false,
        ),
        FailureCategory::Unknown => (
            format!(
                "Classify unknown failure in `{stage}`, collect exact stderr signature, then apply `{strategy:?}` conservatively."
            ),
            false,
            true,
        ),
    }
}

fn normalize_error(error_message: &str) -> String {
    let mut result = String::new();
    let mut previous_space = false;
    for ch in error_message.chars().flat_map(char::to_lowercase).take(240) {
        let normalized = if ch.is_ascii_alphanumeric() { ch } else { ' ' };
        if normalized == ' ' {
            if previous_space {
                continue;
            }
            previous_space = true;
            result.push(' ');
            continue;
        }
        previous_space = false;
        result.push(normalized);
    }
    result.trim().to_string()
}

fn strip_hint_block(content: &str) -> String {
    let start = content.find(HINT_BLOCK_START);
    let end = content.find(HINT_BLOCK_END);
    match (start, end) {
        (Some(s), Some(e)) if e >= s => {
            let head = content[..s].trim();
            let tail = content[e + HINT_BLOCK_END.len()..].trim();
            if head.is_empty() {
                tail.to_string()
            } else if tail.is_empty() {
                head.to_string()
            } else {
                format!("{head}\n\n{tail}")
            }
        }
        _ => content.to_string(),
    }
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

    #[test]
    fn merge_hints_replaces_old_block_and_keeps_content() {
        let hint = FailureExperienceEntry {
            entry_id: "id-1".into(),
            session_id: "s1".into(),
            trace_id: "t1".into(),
            signature: "sig".into(),
            stage: "test_verifier".into(),
            failure_category: FailureCategory::Test,
            repair_strategy: RepairStrategy::Retest,
            recipe_hint: "re-run tests".into(),
            requires_manual_approval: false,
            retry_recommended: true,
            evidence_ref: None,
            created_at_ms: 1,
        };
        let content = "[FailureExperienceHints]\nold\n[EndFailureExperienceHints]\n\ndo work";
        let merged = merge_failure_experience_hints(content, &[hint]);
        assert!(merged.contains("signature=sig"));
        assert!(merged.contains("do work"));
        assert_eq!(merged.matches(HINT_BLOCK_START).count(), 1);
    }

    #[test]
    fn policy_failure_recipe_never_auto_escalates() {
        let (_, manual, retry) = build_recipe(
            "shell_loop",
            &FailureCategory::Policy,
            &RepairStrategy::EscalatePolicy,
        );
        assert!(manual);
        assert!(!retry);
    }

    #[tokio::test]
    async fn record_and_load_failure_experience_roundtrip() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let id = record_failure_experience(
            &db,
            "session-exp",
            "trace-exp",
            "test_verifier",
            &FailureCategory::Test,
            &RepairStrategy::Retest,
            "assertion failed in unit test",
            Some("evidence:stage:session-exp:trace-exp:1"),
        )
        .await
        .expect("record");
        assert!(id.starts_with("failure-exp:session-exp:"));
        let hints = load_failure_experience_hints(&db, "session-exp", 3)
            .await
            .expect("load");
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].stage, "test_verifier");
        assert!(hints[0].retry_recommended);
    }
}
