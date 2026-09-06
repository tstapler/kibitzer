//! Renders an `ImportGraph` as a Mermaid `graph TD` dependency diagram, with edges that
//! close an import cycle highlighted so a viewer can spot them visually, not just by
//! reading the `import-cycles` finding text.

use std::collections::{BTreeMap, BTreeSet};

use crate::architecture_checks::find_cycles;
use crate::config::{Component, component_of};
use crate::import_graph::ImportGraph;

/// Past this many nodes a Mermaid diagram stops being readable, so `render_dependency_graph`
/// falls back to a text note instead of emitting one.
const MAX_NODES: usize = 150;

/// Turns a package/module path into a Mermaid-safe node ID. Mermaid node IDs can't
/// contain `/`, `.`, `-`, or start with a digit, all of which are common in real import
/// paths (`github.com/foo/bar`, `./lib-utils`).
///
/// `pub(crate)`: reused by `arch_diagram.rs`'s `render_component_diagram` so package/symbol
/// paths get the same Mermaid-safe ID treatment instead of a second slugify implementation.
pub(crate) fn slugify(node: &str) -> String {
    let mut slug: String = node
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if slug.is_empty() || slug.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        slug.insert(0, 'n');
    }
    slug
}

/// Renders `graph` as a Mermaid dependency diagram, grouping nodes that resolve to a
/// `components` entry (via [`crate::config::component_of`]) into `subgraph {name} ...
/// end` blocks, in `components`' declaration order; a node matching no component
/// renders outside any subgraph, exactly as before per-component grouping existed.
/// Returns a text-only fallback note instead of a diagram once the node count exceeds
/// [`MAX_NODES`].
pub fn render_dependency_graph(graph: &ImportGraph, components: &[Component]) -> String {
    if graph.nodes.is_empty() {
        return "no import graph edges to diagram".to_string();
    }
    if graph.nodes.len() > MAX_NODES {
        return format!(
            "dependency graph has {} nodes, over the {MAX_NODES}-node diagram cap — \
             pass a narrower `scope` to render a subgraph instead",
            graph.nodes.len()
        );
    }

    let ids: BTreeMap<&str, String> = graph
        .nodes
        .iter()
        .map(|n| (n.as_str(), slugify(n)))
        .collect();

    let cycle_nodes: BTreeSet<String> = find_cycles(graph).into_iter().flatten().collect();

    // Dedup edges (repeated imports between the same two packages/modules should draw
    // one arrow, not one per importing line).
    let mut edges: BTreeSet<(&str, &str)> = BTreeSet::new();
    for edge in &graph.edges {
        edges.insert((edge.from.as_str(), edge.to.as_str()));
    }

    let node_component: BTreeMap<&str, Option<&str>> = graph
        .nodes
        .iter()
        .map(|n| (n.as_str(), component_of(n, components)))
        .collect();

    let mut out = String::from("graph TD\n");
    for component in components {
        let matched: Vec<&str> = graph
            .nodes
            .iter()
            .map(String::as_str)
            .filter(|n| node_component[n] == Some(component.name.as_str()))
            .collect();
        if matched.is_empty() {
            continue;
        }
        out.push_str(&format!("subgraph {}\n", component.name));
        for node in matched {
            out.push_str(&format!("    {}[\"{node}\"]\n", ids[node]));
        }
        out.push_str("end\n");
    }
    for node in &graph.nodes {
        if node_component[node.as_str()].is_none() {
            out.push_str(&format!("    {}[\"{node}\"]\n", ids[node.as_str()]));
        }
    }

    let mut cycle_link_indices = Vec::new();
    for (i, (from, to)) in edges.iter().enumerate() {
        out.push_str(&format!("    {} --> {}\n", ids[from], ids[to]));
        if cycle_nodes.contains(*from) && cycle_nodes.contains(*to) {
            cycle_link_indices.push(i);
        }
    }

    if !cycle_nodes.is_empty() {
        out.push_str("    classDef cycle fill:#fee2e2,stroke:#dc2626,stroke-width:2px;\n");
        let cycle_ids: Vec<&str> = cycle_nodes
            .iter()
            .map(|n| ids[n.as_str()].as_str())
            .collect();
        out.push_str(&format!("    class {} cycle;\n", cycle_ids.join(",")));
    }
    for i in cycle_link_indices {
        out.push_str(&format!(
            "    linkStyle {i} stroke:#dc2626,stroke-width:2px;\n"
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import_graph::ImportEdge;
    use std::path::PathBuf;

    fn edge(from: &str, to: &str) -> ImportEdge {
        ImportEdge {
            from: from.to_string(),
            to: to.to_string(),
            file: PathBuf::from("main.go"),
            line: 1,
        }
    }

    #[test]
    fn renders_nodes_and_edges() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("a".to_string());
        graph.nodes.insert("b".to_string());
        graph.edges.push(edge("a", "b"));

        let out = render_dependency_graph(&graph, &[]);
        assert!(out.starts_with("graph TD\n"));
        assert!(out.contains("[\"a\"]"));
        assert!(out.contains("[\"b\"]"));
        assert!(out.contains("-->"));
        assert!(!out.contains("classDef cycle"));
    }

    #[test]
    fn highlights_cycle_nodes_and_edges() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("a".to_string());
        graph.nodes.insert("b".to_string());
        graph.edges.push(edge("a", "b"));
        graph.edges.push(edge("b", "a"));

        let out = render_dependency_graph(&graph, &[]);
        assert!(out.contains("classDef cycle"));
        assert!(out.contains("linkStyle"));
    }

    #[test]
    fn slugifies_paths_with_special_characters() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("github.com/foo/bar-baz".to_string());
        graph
            .edges
            .push(edge("github.com/foo/bar-baz", "github.com/foo/bar-baz"));

        let out = render_dependency_graph(&graph, &[]);
        assert!(out.contains("[\"github.com/foo/bar-baz\"]"));
        assert!(!out.contains("github.com/foo/bar-baz["));
    }

    #[test]
    fn empty_graph_returns_note_not_empty_diagram() {
        let graph = ImportGraph::default();
        assert_eq!(
            render_dependency_graph(&graph, &[]),
            "no import graph edges to diagram"
        );
    }

    #[test]
    fn mermaid_diagram_groups_nodes_by_component() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("dogfood.example/app/domain".to_string());
        graph.nodes.insert("dogfood.example/app/infra".to_string());
        graph.nodes.insert("dogfood.example/app/other".to_string());
        graph.edges.push(edge(
            "dogfood.example/app/infra",
            "dogfood.example/app/domain",
        ));
        let components = vec![
            Component {
                name: "domain".to_string(),
                paths: vec!["**/domain".to_string()],
            },
            Component {
                name: "infra".to_string(),
                paths: vec!["**/infra".to_string()],
            },
        ];

        let out = render_dependency_graph(&graph, &components);
        assert!(out.contains("subgraph domain"));
        assert!(out.contains("subgraph infra"));

        let domain_id = slugify("dogfood.example/app/domain");
        let infra_id = slugify("dogfood.example/app/infra");
        let other_id = slugify("dogfood.example/app/other");

        // Each subgraph block contains its own matched node's slugified id.
        let domain_block = out
            .split("subgraph domain\n")
            .nth(1)
            .and_then(|s| s.split("end\n").next())
            .unwrap();
        assert!(domain_block.contains(&domain_id));
        let infra_block = out
            .split("subgraph infra\n")
            .nth(1)
            .and_then(|s| s.split("end\n").next())
            .unwrap();
        assert!(infra_block.contains(&infra_id));

        // The unmatched node renders outside any subgraph block, exactly as today.
        assert!(out.contains(&other_id));
        assert!(!domain_block.contains(&other_id));
        assert!(!infra_block.contains(&other_id));

        // Existing edge/cycle-highlighting lines are unchanged.
        assert!(out.contains(&format!("{infra_id} --> {domain_id}")));
    }

    #[test]
    fn over_cap_falls_back_to_text_note() {
        let mut graph = ImportGraph::default();
        for i in 0..(MAX_NODES + 1) {
            graph.nodes.insert(format!("pkg{i}"));
        }
        let out = render_dependency_graph(&graph, &[]);
        assert!(out.contains("over the"));
        assert!(!out.starts_with("graph TD"));
    }
}
