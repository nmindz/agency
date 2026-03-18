#![allow(dead_code)] // Public API consumed by future CLI subcommand

use anyhow::{Context, Result};
use indexmap::IndexMap;

use crate::config::ProjectConfig;
use crate::models::{is_simple_category, Override, PermValue, ResolvedPermission, Sot, SotAgent};
use crate::{comment_gen, generator, jsonc_writer, loader, resolver, template_dag};

/// Generate the full `permissions.jsonc` content from templates + overrides.
///
/// Reads templates and agent overrides from the configured directories,
/// runs the full pipeline (discovery → template loading → DAG construction →
/// effective baseline computation → resolution → delta → comments →
/// formatting), and returns the complete JSONC string.  Does **not** write
/// to disk.
pub fn generate_permissions(config: &ProjectConfig) -> Result<String> {
    // Step 1: Discovery
    let categories = loader::discover_categories(&config.agents_dir)?;
    let templates = loader::load_all_templates_with_parents(&config.templates_dir, &categories)?;

    // Step 2: Build DAG and detect cycles
    let (graph, node_map) = template_dag::build_dag(&templates)?;
    template_dag::detect_cycles(&graph)?;

    // Step 3: Pre-compute effective baselines for each category
    let effective_baselines = compute_effective_baselines(&templates, &graph, &node_map)?;

    // Step 4: Resolution — build temporary Sot + keep override data for comments
    let mut temp_sot: Sot = IndexMap::new();
    let mut agent_data: IndexMap<String, (Override, ResolvedPermission)> = IndexMap::new();

    for (cat_name, agents) in &categories {
        let template = templates
            .get(cat_name)
            .with_context(|| format!("No template loaded for category '{}'", cat_name))?;

        for agent_name in agents {
            let override_data = loader::load_override(&config.agents_dir, agent_name)?;

            // Use DAG-aware resolution if this category has an effective baseline
            let resolved = if let Some(effective) = effective_baselines.get(cat_name) {
                resolver::resolve_with_baseline(effective, cat_name, &override_data)?
            } else {
                resolver::resolve(template, &override_data)?
            };

            temp_sot.insert(
                agent_name.clone(),
                SotAgent {
                    permission: resolved.permission.clone(),
                },
            );
            agent_data.insert(agent_name.clone(), (override_data, resolved));
        }
    }

    // Step 5: Count total agents for is_last tracking
    let total_agents: usize = categories.values().map(|a| a.len()).sum();
    let mut agent_counter: usize = 0;

    // Step 6: File-level header
    let header =
        comment_gen::generate_file_header(config.templates_display(), config.agents_display());

    // Step 7: Build category blocks
    let mut category_blocks: Vec<Vec<String>> = Vec::new();

    for (cat_name, agents) in &categories {
        let template = templates.get(cat_name).unwrap();
        let mut block: Vec<String> = Vec::new();

        // Category header comments
        let parent_names: Option<Vec<String>> = template
            .extends
            .as_ref()
            .map(|ext| ext.parents().iter().map(|s| s.to_string()).collect());
        let cat_header = comment_gen::generate_category_header(template, parent_names.as_deref());
        for line in &cat_header {
            block.push(format!("  {}", line));
        }

        for agent_name in agents {
            agent_counter += 1;
            let is_last = agent_counter == total_agents;

            let (ref override_data, ref resolved) = agent_data[agent_name];

            // Compute delta — use effective baseline template for delta computation
            let delta = if let Some(effective) = effective_baselines.get(cat_name) {
                generator::compute_delta_from_baseline(&temp_sot, effective, agent_name, cat_name)
            } else {
                generator::compute_delta(&temp_sot, template, agent_name, cat_name)
            };

            // Generate agent comments
            let agent_comments =
                comment_gen::generate_agent_header(override_data, template, &delta);

            // Format agent block
            let is_simple = is_simple_category(cat_name);
            let agent_block = jsonc_writer::format_agent_block(
                agent_name,
                &resolved.permission,
                &agent_comments,
                is_simple,
                is_last,
            );

            block.extend(agent_block);
        }

        category_blocks.push(block);
    }

    // Step 8: Assemble document
    let generated = jsonc_writer::assemble_document(&header, &category_blocks);

    // Step 9: Self-validate
    self_validate(&generated, &temp_sot)?;

    Ok(generated)
}

/// Pre-compute effective baselines for all categories by merging their
/// inheritance chains. Returns a map from category name to the merged
/// baseline (as an IndexMap of command→permission).
///
/// For templates without `$extends`, no entry is created (callers fall
/// back to the old `resolve()` path for backward compatibility).
fn compute_effective_baselines(
    templates: &IndexMap<String, crate::models::Template>,
    graph: &petgraph::graph::DiGraph<String, ()>,
    node_map: &std::collections::HashMap<String, petgraph::graph::NodeIndex>,
) -> Result<IndexMap<String, IndexMap<String, PermValue>>> {
    let mut effective: IndexMap<String, IndexMap<String, PermValue>> = IndexMap::new();

    for (cat_name, template) in templates {
        // Only compute effective baselines for templates that have parents
        if template.extends.is_none() {
            continue;
        }

        // Get ancestors in topological order
        let ancestors = template_dag::ancestor_order(graph, node_map, cat_name)?;

        // Build parent baselines list
        let mut parent_baselines: Vec<(String, IndexMap<String, PermValue>)> = Vec::new();
        for ancestor_name in &ancestors {
            let ancestor_template = templates
                .get(ancestor_name)
                .with_context(|| format!("Ancestor template '{}' not found", ancestor_name))?;

            // Parse ancestor's baseline.permission as command map
            let ancestor_baseline: IndexMap<String, PermValue> =
                serde_json::from_value(ancestor_template.baseline.permission.clone())
                    .with_context(|| {
                        format!(
                            "Failed to parse baseline of ancestor template '{}'",
                            ancestor_name
                        )
                    })?;

            parent_baselines.push((ancestor_name.clone(), ancestor_baseline));
        }

        // Parse self's baseline
        let self_baseline: IndexMap<String, PermValue> =
            serde_json::from_value(template.baseline.permission.clone())
                .with_context(|| format!("Failed to parse baseline of template '{}'", cat_name))?;

        // Merge: parents first, then self wins
        let merged = resolver::merge_baselines(&parent_baselines, &self_baseline, cat_name)?;
        effective.insert(cat_name.clone(), merged);
    }

    Ok(effective)
}

/// Parse the generated JSONC back and verify every agent's permission matches
/// the resolved data.  Returns an error with details on any mismatch.
fn self_validate(generated: &str, expected: &Sot) -> Result<()> {
    let parsed_value = jsonc_parser::parse_to_serde_value(generated, &Default::default())
        .map_err(|e| anyhow::anyhow!("Self-validation: JSONC parse failed: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("Self-validation: generated empty JSONC"))?;

    let parsed_sot: Sot = serde_json::from_value(parsed_value)
        .context("Self-validation: failed to deserialize generated JSONC as Sot")?;

    // Check agent count
    if parsed_sot.len() != expected.len() {
        anyhow::bail!(
            "Self-validation: agent count mismatch: generated {} vs expected {}",
            parsed_sot.len(),
            expected.len()
        );
    }

    // Check each agent
    for (agent_name, expected_agent) in expected {
        let parsed_agent = parsed_sot.get(agent_name).ok_or_else(|| {
            anyhow::anyhow!(
                "Self-validation: agent '{}' missing from generated output",
                agent_name
            )
        })?;

        if parsed_agent.permission != expected_agent.permission {
            anyhow::bail!(
                "Self-validation: permission mismatch for agent '{}'\n  Expected: {:?}\n  Got: {:?}",
                agent_name,
                expected_agent.permission,
                parsed_agent.permission
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BashPermission, SotPermission};

    /// Integration test: run the full pipeline against the real data in the
    /// project root and verify it succeeds (returns Ok).
    #[test]
    fn test_generate_permissions_succeeds() {
        let base_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = crate::config::ProjectConfig::resolve(base_dir, None, None, None).unwrap();
        let result = generate_permissions(&config);
        assert!(result.is_ok(), "generate_permissions failed: {:?}", result);

        let output = result.unwrap();
        // Basic sanity checks on the output
        assert!(output.starts_with("//"), "should start with a comment");
        assert!(output.contains('{'), "should contain opening brace");
        assert!(output.ends_with('}'), "should end with closing brace");
    }

    /// Verify the generated output is deterministic: two runs produce the
    /// same string.
    #[test]
    fn test_generate_permissions_deterministic() {
        let base_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = crate::config::ProjectConfig::resolve(base_dir, None, None, None).unwrap();
        let a = generate_permissions(&config).unwrap();
        let b = generate_permissions(&config).unwrap();
        assert_eq!(a, b, "output must be deterministic across runs");
    }

    /// Verify that self_validate catches an agent count mismatch.
    #[test]
    fn test_self_validate_catches_count_mismatch() {
        let generated = r#"{ "agent-a": { "permission": { "bash": "deny" } } }"#;

        // Expected has two agents but generated only has one
        let mut expected: Sot = IndexMap::new();
        expected.insert(
            "agent-a".to_string(),
            SotAgent {
                permission: SotPermission {
                    write: None,
                    edit: None,
                    bash: BashPermission::Simple("deny".to_string()),
                },
            },
        );
        expected.insert(
            "agent-b".to_string(),
            SotAgent {
                permission: SotPermission {
                    write: None,
                    edit: None,
                    bash: BashPermission::Simple("deny".to_string()),
                },
            },
        );

        let result = self_validate(generated, &expected);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("agent count mismatch"), "got: {}", err);
    }

    /// Verify that self_validate catches a missing agent.
    #[test]
    fn test_self_validate_catches_missing_agent() {
        let generated = r#"{ "agent-a": { "permission": { "bash": "deny" } }, "wrong-name": { "permission": { "bash": "deny" } } }"#;

        let mut expected: Sot = IndexMap::new();
        expected.insert(
            "agent-a".to_string(),
            SotAgent {
                permission: SotPermission {
                    write: None,
                    edit: None,
                    bash: BashPermission::Simple("deny".to_string()),
                },
            },
        );
        expected.insert(
            "agent-b".to_string(),
            SotAgent {
                permission: SotPermission {
                    write: None,
                    edit: None,
                    bash: BashPermission::Simple("deny".to_string()),
                },
            },
        );

        let result = self_validate(generated, &expected);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("agent-b"), "got: {}", err);
    }

    /// Verify that self_validate catches a permission value mismatch.
    #[test]
    fn test_self_validate_catches_permission_mismatch() {
        let generated = r#"{ "agent-a": { "permission": { "bash": "allow" } } }"#;

        let mut expected: Sot = IndexMap::new();
        expected.insert(
            "agent-a".to_string(),
            SotAgent {
                permission: SotPermission {
                    write: None,
                    edit: None,
                    bash: BashPermission::Simple("deny".to_string()),
                },
            },
        );

        let result = self_validate(generated, &expected);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("permission mismatch"), "got: {}", err);
    }

    /// Verify that self_validate passes when generated and expected match.
    #[test]
    fn test_self_validate_passes_on_match() {
        let generated = r#"{ "agent-a": { "permission": { "bash": "deny" } } }"#;

        let mut expected: Sot = IndexMap::new();
        expected.insert(
            "agent-a".to_string(),
            SotAgent {
                permission: SotPermission {
                    write: None,
                    edit: None,
                    bash: BashPermission::Simple("deny".to_string()),
                },
            },
        );

        let result = self_validate(generated, &expected);
        assert!(result.is_ok(), "self_validate should pass: {:?}", result);
    }

    /// Verify that self_validate rejects unparseable JSONC.
    #[test]
    fn test_self_validate_rejects_invalid_jsonc() {
        let generated = "not valid json at all {{{";

        let expected: Sot = IndexMap::new();
        let result = self_validate(generated, &expected);
        assert!(result.is_err());
    }

    /// Verify that generate_permissions works with template inheritance.
    /// The vcs-permissions mixin is loaded and merged into all 10 complex
    /// templates that use $extends: ["vcs-permissions"].
    #[test]
    fn test_generate_permissions_with_inheritance() {
        let base_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = crate::config::ProjectConfig::resolve(base_dir, None, None, None).unwrap();
        let result = generate_permissions(&config);
        assert!(
            result.is_ok(),
            "generate_permissions with inheritance failed: {:?}",
            result
        );

        let output = result.unwrap();
        // The output must contain VCS permissions (inherited from vcs-permissions mixin)
        assert!(
            output.contains("git status"),
            "output should contain git permissions from VCS mixin"
        );
        assert!(
            output.contains("jj log"),
            "output should contain jj permissions from VCS mixin"
        );
        assert!(
            output.contains("rtk git"),
            "output should contain rtk git permissions from VCS mixin"
        );
        // The output must also contain category-specific entries
        assert!(
            output.contains("npm"),
            "output should contain npm from backend-api template"
        );
    }
}
