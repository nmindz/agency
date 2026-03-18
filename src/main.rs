use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use opencode_agency::config::ProjectConfig;
use opencode_agency::generator::CommandGroupClassifier;
use opencode_agency::models::{BashPermission, SotPermission};
use opencode_agency::{
    generator, jsonc, loader, models, reporter, resolver, sot_generator, template_dag, validator,
};

#[derive(Parser)]
#[command(name = "agency", about = "OpenCode Agency — Agent Permission Manager")]
struct Cli {
    /// Override the templates directory path
    #[arg(long, global = true)]
    templates_dir: Option<PathBuf>,

    /// Override the agents directory path
    #[arg(long, global = true)]
    agents_dir: Option<PathBuf>,

    /// Override the groups file path
    #[arg(long, global = true)]
    groups_file: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate permissions.jsonc from templates + agents
    GeneratePermissions {
        /// Output file path (default: permissions.jsonc in base dir)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Compare two permissions files (source vs target)
    Compare {
        /// Source permissions file
        source: PathBuf,
        /// Target permissions file to compare against
        target: PathBuf,
    },
    /// Resolve template + override -> final permissions for one agent
    Resolve {
        /// Agent name to resolve
        agent: String,
        /// Show provenance of each permission entry
        #[arg(long)]
        trace: bool,
    },
    /// Validate that permissions.jsonc matches what templates + agents produce
    Validate,
    /// Generate audit-report.md
    AuditReport,
    /// [DEPRECATED] Generate override files from SOT
    GenerateOverrides,
    /// Check if an agent is allowed to run a command (fast permission lookup)
    Can {
        /// Agent name to check permissions for
        agent: String,
        /// Command signature to check (e.g., "git push", "npm install")
        command: String,
        /// Override permissions file path (default: permissions.jsonc in base dir)
        #[arg(short, long)]
        permissions: Option<PathBuf>,
        /// Show the matching rule and rationale for the permission decision
        #[arg(long)]
        explain: bool,
    },
}

fn base_dir() -> PathBuf {
    // Use the directory containing the executable, or fall back to current dir
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn main() {
    let cli = Cli::parse();
    let base = base_dir();

    let config =
        match ProjectConfig::resolve(base, cli.templates_dir, cli.agents_dir, cli.groups_file) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error: {:?}", e);
                process::exit(1);
            }
        };

    // Validate directories for commands that need them
    let needs_dirs = !matches!(cli.command, Commands::Compare { .. } | Commands::Can { .. });
    if needs_dirs {
        if let Err(e) = config.validate_dirs() {
            eprintln!("Error: {:#}", e);
            process::exit(1);
        }
    }

    let result = match cli.command {
        Commands::GeneratePermissions { ref output } => {
            cmd_generate_permissions(&config, output.clone())
        }
        Commands::Compare {
            ref source,
            ref target,
        } => cmd_compare(source, target),
        Commands::Resolve { ref agent, trace } => cmd_resolve(&config, agent, trace),
        Commands::Validate => cmd_validate(&config),
        Commands::AuditReport => cmd_audit_report(&config),
        Commands::GenerateOverrides => cmd_generate_overrides(&config),
        Commands::Can {
            ref agent,
            ref command,
            ref permissions,
            explain,
        } => {
            let perms_path = permissions
                .clone()
                .unwrap_or_else(|| config.base_dir.join("permissions.jsonc"));
            cmd_can(&config, &perms_path, agent, command, explain)
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {:?}", e);
        process::exit(1);
    }
}

fn cmd_generate_permissions(config: &ProjectConfig, output: Option<PathBuf>) -> Result<()> {
    let output_path = output.unwrap_or_else(|| config.base_dir.join("permissions.jsonc"));

    let content = sot_generator::generate_permissions(config)?;
    std::fs::write(&output_path, &content)
        .with_context(|| format!("Writing generated permissions to {}", output_path.display()))?;

    println!("Generated permissions file: {}", output_path.display());
    Ok(())
}

fn cmd_compare(source: &Path, target: &Path) -> Result<()> {
    let source_sot: models::Sot = jsonc::parse_file(source)
        .with_context(|| format!("Loading source file: {}", source.display()))?;
    let target_sot: models::Sot = jsonc::parse_file(target)
        .with_context(|| format!("Loading target file: {}", target.display()))?;

    println!("Comparing: {} vs {}", source.display(), target.display());
    println!();

    let mut matches = 0usize;
    let mut differences = 0usize;
    let mut only_in_source: Vec<String> = Vec::new();
    let mut only_in_target: Vec<String> = Vec::new();
    let mut mismatched: Vec<(String, Vec<String>)> = Vec::new();

    // Get all agents sorted for deterministic output
    let mut all_agents: Vec<String> = source_sot
        .keys()
        .chain(target_sot.keys())
        .cloned()
        .collect();
    all_agents.sort();
    all_agents.dedup();

    for agent_name in &all_agents {
        match (source_sot.get(agent_name), target_sot.get(agent_name)) {
            (Some(src_agent), Some(tgt_agent)) => {
                let diffs = compare_agent_permissions(&src_agent.permission, &tgt_agent.permission);
                if diffs.is_empty() {
                    println!("  PASS  {}", agent_name);
                    matches += 1;
                } else {
                    println!("  FAIL  {}", agent_name);
                    for d in &diffs {
                        println!("         {}", d);
                    }
                    differences += 1;
                    mismatched.push((agent_name.clone(), diffs));
                }
            }
            (Some(_), None) => {
                println!("  NEW   {}", agent_name);
                only_in_source.push(agent_name.clone());
                differences += 1;
            }
            (None, Some(_)) => {
                println!("  MISS  {}", agent_name);
                only_in_target.push(agent_name.clone());
                differences += 1;
            }
            (None, None) => unreachable!(),
        }
    }

    println!();

    if !only_in_source.is_empty() {
        println!(
            "Extra agents (only in source): {}",
            only_in_source.join(", ")
        );
    }
    if !only_in_target.is_empty() {
        println!(
            "Missing agents (only in target): {}",
            only_in_target.join(", ")
        );
    }

    let total = matches + differences;
    println!(
        "\nCompare: {}/{} agents match, {} difference(s) found",
        matches, total, differences
    );

    if differences > 0 {
        process::exit(1);
    }

    Ok(())
}

/// Compare two SotPermission structs, return list of difference descriptions.
fn compare_agent_permissions(a: &SotPermission, b: &SotPermission) -> Vec<String> {
    let mut diffs = Vec::new();

    if a.write != b.write {
        diffs.push(format!(
            "write: source={:?} vs target={:?}",
            a.write, b.write
        ));
    }
    if a.edit != b.edit {
        diffs.push(format!("edit: source={:?} vs target={:?}", a.edit, b.edit));
    }

    match (&a.bash, &b.bash) {
        (BashPermission::Simple(av), BashPermission::Simple(bv)) => {
            if av != bv {
                diffs.push(format!("bash: source='{}' vs target='{}'", av, bv));
            }
        }
        (BashPermission::Detailed(am), BashPermission::Detailed(bm)) => {
            for (k, v) in am {
                match bm.get(k) {
                    Some(rv) if rv == v => {}
                    Some(rv) => {
                        diffs.push(format!("bash.{}: source='{}' vs target='{}'", k, v, rv));
                    }
                    None => {
                        diffs.push(format!("bash.{}: only in source ('{}')", k, v));
                    }
                }
            }
            for k in bm.keys() {
                if !am.contains_key(k) {
                    diffs.push(format!("bash.{}: only in target ('{}')", k, bm[k]));
                }
            }
        }
        _ => {
            diffs.push(format!(
                "bash: type mismatch (source={}, target={})",
                bash_type_name(&a.bash),
                bash_type_name(&b.bash),
            ));
        }
    }

    diffs
}

fn bash_type_name(b: &BashPermission) -> &'static str {
    match b {
        BashPermission::Simple(_) => "Simple",
        BashPermission::Detailed(_) => "Detailed",
    }
}

fn cmd_generate_overrides(config: &ProjectConfig) -> Result<()> {
    eprintln!("WARNING: generate-overrides is deprecated.");
    eprintln!("Templates + agents are now the source of truth.");
    eprintln!("Use 'generate-permissions' to produce permissions.jsonc from templates + agents.");
    eprintln!();

    let sot = loader::load_sot(&config.base_dir)?;
    let categories = loader::discover_categories(&config.agents_dir)?;
    let templates = loader::load_all_templates_with_parents(&config.templates_dir, &categories)?;
    let classifier = CommandGroupClassifier::new(config.groups.clone());

    let results = generator::generate_all(
        &sot,
        &categories,
        &templates,
        &config.agents_dir,
        &classifier,
    )?;
    generator::print_summary(&results);

    Ok(())
}

fn cmd_resolve(config: &ProjectConfig, agent: &str, trace: bool) -> Result<()> {
    let categories = loader::discover_categories(&config.agents_dir)?;
    let templates = loader::load_all_templates_with_parents(&config.templates_dir, &categories)?;

    let (graph, node_map) = template_dag::build_dag(&templates)?;
    template_dag::detect_cycles(&graph)?;

    let (cat_name, _) = categories
        .iter()
        .find(|(_, agents)| agents.contains(&agent.to_string()))
        .with_context(|| format!("Agent '{}' not found in any agent file", agent))?;

    let template = templates
        .get(cat_name)
        .with_context(|| format!("No template for category '{}'", cat_name))?;

    let override_data = loader::load_override(&config.agents_dir, agent)?;

    println!("Agent: {}", agent);
    println!("Category: {}", cat_name);
    println!("Template: {}.jsonc", cat_name);
    if let Some(ref extends) = template.extends {
        println!("Inherits: {}", extends.parents().join(", "));
    }
    println!();

    // Build ancestors and baselines (shared by both trace and non-trace paths)
    let ancestors = if template.extends.is_some() {
        template_dag::ancestor_order(&graph, &node_map, cat_name)?
    } else {
        vec![]
    };

    let mut parent_baselines: Vec<(String, indexmap::IndexMap<String, String>)> = Vec::new();
    for ancestor_name in &ancestors {
        let anc_template = templates.get(ancestor_name).unwrap();
        let anc_baseline: indexmap::IndexMap<String, String> =
            serde_json::from_value(anc_template.baseline.permission.clone())?;
        parent_baselines.push((ancestor_name.clone(), anc_baseline));
    }

    let self_baseline: indexmap::IndexMap<String, String> =
        serde_json::from_value(template.baseline.permission.clone())?;

    if trace {
        let traced = resolver::resolve_with_trace(
            &parent_baselines,
            &self_baseline,
            cat_name,
            &override_data,
        )?;

        let mut inherited_count = 0usize;
        let mut self_count = 0usize;
        let mut agent_count = 0usize;

        for entry in &traced {
            let source_str = match &entry.source {
                resolver::PermissionSource::Inherited(parent) => {
                    inherited_count += 1;
                    format!("inherited from {}", parent)
                }
                resolver::PermissionSource::SelfBaseline(tmpl) => {
                    self_count += 1;
                    format!("self baseline ({})", tmpl)
                }
                resolver::PermissionSource::AgentAdd(agent_name) => {
                    agent_count += 1;
                    format!("agent override: add ({})", agent_name)
                }
            };
            println!(
                "  {:?}: {:?}  \u{2190} {}",
                entry.key, entry.value, source_str
            );
        }

        println!();
        println!("Summary: {} total entries", traced.len());
        println!("  {} inherited from parent template(s)", inherited_count);
        println!("  {} from self baseline", self_count);
        println!("  {} from agent override (add)", agent_count);
    } else {
        // Non-trace: resolve and print JSON
        let resolved = if template.extends.is_some() {
            let effective = resolver::merge_baselines(&parent_baselines, &self_baseline, cat_name)?;
            resolver::resolve_with_baseline(&effective, cat_name, &override_data)?
        } else {
            resolver::resolve(template, &override_data)?
        };

        let perm = &resolved.permission;
        let mut obj = serde_json::Map::new();
        if let Some(ref w) = perm.write {
            obj.insert("write".to_string(), serde_json::Value::String(w.clone()));
        }
        if let Some(ref e) = perm.edit {
            obj.insert("edit".to_string(), serde_json::Value::String(e.clone()));
        }
        match &perm.bash {
            BashPermission::Simple(s) => {
                obj.insert("bash".to_string(), serde_json::Value::String(s.clone()));
            }
            BashPermission::Detailed(map) => {
                let bash_obj: serde_json::Map<String, serde_json::Value> = map
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect();
                obj.insert("bash".to_string(), serde_json::Value::Object(bash_obj));
            }
        }

        let perm_val = serde_json::json!({ "permission": obj });
        println!("{}", serde_json::to_string_pretty(&perm_val)?);
    }

    Ok(())
}

fn cmd_validate(config: &ProjectConfig) -> Result<()> {
    let sot = loader::load_sot(&config.base_dir)?;
    let categories = loader::discover_categories(&config.agents_dir)?;
    let templates = loader::load_all_templates_with_parents(&config.templates_dir, &categories)?;

    let report = validator::validate_all(&sot, &categories, &templates, &config.agents_dir)?;
    validator::print_report(&report);

    if report.failed > 0 {
        process::exit(1);
    }

    Ok(())
}

fn cmd_can(
    config: &ProjectConfig,
    permissions_path: &Path,
    agent: &str,
    command: &str,
    explain: bool,
) -> Result<()> {
    // Auto-generate permissions.jsonc if it doesn't exist yet.
    if !permissions_path.exists() {
        config.validate_dirs()?;
        let content = sot_generator::generate_permissions(config)?;
        std::fs::write(permissions_path, &content).with_context(|| {
            format!(
                "Writing auto-generated permissions to {}",
                permissions_path.display()
            )
        })?;
        eprintln!(
            "Note: Generated {} (not found, auto-generated from templates + agents)",
            permissions_path.display()
        );
    }

    let result = opencode_agency::can::can(permissions_path, agent, command, explain)?;

    // First line: the permission value (allow/deny/ask)
    println!("{}", result.permission_value);

    // If explain was requested, print structured explanation
    if let Some(ref explanation) = result.explanation {
        println!("agent: {}", result.agent);
        println!("command: {}", result.command);
        println!("rule: {}", explanation.rule);
        println!("match: {}", explanation.match_kind);
    }

    // Exit 0 for allow, 1 for deny/ask
    if !result.allowed {
        process::exit(1);
    }

    Ok(())
}

fn cmd_audit_report(config: &ProjectConfig) -> Result<()> {
    let output_path = config.base_dir.join("audit-report.md");

    let sot = loader::load_sot(&config.base_dir)?;
    let categories = loader::discover_categories(&config.agents_dir)?;
    let templates = loader::load_all_templates_with_parents(&config.templates_dir, &categories)?;

    let validation = validator::validate_all(&sot, &categories, &templates, &config.agents_dir)?;
    reporter::generate_report(
        &sot,
        &categories,
        &templates,
        &validation,
        &config.agents_dir,
        &output_path,
    )?;

    Ok(())
}
