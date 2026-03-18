use anyhow::{bail, Context, Result};
use indexmap::IndexMap;

use crate::models::{
    is_simple_category, BashPermission, Override, PermValue, ResolvedPermission, SotPermission,
    Template,
};

/// Resolve an agent's final permissions from template baseline + override.
///
/// For "simple" categories (deny-all, allow-all, general-purpose):
///   The baseline.permission is the full permission object.
///   Apply add/remove on the top-level permission keys.
///
/// For "complex" categories (object):
///   The baseline.permission is the bash sub-object (command → permission).
///   Apply add/remove on the bash sub-object keys.
///   Wrap result as {"bash": {…resolved…}}.
pub fn resolve(template: &Template, override_data: &Override) -> Result<ResolvedPermission> {
    let perm_overrides = override_data.overrides.permission.as_ref();
    let adds = perm_overrides.and_then(|p| p.add.as_ref());
    let removes = perm_overrides.and_then(|p| p.remove.as_ref());

    if is_simple_category(&template.category) {
        resolve_simple(template, adds, removes)
    } else {
        resolve_complex(template, adds, removes)
    }
}

/// Resolve for simple categories where baseline.permission is the full permission object.
fn resolve_simple(
    template: &Template,
    adds: Option<&IndexMap<String, String>>,
    removes: Option<&Vec<String>>,
) -> Result<ResolvedPermission> {
    // Parse the baseline permission as a map
    let baseline: IndexMap<String, serde_json::Value> =
        serde_json::from_value(template.baseline.permission.clone())
            .context("Failed to parse simple baseline as map")?;

    let mut result: IndexMap<String, serde_json::Value> = baseline;

    // Apply adds (insert/overwrite top-level keys)
    if let Some(adds) = adds {
        for (k, v) in adds {
            result.insert(k.clone(), serde_json::Value::String(v.clone()));
        }
    }

    // Apply removes (delete top-level keys)
    if let Some(removes) = removes {
        for k in removes {
            result.shift_remove(k);
        }
    }

    // Convert back to SotPermission
    let write = result
        .get("write")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let edit = result
        .get("edit")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let bash = if let Some(bash_val) = result.get("bash") {
        match bash_val {
            serde_json::Value::String(s) => BashPermission::Simple(s.clone()),
            serde_json::Value::Object(_) => {
                let map: IndexMap<String, String> = serde_json::from_value(bash_val.clone())
                    .context("Failed to parse bash object in simple category")?;
                BashPermission::Detailed(map)
            }
            _ => anyhow::bail!("Unexpected bash value type in simple category"),
        }
    } else {
        anyhow::bail!("No 'bash' key in resolved simple permission")
    };

    Ok(ResolvedPermission {
        permission: SotPermission { write, edit, bash },
    })
}

/// Resolve for complex categories where baseline.permission is the bash sub-object.
fn resolve_complex(
    template: &Template,
    adds: Option<&IndexMap<String, String>>,
    removes: Option<&Vec<String>>,
) -> Result<ResolvedPermission> {
    // Parse the baseline permission as the bash sub-object
    let baseline: IndexMap<String, String> =
        serde_json::from_value(template.baseline.permission.clone())
            .context("Failed to parse complex baseline as bash object")?;

    let mut result = baseline;

    // Apply adds
    if let Some(adds) = adds {
        for (k, v) in adds {
            result.insert(k.clone(), v.clone());
        }
    }

    // Apply removes
    if let Some(removes) = removes {
        for k in removes {
            result.shift_remove(k);
        }
    }

    Ok(ResolvedPermission {
        permission: SotPermission {
            write: None,
            edit: None,
            bash: BashPermission::Detailed(result),
        },
    })
}

/// Merge multiple parent baselines into a single effective baseline.
///
/// Parents are merged left-to-right: entries from earlier parents appear
/// first; later parents can add new entries. If two parents define the
/// same key with different values, an error is returned.
///
/// After parents are merged, the template's own baseline is applied on top
/// (self always wins over parents).
pub fn merge_baselines(
    parent_baselines: &[(String, IndexMap<String, PermValue>)],
    self_baseline: &IndexMap<String, PermValue>,
    template_name: &str,
) -> Result<IndexMap<String, PermValue>> {
    let mut merged: IndexMap<String, PermValue> = IndexMap::new();

    // Merge parents left-to-right
    for (parent_name, parent_baseline) in parent_baselines {
        for (key, value) in parent_baseline {
            if let Some(existing_value) = merged.get(key) {
                if existing_value != value {
                    bail!(
                        "Conflict in template '{}': key '{}' has value '{}' \
                         from a previous parent, but parent '{}' defines it as '{}'. \
                         Resolve by moving the entry to the child template's own baseline.",
                        template_name,
                        key,
                        existing_value,
                        parent_name,
                        value
                    );
                }
                // Same value — no conflict, skip (already present)
            } else {
                merged.insert(key.clone(), value.clone());
            }
        }
    }

    // Self's baseline wins over parents (overwrite or add)
    for (key, value) in self_baseline {
        merged.insert(key.clone(), value.clone());
    }

    Ok(merged)
}

/// Resolve an agent's final permissions from a pre-merged template baseline
/// + agent override.
///
/// This is the multi-template-aware version. The caller pre-computes the
/// effective baseline by merging the full inheritance chain, then passes
/// it here for agent override application.
pub fn resolve_with_baseline(
    effective_baseline: &IndexMap<String, PermValue>,
    category: &str,
    override_data: &Override,
) -> Result<ResolvedPermission> {
    let perm_overrides = override_data.overrides.permission.as_ref();
    let adds = perm_overrides.and_then(|p| p.add.as_ref());
    let removes = perm_overrides.and_then(|p| p.remove.as_ref());

    if is_simple_category(category) {
        // For simple categories, the effective_baseline keys are the top-level
        // permission object keys (write, edit, bash)
        let mut result: IndexMap<String, serde_json::Value> = IndexMap::new();
        for (k, v) in effective_baseline {
            result.insert(k.clone(), serde_json::Value::String(v.clone()));
        }

        // Apply adds
        if let Some(adds) = adds {
            for (k, v) in adds {
                result.insert(k.clone(), serde_json::Value::String(v.clone()));
            }
        }

        // Apply removes
        if let Some(removes) = removes {
            for k in removes {
                result.shift_remove(k);
            }
        }

        // Convert to SotPermission (same logic as resolve_simple)
        let write = result
            .get("write")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let edit = result
            .get("edit")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let bash = if let Some(bash_val) = result.get("bash") {
            match bash_val {
                serde_json::Value::String(s) => BashPermission::Simple(s.clone()),
                serde_json::Value::Object(_) => {
                    let map: IndexMap<String, String> = serde_json::from_value(bash_val.clone())
                        .context("Failed to parse bash object in simple category")?;
                    BashPermission::Detailed(map)
                }
                _ => bail!("Unexpected bash value type in simple category"),
            }
        } else {
            bail!("No 'bash' key in resolved simple permission")
        };

        Ok(ResolvedPermission {
            permission: SotPermission { write, edit, bash },
        })
    } else {
        // For complex categories, effective_baseline IS the bash command map
        let mut result = effective_baseline.clone();

        // Apply adds
        if let Some(adds) = adds {
            for (k, v) in adds {
                result.insert(k.clone(), v.clone());
            }
        }

        // Apply removes
        if let Some(removes) = removes {
            for k in removes {
                result.shift_remove(k);
            }
        }

        Ok(ResolvedPermission {
            permission: SotPermission {
                write: None,
                edit: None,
                bash: BashPermission::Detailed(result),
            },
        })
    }
}

/// Source of a permission entry in trace output.
#[derive(Debug, Clone)]
pub enum PermissionSource {
    /// Entry inherited from a parent template.
    Inherited(String),
    /// Entry from the template's own baseline.
    SelfBaseline(String),
    /// Entry added by agent override.
    AgentAdd(String),
}

/// A traced permission entry with provenance.
#[derive(Debug, Clone)]
pub struct TracedPermission {
    pub key: String,
    pub value: String,
    pub source: PermissionSource,
}

/// Resolve with full provenance tracking.
pub fn resolve_with_trace(
    parent_baselines: &[(String, IndexMap<String, PermValue>)],
    self_baseline: &IndexMap<String, PermValue>,
    template_name: &str,
    override_data: &Override,
) -> Result<Vec<TracedPermission>> {
    let mut merged: IndexMap<String, (PermValue, PermissionSource)> = IndexMap::new();

    for (parent_name, parent_baseline) in parent_baselines {
        for (key, value) in parent_baseline {
            if let Some((existing_value, _)) = merged.get(key) {
                if existing_value != value {
                    bail!(
                        "Conflict in template '{}': key '{}' has value '{}' \
                         from a previous parent, but parent '{}' defines it as '{}'.",
                        template_name,
                        key,
                        existing_value,
                        parent_name,
                        value
                    );
                }
            } else {
                merged.insert(
                    key.clone(),
                    (
                        value.clone(),
                        PermissionSource::Inherited(parent_name.clone()),
                    ),
                );
            }
        }
    }

    for (key, value) in self_baseline {
        merged.insert(
            key.clone(),
            (
                value.clone(),
                PermissionSource::SelfBaseline(template_name.to_string()),
            ),
        );
    }

    let perm_overrides = override_data.overrides.permission.as_ref();
    if let Some(adds) = perm_overrides.and_then(|p| p.add.as_ref()) {
        for (key, value) in adds {
            merged.insert(
                key.clone(),
                (
                    value.clone(),
                    PermissionSource::AgentAdd(override_data.agent.clone()),
                ),
            );
        }
    }

    if let Some(removes) = perm_overrides.and_then(|p| p.remove.as_ref()) {
        for key in removes {
            merged.shift_remove(key);
        }
    }

    Ok(merged
        .into_iter()
        .map(|(key, (value, source))| TracedPermission { key, value, source })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    // Helper to create an IndexMap from pairs
    fn make_map(pairs: Vec<(&str, &str)>) -> IndexMap<String, PermValue> {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    // Helper to create a minimal Override for testing
    fn make_override_with(
        agent: &str,
        extends: &str,
        add: Option<IndexMap<String, PermValue>>,
        remove: Option<Vec<String>>,
    ) -> Override {
        Override {
            schema: "test".to_string(),
            version: "1.0.0".to_string(),
            agent: agent.to_string(),
            extends: extends.to_string(),
            doc: None,
            overrides: OverrideBlock {
                permission: Some(PermissionOverride { add, remove }),
            },
        }
    }

    #[test]
    fn test_merge_baselines_single_parent() {
        let parent = make_map(vec![("git status", "deny"), ("git log", "deny")]);
        let self_bl = make_map(vec![("npm *", "allow")]);
        let result = merge_baselines(&[("parent".to_string(), parent)], &self_bl, "child").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result.get("git status").unwrap(), "deny");
        assert_eq!(result.get("npm *").unwrap(), "allow");
    }

    #[test]
    fn test_merge_baselines_multiple_parents_no_conflict() {
        let p1 = make_map(vec![("git status", "deny")]);
        let p2 = make_map(vec![("jj log", "deny")]);
        let self_bl = make_map(vec![("npm *", "allow")]);
        let result = merge_baselines(
            &[("p1".to_string(), p1), ("p2".to_string(), p2)],
            &self_bl,
            "child",
        )
        .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_merge_baselines_same_value_no_conflict() {
        let p1 = make_map(vec![("git status", "deny")]);
        let p2 = make_map(vec![("git status", "deny")]); // same value = OK
        let self_bl = IndexMap::new();
        let result = merge_baselines(
            &[("p1".to_string(), p1), ("p2".to_string(), p2)],
            &self_bl,
            "child",
        )
        .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("git status").unwrap(), "deny");
    }

    #[test]
    fn test_merge_baselines_conflict_error() {
        let p1 = make_map(vec![("git status", "deny")]);
        let p2 = make_map(vec![("git status", "allow")]); // different value = conflict
        let self_bl = IndexMap::new();
        let result = merge_baselines(
            &[("p1".to_string(), p1), ("p2".to_string(), p2)],
            &self_bl,
            "child",
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Conflict"),
            "Error should mention conflict: {}",
            err
        );
        assert!(
            err.contains("git status"),
            "Error should mention key: {}",
            err
        );
        assert!(
            err.contains("p2"),
            "Error should mention parent name: {}",
            err
        );
    }

    #[test]
    fn test_merge_baselines_self_wins() {
        let parent = make_map(vec![("npm *", "allow")]);
        let self_bl = make_map(vec![("npm *", "ask")]); // self overrides parent
        let result = merge_baselines(&[("parent".to_string(), parent)], &self_bl, "child").unwrap();
        assert_eq!(result.get("npm *").unwrap(), "ask");
    }

    #[test]
    fn test_merge_baselines_empty_parents() {
        let self_bl = make_map(vec![("npm *", "allow")]);
        let result = merge_baselines(&[], &self_bl, "child").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("npm *").unwrap(), "allow");
    }

    #[test]
    fn test_merge_baselines_empty_self() {
        let parent = make_map(vec![("git status", "deny")]);
        let self_bl = IndexMap::new();
        let result = merge_baselines(&[("parent".to_string(), parent)], &self_bl, "child").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("git status").unwrap(), "deny");
    }

    #[test]
    fn test_resolve_with_baseline_complex() {
        let effective = make_map(vec![("npm *", "allow"), ("git status", "deny")]);
        let ov = make_override_with(
            "test-agent",
            "backend-api",
            Some(make_map(vec![("docker *", "ask")])),
            Some(vec!["git status".to_string()]),
        );
        let result = resolve_with_baseline(&effective, "backend-api", &ov).unwrap();
        match &result.permission.bash {
            BashPermission::Detailed(map) => {
                assert_eq!(map.get("npm *").unwrap(), "allow");
                assert_eq!(map.get("docker *").unwrap(), "ask");
                assert!(
                    map.get("git status").is_none(),
                    "git status should be removed"
                );
            }
            _ => panic!("Expected Detailed bash permission"),
        }
    }

    #[test]
    fn test_resolve_with_baseline_simple() {
        let effective = make_map(vec![("bash", "deny")]);
        let ov = make_override_with("plan", "planning", None, None);
        let result = resolve_with_baseline(&effective, "planning", &ov).unwrap();
        assert_eq!(
            result.permission.bash,
            BashPermission::Simple("deny".to_string())
        );
    }

    #[test]
    fn test_resolve_backward_compat() {
        // Existing resolve() function still works
        let template = Template {
            schema: "test".to_string(),
            version: "1.0.0".to_string(),
            category: "backend-api".to_string(),
            description: "test".to_string(),
            purpose: "test".to_string(),
            doc: None,
            extends: None,
            baseline: TemplateBaseline {
                permission: serde_json::json!({"npm *": "allow", "git status": "deny"}),
            },
        };
        let ov = make_override_with(
            "test-agent",
            "backend-api",
            Some(make_map(vec![("docker *", "ask")])),
            None,
        );
        let result = resolve(&template, &ov).unwrap();
        match &result.permission.bash {
            BashPermission::Detailed(map) => {
                assert_eq!(map.get("npm *").unwrap(), "allow");
                assert_eq!(map.get("docker *").unwrap(), "ask");
            }
            _ => panic!("Expected Detailed bash permission"),
        }
    }

    #[test]
    fn test_resolve_with_trace_inherited() {
        let parent = make_map(vec![("git status", "deny")]);
        let self_bl = make_map(vec![("npm *", "allow")]);
        let ov = make_override_with("test", "cat", None, None);
        let traced = resolve_with_trace(
            &[("vcs-permissions".to_string(), parent)],
            &self_bl,
            "cat",
            &ov,
        )
        .unwrap();
        let git_entry = traced.iter().find(|t| t.key == "git status").unwrap();
        assert!(
            matches!(&git_entry.source, PermissionSource::Inherited(p) if p == "vcs-permissions")
        );
    }

    #[test]
    fn test_resolve_with_trace_self_baseline() {
        let parent = make_map(vec![("git status", "deny")]);
        let self_bl = make_map(vec![("npm *", "allow")]);
        let ov = make_override_with("test", "cat", None, None);
        let traced =
            resolve_with_trace(&[("vcs".to_string(), parent)], &self_bl, "backend-api", &ov)
                .unwrap();
        let npm_entry = traced.iter().find(|t| t.key == "npm *").unwrap();
        assert!(
            matches!(&npm_entry.source, PermissionSource::SelfBaseline(t) if t == "backend-api")
        );
    }

    #[test]
    fn test_resolve_with_trace_agent_add() {
        let self_bl = make_map(vec![("npm *", "allow")]);
        let ov = make_override_with(
            "x-backend",
            "cat",
            Some(make_map(vec![("docker *", "ask")])),
            None,
        );
        let traced = resolve_with_trace(&[], &self_bl, "cat", &ov).unwrap();
        let docker_entry = traced.iter().find(|t| t.key == "docker *").unwrap();
        assert!(matches!(&docker_entry.source, PermissionSource::AgentAdd(a) if a == "x-backend"));
    }

    #[test]
    fn test_resolve_with_trace_self_overrides_parent() {
        let parent = make_map(vec![("npm *", "allow")]);
        let self_bl = make_map(vec![("npm *", "ask")]);
        let ov = make_override_with("test", "cat", None, None);
        let traced =
            resolve_with_trace(&[("parent".to_string(), parent)], &self_bl, "child", &ov).unwrap();
        let npm_entry = traced.iter().find(|t| t.key == "npm *").unwrap();
        assert_eq!(npm_entry.value, "ask");
        assert!(matches!(
            &npm_entry.source,
            PermissionSource::SelfBaseline(_)
        ));
    }

    #[test]
    fn test_resolve_with_trace_counts() {
        let parent = make_map(vec![("git status", "deny"), ("jj log", "deny")]);
        let self_bl = make_map(vec![("npm *", "allow")]);
        let ov = make_override_with(
            "agent",
            "cat",
            Some(make_map(vec![("docker *", "ask")])),
            None,
        );
        let traced =
            resolve_with_trace(&[("vcs".to_string(), parent)], &self_bl, "cat", &ov).unwrap();
        assert_eq!(traced.len(), 4);
        let inherited = traced
            .iter()
            .filter(|t| matches!(&t.source, PermissionSource::Inherited(_)))
            .count();
        let self_count = traced
            .iter()
            .filter(|t| matches!(&t.source, PermissionSource::SelfBaseline(_)))
            .count();
        let agent_count = traced
            .iter()
            .filter(|t| matches!(&t.source, PermissionSource::AgentAdd(_)))
            .count();
        assert_eq!(inherited, 2);
        assert_eq!(self_count, 1);
        assert_eq!(agent_count, 1);
    }
}
