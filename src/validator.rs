use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;

use crate::loader;
use crate::models::{
    AgentValidationResult, BashPermission, CategoryMap, Sot, SotPermission, Template,
    ValidationReport,
};
use crate::resolver;

/// Validate that for every agent: resolve(template, override) matches permissions.jsonc.
pub fn validate_all(
    sot: &Sot,
    categories: &CategoryMap,
    templates: &IndexMap<String, Template>,
    agents_dir: &Path,
) -> Result<ValidationReport> {
    let mut results = Vec::new();
    let mut passed = 0;
    let mut failed = 0;

    // Build DAG for effective baseline computation
    let (graph, node_map) = crate::template_dag::build_dag(templates)?;
    crate::template_dag::detect_cycles(&graph)?;

    // Pre-compute effective baselines
    let effective_baselines =
        compute_effective_baselines_for_validation(templates, &graph, &node_map)?;

    for (cat_name, agents) in categories {
        let template = templates
            .get(cat_name.as_str())
            .with_context(|| format!("No template loaded for category '{}'", cat_name))?;

        for agent_name in agents {
            let override_data = loader::load_override(agents_dir, agent_name)
                .with_context(|| format!("Loading override for agent '{}'", agent_name))?;

            let resolved = if let Some(effective) = effective_baselines.get(cat_name.as_str()) {
                crate::resolver::resolve_with_baseline(effective, cat_name, &override_data)
                    .with_context(|| format!("Resolving agent '{}'", agent_name))?
            } else {
                resolver::resolve(template, &override_data)
                    .with_context(|| format!("Resolving agent '{}'", agent_name))?
            };

            let sot_entry = sot
                .get(agent_name.as_str())
                .with_context(|| format!("Agent '{}' not found in SOT", agent_name))?;

            let validation = compare_permissions(
                agent_name,
                cat_name,
                &resolved.permission,
                &sot_entry.permission,
            );

            if validation.passed {
                passed += 1;
            } else {
                failed += 1;
            }
            results.push(validation);
        }
    }

    Ok(ValidationReport {
        total: passed + failed,
        passed,
        failed,
        results,
    })
}

/// Pre-compute effective baselines for templates with `$extends`.
fn compute_effective_baselines_for_validation(
    templates: &IndexMap<String, Template>,
    graph: &petgraph::graph::DiGraph<String, ()>,
    node_map: &std::collections::HashMap<String, petgraph::graph::NodeIndex>,
) -> Result<IndexMap<String, IndexMap<String, crate::models::PermValue>>> {
    let mut effective: IndexMap<String, IndexMap<String, crate::models::PermValue>> =
        IndexMap::new();

    for (cat_name, template) in templates {
        if template.extends.is_none() {
            continue;
        }

        let ancestors = crate::template_dag::ancestor_order(graph, node_map, cat_name)?;

        let mut parent_baselines: Vec<(String, IndexMap<String, crate::models::PermValue>)> =
            Vec::new();
        for ancestor_name in &ancestors {
            let ancestor_template = templates
                .get(ancestor_name)
                .with_context(|| format!("Ancestor template '{}' not found", ancestor_name))?;

            let ancestor_baseline: IndexMap<String, crate::models::PermValue> =
                serde_json::from_value(ancestor_template.baseline.permission.clone())
                    .with_context(|| {
                        format!("Failed to parse baseline of ancestor '{}'", ancestor_name)
                    })?;

            parent_baselines.push((ancestor_name.clone(), ancestor_baseline));
        }

        let self_baseline: IndexMap<String, crate::models::PermValue> =
            serde_json::from_value(template.baseline.permission.clone())
                .with_context(|| format!("Failed to parse baseline of '{}'", cat_name))?;

        let merged = crate::resolver::merge_baselines(&parent_baselines, &self_baseline, cat_name)?;
        effective.insert(cat_name.clone(), merged);
    }

    Ok(effective)
}

/// Compare resolved permissions against SOT permissions for one agent.
fn compare_permissions(
    agent: &str,
    category: &str,
    resolved: &SotPermission,
    sot: &SotPermission,
) -> AgentValidationResult {
    let mut extra_in_resolved = Vec::new();
    let mut missing_from_resolved = Vec::new();
    let mut value_mismatches = Vec::new();

    // Compare write/edit
    if resolved.write != sot.write {
        value_mismatches.push((
            "write".to_string(),
            format!("{:?}", resolved.write),
            format!("{:?}", sot.write),
        ));
    }
    if resolved.edit != sot.edit {
        value_mismatches.push((
            "edit".to_string(),
            format!("{:?}", resolved.edit),
            format!("{:?}", sot.edit),
        ));
    }

    // Compare bash
    match (&resolved.bash, &sot.bash) {
        (BashPermission::Simple(a), BashPermission::Simple(b)) => {
            if a != b {
                value_mismatches.push(("bash".to_string(), a.clone(), b.clone()));
            }
        }
        (BashPermission::Detailed(a), BashPermission::Detailed(b)) => {
            // Keys in resolved but not in SOT, or values differ
            for (k, v) in a {
                match b.get(k) {
                    Some(sv) if sv == v => {}
                    Some(sv) => {
                        value_mismatches.push((k.clone(), v.clone(), sv.clone()));
                    }
                    None => {
                        extra_in_resolved.push(k.clone());
                    }
                }
            }
            // Keys in SOT but not in resolved
            for k in b.keys() {
                if !a.contains_key(k) {
                    missing_from_resolved.push(k.clone());
                }
            }
        }
        (BashPermission::Simple(a), BashPermission::Detailed(_)) => {
            value_mismatches.push((
                "bash".to_string(),
                format!("Simple({})", a),
                "Detailed(...)".to_string(),
            ));
        }
        (BashPermission::Detailed(_), BashPermission::Simple(b)) => {
            value_mismatches.push((
                "bash".to_string(),
                "Detailed(...)".to_string(),
                format!("Simple({})", b),
            ));
        }
    }

    let passed = extra_in_resolved.is_empty()
        && missing_from_resolved.is_empty()
        && value_mismatches.is_empty();

    AgentValidationResult {
        agent: agent.to_string(),
        category: category.to_string(),
        passed,
        extra_in_resolved,
        missing_from_resolved,
        value_mismatches,
    }
}

/// Print the validation report summary.
pub fn print_report(report: &ValidationReport) {
    println!(
        "\nRound-trip validation: {}/{} agents passed\n",
        report.passed, report.total
    );

    if report.failed > 0 {
        println!("Errors:");
        for r in &report.results {
            if !r.passed {
                println!("  FAIL  {} ({})", r.agent, r.category);
                if !r.extra_in_resolved.is_empty() {
                    let keys: Vec<&str> = r
                        .extra_in_resolved
                        .iter()
                        .take(5)
                        .map(|s| s.as_str())
                        .collect();
                    let ellipsis = if r.extra_in_resolved.len() > 5 {
                        "..."
                    } else {
                        ""
                    };
                    println!("         Extra in resolved: {:?}{}", keys, ellipsis);
                }
                if !r.missing_from_resolved.is_empty() {
                    let keys: Vec<&str> = r
                        .missing_from_resolved
                        .iter()
                        .take(5)
                        .map(|s| s.as_str())
                        .collect();
                    let ellipsis = if r.missing_from_resolved.len() > 5 {
                        "..."
                    } else {
                        ""
                    };
                    println!("         Missing from resolved: {:?}{}", keys, ellipsis);
                }
                for (k, rv, sv) in &r.value_mismatches {
                    println!(
                        "         Mismatch on '{}': resolved='{}' vs sot='{}'",
                        k, rv, sv
                    );
                }
            }
        }
        println!();
        println!("RESULT: FAIL ({} agent(s) have mismatches)", report.failed);
    } else {
        println!("RESULT: PASS \u{2014} permissions.jsonc matches templates + agents");
    }
}
