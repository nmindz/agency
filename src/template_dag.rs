use std::collections::{HashMap, HashSet};

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Dfs;

use crate::models::Template;

/// Build a directed graph of template inheritance relationships.
///
/// Nodes are template category names. Edges point from child to parent
/// (i.e., if B extends A, there is an edge B → A).
///
/// Returns the graph and a mapping from category name to node index.
pub fn build_dag(
    templates: &IndexMap<String, Template>,
) -> Result<(DiGraph<String, ()>, HashMap<String, NodeIndex>)> {
    let mut graph = DiGraph::new();
    let mut node_map: HashMap<String, NodeIndex> = HashMap::new();

    // Add all templates as nodes
    for cat_name in templates.keys() {
        let idx = graph.add_node(cat_name.clone());
        node_map.insert(cat_name.clone(), idx);
    }

    // Add edges from child → parent
    for (cat_name, template) in templates {
        if let Some(ref extends) = template.extends {
            let child_idx = node_map[cat_name];
            for parent_name in extends.parents() {
                let parent_idx = node_map.get(parent_name).with_context(|| {
                    format!(
                        "Template '{}' extends '{}', but no template file found for '{}'",
                        cat_name, parent_name, parent_name
                    )
                })?;
                graph.add_edge(child_idx, *parent_idx, ());
            }
        }
    }

    Ok((graph, node_map))
}

/// Detect cycles in the template DAG.
///
/// Returns `Ok(())` if no cycles exist, or an error with a human-readable
/// cycle path (e.g., "Cycle detected in template inheritance: A → B → C → A").
pub fn detect_cycles(graph: &DiGraph<String, ()>) -> Result<()> {
    // petgraph's toposort returns Err(Cycle) if a cycle exists
    match toposort(graph, None) {
        Ok(_) => Ok(()),
        Err(cycle) => {
            let start = cycle.node_id();
            let cycle_path = trace_cycle(graph, start);
            bail!("Cycle detected in template inheritance: {}", cycle_path)
        }
    }
}

/// Trace a cycle path starting from the given node for error reporting.
fn trace_cycle(graph: &DiGraph<String, ()>, start: NodeIndex) -> String {
    let mut path = vec![graph[start].clone()];
    let mut visited = HashMap::new();
    visited.insert(start, 0usize);

    let mut current = start;
    loop {
        let mut found_next = false;
        for neighbor in graph.neighbors(current) {
            if neighbor == start {
                // Completed the cycle
                path.push(graph[start].clone());
                return path.join(" \u{2192} ");
            }
            if let std::collections::hash_map::Entry::Vacant(e) = visited.entry(neighbor) {
                e.insert(path.len());
                path.push(graph[neighbor].clone());
                current = neighbor;
                found_next = true;
                break;
            }
        }
        if !found_next {
            // Fallback: just show what we have
            path.push(graph[start].clone());
            return path.join(" \u{2192} ");
        }
    }
}

/// Return the ancestors of a template in topological order (parents first).
///
/// For template T with `$extends: [A, B]` where A extends C:
/// Returns `[C, A, B]` — all ancestors in dependency order, then T's
/// own parents in declaration order. T itself is NOT included.
///
/// The returned order is the merge order: earlier templates are merged first,
/// later templates can override earlier ones (but conflicts between siblings
/// at the same level are errors).
pub fn ancestor_order(
    graph: &DiGraph<String, ()>,
    node_map: &HashMap<String, NodeIndex>,
    template_name: &str,
) -> Result<Vec<String>> {
    let node_idx = node_map
        .get(template_name)
        .with_context(|| format!("Template '{}' not found in DAG", template_name))?;

    // DFS from this node following edges (child → parent), collect all reachable
    let mut ancestors: HashSet<NodeIndex> = HashSet::new();
    let mut dfs = Dfs::new(graph, *node_idx);
    while let Some(visited) = dfs.next(graph) {
        if visited != *node_idx {
            ancestors.insert(visited);
        }
    }

    if ancestors.is_empty() {
        return Ok(Vec::new());
    }

    // Use the full topological sort for correct ordering
    let topo = toposort(graph, None)
        .map_err(|_| anyhow::anyhow!("Cycle detected (should have been caught earlier)"))?;

    // petgraph's toposort with child→parent edges returns children first
    // (nodes with no incoming edges first, i.e., leaf nodes).
    // We need parents first, so reverse.
    let ordered: Vec<String> = topo
        .into_iter()
        .rev()
        .filter(|idx| ancestors.contains(idx))
        .map(|idx| graph[idx].clone())
        .collect();

    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Extends, TemplateBaseline};

    fn make_test_template(category: &str, extends: Option<Extends>) -> Template {
        Template {
            schema: "test".to_string(),
            version: "2.0.0".to_string(),
            category: category.to_string(),
            description: format!("{} template", category),
            purpose: "test".to_string(),
            doc: None,
            extends,
            baseline: TemplateBaseline {
                permission: serde_json::json!({"bash": "deny"}),
            },
        }
    }

    #[test]
    fn test_build_dag_no_extends() {
        let mut templates = IndexMap::new();
        templates.insert("a".to_string(), make_test_template("a", None));
        templates.insert("b".to_string(), make_test_template("b", None));
        templates.insert("c".to_string(), make_test_template("c", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(node_map.len(), 3);
    }

    #[test]
    fn test_build_dag_single_parent() {
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("b".to_string()))),
        );
        templates.insert("b".to_string(), make_test_template("b", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);

        // Edge from a → b (child → parent)
        let a_idx = node_map["a"];
        let b_idx = node_map["b"];
        assert!(graph.contains_edge(a_idx, b_idx));
    }

    #[test]
    fn test_build_dag_multiple_parents() {
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template(
                "a",
                Some(Extends::Multiple(vec!["b".to_string(), "c".to_string()])),
            ),
        );
        templates.insert("b".to_string(), make_test_template("b", None));
        templates.insert("c".to_string(), make_test_template("c", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);

        let a_idx = node_map["a"];
        let b_idx = node_map["b"];
        let c_idx = node_map["c"];
        assert!(graph.contains_edge(a_idx, b_idx));
        assert!(graph.contains_edge(a_idx, c_idx));
    }

    #[test]
    fn test_build_dag_missing_parent_error() {
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("nonexistent".to_string()))),
        );

        let result = build_dag(&templates);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no template file found"),
            "Error should mention missing template: {}",
            err
        );
    }

    #[test]
    fn test_detect_cycles_no_cycle() {
        // Linear chain: a → b → c
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("b".to_string()))),
        );
        templates.insert(
            "b".to_string(),
            make_test_template("b", Some(Extends::Single("c".to_string()))),
        );
        templates.insert("c".to_string(), make_test_template("c", None));

        let (graph, _) = build_dag(&templates).unwrap();
        assert!(detect_cycles(&graph).is_ok());
    }

    #[test]
    fn test_detect_cycles_self_reference() {
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("a".to_string()))),
        );

        let (graph, _) = build_dag(&templates).unwrap();
        let result = detect_cycles(&graph);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Cycle detected"),
            "Error should mention cycle: {}",
            err
        );
    }

    #[test]
    fn test_detect_cycles_two_node_cycle() {
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("b".to_string()))),
        );
        templates.insert(
            "b".to_string(),
            make_test_template("b", Some(Extends::Single("a".to_string()))),
        );

        let (graph, _) = build_dag(&templates).unwrap();
        let result = detect_cycles(&graph);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Cycle detected"),
            "Error should mention cycle: {}",
            err
        );
    }

    #[test]
    fn test_detect_cycles_three_node_cycle() {
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("b".to_string()))),
        );
        templates.insert(
            "b".to_string(),
            make_test_template("b", Some(Extends::Single("c".to_string()))),
        );
        templates.insert(
            "c".to_string(),
            make_test_template("c", Some(Extends::Single("a".to_string()))),
        );

        let (graph, _) = build_dag(&templates).unwrap();
        let result = detect_cycles(&graph);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Cycle detected"),
            "Error should mention cycle: {}",
            err
        );
    }

    #[test]
    fn test_ancestor_order_no_parents() {
        let mut templates = IndexMap::new();
        templates.insert("a".to_string(), make_test_template("a", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        let ancestors = ancestor_order(&graph, &node_map, "a").unwrap();
        assert!(ancestors.is_empty());
    }

    #[test]
    fn test_ancestor_order_single_parent() {
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("b".to_string()))),
        );
        templates.insert("b".to_string(), make_test_template("b", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        let ancestors = ancestor_order(&graph, &node_map, "a").unwrap();
        assert_eq!(ancestors, vec!["b"]);
    }

    #[test]
    fn test_ancestor_order_chain() {
        // a extends b, b extends c → ancestors of a = [c, b] (deepest first)
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template("a", Some(Extends::Single("b".to_string()))),
        );
        templates.insert(
            "b".to_string(),
            make_test_template("b", Some(Extends::Single("c".to_string()))),
        );
        templates.insert("c".to_string(), make_test_template("c", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        let ancestors = ancestor_order(&graph, &node_map, "a").unwrap();
        assert_eq!(ancestors, vec!["c", "b"]);
    }

    #[test]
    fn test_ancestor_order_multiple_parents() {
        // a extends [b, c] → ancestors of a contain both b and c
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template(
                "a",
                Some(Extends::Multiple(vec!["b".to_string(), "c".to_string()])),
            ),
        );
        templates.insert("b".to_string(), make_test_template("b", None));
        templates.insert("c".to_string(), make_test_template("c", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        let ancestors = ancestor_order(&graph, &node_map, "a").unwrap();
        assert_eq!(ancestors.len(), 2);
        assert!(ancestors.contains(&"b".to_string()));
        assert!(ancestors.contains(&"c".to_string()));
    }

    #[test]
    fn test_ancestor_order_diamond() {
        // Diamond: a extends [b, c], b extends d, c extends d
        // ancestors of a should contain d exactly once
        let mut templates = IndexMap::new();
        templates.insert(
            "a".to_string(),
            make_test_template(
                "a",
                Some(Extends::Multiple(vec!["b".to_string(), "c".to_string()])),
            ),
        );
        templates.insert(
            "b".to_string(),
            make_test_template("b", Some(Extends::Single("d".to_string()))),
        );
        templates.insert(
            "c".to_string(),
            make_test_template("c", Some(Extends::Single("d".to_string()))),
        );
        templates.insert("d".to_string(), make_test_template("d", None));

        let (graph, node_map) = build_dag(&templates).unwrap();
        let ancestors = ancestor_order(&graph, &node_map, "a").unwrap();

        // d appears exactly once
        let d_count = ancestors.iter().filter(|s| *s == "d").count();
        assert_eq!(d_count, 1, "d should appear exactly once in ancestors");

        // All three ancestors present
        assert_eq!(ancestors.len(), 3);
        assert!(ancestors.contains(&"b".to_string()));
        assert!(ancestors.contains(&"c".to_string()));
        assert!(ancestors.contains(&"d".to_string()));

        // d should come before b and c (deepest ancestor first)
        let d_pos = ancestors.iter().position(|s| s == "d").unwrap();
        let b_pos = ancestors.iter().position(|s| s == "b").unwrap();
        let c_pos = ancestors.iter().position(|s| s == "c").unwrap();
        assert!(
            d_pos < b_pos,
            "d (pos {}) should come before b (pos {})",
            d_pos,
            b_pos
        );
        assert!(
            d_pos < c_pos,
            "d (pos {}) should come before c (pos {})",
            d_pos,
            c_pos
        );
    }
}
