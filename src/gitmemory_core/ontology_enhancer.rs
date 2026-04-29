use anyhow::Result;
use autoloop_state_adapter::StateStore;

use super::schema_registry::{SchemaRegistry, SchemaRegistrySnapshot};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OntologyConcept {
    pub concept: String,
    pub canonical_type: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OntologyEnhancement {
    pub session_id: String,
    pub trace_id: String,
    pub taxonomy_version: String,
    pub concepts: Vec<OntologyConcept>,
}

pub struct OntologyEnhancer;

impl OntologyEnhancer {
    pub async fn enhance(
        db: &StateStore,
        repo_root: &std::path::Path,
        session_id: &str,
        trace_id: &str,
    ) -> Result<OntologyEnhancement> {
        let schema = SchemaRegistry::load(repo_root)?;
        let mut concepts = seed_from_taxonomy(&schema);
        concepts.sort_by(|left, right| left.concept.cmp(&right.concept));
        concepts.dedup_by(|left, right| left.concept == right.concept);

        let enhancement = OntologyEnhancement {
            session_id: session_id.to_string(),
            trace_id: trace_id.to_string(),
            taxonomy_version: schema.version,
            concepts,
        };
        db.upsert_json_knowledge(
            format!("memory:ontology:{}:{}:latest", session_id, trace_id),
            &enhancement,
            "ontology-enhancer",
        )
        .await?;
        Ok(enhancement)
    }
}

fn seed_from_taxonomy(schema: &SchemaRegistrySnapshot) -> Vec<OntologyConcept> {
    let mut concepts = Vec::<OntologyConcept>::new();
    for (concept, canonical_type) in &schema.taxonomy {
        concepts.push(OntologyConcept {
            concept: concept.clone(),
            canonical_type: canonical_type.clone(),
            confidence: 0.95,
        });
    }
    for rule in &schema.block_rules {
        concepts.push(OntologyConcept {
            concept: rule.id.clone(),
            canonical_type: "block_rule".to_string(),
            confidence: if rule.required { 0.9 } else { 0.75 },
        });
    }
    concepts
}

