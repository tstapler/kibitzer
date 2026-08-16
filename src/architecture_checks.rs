use std::collections::HashMap;
use std::path::PathBuf;

use crate::import_graph::ImportGraph;

/// A finding produced by a whole-repo architecture checker. Carries a file+line where
/// the underlying import graph could attribute one (e.g. the specific import statement
/// that closes a cycle), so output still fits the `{file}:{line}: {message}` convention
/// every other checker follows — `None` when a finding is graph-wide rather than tied
/// to one edge (not needed yet, but `ImportCycleChecker` always has an edge to point at).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchFinding {
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub message: String,
}

pub trait ArchitectureChecker {
    fn name(&self) -> &str;
    fn check(&self, graph: &ImportGraph) -> Vec<ArchFinding>;
}

pub fn registry() -> Vec<Box<dyn ArchitectureChecker>> {
    vec![Box::new(ImportCycleChecker)]
}

pub fn lookup(name: &str) -> Option<Box<dyn ArchitectureChecker>> {
    registry().into_iter().find(|c| c.name() == name)
}

pub struct ImportCycleChecker;

impl ArchitectureChecker for ImportCycleChecker {
    fn name(&self) -> &str {
        "import-cycles"
    }

    fn check(&self, graph: &ImportGraph) -> Vec<ArchFinding> {
        let cycles = find_cycles(graph);
        cycles
            .into_iter()
            .map(|cycle| {
                let edge = graph
                    .edges
                    .iter()
                    .find(|e| e.from == cycle[0] && cycle.contains(&e.to));
                let mut path = cycle.clone();
                path.push(cycle[0].clone());
                ArchFinding {
                    file: edge.map(|e| e.file.clone()),
                    line: edge.map(|e| e.line),
                    message: format!("import cycle: {}", path.join(" -> ")),
                }
            })
            .collect()
    }
}

/// Tarjan's SCC algorithm, keeping only components with more than one node (or a
/// single node with a self-edge) — those are the only SCCs that represent an actual
/// import cycle rather than an isolated, acyclic package.
fn find_cycles(graph: &ImportGraph) -> Vec<Vec<String>> {
    struct Tarjan<'a> {
        graph: &'a ImportGraph,
        index_counter: usize,
        stack: Vec<String>,
        on_stack: HashMap<String, bool>,
        indices: HashMap<String, usize>,
        lowlink: HashMap<String, usize>,
        sccs: Vec<Vec<String>>,
    }

    impl<'a> Tarjan<'a> {
        fn strongconnect(&mut self, node: &str) {
            self.indices.insert(node.to_string(), self.index_counter);
            self.lowlink.insert(node.to_string(), self.index_counter);
            self.index_counter += 1;
            self.stack.push(node.to_string());
            self.on_stack.insert(node.to_string(), true);

            for edge in self.graph.edges_from(node) {
                let successor = edge.to.clone();
                if !self.indices.contains_key(&successor) {
                    self.strongconnect(&successor);
                    let successor_low = self.lowlink[&successor];
                    let node_low = self.lowlink[node];
                    self.lowlink.insert(node.to_string(), node_low.min(successor_low));
                } else if *self.on_stack.get(&successor).unwrap_or(&false) {
                    let successor_index = self.indices[&successor];
                    let node_low = self.lowlink[node];
                    self.lowlink.insert(node.to_string(), node_low.min(successor_index));
                }
            }

            if self.lowlink[node] == self.indices[node] {
                let mut component = Vec::new();
                loop {
                    let member = self.stack.pop().unwrap();
                    self.on_stack.insert(member.clone(), false);
                    let is_member_node = member == node;
                    component.push(member);
                    if is_member_node {
                        break;
                    }
                }
                self.sccs.push(component);
            }
        }
    }

    let mut tarjan = Tarjan {
        graph,
        index_counter: 0,
        stack: Vec::new(),
        on_stack: HashMap::new(),
        indices: HashMap::new(),
        lowlink: HashMap::new(),
        sccs: Vec::new(),
    };

    for node in &graph.nodes {
        if !tarjan.indices.contains_key(node) {
            tarjan.strongconnect(node);
        }
    }

    tarjan
        .sccs
        .into_iter()
        .filter(|scc| {
            scc.len() > 1 || graph.edges_from(&scc[0]).any(|e| e.to == scc[0])
        })
        .collect()
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
            file: PathBuf::from(format!("{from}.go")),
            line: 1,
        }
    }

    #[test]
    fn lookup_finds_import_cycles_checker() {
        assert!(lookup("import-cycles").is_some());
    }

    #[test]
    fn lookup_returns_none_for_unknown_name() {
        assert!(lookup("does-not-exist").is_none());
    }

    #[test]
    fn detects_two_node_cycle() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("a".to_string());
        graph.nodes.insert("b".to_string());
        graph.edges.push(edge("a", "b"));
        graph.edges.push(edge("b", "a"));

        let findings = ImportCycleChecker.check(&graph);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("import cycle"));
    }

    #[test]
    fn acyclic_graph_has_no_findings() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("a".to_string());
        graph.nodes.insert("b".to_string());
        graph.nodes.insert("c".to_string());
        graph.edges.push(edge("a", "b"));
        graph.edges.push(edge("b", "c"));

        assert!(ImportCycleChecker.check(&graph).is_empty());
    }

    #[test]
    fn detects_three_node_cycle() {
        let mut graph = ImportGraph::default();
        for n in ["a", "b", "c"] {
            graph.nodes.insert(n.to_string());
        }
        graph.edges.push(edge("a", "b"));
        graph.edges.push(edge("b", "c"));
        graph.edges.push(edge("c", "a"));

        let findings = ImportCycleChecker.check(&graph);

        assert_eq!(findings.len(), 1);
    }
}
