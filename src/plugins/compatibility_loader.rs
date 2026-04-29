use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};

use anyhow::Result;

use crate::contracts::plugin::{
    PLUGIN_API_VERSION_V2, PluginCapabilityDescriptor, PluginCompatSpec, PluginKind,
    PluginManifestContract, PluginRisk,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompatibilityDiscoveryReport {
    pub root: String,
    pub discovered_count: usize,
    pub entries: Vec<CompatibilityEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompatibilityEntry {
    pub manifest_path: String,
    pub format: String,
    pub parse_status: String,
    pub plugin_id: Option<String>,
    pub signature_ref_present: bool,
    pub manifest: Option<PluginManifestContract>,
    pub compiled: Option<CompatibilityCompileOutput>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CompatibilityCompileOutput {
    pub hooks: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub skills: Vec<String>,
    pub merged_scopes: Vec<String>,
}

pub struct PluginCompatibilityLoader;

impl PluginCompatibilityLoader {
    pub fn discover(root: impl AsRef<Path>) -> Result<CompatibilityDiscoveryReport> {
        let root = root.as_ref().to_path_buf();
        let mut entries = Vec::new();

        for candidate in candidate_manifest_paths(&root) {
            if !candidate.exists() {
                continue;
            }
            let format = detect_format(&root, &candidate);
            let raw = fs::read_to_string(&candidate);
            match raw {
                Ok(content) => {
                    let parsed = parse_manifest_compat(&content, &candidate, &format);
                    entries.push(parsed);
                }
                Err(error) => entries.push(CompatibilityEntry {
                    manifest_path: candidate.display().to_string(),
                    format,
                    parse_status: "read_error".to_string(),
                    plugin_id: None,
                    signature_ref_present: false,
                    manifest: None,
                    compiled: None,
                    notes: vec![format!("read failed: {}", error)],
                }),
            }
        }

        Ok(CompatibilityDiscoveryReport {
            root: root.display().to_string(),
            discovered_count: entries.len(),
            entries,
        })
    }
}

fn candidate_manifest_paths(root: &Path) -> Vec<PathBuf> {
    vec![
        root.join("plugin.json"),
        root.join(".claude-plugin").join("plugin.json"),
        root.join(".codex-plugin").join("plugin.json"),
    ]
}

fn detect_format(root: &Path, path: &Path) -> String {
    if path == root.join("plugin.json") {
        "plugin.json".to_string()
    } else if path == root.join(".claude-plugin").join("plugin.json") {
        ".claude-plugin".to_string()
    } else if path == root.join(".codex-plugin").join("plugin.json") {
        ".codex-plugin".to_string()
    } else {
        "unknown".to_string()
    }
}

fn parse_manifest_compat(content: &str, path: &Path, format: &str) -> CompatibilityEntry {
    let value = match serde_json::from_str::<serde_json::Value>(content) {
        Ok(value) => value,
        Err(error) => {
            return CompatibilityEntry {
                manifest_path: path.display().to_string(),
                format: format.to_string(),
                parse_status: "json_error".to_string(),
                plugin_id: None,
                signature_ref_present: false,
                manifest: None,
                compiled: None,
                notes: vec![format!("json parse failed: {}", error)],
            };
        }
    };

    let manifest = if let Ok(strict) = serde_json::from_value::<PluginManifestContract>(value.clone()) {
        strict
    } else {
        normalize_compat_manifest(&value, path)
    };
    let mut manifest = manifest;
    let compiled = compile_compat_layers(&value, path, &manifest);
    apply_compiled_layers(&mut manifest, &compiled);

    let plugin_id = Some(manifest.id.clone());
    let signature_ref_present = manifest.signature_ref.is_some();
    let mut notes = Vec::new();
    if !signature_ref_present {
        notes.push("signature_ref missing (install/enable will be rejected)".to_string());
    }

    CompatibilityEntry {
        manifest_path: path.display().to_string(),
        format: format.to_string(),
        parse_status: "ok".to_string(),
        plugin_id,
        signature_ref_present,
        manifest: Some(manifest),
        compiled: Some(compiled),
        notes,
    }
}

fn normalize_compat_manifest(value: &serde_json::Value, path: &Path) -> PluginManifestContract {
    let id = value
        .get("id")
        .or_else(|| value.get("plugin_id"))
        .or_else(|| value.get("name"))
        .and_then(serde_json::Value::as_str)
        .map(|raw| {
            if raw.starts_with("plugin:") {
                raw.to_string()
            } else {
                format!("plugin:{}", raw.replace(' ', "-").to_ascii_lowercase())
            }
        })
        .unwrap_or_else(|| format!("plugin:{}", path.file_stem().and_then(|v| v.to_str()).unwrap_or("unknown")));

    let version = value
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("v2")
        .to_string();

    let kind = parse_kind(
        value
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("other"),
    );

    let api_version = value
        .get("api_version")
        .or_else(|| value.get("compat").and_then(|compat| compat.get("api_version")))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(PLUGIN_API_VERSION_V2)
        .to_string();

    let signature_ref = value
        .get("signature_ref")
        .or_else(|| value.get("signature"))
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string);

    let source = value
        .get("source")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("file://{}", path.display()));

    PluginManifestContract {
        id: id.clone(),
        plugin_id: id,
        version,
        kind,
        capability: PluginCapabilityDescriptor {
            capability_id: value
                .get("capability_id")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| "plugin.invoke".to_string()),
            description: value
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("compat plugin")
                .to_string(),
            scopes: vec!["plugin.invoke".to_string()],
        },
        risk: PluginRisk::Medium,
        compat: PluginCompatSpec {
            api_version: api_version.clone(),
            compatible_api_versions: vec![api_version],
            min_core_version: PLUGIN_API_VERSION_V2.to_string(),
            max_core_version: None,
        },
        name: value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("compat-plugin")
            .to_string(),
        source,
        signature_ref,
        permissions: Vec::new(),
        hooks: Vec::new(),
        commands: Vec::new(),
        event_contract_version: "plugin-event-v2".to_string(),
        lifecycle_contract_version: "plugin-lifecycle-v2".to_string(),
        isolation: Default::default(),
        facade: Default::default(),
        metadata: BTreeMap::new(),
    }
}

fn parse_kind(kind: &str) -> PluginKind {
    match kind.to_ascii_lowercase().as_str() {
        "graph_projection" | "graph" => PluginKind::GraphProjection,
        "vector_projection" | "vector" => PluginKind::VectorProjection,
        "search_projection" | "search" => PluginKind::SearchProjection,
        "supermemory_federation" | "supermemory" => PluginKind::SupermemoryFederation,
        "source_adapter" | "source" => PluginKind::SourceAdapter,
        "hook" => PluginKind::Hook,
        "tool" => PluginKind::Tool,
        "transport" => PluginKind::Transport,
        "service" => PluginKind::Service,
        _ => PluginKind::Other,
    }
}

fn compile_compat_layers(
    value: &serde_json::Value,
    path: &Path,
    manifest: &PluginManifestContract,
) -> CompatibilityCompileOutput {
    let mut hooks = collect_named_items(value.get("hooks"));
    if hooks.is_empty() {
        hooks = collect_named_items(value.get("hook"));
    }

    let mut mcp_servers = collect_mcp_servers(value.get("mcp"))
        .into_iter()
        .chain(collect_mcp_servers(value.get("mcpServers")))
        .collect::<Vec<_>>();
    mcp_servers.sort();
    mcp_servers.dedup();

    let mut skills = collect_named_items(value.get("skills"));
    skills.extend(discover_skill_files(path));
    skills.sort();
    skills.dedup();

    let mut merged_scopes = manifest.capability.scopes.clone();
    merge_unique(&mut merged_scopes, "plugin.invoke".to_string());
    if !hooks.is_empty() {
        merge_unique(&mut merged_scopes, "plugin.hooks".to_string());
    }
    if !mcp_servers.is_empty() {
        merge_unique(&mut merged_scopes, "plugin.mcp.dispatch".to_string());
    }
    if !skills.is_empty() {
        merge_unique(&mut merged_scopes, "plugin.skills.read".to_string());
    }

    CompatibilityCompileOutput {
        hooks,
        mcp_servers,
        skills,
        merged_scopes,
    }
}

fn apply_compiled_layers(
    manifest: &mut PluginManifestContract,
    compiled: &CompatibilityCompileOutput,
) {
    for hook in &compiled.hooks {
        merge_unique(&mut manifest.hooks, hook.clone());
    }
    for server in &compiled.mcp_servers {
        merge_unique(
            &mut manifest.commands,
            format!("mcp::{server}::invoke"),
        );
    }
    for skill in &compiled.skills {
        manifest
            .metadata
            .entry(format!("skill:{skill}"))
            .or_insert_with(|| "discovered".to_string());
    }
    manifest.capability.scopes = compiled.merged_scopes.clone();
    manifest.metadata.insert(
        "compat_compiled_hooks_count".to_string(),
        compiled.hooks.len().to_string(),
    );
    manifest.metadata.insert(
        "compat_compiled_mcp_count".to_string(),
        compiled.mcp_servers.len().to_string(),
    );
    manifest.metadata.insert(
        "compat_compiled_skills_count".to_string(),
        compiled.skills.len().to_string(),
    );
}

fn collect_named_items(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::Object(map) => map.keys().cloned().collect(),
        serde_json::Value::String(item) => vec![item.to_string()],
        _ => Vec::new(),
    }
}

fn collect_mcp_servers(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    match value {
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::Object(map) => map.keys().cloned().collect(),
        serde_json::Value::String(item) => vec![item.to_string()],
        _ => Vec::new(),
    }
}

fn discover_skill_files(path: &Path) -> Vec<String> {
    let Some(root) = path.parent() else {
        return Vec::new();
    };
    let candidates = [
        root.join("skills"),
        root.join(".claude-plugin").join("skills"),
        root.join(".codex-plugin").join("skills"),
    ];
    let mut results = Vec::new();
    for dir in candidates {
        if !dir.exists() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let candidate = entry.path();
                if candidate
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                {
                    if let Some(stem) = candidate.file_stem().and_then(|stem| stem.to_str()) {
                        results.push(stem.to_string());
                    }
                }
            }
        }
    }
    results
}

fn merge_unique<T: Eq>(target: &mut Vec<T>, item: T) {
    if !target.contains(&item) {
        target.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_and_parses_plugin_json_and_claude_plugin() {
        let mut root = std::env::temp_dir();
        root.push(format!("autoloop-compat-loader-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".claude-plugin")).expect("mkdir");

        fs::write(
            root.join("plugin.json"),
            r#"{"id":"plugin:search-a","kind":"search","signature_ref":"sig-a"}"#,
        )
        .expect("write plugin.json");
        fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"Graph B","kind":"graph"}"#,
        )
        .expect("write claude plugin");

        let report = PluginCompatibilityLoader::discover(&root).expect("discover");
        assert_eq!(report.discovered_count, 2);
        assert!(report.entries.iter().any(|entry| entry.plugin_id.as_deref() == Some("plugin:search-a")));
        assert!(report.entries.iter().any(|entry| entry.format == ".claude-plugin"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn compiles_hooks_mcp_and_skills_into_manifest() {
        let mut root = std::env::temp_dir();
        root.push(format!("autoloop-compat-compile-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("skills")).expect("mkdir skills");
        fs::write(root.join("skills").join("retrieval.md"), "# retrieval").expect("skill file");
        fs::write(
            root.join("plugin.json"),
            r#"{
                "id":"plugin:compat-merge",
                "kind":"tool",
                "signature_ref":"sig-merge",
                "hooks":["pre_tool_use","on_result"],
                "mcpServers":{"local-mcp":{"url":"http://localhost"}},
                "skills":["planner"]
            }"#,
        )
        .expect("write plugin");

        let report = PluginCompatibilityLoader::discover(&root).expect("discover");
        let entry = report
            .entries
            .iter()
            .find(|item| item.plugin_id.as_deref() == Some("plugin:compat-merge"))
            .expect("entry");
        let compiled = entry.compiled.as_ref().expect("compiled");
        assert!(compiled.hooks.iter().any(|hook| hook == "pre_tool_use"));
        assert!(compiled.mcp_servers.iter().any(|server| server == "local-mcp"));
        assert!(compiled.skills.iter().any(|skill| skill == "planner"));
        assert!(compiled.skills.iter().any(|skill| skill == "retrieval"));

        let manifest = entry.manifest.as_ref().expect("manifest");
        assert!(manifest.hooks.iter().any(|hook| hook == "pre_tool_use"));
        assert!(manifest
            .commands
            .iter()
            .any(|command| command == "mcp::local-mcp::invoke"));
        assert!(manifest
            .capability
            .scopes
            .iter()
            .any(|scope| scope == "plugin.skills.read"));

        let _ = fs::remove_dir_all(&root);
    }
}
