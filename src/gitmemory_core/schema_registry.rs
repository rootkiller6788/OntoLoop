use std::{collections::BTreeMap, fs, path::Path};

use anyhow::Result;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlockSchemaRule {
    pub id: String,
    pub kind: String,
    pub required: bool,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlacementRule {
    pub id: String,
    pub pattern: String,
    pub target_template: String,
    pub priority: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SchemaRegistrySnapshot {
    pub version: String,
    pub block_rules: Vec<BlockSchemaRule>,
    pub placement_rules: Vec<PlacementRule>,
    pub taxonomy: BTreeMap<String, String>,
}

pub struct SchemaRegistry;

impl SchemaRegistry {
    pub fn load(repo_root: &Path) -> Result<SchemaRegistrySnapshot> {
        let path = repo_root.join(".gitmemory").join("schema_registry.json");
        if path.exists() {
            let raw = fs::read_to_string(path)?;
            if let Ok(snapshot) = serde_json::from_str::<SchemaRegistrySnapshot>(&raw) {
                return Ok(snapshot);
            }
        }
        Ok(default_snapshot())
    }
}

fn default_snapshot() -> SchemaRegistrySnapshot {
    SchemaRegistrySnapshot {
        version: "schema-registry-v1".to_string(),
        block_rules: vec![
            BlockSchemaRule {
                id: "frontmatter".to_string(),
                kind: "frontmatter".to_string(),
                required: false,
                max_bytes: 8 * 1024,
            },
            BlockSchemaRule {
                id: "heading".to_string(),
                kind: "heading".to_string(),
                required: true,
                max_bytes: 4 * 1024,
            },
            BlockSchemaRule {
                id: "list_item".to_string(),
                kind: "list_item".to_string(),
                required: false,
                max_bytes: 4 * 1024,
            },
            BlockSchemaRule {
                id: "paragraph".to_string(),
                kind: "paragraph".to_string(),
                required: true,
                max_bytes: 32 * 1024,
            },
            BlockSchemaRule {
                id: "code_fence".to_string(),
                kind: "code_fence".to_string(),
                required: false,
                max_bytes: 256 * 1024,
            },
        ],
        placement_rules: vec![
            PlacementRule {
                id: "already-canonical".to_string(),
                pattern: "canonical/*".to_string(),
                target_template: "{source}".to_string(),
                priority: 100,
            },
            PlacementRule {
                id: "namespace-path-v1".to_string(),
                pattern: "*".to_string(),
                target_template: "canonical/{namespace}/{sanitized_stem}.md".to_string(),
                priority: 10,
            },
        ],
        taxonomy: BTreeMap::from([
            ("memory".to_string(), "knowledge".to_string()),
            ("strategy".to_string(), "heuristic".to_string()),
            ("evidence".to_string(), "audit".to_string()),
        ]),
    }
}
