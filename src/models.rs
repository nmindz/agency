use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Permission value: "allow", "deny", or "ask"
pub type PermValue = String;

/// The bash permission can be either a simple string ("deny") or a detailed object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum BashPermission {
    Simple(String),
    Detailed(IndexMap<String, PermValue>),
}

/// Category name → sorted list of agent names (replaces Index)
pub type CategoryMap = IndexMap<String, Vec<String>>;

/// Top-level SOT structure: agent_name → SotAgent
pub type Sot = IndexMap<String, SotAgent>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SotAgent {
    pub permission: SotPermission,
}

/// SOT permission block — uses flatten to capture write/edit/bash flexibly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SotPermission {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edit: Option<String>,
    pub bash: BashPermission,
}

/// Documentation metadata block (`$doc`).
///
/// Provides structured metadata for generators and audit reports.
/// All fields are optional for backward compatibility — files without
/// a `$doc` block continue to parse without error.
///
/// Convention:
/// - Templates typically use `baseline_rationale` and `security_note`.
/// - Overrides typically use `agent_summary` and `override_rationale`.
/// - Any field can appear in either context; the convention is advisory.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct DocBlock {
    /// What the baseline provides and why.
    pub baseline_rationale: Option<String>,
    /// Security-relevant info about the category.
    pub security_note: Option<String>,
    /// One-line description of what this agent does.
    pub agent_summary: Option<String>,
    /// Why this agent differs from its baseline.
    pub override_rationale: Option<String>,
}

/// Template inheritance declaration.
///
/// Supports backward-compatible single-parent and new multi-parent forms:
/// - Single: `"$extends": "vcs-permissions"`
/// - Multiple: `"$extends": ["vcs-permissions", "node-toolchain"]`
///
/// Uses `#[serde(untagged)]` for transparent JSON deserialization,
/// following the same pattern as `BashPermission`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Extends {
    /// Single parent template reference.
    Single(String),
    /// Multiple parent template references (resolved left-to-right).
    Multiple(Vec<String>),
}

impl Extends {
    /// Return the parent template names as a slice-like iterator.
    pub fn parents(&self) -> Vec<&str> {
        match self {
            Extends::Single(s) => vec![s.as_str()],
            Extends::Multiple(v) => v.iter().map(|s| s.as_str()).collect(),
        }
    }
}

/// Template file structure
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Template {
    #[serde(rename = "$schema")]
    pub schema: String,
    #[serde(rename = "$version")]
    pub version: String,
    pub category: String,
    pub description: String,
    pub purpose: String,
    /// Optional documentation metadata block.
    /// Present in schema v1.1.0+; absent in v1.0.0.
    #[serde(default, rename = "$doc")]
    pub doc: Option<DocBlock>,
    /// Optional parent templates to inherit from.
    /// Absent for root templates and mixin templates.
    /// Present for templates that build upon shared permission blocks.
    #[serde(default, rename = "$extends")]
    pub extends: Option<Extends>,
    pub baseline: TemplateBaseline,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TemplateBaseline {
    /// For "simple" categories (deny-all, allow-all, general-purpose):
    ///   This is a full permission object like {"bash": "deny"} or {"write":"deny","edit":"deny","bash":"deny"}
    ///   or {"bash": {"*": "allow"}}
    ///
    /// For "complex" categories (object):
    ///   This is the bash sub-object directly (command → permission map)
    pub permission: serde_json::Value,
}

/// Override file structure
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Override {
    #[serde(rename = "$schema")]
    pub schema: String,
    #[serde(rename = "$version")]
    pub version: String,
    pub agent: String,
    #[serde(rename = "$extends")]
    pub extends: String,
    /// Optional documentation metadata block.
    /// Present in schema v1.1.0+; absent in v1.0.0.
    #[serde(default, rename = "$doc")]
    pub doc: Option<DocBlock>,
    pub overrides: OverrideBlock,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OverrideBlock {
    #[serde(default)]
    pub permission: Option<PermissionOverride>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PermissionOverride {
    #[serde(default)]
    pub add: Option<IndexMap<String, PermValue>>,
    #[serde(default)]
    pub remove: Option<Vec<String>>,
}

/// Simple categories where the template baseline.permission is the FULL permission object
/// (not just the bash sub-object).
pub const SIMPLE_CATEGORIES: &[&str] = &[
    "orchestration",
    "planning",
    "research-knowledge",
    "general-purpose",
    "unrestricted",
];

pub fn is_simple_category(cat: &str) -> bool {
    SIMPLE_CATEGORIES.contains(&cat)
}

/// Result of resolving an agent's permissions
#[derive(Debug, Clone)]
pub struct ResolvedPermission {
    /// The full permission object as a JSON value
    pub permission: SotPermission,
}

/// Delta computed between SOT and template baseline
#[derive(Debug, Clone)]
pub struct Delta {
    pub adds: Vec<(String, String)>,
    pub removes: Vec<String>,
}

/// Validation result for one agent
#[derive(Debug, Clone)]
pub struct AgentValidationResult {
    pub agent: String,
    pub category: String,
    pub passed: bool,
    pub extra_in_resolved: Vec<String>,
    pub missing_from_resolved: Vec<String>,
    pub value_mismatches: Vec<(String, String, String)>,
}

/// Validation report for all agents
#[derive(Debug, Clone)]
pub struct ValidationReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<AgentValidationResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_with_doc_block() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "1.1.0",
            "category": "backend-api",
            "description": "Backend agents",
            "purpose": "Server-side engineers",
            "$doc": {
                "baseline_rationale": "Standard backend toolchain",
                "security_note": "Docker excluded from baseline"
            },
            "baseline": {
                "permission": { "npm *": "allow" }
            }
        }"#;

        let template: Template = serde_json::from_str(json).unwrap();
        assert_eq!(template.version, "1.1.0");
        assert_eq!(template.category, "backend-api");

        let doc = template.doc.expect("$doc should be Some");
        assert_eq!(
            doc.baseline_rationale.as_deref(),
            Some("Standard backend toolchain")
        );
        assert_eq!(
            doc.security_note.as_deref(),
            Some("Docker excluded from baseline")
        );
        assert_eq!(doc.agent_summary, None);
        assert_eq!(doc.override_rationale, None);
    }

    #[test]
    fn template_without_doc_block() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "1.0.0",
            "category": "backend-api",
            "description": "Backend agents",
            "purpose": "Server-side engineers",
            "baseline": {
                "permission": { "npm *": "allow" }
            }
        }"#;

        let template: Template = serde_json::from_str(json).unwrap();
        assert_eq!(template.version, "1.0.0");
        assert!(template.doc.is_none(), "$doc should be None when absent");
    }

    #[test]
    fn override_with_doc_block() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json",
            "$version": "1.1.0",
            "agent": "x-backend-engineer",
            "$extends": "backend-api",
            "$doc": {
                "agent_summary": "Backend APIs, Prisma, PostgreSQL",
                "override_rationale": "Adds Next.js and Docker"
            },
            "overrides": {}
        }"#;

        let ov: Override = serde_json::from_str(json).unwrap();
        assert_eq!(ov.version, "1.1.0");
        assert_eq!(ov.agent, "x-backend-engineer");

        let doc = ov.doc.expect("$doc should be Some");
        assert_eq!(
            doc.agent_summary.as_deref(),
            Some("Backend APIs, Prisma, PostgreSQL")
        );
        assert_eq!(
            doc.override_rationale.as_deref(),
            Some("Adds Next.js and Docker")
        );
        assert_eq!(doc.baseline_rationale, None);
        assert_eq!(doc.security_note, None);
    }

    #[test]
    fn override_without_doc_block() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json",
            "$version": "1.0.0",
            "agent": "plan",
            "$extends": "planning",
            "overrides": {}
        }"#;

        let ov: Override = serde_json::from_str(json).unwrap();
        assert_eq!(ov.version, "1.0.0");
        assert!(ov.doc.is_none(), "$doc should be None when absent");
    }

    #[test]
    fn doc_block_partial_fields() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json",
            "$version": "1.1.0",
            "agent": "test-agent",
            "$extends": "test-category",
            "$doc": {
                "agent_summary": "Test agent for partial fields"
            },
            "overrides": {}
        }"#;

        let ov: Override = serde_json::from_str(json).unwrap();
        let doc = ov.doc.expect("$doc should be Some");

        assert_eq!(
            doc.agent_summary.as_deref(),
            Some("Test agent for partial fields")
        );
        assert_eq!(doc.baseline_rationale, None);
        assert_eq!(doc.security_note, None);
        assert_eq!(doc.override_rationale, None);
    }

    #[test]
    fn doc_block_empty_object() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/agent.json",
            "$version": "1.1.0",
            "agent": "test-agent",
            "$extends": "test-category",
            "$doc": {},
            "overrides": {}
        }"#;

        let ov: Override = serde_json::from_str(json).unwrap();
        let doc = ov.doc.expect("$doc should be Some even when empty object");

        assert_eq!(doc.baseline_rationale, None);
        assert_eq!(doc.security_note, None);
        assert_eq!(doc.agent_summary, None);
        assert_eq!(doc.override_rationale, None);
    }

    #[test]
    fn doc_block_all_fields() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "1.1.0",
            "category": "test-category",
            "description": "Test",
            "purpose": "Test",
            "$doc": {
                "baseline_rationale": "rationale",
                "security_note": "note",
                "agent_summary": "summary",
                "override_rationale": "override"
            },
            "baseline": {
                "permission": { "bash": "deny" }
            }
        }"#;

        let template: Template = serde_json::from_str(json).unwrap();
        let doc = template.doc.expect("$doc should be Some");

        assert_eq!(doc.baseline_rationale.as_deref(), Some("rationale"));
        assert_eq!(doc.security_note.as_deref(), Some("note"));
        assert_eq!(doc.agent_summary.as_deref(), Some("summary"));
        assert_eq!(doc.override_rationale.as_deref(), Some("override"));
    }

    #[test]
    fn test_extends_single_deserialize() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "2.0.0",
            "$extends": "vcs-permissions",
            "category": "backend-api",
            "description": "Backend agents",
            "purpose": "Server-side engineers",
            "baseline": { "permission": { "npm *": "allow" } }
        }"#;
        let t: Template = serde_json::from_str(json).unwrap();
        assert_eq!(
            t.extends,
            Some(Extends::Single("vcs-permissions".to_string()))
        );
    }

    #[test]
    fn test_extends_multiple_deserialize() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "2.0.0",
            "$extends": ["vcs-permissions", "node-toolchain"],
            "category": "backend-api",
            "description": "Backend agents",
            "purpose": "Server-side engineers",
            "baseline": { "permission": { "npm *": "allow" } }
        }"#;
        let t: Template = serde_json::from_str(json).unwrap();
        assert_eq!(
            t.extends,
            Some(Extends::Multiple(vec![
                "vcs-permissions".to_string(),
                "node-toolchain".to_string()
            ]))
        );
    }

    #[test]
    fn test_extends_absent() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "1.0.0",
            "category": "backend-api",
            "description": "Backend agents",
            "purpose": "Server-side engineers",
            "baseline": { "permission": { "npm *": "allow" } }
        }"#;
        let t: Template = serde_json::from_str(json).unwrap();
        assert!(t.extends.is_none());
    }

    #[test]
    fn test_extends_parents_single() {
        let ext = Extends::Single("vcs-permissions".to_string());
        assert_eq!(ext.parents(), vec!["vcs-permissions"]);
    }

    #[test]
    fn test_extends_parents_multiple() {
        let ext = Extends::Multiple(vec![
            "vcs-permissions".to_string(),
            "node-toolchain".to_string(),
        ]);
        assert_eq!(ext.parents(), vec!["vcs-permissions", "node-toolchain"]);
    }

    #[test]
    fn test_template_with_extends_parses() {
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "2.0.0",
            "$extends": ["vcs-permissions"],
            "category": "lang-rust",
            "description": "Rust agents",
            "purpose": "Rust development",
            "$doc": {
                "baseline_rationale": "Rust toolchain",
                "security_note": "None"
            },
            "baseline": { "permission": { "cargo *": "allow" } }
        }"#;
        let t: Template = serde_json::from_str(json).unwrap();
        assert_eq!(t.category, "lang-rust");
        assert_eq!(
            t.extends,
            Some(Extends::Multiple(vec!["vcs-permissions".to_string()]))
        );
        assert!(t.doc.is_some());
    }

    #[test]
    fn test_template_without_extends_still_parses() {
        // Existing v1.0.0 template without $extends — backward compatibility
        let json = r#"{
            "$schema": "https://raw.githubusercontent.com/nmindz/agency/master/schemas/template.json",
            "$version": "1.0.0",
            "category": "orchestration",
            "description": "Orchestration agents",
            "purpose": "Coordinate work",
            "baseline": { "permission": { "bash": "deny" } }
        }"#;
        let t: Template = serde_json::from_str(json).unwrap();
        assert_eq!(t.category, "orchestration");
        assert!(t.extends.is_none(), "$extends should be None when absent");
    }
}
