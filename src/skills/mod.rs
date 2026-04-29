pub mod foundry;

use anyhow::{Result, bail};
use autoloop_state_adapter::StateStore;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillManifest {
    pub skill_id: String,
    pub name: String,
    pub source: String,
    pub status: String,
    pub markdown: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillBuildArtifact {
    pub skill_id: String,
    pub build_id: String,
    pub builder: String,
    pub compiled_prompt: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillInstallRecord {
    pub record_id: String,
    pub skill_id: String,
    pub action: String,
    pub package_id: String,
    pub artifact_path: String,
    pub source: String,
    pub status: String,
    pub reason: String,
    pub created_at_ms: u64,
}

#[derive(Clone)]
pub struct SkillRegistry {
    db: StateStore,
}

impl SkillRegistry {
    pub fn new(db: StateStore) -> Self {
        Self { db }
    }

    pub async fn register(
        &self,
        skill_id: &str,
        source: &str,
        markdown: &str,
        signal: &crate::memory::LearningSignal,
    ) -> Result<SkillManifest> {
        self
            .validate_learning_signal(signal, "skill_registry.register")
            .await?;
        if skill_id.trim().is_empty() {
            bail!("skill_id cannot be empty");
        }
        let now = current_time_ms();
        let existing = self
            .db
            .get_knowledge(&format!("skills:manifest:{skill_id}"))
            .await?
            .and_then(|record| serde_json::from_str::<SkillManifest>(&record.value).ok());
        let created_at_ms = existing
            .as_ref()
            .map(|item| item.created_at_ms)
            .unwrap_or(now);
        let manifest = SkillManifest {
            skill_id: skill_id.to_string(),
            name: skill_id.to_string(),
            source: source.to_string(),
            status: "active".into(),
            markdown: markdown.to_string(),
            created_at_ms,
            updated_at_ms: now,
        };
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}"),
                &manifest,
                "skill-registry",
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}:signal:{}", signal.signal_id),
                signal,
                "skill-registry",
            )
            .await?;
        Ok(manifest)
    }

    pub async fn build(&self, skill_id: &str, builder: &str) -> Result<SkillBuildArtifact> {
        let manifest = self
            .db
            .get_knowledge(&format!("skills:manifest:{skill_id}"))
            .await?
            .and_then(|record| serde_json::from_str::<SkillManifest>(&record.value).ok())
            .ok_or_else(|| anyhow::anyhow!("skill not found: {skill_id}"))?;
        let now = current_time_ms();
        let artifact = SkillBuildArtifact {
            skill_id: skill_id.to_string(),
            build_id: format!("{skill_id}:{now}"),
            builder: builder.to_string(),
            compiled_prompt: format!(
                "### skill:{}\n# source\n{}\n# markdown\n{}",
                skill_id, manifest.source, manifest.markdown
            ),
            created_at_ms: now,
        };
        self.db
            .upsert_json_knowledge(
                format!("skills:build:{skill_id}:latest"),
                &artifact,
                "skill-builder",
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                format!("skills:build:{skill_id}:{}", artifact.build_id),
                &artifact,
                "skill-builder",
            )
            .await?;
        Ok(artifact)
    }

    pub async fn list(&self) -> Result<Vec<SkillManifest>> {
        let mut manifests = self
            .db
            .list_knowledge_by_prefix("skills:manifest:")
            .await?
            .into_iter()
            .filter_map(|record| serde_json::from_str::<SkillManifest>(&record.value).ok())
            .collect::<Vec<_>>();
        manifests.sort_by(|a, b| a.skill_id.cmp(&b.skill_id));
        Ok(manifests)
    }

    pub async fn remove(&self, _skill_id: &str) -> Result<()> {
        // removal is considered lifecycle governance and must still be traceable.
        bail!(
            "skill_registry.remove now requires traceable learning signal; use remove_with_signal"
        );
    }

    pub async fn remove_with_signal(
        &self,
        skill_id: &str,
        signal: &crate::memory::LearningSignal,
    ) -> Result<()> {
        self
            .validate_learning_signal(signal, "skill_registry.remove")
            .await?;
        let now = current_time_ms();
        let tombstone = serde_json::json!({
            "skill_id": skill_id,
            "status": "retired",
            "retired_at_ms": now,
            "learning_signal": signal,
        });
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}"),
                &tombstone,
                "skill-registry",
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}:signal:{}", signal.signal_id),
                signal,
                "skill-registry",
            )
            .await?;
        Ok(())
    }

    pub async fn install_from_package(
        &self,
        package: &crate::contracts::skill_foundry::PackageMeta,
        source: &str,
        markdown: &str,
        signal: &crate::memory::LearningSignal,
    ) -> Result<SkillManifest> {
        self
            .validate_learning_signal(signal, "skill_registry.install_from_package")
            .await?;
        let skill_id = package.skill_name.as_str();
        let now = current_time_ms();
        let manifest = SkillManifest {
            skill_id: skill_id.to_string(),
            name: package.skill_name.clone(),
            source: source.to_string(),
            status: if package.enabled {
                "active".to_string()
            } else {
                "installed".to_string()
            },
            markdown: markdown.to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}"),
                &manifest,
                "skill-registry",
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}:signal:{}", signal.signal_id),
                signal,
                "skill-registry",
            )
            .await?;

        let record = SkillInstallRecord {
            record_id: format!("install:{skill_id}:{now}"),
            skill_id: skill_id.to_string(),
            action: "install".to_string(),
            package_id: package.package_id.clone(),
            artifact_path: package.artifact_path.clone(),
            source: source.to_string(),
            status: manifest.status.clone(),
            reason: "package_install".to_string(),
            created_at_ms: now,
        };
        self.db
            .upsert_json_knowledge(
                format!("skills:install:{skill_id}:{now}"),
                &record,
                "skill-registry",
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                format!("skills:install:{skill_id}:latest"),
                &record,
                "skill-registry",
            )
            .await?;
        Ok(manifest)
    }

    pub async fn set_enabled(
        &self,
        skill_id: &str,
        enabled: bool,
        reason: &str,
        signal: &crate::memory::LearningSignal,
    ) -> Result<SkillManifest> {
        self
            .validate_learning_signal(signal, "skill_registry.set_enabled")
            .await?;
        let now = current_time_ms();
        let existing = self
            .db
            .get_knowledge(&format!("skills:manifest:{skill_id}"))
            .await?
            .and_then(|record| serde_json::from_str::<SkillManifest>(&record.value).ok())
            .or_else(|| synthesize_manifest_from_workspace(skill_id, now));
        let manifest = SkillManifest {
            status: if enabled {
                "active".to_string()
            } else {
                "disabled".to_string()
            },
            updated_at_ms: now,
            ..existing.ok_or_else(|| anyhow::anyhow!("skill not found: {skill_id}"))?
        };
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}"),
                &manifest,
                "skill-registry",
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                format!("skills:manifest:{skill_id}:signal:{}", signal.signal_id),
                signal,
                "skill-registry",
            )
            .await?;
        let record = SkillInstallRecord {
            record_id: format!(
                "{}:{skill_id}:{now}",
                if enabled { "enable" } else { "disable" }
            ),
            skill_id: skill_id.to_string(),
            action: if enabled {
                "enable".to_string()
            } else {
                "disable".to_string()
            },
            package_id: "n/a".to_string(),
            artifact_path: "n/a".to_string(),
            source: manifest.source.clone(),
            status: manifest.status.clone(),
            reason: reason.to_string(),
            created_at_ms: now,
        };
        self.db
            .upsert_json_knowledge(
                format!("skills:install:{skill_id}:{now}"),
                &record,
                "skill-registry",
            )
            .await?;
        self.db
            .upsert_json_knowledge(
                format!("skills:install:{skill_id}:latest"),
                &record,
                "skill-registry",
            )
            .await?;
        Ok(manifest)
    }

    async fn validate_learning_signal(
        &self,
        signal: &crate::memory::LearningSignal,
        target: &str,
    ) -> Result<()> {
        if signal.evidence_ref.trim().is_empty() {
            let now = current_time_ms();
            let session_id = if signal.session_id.trim().is_empty() {
                "global-skill-registry"
            } else {
                signal.session_id.as_str()
            };
            self.db
                .upsert_json_knowledge(
                    format!(
                        "evidence:memory:{session_id}:learning-signal-reject:skill-registry:{now}"
                    ),
                    &serde_json::json!({
                        "reject_id": format!("learning-signal-reject:skill-registry:{now}"),
                        "session_id": session_id,
                        "trace_id": signal.trace_id,
                        "source": signal.source,
                        "target": target,
                        "reason": "learning_signal.missing_evidence_ref",
                        "evidence_ref": serde_json::Value::Null,
                        "created_at_ms": now,
                    }),
                    "learning-signal-guard",
                )
                .await?;
            bail!("learning signal rejected for `{target}`: evidence_ref is required");
        }
        Ok(())
    }
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn synthesize_manifest_from_workspace(skill_id: &str, now: u64) -> Option<SkillManifest> {
    let direct = std::path::Path::new("skills").join(skill_id).join("SKILL.md");
    let slug = slugify(skill_id);
    let slugged = std::path::Path::new("skills").join(&slug).join("SKILL.md");
    let source = if direct.exists() {
        format!("foundry://skills/{skill_id}")
    } else if slugged.exists() {
        format!("foundry://skills/{slug}")
    } else {
        return None;
    };
    let markdown = if direct.exists() {
        std::fs::read_to_string(direct).unwrap_or_else(|_| "# skill\n".to_string())
    } else {
        std::fs::read_to_string(slugged).unwrap_or_else(|_| "# skill\n".to_string())
    };
    Some(SkillManifest {
        skill_id: skill_id.to_string(),
        name: skill_id.to_string(),
        source,
        status: "installed".to_string(),
        markdown,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

fn slugify(skill_id: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in skill_id.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoloop_state_adapter::{StateStoreBackend, StateStoreConfig};

    #[tokio::test]
    async fn skill_registry_rejects_missing_learning_signal_evidence_ref() {
        let db = StateStore::from_config(&StateStoreConfig {
            enabled: true,
            backend: StateStoreBackend::InMemory,
            uri: "http://state_store:3000".into(),
            module_name: "autoloop_core".into(),
            namespace: "autoloop".into(),
            pool_size: 2,
        });
        let registry = SkillRegistry::new(db.clone());
        let result = registry
            .register(
                "skill:reject",
                "test://skill",
                "# skill",
                &crate::memory::LearningSignal {
                    signal_id: "sig:reject".into(),
                    session_id: "session-skill-reject".into(),
                    trace_id: "trace-skill-reject".into(),
                    source: "skills.tests".into(),
                    evidence_ref: "".into(),
                    metadata: std::collections::BTreeMap::new(),
                },
            )
            .await;
        assert!(result.is_err(), "skill registry must reject missing evidence_ref");
        let rejects = db
            .list_knowledge_by_prefix(
                "evidence:memory:session-skill-reject:learning-signal-reject:skill-registry:",
            )
            .await
            .expect("list reject records");
        assert!(!rejects.is_empty(), "reject reason must be persisted");
    }
}

