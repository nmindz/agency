use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;

use crate::models::{BashPermission, Sot, SotAgent};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a `can()` permission check.
#[derive(Debug, Clone)]
pub struct CanResult {
    pub allowed: bool,
    pub permission_value: String,
    pub agent: String,
    pub command: String,
    pub explanation: Option<CanExplanation>,
}

/// Detailed explanation of how the permission decision was made.
#[derive(Debug, Clone)]
pub struct CanExplanation {
    pub rule: String,
    pub match_kind: String,
}

/// How a command pattern matched.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchKind {
    Exact,
    Wildcard { prefix: String },
    CatchAll,
    Blanket,
    Default,
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

struct MatchResult<'a> {
    pattern: &'a str,
    permission: &'a str,
    kind: MatchKind,
}

// ---------------------------------------------------------------------------
// JSONC comment stripping
// ---------------------------------------------------------------------------

fn strip_jsonc_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    while i < len {
        match bytes[i] {
            b'"' => {
                out.push(b'"');
                i += 1;
                while i < len {
                    match bytes[i] {
                        b'\\' => {
                            out.push(b'\\');
                            i += 1;
                            if i < len {
                                out.push(bytes[i]);
                                i += 1;
                            }
                        }
                        b'"' => {
                            out.push(b'"');
                            i += 1;
                            break;
                        }
                        _ => {
                            out.push(bytes[i]);
                            i += 1;
                        }
                    }
                }
            }
            b'/' if i + 1 < len => match bytes[i + 1] {
                b'/' => {
                    while i < len && bytes[i] != b'\n' {
                        out.push(b' ');
                        i += 1;
                    }
                }
                b'*' => {
                    out.push(b' ');
                    out.push(b' ');
                    i += 2;
                    while i + 1 < len {
                        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            out.push(b' ');
                            out.push(b' ');
                            i += 2;
                            break;
                        }
                        if bytes[i] == b'\n' {
                            out.push(b'\n');
                        } else {
                            out.push(b' ');
                        }
                        i += 1;
                    }
                }
                _ => {
                    out.push(b'/');
                    i += 1;
                }
            },
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    // Safety: we only ever push valid ASCII bytes or preserve existing UTF-8
    // sequences byte-by-byte; the output is always valid UTF-8.
    unsafe { String::from_utf8_unchecked(out) }
}

// ---------------------------------------------------------------------------
// Command matching
// ---------------------------------------------------------------------------

fn match_command<'a>(
    rules: &'a IndexMap<String, String>,
    command: &str,
) -> Option<MatchResult<'a>> {
    let mut best_match: Option<MatchResult<'a>> = None;
    let mut best_specificity: usize = 0;

    for (pattern, permission) in rules {
        if pattern == command {
            return Some(MatchResult {
                pattern: pattern.as_str(),
                permission: permission.as_str(),
                kind: MatchKind::Exact,
            });
        }

        if pattern.ends_with(" *") {
            let prefix = &pattern[..pattern.len() - 2];
            if command == prefix
                || (command.len() > prefix.len()
                    && command.as_bytes()[prefix.len()] == b' '
                    && command.starts_with(prefix))
            {
                let specificity = prefix.len();
                if specificity > best_specificity {
                    best_specificity = specificity;
                    best_match = Some(MatchResult {
                        pattern: pattern.as_str(),
                        permission: permission.as_str(),
                        kind: MatchKind::Wildcard {
                            prefix: prefix.to_string(),
                        },
                    });
                }
            }
        }

        if pattern == "*" && best_match.is_none() {
            best_match = Some(MatchResult {
                pattern: pattern.as_str(),
                permission: permission.as_str(),
                kind: MatchKind::CatchAll,
            });
        }
    }

    best_match
}

// ---------------------------------------------------------------------------
// Binary cache helpers
// ---------------------------------------------------------------------------

const CACHE_FILE_NAME: &str = "permissions.cache";

/// Check if the binary cache is fresh relative to the source permissions file.
///
/// The cache is keyed solely on the `permissions.jsonc` modification time.
/// Template and agent file changes only take effect after `generate-permissions`
/// updates `permissions.jsonc`, which then naturally invalidates this cache.
fn cache_is_fresh(source: &Path, cache: &Path) -> bool {
    let Ok(src_meta) = fs::metadata(source) else {
        return false;
    };
    let Ok(cache_meta) = fs::metadata(cache) else {
        return false;
    };
    let Ok(src_mtime) = src_meta.modified() else {
        return false;
    };
    let Ok(cache_mtime) = cache_meta.modified() else {
        return false;
    };
    cache_mtime >= src_mtime
}

fn load_from_cache(cache_path: &Path) -> Option<HashMap<String, SotAgent>> {
    let data = fs::read(cache_path).ok()?;
    bincode::deserialize(&data).ok()
}

fn write_cache(cache_path: &Path, agents: &HashMap<String, SotAgent>) {
    if let Ok(data) = bincode::serialize(agents) {
        let _ = fs::write(cache_path, data);
    }
}

fn parse_and_cache(permissions_path: &Path, cache_path: &Path, agent: &str) -> Result<SotAgent> {
    let raw = fs::read_to_string(permissions_path)
        .with_context(|| format!("reading {}", permissions_path.display()))?;
    let stripped = strip_jsonc_comments(&raw);
    let sot: Sot = serde_json::from_str(&stripped)
        .with_context(|| format!("parsing {}", permissions_path.display()))?;

    // Convert to HashMap for cache storage
    let map: HashMap<String, SotAgent> = sot.into_iter().collect();

    let result = map
        .get(agent)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not found in permissions", agent));

    write_cache(cache_path, &map);

    result
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Check whether `agent` is allowed to run `command` according to the
/// permissions file at `permissions_path`.
///
/// When `explain` is true the returned [`CanResult`] includes a
/// [`CanExplanation`] describing which rule matched and how.
pub fn can(
    permissions_path: &Path,
    agent: &str,
    command: &str,
    explain: bool,
) -> Result<CanResult> {
    if !permissions_path.exists() {
        bail!("permissions file not found: {}", permissions_path.display());
    }

    let cache_path = permissions_path.with_file_name(CACHE_FILE_NAME);

    // Try the binary cache first, then fall back to parsing.
    let agent_data: SotAgent = if cache_is_fresh(permissions_path, &cache_path) {
        if let Some(map) = load_from_cache(&cache_path) {
            match map.get(agent).cloned() {
                Some(a) => a,
                None => {
                    // Agent not in cache — maybe cache is stale for this
                    // agent. Re-parse.
                    parse_and_cache(permissions_path, &cache_path, agent)?
                }
            }
        } else {
            // Cache corrupt — re-parse.
            parse_and_cache(permissions_path, &cache_path, agent)?
        }
    } else {
        parse_and_cache(permissions_path, &cache_path, agent)?
    };

    // Resolve the permission value.
    match &agent_data.permission.bash {
        BashPermission::Simple(value) => {
            let allowed = value != "deny";
            let kind = MatchKind::Blanket;
            Ok(CanResult {
                allowed,
                permission_value: value.clone(),
                agent: agent.to_string(),
                command: command.to_string(),
                explanation: if explain {
                    Some(CanExplanation {
                        rule: format!("bash = \"{}\"", value),
                        match_kind: format!("{:?}", kind),
                    })
                } else {
                    None
                },
            })
        }
        BashPermission::Detailed(rules) => {
            if let Some(m) = match_command(rules, command) {
                let allowed = m.permission != "deny";
                Ok(CanResult {
                    allowed,
                    permission_value: m.permission.to_string(),
                    agent: agent.to_string(),
                    command: command.to_string(),
                    explanation: if explain {
                        Some(CanExplanation {
                            rule: format!("\"{}\" → \"{}\"", m.pattern, m.permission),
                            match_kind: format!("{:?}", m.kind),
                        })
                    } else {
                        None
                    },
                })
            } else {
                // No matching rule — default deny.
                let kind = MatchKind::Default;
                Ok(CanResult {
                    allowed: false,
                    permission_value: "deny".to_string(),
                    agent: agent.to_string(),
                    command: command.to_string(),
                    explanation: if explain {
                        Some(CanExplanation {
                            rule: "(no matching rule — default deny)".to_string(),
                            match_kind: format!("{:?}", kind),
                        })
                    } else {
                        None
                    },
                })
            }
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn rules(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Write a permissions file containing a single agent with the given bash
    /// permission (as a raw JSON fragment).
    fn write_perms(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("permissions.jsonc");
        fs::write(&path, content).unwrap();
        path
    }

    // -----------------------------------------------------------------------
    // Command matching
    // -----------------------------------------------------------------------

    #[test]
    fn test_match_command_exact() {
        let r = rules(&[("npm test", "allow"), ("npm *", "deny")]);
        let m = match_command(&r, "npm test").unwrap();
        assert_eq!(m.permission, "allow");
    }

    #[test]
    fn test_match_command_wildcard() {
        let r = rules(&[("npm *", "allow")]);
        let m = match_command(&r, "npm install").unwrap();
        assert_eq!(m.permission, "allow");
    }

    #[test]
    fn test_match_command_prefix_alone() {
        let r = rules(&[("npm *", "allow")]);
        let m = match_command(&r, "npm").unwrap();
        assert_eq!(m.permission, "allow");
    }

    #[test]
    fn test_match_command_most_specific_wins() {
        let r = rules(&[("git *", "deny"), ("git commit *", "allow"), ("*", "deny")]);
        let m = match_command(&r, "git commit -m foo").unwrap();
        assert_eq!(m.permission, "allow");
    }

    #[test]
    fn test_match_command_no_match() {
        let r = rules(&[("npm *", "allow")]);
        assert!(match_command(&r, "cargo build").is_none());
    }

    #[test]
    fn test_match_command_catch_all() {
        let r = rules(&[("*", "allow")]);
        let m = match_command(&r, "anything").unwrap();
        assert_eq!(m.permission, "allow");
    }

    #[test]
    fn test_match_command_catch_all_loses_to_specific() {
        let r = rules(&[("*", "deny"), ("npm *", "allow")]);
        let m = match_command(&r, "npm install").unwrap();
        assert_eq!(m.permission, "allow");
    }

    // -----------------------------------------------------------------------
    // Match kind
    // -----------------------------------------------------------------------

    #[test]
    fn test_match_kind_exact() {
        let r = rules(&[("npm test", "allow")]);
        let m = match_command(&r, "npm test").unwrap();
        assert_eq!(m.kind, MatchKind::Exact);
    }

    #[test]
    fn test_match_kind_wildcard() {
        let r = rules(&[("npm *", "allow")]);
        let m = match_command(&r, "npm install").unwrap();
        assert_eq!(
            m.kind,
            MatchKind::Wildcard {
                prefix: "npm".to_string()
            }
        );
    }

    #[test]
    fn test_match_kind_catch_all() {
        let r = rules(&[("*", "deny")]);
        let m = match_command(&r, "anything").unwrap();
        assert_eq!(m.kind, MatchKind::CatchAll);
    }

    // -----------------------------------------------------------------------
    // can() function
    // -----------------------------------------------------------------------

    #[test]
    fn test_can_simple_deny() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{
                "plan": {
                    "permission": {
                        "bash": "deny"
                    }
                }
            }"#,
        );

        let res = can(&perms, "plan", "anything", false).unwrap();
        assert!(!res.allowed);
        assert_eq!(res.permission_value, "deny");
    }

    #[test]
    fn test_can_simple_allow() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{
                "build": {
                    "permission": {
                        "bash": {
                            "npm test": "allow"
                        }
                    }
                }
            }"#,
        );

        let res = can(&perms, "build", "npm test", false).unwrap();
        assert!(res.allowed);
        assert_eq!(res.permission_value, "allow");
        assert!(res.explanation.is_none());
    }

    #[test]
    fn test_can_detailed_allow() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{
                "build": {
                    "permission": {
                        "bash": {
                            "npm *": "allow",
                            "*": "deny"
                        }
                    }
                }
            }"#,
        );

        let res = can(&perms, "build", "npm install", true).unwrap();
        assert!(res.allowed);
        assert_eq!(res.permission_value, "allow");
        let expl = res.explanation.unwrap();
        assert!(expl.rule.contains("npm *"));
        assert!(expl.match_kind.contains("Wildcard"));
    }

    #[test]
    fn test_can_detailed_deny() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{
                "build": {
                    "permission": {
                        "bash": {
                            "npm *": "allow",
                            "*": "deny"
                        }
                    }
                }
            }"#,
        );

        let res = can(&perms, "build", "rm -rf /", true).unwrap();
        assert!(!res.allowed);
        assert_eq!(res.permission_value, "deny");
        let expl = res.explanation.unwrap();
        assert!(expl.match_kind.contains("CatchAll"));
    }

    #[test]
    fn test_can_agent_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{
                "build": {
                    "permission": {
                        "bash": "allow"
                    }
                }
            }"#,
        );

        let res = can(&perms, "nonexistent", "npm test", false);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("nonexistent"));
    }

    #[test]
    fn test_can_file_not_found() {
        let res = can(
            Path::new("/tmp/does-not-exist-agency-test.jsonc"),
            "build",
            "npm test",
            false,
        );
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));
    }

    #[test]
    fn test_can_no_matching_rule_defaults_deny() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{
                "build": {
                    "permission": {
                        "bash": {
                            "npm *": "allow"
                        }
                    }
                }
            }"#,
        );

        let res = can(&perms, "build", "cargo build", true).unwrap();
        assert!(!res.allowed);
        assert_eq!(res.permission_value, "deny");
        let expl = res.explanation.unwrap();
        assert!(expl.rule.contains("default deny"));
        assert!(expl.match_kind.contains("Default"));
    }

    // -----------------------------------------------------------------------
    // Cache
    // -----------------------------------------------------------------------

    #[test]
    fn test_cache_used_on_second_call() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{ "a": { "permission": { "bash": "allow" } } }"#,
        );
        let cache = dir.path().join(CACHE_FILE_NAME);

        // First call — creates cache.
        can(&perms, "a", "x", false).unwrap();
        assert!(cache.exists());

        let cache_size_1 = fs::metadata(&cache).unwrap().len();

        // Second call — should hit cache (verify it still works and the
        // cache file has the same size, meaning it was not corrupted).
        let res = can(&perms, "a", "x", false).unwrap();
        assert!(res.allowed);

        let cache_size_2 = fs::metadata(&cache).unwrap().len();
        assert_eq!(cache_size_1, cache_size_2, "cache file should be stable");
    }

    #[test]
    fn test_cache_invalidated_on_change() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{ "a": { "permission": { "bash": "deny" } } }"#,
        );

        let res1 = can(&perms, "a", "x", false).unwrap();
        assert!(!res1.allowed);

        // Tiny sleep to ensure mtime differs.
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Update the source file.
        fs::write(&perms, r#"{ "a": { "permission": { "bash": "allow" } } }"#).unwrap();

        let res2 = can(&perms, "a", "x", false).unwrap();
        assert!(res2.allowed, "should re-parse after source change");
    }

    #[test]
    fn test_cache_corrupt_fallback() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(
            dir.path(),
            r#"{ "a": { "permission": { "bash": "allow" } } }"#,
        );
        let cache = dir.path().join(CACHE_FILE_NAME);

        // Create cache normally.
        can(&perms, "a", "x", false).unwrap();
        assert!(cache.exists());

        // Corrupt the cache.
        fs::write(&cache, b"not valid bincode").unwrap();

        // Touch the cache so its mtime >= source mtime, forcing cache path.
        // We do this by re-writing it after a small sleep.
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&cache, b"not valid bincode").unwrap();

        // Should still succeed by falling back to parsing.
        let res = can(&perms, "a", "x", false).unwrap();
        assert!(res.allowed);
    }

    // -----------------------------------------------------------------------
    // Comment stripping
    // -----------------------------------------------------------------------

    #[test]
    fn test_strip_line_comment() {
        let input = r#"{ "a": 1 } // comment"#;
        let out = strip_jsonc_comments(input);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn test_strip_block_comment() {
        let input = "{ /* block */ \"a\": 1 }";
        let out = strip_jsonc_comments(input);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn test_strip_strings_with_slashes() {
        let input = r#"{ "url": "https://example.com" }"#;
        let out = strip_jsonc_comments(input);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["url"], "https://example.com");
    }

    #[test]
    fn test_strip_strings_with_double_slashes() {
        let input = r#"{ "path": "a//b" }"#;
        let out = strip_jsonc_comments(input);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["path"], "a//b");
    }

    #[test]
    fn test_strip_escaped_quotes() {
        let input = r#"{ "val": "he said \"hi\"" }"#;
        let out = strip_jsonc_comments(input);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["val"], r#"he said "hi""#);
    }

    #[test]
    fn test_strip_mixed_comments() {
        let input = r#"{
            // line comment
            "a": 1, /* block */
            "b": 2
        }"#;
        let out = strip_jsonc_comments(input);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], 2);
    }

    #[test]
    fn test_strip_no_comments() {
        let input = r#"{ "a": 1, "b": 2 }"#;
        let out = strip_jsonc_comments(input);
        assert_eq!(out, input);
    }

    // -----------------------------------------------------------------------
    // Performance
    // -----------------------------------------------------------------------

    fn make_large_perms(n_rules: usize) -> String {
        let mut lines = Vec::with_capacity(n_rules + 4);
        lines.push(r#"{ "perf-agent": { "permission": { "bash": {"#.to_string());
        for i in 0..n_rules {
            let comma = if i + 1 < n_rules { "," } else { "" };
            lines.push(format!(r#"  "cmd-{} *": "allow"{}"#, i, comma));
        }
        lines.push(r#"} } } }"#.to_string());
        lines.join("\n")
    }

    #[test]
    fn test_perf_10k() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(dir.path(), &make_large_perms(10_000));
        let start = std::time::Instant::now();
        let res = can(&perms, "perf-agent", "cmd-9999 run", false).unwrap();
        let elapsed = start.elapsed();
        assert!(res.allowed);
        assert!(
            elapsed.as_millis() < 5000,
            "10k rules took {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_perf_20k() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(dir.path(), &make_large_perms(20_000));
        let start = std::time::Instant::now();
        let res = can(&perms, "perf-agent", "cmd-19999 run", false).unwrap();
        let elapsed = start.elapsed();
        assert!(res.allowed);
        assert!(
            elapsed.as_millis() < 10000,
            "20k rules took {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_perf_100k() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(dir.path(), &make_large_perms(100_000));
        let start = std::time::Instant::now();
        let res = can(&perms, "perf-agent", "cmd-99999 run", false).unwrap();
        let elapsed = start.elapsed();
        assert!(res.allowed);
        assert!(
            elapsed.as_millis() < 30000,
            "100k rules took {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_perf_100k_cached() {
        let dir = tempfile::TempDir::new().unwrap();
        let perms = write_perms(dir.path(), &make_large_perms(100_000));

        // First call — populates cache.
        can(&perms, "perf-agent", "cmd-0 run", false).unwrap();

        // Second call — from cache.
        let start = std::time::Instant::now();
        let res = can(&perms, "perf-agent", "cmd-99999 run", false).unwrap();
        let elapsed = start.elapsed();
        assert!(res.allowed);
        assert!(
            elapsed.as_millis() < 5000,
            "100k cached took {}ms",
            elapsed.as_millis()
        );
    }
}
