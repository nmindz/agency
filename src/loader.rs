use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;

use crate::jsonc;
use crate::models::{CategoryMap, Override, Sot, Template};

/// Load the generated permissions file (permissions.jsonc).
pub fn load_sot(base_dir: &Path) -> Result<Sot> {
    jsonc::parse_file(&base_dir.join("permissions.jsonc"))
}

/// Load a single template file from the _templates directory.
pub fn load_template(templates_dir: &Path, filename: &str) -> Result<Template> {
    jsonc::parse_file(&templates_dir.join(filename))
}

/// Load a single override file from the agents directory.
pub fn load_override(agents_dir: &Path, agent_name: &str) -> Result<Override> {
    jsonc::parse_file(&agents_dir.join(format!("{}.jsonc", agent_name)))
}

/// Discover categories by scanning agent files for their `$extends` field.
///
/// Returns a CategoryMap: category_name → sorted list of agent names.
pub fn discover_categories(agents_dir: &Path) -> Result<CategoryMap> {
    #[derive(serde::Deserialize)]
    struct OverrideHeader {
        #[serde(rename = "$extends")]
        extends: String,
    }

    let mut raw: IndexMap<String, Vec<String>> = IndexMap::new();

    let entries = std::fs::read_dir(agents_dir)
        .with_context(|| format!("Reading agents directory: {}", agents_dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // Skip non-.jsonc files
        if path.extension().and_then(|e| e.to_str()) != Some("jsonc") {
            continue;
        }

        let agent_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid filename: {}", path.display()))?
            .to_string();

        let header: OverrideHeader = jsonc::parse_file(&path)
            .with_context(|| format!("Parsing agent header for '{}'", agent_name))?;

        raw.entry(header.extends).or_default().push(agent_name);
    }

    // Sort agent lists within each category
    for agents in raw.values_mut() {
        agents.sort();
    }

    // Sort categories by key for deterministic output
    raw.sort_keys();

    Ok(raw)
}

/// Load all templates referenced by a CategoryMap.
pub fn load_all_templates(
    templates_dir: &Path,
    categories: &CategoryMap,
) -> Result<IndexMap<String, Template>> {
    let mut templates = IndexMap::new();

    for cat_name in categories.keys() {
        let filename = format!("{}.jsonc", cat_name);
        let template = load_template(templates_dir, &filename)
            .with_context(|| format!("Loading template for category '{}'", cat_name))?;
        templates.insert(cat_name.clone(), template);
    }

    Ok(templates)
}

/// Load all template files referenced by agents, PLUS any parent templates
/// discovered via `$extends` chains.
///
/// This iteratively loads templates and discovers parent references,
/// ensuring mixin templates (not directly referenced by agents) are
/// also loaded when referenced via `$extends`.
pub fn load_all_templates_with_parents(
    templates_dir: &Path,
    categories: &CategoryMap,
) -> Result<IndexMap<String, Template>> {
    let mut templates = IndexMap::new();
    let mut to_load: Vec<String> = categories.keys().cloned().collect();
    let mut loaded: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Iteratively load templates and discover parents
    while let Some(cat_name) = to_load.pop() {
        if loaded.contains(&cat_name) {
            continue;
        }

        let filename = format!("{}.jsonc", cat_name);
        let template = load_template(templates_dir, &filename)
            .with_context(|| format!("Loading template for '{}'", cat_name))?;

        // Queue parent templates for loading
        if let Some(ref extends) = template.extends {
            for parent in extends.parents() {
                if !loaded.contains(parent) {
                    to_load.push(parent.to_string());
                }
            }
        }

        templates.insert(cat_name.clone(), template);
        loaded.insert(cat_name);
    }

    templates.sort_keys();
    Ok(templates)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that discover_categories does NOT include vcs-permissions
    /// (it's a mixin — no agents extend it directly).
    #[test]
    fn test_mixin_not_in_category_map() {
        let base_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = crate::config::ProjectConfig::resolve(base_dir, None, None, None).unwrap();
        let categories = discover_categories(&config.agents_dir).unwrap();
        assert!(
            !categories.contains_key("vcs-permissions"),
            "vcs-permissions should NOT be in category map (no agents extend it)"
        );
    }

    /// Verify that load_all_templates_with_parents discovers and loads
    /// the vcs-permissions mixin even though no agent extends it directly.
    #[test]
    fn test_mixin_loaded_via_parent_discovery() {
        let base_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = crate::config::ProjectConfig::resolve(base_dir, None, None, None).unwrap();
        let categories = discover_categories(&config.agents_dir).unwrap();
        let templates =
            load_all_templates_with_parents(&config.templates_dir, &categories).unwrap();

        // vcs-permissions should be loaded even though it's not in categories
        assert!(
            templates.contains_key("vcs-permissions"),
            "vcs-permissions mixin should be loaded via parent discovery"
        );
        // It should also load all category templates
        assert!(
            templates.contains_key("backend-api"),
            "backend-api should be loaded"
        );
        assert!(
            templates.contains_key("planning"),
            "planning should be loaded"
        );
    }

    /// Verify that the effective baseline for backend-api includes entries
    /// from both its own baseline AND the vcs-permissions parent.
    #[test]
    fn test_effective_baseline_includes_parent_entries() {
        let base_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = crate::config::ProjectConfig::resolve(base_dir, None, None, None).unwrap();
        let categories = discover_categories(&config.agents_dir).unwrap();
        let templates =
            load_all_templates_with_parents(&config.templates_dir, &categories).unwrap();

        let backend = templates.get("backend-api").unwrap();
        assert!(
            backend.extends.is_some(),
            "backend-api should have $extends"
        );

        // Build DAG and compute effective baseline
        let (graph, node_map) = crate::template_dag::build_dag(&templates).unwrap();
        let ancestors =
            crate::template_dag::ancestor_order(&graph, &node_map, "backend-api").unwrap();
        assert!(
            ancestors.contains(&"vcs-permissions".to_string()),
            "backend-api ancestors should include vcs-permissions"
        );

        // Merge baselines
        let mut parent_baselines = Vec::new();
        for anc_name in &ancestors {
            let anc = templates.get(anc_name).unwrap();
            let bl: IndexMap<String, String> =
                serde_json::from_value(anc.baseline.permission.clone()).unwrap();
            parent_baselines.push((anc_name.clone(), bl));
        }
        let self_bl: IndexMap<String, String> =
            serde_json::from_value(backend.baseline.permission.clone()).unwrap();
        let effective =
            crate::resolver::merge_baselines(&parent_baselines, &self_bl, "backend-api").unwrap();

        // Should contain VCS entries from parent
        assert!(
            effective.contains_key("git status"),
            "effective baseline should contain git status from VCS mixin"
        );
        assert!(
            effective.contains_key("jj log"),
            "effective baseline should contain jj log from VCS mixin"
        );
        // Should also contain self entries
        assert!(
            effective.contains_key("npm *"),
            "effective baseline should contain npm * from backend-api itself"
        );
    }

    /// Verify that the effective baseline includes the template's own
    /// entries (not just parent entries).
    #[test]
    fn test_effective_baseline_includes_self_entries() {
        let base_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = crate::config::ProjectConfig::resolve(base_dir, None, None, None).unwrap();
        let categories = discover_categories(&config.agents_dir).unwrap();
        let templates =
            load_all_templates_with_parents(&config.templates_dir, &categories).unwrap();

        let lang_rust = templates.get("lang-rust").unwrap();
        assert!(
            lang_rust.extends.is_some(),
            "lang-rust should have $extends"
        );

        let (graph, node_map) = crate::template_dag::build_dag(&templates).unwrap();
        let ancestors =
            crate::template_dag::ancestor_order(&graph, &node_map, "lang-rust").unwrap();

        let mut parent_baselines = Vec::new();
        for anc_name in &ancestors {
            let anc = templates.get(anc_name).unwrap();
            let bl: IndexMap<String, String> =
                serde_json::from_value(anc.baseline.permission.clone()).unwrap();
            parent_baselines.push((anc_name.clone(), bl));
        }
        let self_bl: IndexMap<String, String> =
            serde_json::from_value(lang_rust.baseline.permission.clone()).unwrap();
        let effective =
            crate::resolver::merge_baselines(&parent_baselines, &self_bl, "lang-rust").unwrap();

        // lang-rust specific entries should be present
        assert!(
            effective.contains_key("cargo *"),
            "effective baseline should contain cargo * from lang-rust itself"
        );
        assert!(
            effective.contains_key("rustc *"),
            "effective baseline should contain rustc * from lang-rust itself"
        );
        // VCS entries should also be present
        assert!(
            effective.contains_key("git status"),
            "effective baseline should contain git status from VCS mixin"
        );
    }
}
