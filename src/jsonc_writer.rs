#![allow(dead_code)] // Functions used by future modules (comment_gen, sot_generator)

use crate::models::{BashPermission, SotPermission};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn indent(level: usize) -> String {
    "  ".repeat(level)
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

// ---------------------------------------------------------------------------
// Permission formatters
// ---------------------------------------------------------------------------

/// Format permission block content for a simple category.
///
/// Returns indented lines to embed inside a `"permission": { ... }` block.
///
/// Three variants:
/// - **A**: `bash: "deny"` → single `"bash": "deny"` line
/// - **B**: `write + edit + bash` → all three keys
/// - **C**: `bash: { "*": "allow" }` → nested object
pub fn format_simple_permission(perm: &SotPermission, base: usize) -> Vec<String> {
    let mut entries: Vec<Vec<String>> = Vec::new();

    if let Some(ref w) = perm.write {
        entries.push(vec![format!(
            "{}{}: {}",
            indent(base),
            json_str("write"),
            json_str(w)
        )]);
    }

    if let Some(ref e) = perm.edit {
        entries.push(vec![format!(
            "{}{}: {}",
            indent(base),
            json_str("edit"),
            json_str(e)
        )]);
    }

    // bash is always present
    match &perm.bash {
        BashPermission::Simple(s) => {
            entries.push(vec![format!(
                "{}{}: {}",
                indent(base),
                json_str("bash"),
                json_str(s)
            )]);
        }
        BashPermission::Detailed(map) => {
            let mut bash_lines = Vec::new();
            bash_lines.push(format!("{}{}: {{", indent(base), json_str("bash")));
            let count = map.len();
            for (i, (k, v)) in map.iter().enumerate() {
                let comma = if i < count - 1 { "," } else { "" };
                bash_lines.push(format!(
                    "{}{}: {}{}",
                    indent(base + 1),
                    json_str(k),
                    json_str(v),
                    comma
                ));
            }
            bash_lines.push(format!("{}}}", indent(base)));
            entries.push(bash_lines);
        }
    }

    // Join entries, adding trailing commas to all but the last
    let total = entries.len();
    let mut lines = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        let is_last = i == total - 1;
        let entry_len = entry.len();
        for (j, line) in entry.into_iter().enumerate() {
            if !is_last && j == entry_len - 1 {
                // Last line of a non-last entry gets a comma
                lines.push(format!("{},", line));
            } else {
                lines.push(line);
            }
        }
    }

    lines
}

/// Format permission block content for a complex category.
///
/// The `bash` field is always `BashPermission::Detailed(map)`. Returns lines
/// for the `"bash": { ... }` block with each command→permission entry on its
/// own line.
pub fn format_complex_permission(perm: &SotPermission, base: usize) -> Vec<String> {
    let mut lines = Vec::new();

    match &perm.bash {
        BashPermission::Detailed(map) => {
            lines.push(format!("{}{}: {{", indent(base), json_str("bash")));
            let count = map.len();
            for (i, (k, v)) in map.iter().enumerate() {
                let comma = if i < count - 1 { "," } else { "" };
                lines.push(format!(
                    "{}{}: {}{}",
                    indent(base + 1),
                    json_str(k),
                    json_str(v),
                    comma
                ));
            }
            lines.push(format!("{}}}", indent(base)));
        }
        BashPermission::Simple(s) => {
            // Fallback: shouldn't happen for complex categories, but handle gracefully
            lines.push(format!(
                "{}{}: {}",
                indent(base),
                json_str("bash"),
                json_str(s)
            ));
        }
    }

    lines
}

// ---------------------------------------------------------------------------
// Agent block formatter
// ---------------------------------------------------------------------------

/// Format a complete agent block: comment lines + `"agent-name": { "permission": { ... } }`.
///
/// - `comments` are pre-formatted lines from `comment_gen` (already contain `//` prefix).
/// - `is_simple` selects between `format_simple_permission` and `format_complex_permission`.
/// - `is_last` controls whether a trailing comma appears after the closing `}`.
pub fn format_agent_block(
    name: &str,
    perm: &SotPermission,
    comments: &[String],
    is_simple: bool,
    is_last: bool,
) -> Vec<String> {
    let mut lines = Vec::new();

    // Comment lines at indent level 1
    for comment in comments {
        lines.push(format!("{}{}", indent(1), comment));
    }

    // Agent key opening
    lines.push(format!("{}{}: {{", indent(1), json_str(name)));

    // "permission" opening
    lines.push(format!("{}{}: {{", indent(2), json_str("permission")));

    // Permission content at indent level 3
    let perm_lines = if is_simple {
        format_simple_permission(perm, 3)
    } else {
        format_complex_permission(perm, 3)
    };
    lines.extend(perm_lines);

    // Close "permission"
    lines.push(format!("{}}}", indent(2)));

    // Close agent block
    if is_last {
        lines.push(format!("{}}}", indent(1)));
    } else {
        lines.push(format!("{}}},", indent(1)));
    }

    lines
}

// ---------------------------------------------------------------------------
// Document assembler
// ---------------------------------------------------------------------------

/// Assemble the full JSONC document from header lines and category blocks.
///
/// Structure:
/// ```text
/// // header line 1
/// // header line 2
/// {
///   [category 1 agent blocks]
///
///   [category 2 agent blocks]
/// }
/// ```
pub fn assemble_document(header_lines: &[String], category_blocks: &[Vec<String>]) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Header lines (level 0)
    for line in header_lines {
        parts.push(line.clone());
    }

    // Opening brace
    parts.push("{".to_string());

    // Category blocks with blank-line separators between them
    for (i, block) in category_blocks.iter().enumerate() {
        if i > 0 {
            parts.push(String::new()); // blank line between categories
        }
        for line in block {
            parts.push(line.clone());
        }
    }

    // Closing brace
    parts.push("}".to_string());

    parts.join("\n")
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;

    // -----------------------------------------------------------------------
    // Helpers to build test fixtures
    // -----------------------------------------------------------------------

    fn simple_bash_deny() -> SotPermission {
        SotPermission {
            write: None,
            edit: None,
            bash: BashPermission::Simple("deny".to_string()),
        }
    }

    fn simple_write_edit_bash() -> SotPermission {
        SotPermission {
            write: Some("deny".to_string()),
            edit: Some("deny".to_string()),
            bash: BashPermission::Simple("deny".to_string()),
        }
    }

    fn simple_bash_object() -> SotPermission {
        let mut map = IndexMap::new();
        map.insert("*".to_string(), "allow".to_string());
        SotPermission {
            write: None,
            edit: None,
            bash: BashPermission::Detailed(map),
        }
    }

    fn complex_three_entries() -> SotPermission {
        let mut map = IndexMap::new();
        map.insert("npm *".to_string(), "allow".to_string());
        map.insert("npx *".to_string(), "allow".to_string());
        map.insert("node *".to_string(), "allow".to_string());
        SotPermission {
            write: None,
            edit: None,
            bash: BashPermission::Detailed(map),
        }
    }

    fn complex_single_entry() -> SotPermission {
        let mut map = IndexMap::new();
        map.insert("npm *".to_string(), "allow".to_string());
        SotPermission {
            write: None,
            edit: None,
            bash: BashPermission::Detailed(map),
        }
    }

    // -----------------------------------------------------------------------
    // format_simple_permission
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_simple_bash_deny() {
        let perm = simple_bash_deny();
        let lines = format_simple_permission(&perm, 3);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], r#"      "bash": "deny""#);
    }

    #[test]
    fn test_format_simple_write_edit_bash() {
        let perm = simple_write_edit_bash();
        let lines = format_simple_permission(&perm, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], r#"      "write": "deny","#);
        assert_eq!(lines[1], r#"      "edit": "deny","#);
        assert_eq!(lines[2], r#"      "bash": "deny""#);
    }

    #[test]
    fn test_format_simple_bash_object() {
        let perm = simple_bash_object();
        let lines = format_simple_permission(&perm, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], r#"      "bash": {"#);
        assert_eq!(lines[1], r#"        "*": "allow""#);
        assert_eq!(lines[2], r#"      }"#);
    }

    // -----------------------------------------------------------------------
    // format_complex_permission
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_complex_permission() {
        let perm = complex_three_entries();
        let lines = format_complex_permission(&perm, 3);
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], r#"      "bash": {"#);
        assert_eq!(lines[1], r#"        "npm *": "allow","#);
        assert_eq!(lines[2], r#"        "npx *": "allow","#);
        assert_eq!(lines[3], r#"        "node *": "allow""#);
        assert_eq!(lines[4], r#"      }"#);
        // Verify last entry has no trailing comma
        assert!(!lines[3].ends_with(','));
        // Verify non-last entries have trailing commas
        assert!(lines[1].ends_with(','));
        assert!(lines[2].ends_with(','));
    }

    #[test]
    fn test_format_complex_single_entry() {
        let perm = complex_single_entry();
        let lines = format_complex_permission(&perm, 3);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], r#"      "bash": {"#);
        assert_eq!(lines[1], r#"        "npm *": "allow""#);
        assert_eq!(lines[2], r#"      }"#);
        // Single entry — no trailing comma
        assert!(!lines[1].ends_with(','));
    }

    // -----------------------------------------------------------------------
    // format_agent_block
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_agent_block_with_comments() {
        let perm = complex_three_entries();
        let comments = vec![
            "// --- x-backend-engineer ---".to_string(),
            "// Backend APIs, Prisma, PostgreSQL".to_string(),
            "// Overrides: +1 add (docker *)".to_string(),
        ];
        let lines = format_agent_block("x-backend-engineer", &perm, &comments, false, false);

        // Comment lines are indented at level 1
        assert_eq!(lines[0], "  // --- x-backend-engineer ---");
        assert_eq!(lines[1], "  // Backend APIs, Prisma, PostgreSQL");
        assert_eq!(lines[2], "  // Overrides: +1 add (docker *)");

        // Agent key opening
        assert_eq!(lines[3], r#"  "x-backend-engineer": {"#);

        // "permission" opening
        assert_eq!(lines[4], r#"    "permission": {"#);

        // bash block
        assert_eq!(lines[5], r#"      "bash": {"#);
        assert_eq!(lines[6], r#"        "npm *": "allow","#);
        assert_eq!(lines[7], r#"        "npx *": "allow","#);
        assert_eq!(lines[8], r#"        "node *": "allow""#);
        assert_eq!(lines[9], r#"      }"#);

        // Close permission
        assert_eq!(lines[10], r#"    }"#);

        // Close agent (not last → trailing comma)
        assert_eq!(lines[11], r#"  },"#);
    }

    #[test]
    fn test_format_agent_block_last_no_comma() {
        let perm = simple_bash_deny();
        let comments = vec!["// --- plan ---".to_string()];
        let lines = format_agent_block("plan", &perm, &comments, true, true);

        let last = lines.last().unwrap();
        assert_eq!(last, "  }");
        assert!(!last.ends_with(','));
    }

    #[test]
    fn test_format_agent_block_not_last_has_comma() {
        let perm = simple_bash_deny();
        let comments = vec!["// --- plan ---".to_string()];
        let lines = format_agent_block("plan", &perm, &comments, true, false);

        let last = lines.last().unwrap();
        assert_eq!(last, "  },");
        assert!(last.ends_with(','));
    }

    // -----------------------------------------------------------------------
    // assemble_document
    // -----------------------------------------------------------------------

    #[test]
    fn test_assemble_document() {
        let header = vec![
            "// ============================================================================="
                .to_string(),
            "// OpenCode Agent Permissions — Source of Truth".to_string(),
            "// Generated by agency from _templates/ + _agents/".to_string(),
            "// ============================================================================="
                .to_string(),
        ];

        // Category 1: a single agent
        let cat1 = format_agent_block(
            "x-backend-engineer",
            &complex_three_entries(),
            &["// --- x-backend-engineer ---".to_string()],
            false,
            false,
        );

        // Category 2: a single agent (last in document)
        let cat2 = format_agent_block(
            "plan",
            &simple_bash_deny(),
            &["// --- plan ---".to_string()],
            true,
            true,
        );

        let doc = assemble_document(&header, &[cat1, cat2]);

        // Verify structure
        assert!(doc.starts_with("// ="));
        assert!(doc.contains("{"));
        assert!(doc.ends_with("}"));

        // Verify blank line between categories
        let doc_lines: Vec<&str> = doc.lines().collect();

        // Find the closing `},` of category 1 and verify a blank line follows
        let comma_close_idx = doc_lines
            .iter()
            .position(|l| *l == "  },")
            .expect("should have agent closing with comma");
        assert_eq!(
            doc_lines[comma_close_idx + 1],
            "",
            "blank line between categories"
        );

        // No blank line before the first category
        let opening_brace_idx = doc_lines
            .iter()
            .position(|l| *l == "{")
            .expect("should have opening brace");
        assert_ne!(
            doc_lines[opening_brace_idx + 1],
            "",
            "no blank line after opening brace"
        );

        // No blank line after the last category (before closing `}`)
        let closing_brace_idx = doc_lines
            .iter()
            .rposition(|l| *l == "}")
            .expect("should have closing brace");
        assert_ne!(
            doc_lines[closing_brace_idx - 1],
            "",
            "no blank line before closing brace"
        );
    }

    // -----------------------------------------------------------------------
    // Trailing comma correctness
    // -----------------------------------------------------------------------

    #[test]
    fn test_trailing_comma_correctness() {
        // Build a document with two categories, two agents each
        let header = vec!["// header".to_string()];

        let mut cat1 = Vec::new();
        cat1.extend(format_agent_block(
            "agent-a",
            &simple_write_edit_bash(),
            &["// --- agent-a ---".to_string()],
            true,
            false,
        ));
        cat1.extend(format_agent_block(
            "agent-b",
            &simple_bash_object(),
            &["// --- agent-b ---".to_string()],
            true,
            false,
        ));

        let cat2 = format_agent_block(
            "agent-z",
            &simple_bash_deny(),
            &["// --- agent-z ---".to_string()],
            true,
            true,
        );

        let doc = assemble_document(&header, &[cat1, cat2]);

        // Parse-level check: no trailing comma before any `}`
        let doc_lines: Vec<&str> = doc.lines().collect();
        for (i, line) in doc_lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed == "}" || trimmed == "}," {
                // The line before a closing brace must not end with a comma
                if i > 0 {
                    let prev = doc_lines[i - 1].trim();
                    // Skip blank lines
                    if !prev.is_empty() {
                        assert!(
                            !prev.ends_with(','),
                            "line {} ({:?}) precedes a closing brace but has trailing comma",
                            i - 1,
                            prev
                        );
                    }
                }
            }
        }

        // Verify last agent has no comma
        assert!(doc.contains("  }"));
        // Collect all closing braces and verify the last is document-level
        let agent_closes: Vec<(usize, &&str)> = doc_lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.trim() == "}" || l.trim() == "},")
            .collect();

        // The very last `}` is the document close, the one before is the last agent
        let last_doc_close = agent_closes.last().unwrap();
        assert_eq!(last_doc_close.1.trim(), "}");
    }
}
