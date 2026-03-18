use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;

use crate::generator::{self, GenerateResult};
use crate::models::{is_simple_category, CategoryMap, Sot, Template, ValidationReport};

/// Generate the audit-report.md file.
pub fn generate_report(
    sot: &Sot,
    categories: &CategoryMap,
    templates: &IndexMap<String, Template>,
    validation: &ValidationReport,
    agents_dir: &Path,
    output_path: &Path,
) -> Result<()> {
    let agents_display = agents_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("_agents");
    // Compute deltas for the report
    let mut deltas: Vec<GenerateResult> = Vec::new();
    for (cat_name, agents) in categories {
        let template = templates
            .get(cat_name.as_str())
            .with_context(|| format!("No template for category '{}'", cat_name))?;
        for agent_name in agents {
            let delta = generator::compute_delta(sot, template, agent_name, cat_name);
            deltas.push(GenerateResult {
                agent: agent_name.clone(),
                category: cat_name.clone(),
                adds: delta.adds.len(),
                removes: delta.removes.len(),
            });
        }
    }
    deltas.sort_by(|a, b| a.category.cmp(&b.category).then(a.agent.cmp(&b.agent)));

    let total_agents = deltas.len();
    let total_categories = categories.len();
    let zero_delta = deltas.iter().filter(|d| d.adds + d.removes == 0).count();
    let with_overrides = total_agents - zero_delta;
    let pass_status = if validation.failed == 0 {
        format!("\u{2705} PASS ({}/{})", validation.passed, validation.total)
    } else {
        format!("\u{274c} FAIL ({}/{})", validation.passed, validation.total)
    };

    let mut md = String::new();

    // Header
    md.push_str("# Agency Audit Report\n\n");
    md.push_str(&format!("**Generated:** {}\n", chrono_date()));
    md.push_str(&format!(
        "**Agents:** {} | **Categories:** {} | **Round-trip validation:** {}\n\n",
        total_agents, total_categories, pass_status
    ));
    md.push_str("---\n\n");

    // Executive Summary
    md.push_str("## Executive Summary\n\n");
    md.push_str(&format!(
        "All {} agents' permissions are generated from **category template baselines** ({} templates in `_templates/`) and **per-agent agent files** ({} files in `{}/`). The generated `permissions.jsonc` is produced by `generate-permissions` and validated against templates + agents.\n\n",
        total_agents, total_categories, total_agents, agents_display
    ));
    md.push_str("**Key findings:**\n\n");
    md.push_str(&format!(
        "- **{} of {} agents ({}%)** are zero-delta \u{2014} their SOT matches their category template exactly\n",
        zero_delta,
        total_agents,
        zero_delta * 100 / total_agents
    ));
    md.push_str(&format!(
        "- **{} agents ({}%)** have overrides, ranging from {} to {} entries\n",
        with_overrides,
        with_overrides * 100 / total_agents,
        deltas
            .iter()
            .filter(|d| d.adds + d.removes > 0)
            .map(|d| d.adds + d.removes)
            .min()
            .unwrap_or(0),
        deltas.iter().map(|d| d.adds + d.removes).max().unwrap_or(0)
    ));

    // Anomaly detection
    let anomalies: Vec<&GenerateResult> = deltas.iter().filter(|d| d.removes >= 10).collect();
    if !anomalies.is_empty() {
        for a in &anomalies {
            md.push_str(&format!(
                "- **1 anomaly flagged**: `{}` has {} removes from `{}`, suggesting possible miscategorization\n",
                a.agent, a.removes, a.category
            ));
        }
    }

    let mixin_count = templates
        .iter()
        .filter(|(name, t)| t.extends.is_none() && !categories.contains_key(*name))
        .count();
    let leaf_count = templates
        .iter()
        .filter(|(_, t)| t.extends.is_some())
        .count();
    if mixin_count > 0 || leaf_count > 0 {
        md.push_str(&format!(
            "- **Template inheritance**: {} mixin template(s), {} leaf template(s) using `$extends`\n",
            mixin_count, leaf_count
        ));
    }

    md.push_str("\n---\n\n");

    // Template Inheritance
    md.push_str("## Template Inheritance\n\n");
    md.push_str("| Template | Type | Inherits From | Agents |\n");
    md.push_str("| --- | --- | --- | :---: |\n");

    for (cat_name, template) in templates {
        let template_type;
        let inherits_from;
        if let Some(ref extends) = template.extends {
            template_type = "Leaf";
            inherits_from = extends.parents().join(", ");
        } else if categories.contains_key(cat_name) {
            template_type = "Root";
            inherits_from = "\u{2014}".to_string();
        } else {
            template_type = "Mixin";
            inherits_from = "\u{2014}".to_string();
        };
        let agent_count = categories
            .get(cat_name)
            .map(|a| a.len().to_string())
            .unwrap_or_else(|| "\u{2014}".to_string());
        md.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            cat_name, template_type, inherits_from, agent_count
        ));
    }
    md.push_str("\n---\n\n");

    // Delta Summary Table
    md.push_str("## Delta Summary Table\n\n");
    md.push_str("| Agent                             | Category           | Adds | Removes | Total | Notes                                              |\n");
    md.push_str("| --------------------------------- | ------------------ | :--: | :-----: | :---: | -------------------------------------------------- |\n");

    for d in &deltas {
        let total = d.adds + d.removes;
        let notes = build_notes(d, sot, templates);
        md.push_str(&format!(
            "| **{}**{} | {}{} | {:>4} | {:>7} | {:>5} | {} |\n",
            d.agent,
            " ".repeat(33usize.saturating_sub(d.agent.len() + 4)),
            d.category,
            " ".repeat(18usize.saturating_sub(d.category.len())),
            total.min(9999).to_string(),
            format!("{}", d.removes),
            format!("{}", total),
            notes,
        ));
    }

    md.push_str("\n---\n\n");

    // Distribution by Delta Size
    md.push_str("## Distribution by Delta Size\n\n");
    let mut dist: IndexMap<usize, Vec<String>> = IndexMap::new();
    for d in &deltas {
        let total = d.adds + d.removes;
        dist.entry(total).or_default().push(d.agent.clone());
    }
    // Sort by delta size
    let mut dist_sorted: Vec<(usize, Vec<String>)> = dist.into_iter().collect();
    dist_sorted.sort_by_key(|(k, _)| *k);

    md.push_str("| Delta | Count | Agents |\n");
    md.push_str("| ----: | :---: | --- |\n");
    for (size, agents) in &dist_sorted {
        let agent_list = if agents.len() > 8 {
            let preview: Vec<&str> = agents.iter().take(8).map(|s| s.as_str()).collect();
            format!("{}, ...", preview.join(", "))
        } else {
            agents.join(", ")
        };
        md.push_str(&format!(
            "| {} | {} | {} |\n",
            size,
            agents.len(),
            agent_list
        ));
    }

    md.push_str("\n---\n\n");

    // Category Profiles
    md.push_str("## Category Profiles\n\n");
    md.push_str("### Zero-Delta Categories (all agents match template exactly)\n\n");
    md.push_str("| Category | Agents | Template Type |\n");
    md.push_str("| --- | :---: | --- |\n");

    for (cat_name, agents) in categories {
        let all_zero = agents.iter().all(|a| {
            deltas
                .iter()
                .any(|d| d.agent == *a && d.adds + d.removes == 0)
        });
        if all_zero {
            let template_type = if cat_name == "general-purpose" {
                "`\"write/edit/bash\": \"deny\"`".to_string()
            } else if cat_name == "unrestricted" {
                "`\"*\": \"allow\"`".to_string()
            } else if is_simple_category(cat_name) {
                "`\"bash\": \"deny\"`".to_string()
            } else {
                "Complex bash object".to_string()
            };
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                cat_name,
                agents.len(),
                template_type
            ));
        }
    }

    md.push_str("\n### Categories With Overrides\n\n");

    for (cat_name, agents) in categories {
        let cat_deltas: Vec<&GenerateResult> =
            deltas.iter().filter(|d| d.category == *cat_name).collect();
        let has_overrides = cat_deltas.iter().any(|d| d.adds + d.removes > 0);
        if !has_overrides {
            continue;
        }

        let override_count = cat_deltas.iter().filter(|d| d.adds + d.removes > 0).count();
        md.push_str(&format!(
            "**{}** ({} agents, {} with overrides)\n\n",
            cat_name,
            agents.len(),
            override_count
        ));

        for d in &cat_deltas {
            let total = d.adds + d.removes;
            if total > 0 {
                md.push_str(&format!(
                    "- `{}`: {} adds, {} removes\n",
                    d.agent, d.adds, d.removes
                ));
            } else {
                md.push_str(&format!("- `{}`: matches baseline exactly\n", d.agent));
            }
        }
        md.push('\n');
    }

    md.push_str("---\n\n");

    // Anomalies
    if !anomalies.is_empty() {
        md.push_str("## Anomalies\n\n");
        for a in &anomalies {
            md.push_str(&format!(
                "### \u{26a0}\u{fe0f} `{}` \u{2014} Possible Miscategorization\n\n",
                a.agent
            ));
            md.push_str(&format!("**Current category:** `{}`\n", a.category));
            md.push_str(&format!(
                "**Delta:** {} adds, {} removes (total: {})\n\n",
                a.adds,
                a.removes,
                a.adds + a.removes
            ));
        }
        md.push_str("---\n\n");
    }

    // File Inventory
    md.push_str("## File Inventory\n\n");
    md.push_str("| Path | Description |\n");
    md.push_str("| --- | --- |\n");
    md.push_str(&format!(
        "| `permissions.jsonc` | Generated from templates + {} |\n",
        agents_display
    ));
    md.push_str(&format!(
        "| `_templates/*.jsonc` | {} templates ({} leaf + {} mixin + {} root) |\n",
        templates.len(),
        templates
            .iter()
            .filter(|(_, t)| t.extends.is_some())
            .count(),
        templates
            .iter()
            .filter(|(name, t)| t.extends.is_none() && !categories.contains_key(*name))
            .count(),
        templates
            .iter()
            .filter(|(name, t)| t.extends.is_none() && categories.contains_key(*name))
            .count(),
    ));
    md.push_str(&format!(
        "| `{}/*.jsonc` | 44 per-agent files |\n",
        agents_display
    ));
    md.push_str("| `audit-report.md` | This report |\n\n");

    md.push_str("---\n\n");

    // Validation
    md.push_str("## Validation\n\n");
    md.push_str("```\n");
    md.push_str("$ agency validate\n\n");
    md.push_str(&format!(
        "Round-trip validation: {}/{} agents passed\n\n",
        validation.passed, validation.total
    ));
    if validation.failed == 0 {
        md.push_str(&format!(
            "RESULT: PASS \u{2014} permissions.jsonc matches templates + {}\n",
            agents_display
        ));
    } else {
        md.push_str(&format!(
            "RESULT: FAIL ({} agent(s) have mismatches)\n",
            validation.failed
        ));
    }
    md.push_str("```\n");

    std::fs::write(output_path, &md)
        .with_context(|| format!("Writing report to {}", output_path.display()))?;

    println!("Audit report written to {}", output_path.display());
    Ok(())
}

/// Build a short notes string for the delta summary table.
fn build_notes(d: &GenerateResult, sot: &Sot, templates: &IndexMap<String, Template>) -> String {
    if d.adds + d.removes == 0 {
        return String::new();
    }

    let template = match templates.get(&d.category) {
        Some(t) => t,
        None => return String::new(),
    };

    let delta = generator::compute_delta(sot, template, &d.agent, &d.category);

    let mut parts = Vec::new();
    if !delta.adds.is_empty() {
        let add_keys: Vec<String> = delta
            .adds
            .iter()
            .take(4)
            .map(|(k, _)| format!("+`{}`", k))
            .collect();
        parts.push(add_keys.join(", "));
    }
    if !delta.removes.is_empty() && delta.removes.len() <= 3 {
        let rem_keys: Vec<String> = delta
            .removes
            .iter()
            .take(3)
            .map(|k| format!("\u{2212}`{}`", k))
            .collect();
        parts.push(rem_keys.join(", "));
    }

    parts.join(", ")
}

/// Simple date string (YYYY-MM-DD) without pulling in chrono.
fn chrono_date() -> String {
    // We'll use a simple approach: read from system
    let now = std::time::SystemTime::now();
    let since_epoch = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since_epoch.as_secs();

    // Simple date calculation
    let days = secs / 86400;
    let (year, month, day) = days_to_date(days);
    format!("{:04}-{:02}-{:02}", year, month, day)
}

/// Convert days since epoch to (year, month, day).
fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
