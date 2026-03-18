use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;

use crate::models::{is_simple_category, CategoryMap, Delta, Sot, Template};

// ---------------------------------------------------------------------------
// Group Classification
// ---------------------------------------------------------------------------

/// Configurable command group classifier.
///
/// Maps command prefixes to human-readable group names. Key insertion order
/// in the underlying `IndexMap` defines display order. Commands that don't
/// match any prefix are classified as "Other".
pub struct CommandGroupClassifier {
    groups: IndexMap<String, Vec<String>>,
}

impl CommandGroupClassifier {
    pub fn new(groups: IndexMap<String, Vec<String>>) -> Self {
        Self { groups }
    }

    /// Classify a command key into its group name.
    pub fn get_group(&self, key: &str) -> String {
        let k = key.to_lowercase();
        for (group_name, prefixes) in &self.groups {
            for prefix in prefixes {
                if k == *prefix || k.starts_with(&format!("{} ", prefix)) {
                    return group_name.clone();
                }
            }
        }
        "Other".to_string()
    }

    /// Return the group display order: configured groups followed by "Other".
    pub fn group_order(&self) -> Vec<String> {
        let mut order: Vec<String> = self.groups.keys().cloned().collect();
        order.push("Other".to_string());
        order
    }
}

// ---------------------------------------------------------------------------
// Delta Computation
// ---------------------------------------------------------------------------

/// Compute the delta between SOT and template baseline for one agent.
pub fn compute_delta(
    sot: &Sot,
    template: &Template,
    agent_name: &str,
    category_name: &str,
) -> Delta {
    let sot_entry = match sot.get(agent_name) {
        Some(e) => e,
        None => {
            return Delta {
                adds: vec![],
                removes: vec![],
            };
        }
    };

    if is_simple_category(category_name) {
        compute_simple_delta(sot_entry, template)
    } else {
        compute_complex_delta(sot_entry, template)
    }
}

/// Delta for simple categories: compare the full permission objects.
fn compute_simple_delta(sot_entry: &crate::models::SotAgent, template: &Template) -> Delta {
    // Parse template baseline as a map
    let template_perm: IndexMap<String, serde_json::Value> =
        serde_json::from_value(template.baseline.permission.clone()).unwrap_or_default();

    // Build SOT permission as a map
    let mut sot_perm: IndexMap<String, serde_json::Value> = IndexMap::new();
    if let Some(ref w) = sot_entry.permission.write {
        sot_perm.insert("write".to_string(), serde_json::Value::String(w.clone()));
    }
    if let Some(ref e) = sot_entry.permission.edit {
        sot_perm.insert("edit".to_string(), serde_json::Value::String(e.clone()));
    }
    match &sot_entry.permission.bash {
        crate::models::BashPermission::Simple(s) => {
            sot_perm.insert("bash".to_string(), serde_json::Value::String(s.clone()));
        }
        crate::models::BashPermission::Detailed(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                .collect();
            sot_perm.insert("bash".to_string(), serde_json::Value::Object(obj));
        }
    }

    let mut adds = Vec::new();
    let mut removes = Vec::new();

    // Keys in SOT but not in template, or values differ
    for (k, v) in &sot_perm {
        match template_perm.get(k) {
            Some(tv) if tv == v => {}
            _ => {
                let val_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                adds.push((k.clone(), val_str));
            }
        }
    }
    // Keys in template but not in SOT
    for k in template_perm.keys() {
        if !sot_perm.contains_key(k) {
            removes.push(k.clone());
        }
    }

    Delta { adds, removes }
}

/// Compute the delta between SOT and an effective baseline for one agent.
///
/// This is the DAG-aware version — the effective baseline is pre-computed
/// by merging the template's inheritance chain.
pub fn compute_delta_from_baseline(
    sot: &Sot,
    effective_baseline: &IndexMap<String, crate::models::PermValue>,
    agent_name: &str,
    category_name: &str,
) -> Delta {
    let sot_entry = match sot.get(agent_name) {
        Some(e) => e,
        None => {
            return Delta {
                adds: vec![],
                removes: vec![],
            };
        }
    };

    if is_simple_category(category_name) {
        // For simple categories, effective_baseline has top-level keys
        let template_perm: IndexMap<String, serde_json::Value> = effective_baseline
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();

        // Build SOT permission as a map (same as compute_simple_delta)
        let mut sot_perm: IndexMap<String, serde_json::Value> = IndexMap::new();
        if let Some(ref w) = sot_entry.permission.write {
            sot_perm.insert("write".to_string(), serde_json::Value::String(w.clone()));
        }
        if let Some(ref e) = sot_entry.permission.edit {
            sot_perm.insert("edit".to_string(), serde_json::Value::String(e.clone()));
        }
        match &sot_entry.permission.bash {
            crate::models::BashPermission::Simple(s) => {
                sot_perm.insert("bash".to_string(), serde_json::Value::String(s.clone()));
            }
            crate::models::BashPermission::Detailed(map) => {
                let obj: serde_json::Map<String, serde_json::Value> = map
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                sot_perm.insert("bash".to_string(), serde_json::Value::Object(obj));
            }
        }

        let mut adds = Vec::new();
        let mut removes = Vec::new();

        for (k, v) in &sot_perm {
            match template_perm.get(k) {
                Some(tv) if tv == v => {}
                _ => {
                    let val_str = match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    adds.push((k.clone(), val_str));
                }
            }
        }
        for k in template_perm.keys() {
            if !sot_perm.contains_key(k) {
                removes.push(k.clone());
            }
        }

        Delta { adds, removes }
    } else {
        // For complex categories, effective_baseline IS the bash command map
        let sot_bash = match &sot_entry.permission.bash {
            crate::models::BashPermission::Simple(_) => {
                return Delta {
                    adds: vec![],
                    removes: vec![],
                };
            }
            crate::models::BashPermission::Detailed(map) => map,
        };

        let mut adds = Vec::new();
        let mut removes = Vec::new();

        for (k, v) in sot_bash {
            match effective_baseline.get(k) {
                Some(tv) if tv == v => {}
                _ => {
                    adds.push((k.clone(), v.clone()));
                }
            }
        }
        for k in effective_baseline.keys() {
            if !sot_bash.contains_key(k) {
                removes.push(k.clone());
            }
        }

        Delta { adds, removes }
    }
}

/// Delta for complex categories: compare bash sub-objects.
fn compute_complex_delta(sot_entry: &crate::models::SotAgent, template: &Template) -> Delta {
    // Template baseline.permission IS the bash sub-object
    let template_bash: IndexMap<String, String> =
        serde_json::from_value(template.baseline.permission.clone()).unwrap_or_default();

    // SOT bash sub-object
    let sot_bash = match &sot_entry.permission.bash {
        crate::models::BashPermission::Simple(_) => {
            // Shouldn't happen for complex categories
            return Delta {
                adds: vec![],
                removes: vec![],
            };
        }
        crate::models::BashPermission::Detailed(map) => map,
    };

    let mut adds = Vec::new();
    let mut removes = Vec::new();

    // Keys in SOT but not in template, or values differ
    for (k, v) in sot_bash {
        match template_bash.get(k) {
            Some(tv) if tv == v => {}
            _ => {
                adds.push((k.clone(), v.clone()));
            }
        }
    }
    // Keys in template but not in SOT
    for k in template_bash.keys() {
        if !sot_bash.contains_key(k) {
            removes.push(k.clone());
        }
    }

    Delta { adds, removes }
}

// ---------------------------------------------------------------------------
// JSONC Output
// ---------------------------------------------------------------------------

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

/// Build the full JSONC override file content.
fn build_override_jsonc(
    agent_name: &str,
    category_name: &str,
    delta: &Delta,
    classifier: &CommandGroupClassifier,
) -> String {
    let mut lines = Vec::new();
    let add_count = delta.adds.len();
    let remove_count = delta.removes.len();
    let total = add_count + remove_count;

    // Header comments
    lines.push(format!("// Override: {}", agent_name));
    lines.push(format!("// Extends: {}", category_name));

    if total == 0 {
        lines.push("// Delta: none".to_string());
    } else {
        let mut parts = Vec::new();
        if add_count > 0 {
            let word = if add_count == 1 { "entry" } else { "entries" };
            parts.push(format!("adds {} {}", add_count, word));
        }
        if remove_count > 0 {
            let word = if remove_count == 1 {
                "entry"
            } else {
                "entries"
            };
            parts.push(format!("removes {} {}", remove_count, word));
        }
        lines.push(format!("// Delta: {}", parts.join(", ")));
    }

    lines.push("{".to_string());
    lines.push(r#"  "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json","#.to_string());
    lines.push(r#"  "$version": "1.0.0","#.to_string());
    lines.push(format!("  \"agent\": {},", json_str(agent_name)));
    lines.push(format!("  \"$extends\": {},", json_str(category_name)));

    if total == 0 {
        lines.push("  \"overrides\": {}".to_string());
    } else {
        lines.push("  \"overrides\": {".to_string());
        lines.push("    \"permission\": {".to_string());

        let has_add = add_count > 0;
        let has_remove = remove_count > 0;

        if has_add {
            lines.push("      \"add\": {".to_string());

            let use_groups = add_count >= 3;
            if use_groups {
                let mut grouped: IndexMap<String, Vec<(&str, &str)>> = IndexMap::new();
                for (k, v) in &delta.adds {
                    let g = classifier.get_group(k);
                    grouped.entry(g).or_default().push((k.as_str(), v.as_str()));
                }

                // Order by classifier group_order
                let group_order = classifier.group_order();
                let mut ordered: Vec<(String, Vec<(&str, &str)>)> = Vec::new();
                for g in &group_order {
                    if let Some(entries) = grouped.get(g) {
                        ordered.push((g.clone(), entries.clone()));
                    }
                }
                // Any remaining not in group_order
                for (g, entries) in &grouped {
                    if !ordered.iter().any(|(og, _)| og == g) {
                        ordered.push((g.clone(), entries.clone()));
                    }
                }

                // Flatten with group markers
                let mut all_entries: Vec<(&str, &str, bool)> = Vec::new(); // (key, value, is_group_header)
                for (g_name, g_entries) in &ordered {
                    all_entries.push((g_name, "", true));
                    for (k, v) in g_entries {
                        all_entries.push((k, v, false));
                    }
                }

                for (idx, (k, v, is_group)) in all_entries.iter().enumerate() {
                    if *is_group {
                        lines.push(format!("        // --- {} ---", k));
                    } else {
                        // Check if this is the last non-group entry
                        let is_last = !all_entries[idx + 1..].iter().any(|(_, _, ig)| !ig);
                        let comma = if is_last { "" } else { "," };
                        lines.push(format!("        {}: {}{}", json_str(k), json_str(v), comma));
                    }
                }
            } else {
                for (idx, (k, v)) in delta.adds.iter().enumerate() {
                    let comma = if idx == delta.adds.len() - 1 { "" } else { "," };
                    lines.push(format!("        {}: {}{}", json_str(k), json_str(v), comma));
                }
            }

            if has_remove {
                lines.push("      },".to_string());
            } else {
                lines.push("      }".to_string());
            }
        }

        if has_remove {
            lines.push("      \"remove\": [".to_string());
            for (idx, k) in delta.removes.iter().enumerate() {
                let comma = if idx == delta.removes.len() - 1 {
                    ""
                } else {
                    ","
                };
                lines.push(format!("        {}{}", json_str(k), comma));
            }
            lines.push("      ]".to_string());
        }

        lines.push("    }".to_string());
        lines.push("  }".to_string());
    }

    lines.push("}".to_string());
    lines.join("\n") + "\n"
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Result of generating overrides
pub struct GenerateResult {
    pub agent: String,
    pub category: String,
    pub adds: usize,
    pub removes: usize,
}

/// Generate all override files.
pub fn generate_all(
    sot: &Sot,
    categories: &CategoryMap,
    templates: &IndexMap<String, Template>,
    agents_dir: &Path,
    classifier: &CommandGroupClassifier,
) -> Result<Vec<GenerateResult>> {
    std::fs::create_dir_all(agents_dir)
        .with_context(|| format!("Creating agents directory: {}", agents_dir.display()))?;

    let mut results = Vec::new();

    for (cat_name, agents) in categories {
        let template = templates
            .get(cat_name.as_str())
            .with_context(|| format!("No template for category '{}'", cat_name))?;

        for agent_name in agents {
            let delta = compute_delta(sot, template, agent_name, cat_name);
            let content = build_override_jsonc(agent_name, cat_name, &delta, classifier);

            let out_path = agents_dir.join(format!("{}.jsonc", agent_name));
            std::fs::write(&out_path, &content)
                .with_context(|| format!("Writing override file: {}", out_path.display()))?;

            results.push(GenerateResult {
                agent: agent_name.clone(),
                category: cat_name.clone(),
                adds: delta.adds.len(),
                removes: delta.removes.len(),
            });
        }
    }

    // Sort by category then agent name
    results.sort_by(|a, b| a.category.cmp(&b.category).then(a.agent.cmp(&b.agent)));

    Ok(results)
}

/// Print summary table after generation.
pub fn print_summary(results: &[GenerateResult]) {
    println!();
    println!(
        "| {:<35} | {:<18} | {:>4} | {:>7} | {:>5} |",
        "Agent", "Category", "Adds", "Removes", "Total"
    );
    println!(
        "| {:<35} | {:<18} | {:>4} | {:>7} | {:>5} |",
        "---", "---", "---", "---", "---"
    );

    for r in results {
        let total = r.adds + r.removes;
        println!(
            "| {:<35} | {:<18} | {:>4} | {:>7} | {:>5} |",
            r.agent, r.category, r.adds, r.removes, total
        );
    }

    println!();
    println!("Generated {} agent files", results.len());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_groups;

    fn make_classifier() -> CommandGroupClassifier {
        CommandGroupClassifier::new(default_groups())
    }

    #[test]
    fn test_classifier_known_prefix() {
        let c = make_classifier();
        assert_eq!(c.get_group("npm install"), "Package Managers & Runtimes");
    }

    #[test]
    fn test_classifier_exact_match() {
        let c = make_classifier();
        assert_eq!(c.get_group("docker"), "Docker");
    }

    #[test]
    fn test_classifier_unknown_command() {
        let c = make_classifier();
        assert_eq!(c.get_group("pulumi up"), "Other");
    }

    #[test]
    fn test_classifier_case_insensitive() {
        let c = make_classifier();
        assert_eq!(c.get_group("NPM install"), "Package Managers & Runtimes");
    }

    #[test]
    fn test_classifier_custom_groups() {
        let mut custom = IndexMap::new();
        custom.insert(
            "My Tools".to_string(),
            vec!["foo".to_string(), "bar".to_string()],
        );
        let c = CommandGroupClassifier::new(custom);
        assert_eq!(c.get_group("foo run"), "My Tools");
        assert_eq!(c.get_group("bar"), "My Tools");
        assert_eq!(c.get_group("baz"), "Other");
    }

    #[test]
    fn test_group_order_matches_insertion_plus_other() {
        let c = make_classifier();
        let order = c.group_order();
        // First should be "Package Managers & Runtimes"
        assert_eq!(order[0], "Package Managers & Runtimes");
        // Last should always be "Other"
        assert_eq!(order[order.len() - 1], "Other");
        // Total should be default group count + 1 (for "Other")
        assert_eq!(order.len(), default_groups().len() + 1);
    }
}
