//! Renders an `ImportGraph` as a Mermaid `graph TD` dependency diagram, with edges that
//! close an import cycle highlighted so a viewer can spot them visually, not just by
//! reading the `import-cycles` finding text.

use std::collections::{BTreeMap, BTreeSet};

use crate::architecture_checks::find_cycles;
use crate::import_graph::ImportGraph;

/// Past this many nodes a Mermaid diagram stops being readable, so `render_dependency_graph`
/// falls back to a text note instead of emitting one.
const MAX_NODES: usize = 150;

/// Turns a package/module path into a Mermaid-safe node ID. Mermaid node IDs can't
/// contain `/`, `.`, `-`, or start with a digit, all of which are common in real import
/// paths (`github.com/foo/bar`, `./lib-utils`).
fn slugify(node: &str) -> String {
    let mut slug: String = node
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if slug.is_empty() || slug.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        slug.insert(0, 'n');
    }
    slug
}

/// Renders `graph` as a Mermaid dependency diagram. Returns a text-only fallback note
/// instead of a diagram once the node count exceeds [`MAX_NODES`].
pub fn render_dependency_graph(graph: &ImportGraph) -> String {
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

    let mut out = String::from("graph TD\n");
    for node in &graph.nodes {
        out.push_str(&format!("    {}[\"{node}\"]\n", ids[node.as_str()]));
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

        let out = render_dependency_graph(&graph);
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

        let out = render_dependency_graph(&graph);
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

        let out = render_dependency_graph(&graph);
        assert!(out.contains("[\"github.com/foo/bar-baz\"]"));
        assert!(!out.contains("github.com/foo/bar-baz["));
    }

    #[test]
    fn empty_graph_returns_note_not_empty_diagram() {
        let graph = ImportGraph::default();
        assert_eq!(
            render_dependency_graph(&graph),
            "no import graph edges to diagram"
        );
    }

    #[test]
    fn over_cap_falls_back_to_text_note() {
        let mut graph = ImportGraph::default();
        for i in 0..(MAX_NODES + 1) {
            graph.nodes.insert(format!("pkg{i}"));
        }
        let out = render_dependency_graph(&graph);
        assert!(out.contains("over the"));
        assert!(!out.starts_with("graph TD"));
    }
}
