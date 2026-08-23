use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::ArchitectureConfig;
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
    fn check(&self, graph: &ImportGraph, config: &ArchitectureConfig) -> Vec<ArchFinding>;
}

pub fn registry() -> Vec<Box<dyn ArchitectureChecker>> {
    vec![
        Box::new(ImportCycleChecker),
        Box::new(LayeringChecker),
        Box::new(CouplingChecker),
    ]
}

pub fn lookup(name: &str) -> Option<Box<dyn ArchitectureChecker>> {
    registry().into_iter().find(|c| c.name() == name)
}

pub struct ImportCycleChecker;

impl ArchitectureChecker for ImportCycleChecker {
    fn name(&self) -> &str {
        "import-cycles"
    }

    fn check(&self, graph: &ImportGraph, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
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

/// Returns the index into `config.layers` of the layer `node`'s path belongs to, by
/// matching a layer name against any `/`-separated segment of `node` exactly. `None`
/// means `node` isn't part of the declared layering and is ignored by the checker —
/// e.g. third-party/vendored packages, or a project that hasn't finished migrating
/// every package into a named layer yet.
fn layer_of(node: &str, layers: &[String]) -> Option<usize> {
    let segments: Vec<&str> = node.split('/').collect();
    layers
        .iter()
        .position(|layer| segments.contains(&layer.as_str()))
}

pub struct LayeringChecker;

impl ArchitectureChecker for LayeringChecker {
    fn name(&self) -> &str {
        "layering"
    }

    /// Flags an import edge that runs from a later-declared layer back into an
    /// earlier-declared one — e.g. `infra` importing `domain` when
    /// `config.layers == ["handlers", "domain", "infra"]`. Higher layers (earlier in
    /// the list) are expected to depend on lower ones; the reverse means a lower layer
    /// has taken on a dependency it shouldn't know about.
    fn check(&self, graph: &ImportGraph, config: &ArchitectureConfig) -> Vec<ArchFinding> {
        if config.layers.is_empty() {
            return Vec::new();
        }
        graph
            .edges
            .iter()
            .filter_map(|edge| {
                let from_layer = layer_of(&edge.from, &config.layers)?;
                let to_layer = layer_of(&edge.to, &config.layers)?;
                if to_layer < from_layer {
                    Some(ArchFinding {
                        file: Some(edge.file.clone()),
                        line: Some(edge.line),
                        message: format!(
                            "layering violation: {} (layer '{}') imports {} (layer '{}') — \
                             '{}' is declared as a lower layer than '{}'",
                            edge.from,
                            config.layers[from_layer],
                            edge.to,
                            config.layers[to_layer],
                            config.layers[from_layer],
                            config.layers[to_layer]
                        ),
                    })
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Fan-out/fan-in beyond this many distinct packages is flagged by `coupling`. Fixed
/// for now, matching `LONG_FUNCTION_LINES`'s fixed-then-configurable precedent in
/// `src/rules.rs` — a per-project override can follow later if the default proves
/// noisy in practice.
const MAX_FAN_OUT: usize = 10;
const MAX_FAN_IN: usize = 10;

pub struct CouplingChecker;

impl ArchitectureChecker for CouplingChecker {
    fn name(&self) -> &str {
        "coupling"
    }

    fn check(&self, graph: &ImportGraph, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let mut fan_out: HashMap<&str, usize> = HashMap::new();
        let mut fan_in: HashMap<&str, usize> = HashMap::new();
        for edge in &graph.edges {
            *fan_out.entry(edge.from.as_str()).or_default() += 1;
            *fan_in.entry(edge.to.as_str()).or_default() += 1;
        }

        let mut findings: Vec<ArchFinding> = graph
            .nodes
            .iter()
            .filter_map(|node| {
                let out = *fan_out.get(node.as_str()).unwrap_or(&0);
                if out > MAX_FAN_OUT {
                    Some(ArchFinding {
                        file: None,
                        line: None,
                        message: format!(
                            "[coupling] {node} imports {out} distinct packages (over {MAX_FAN_OUT}) \
                             — consider splitting its responsibilities"
                        ),
                    })
                } else {
                    None
                }
            })
            .collect();
        findings.extend(graph.nodes.iter().filter_map(|node| {
            let in_count = *fan_in.get(node.as_str()).unwrap_or(&0);
            if in_count > MAX_FAN_IN {
                Some(ArchFinding {
                    file: None,
                    line: None,
                    message: format!(
                        "[coupling] {node} is imported by {in_count} distinct packages \
                         (over {MAX_FAN_IN}) — changes to it have a wide blast radius"
                    ),
                })
            } else {
                None
            }
        }));
        findings
    }
}

/// Tarjan's SCC algorithm, keeping only components with more than one node (or a
/// single node with a self-edge) — those are the only SCCs that represent an actual
/// import cycle rather than an isolated, acyclic package. Exposed (not just used
/// internally by `ImportCycleChecker`) so `src/mermaid.rs` can highlight cycle edges
/// in the rendered dependency diagram without re-deriving SCCs itself.
pub fn find_cycles(graph: &ImportGraph) -> Vec<Vec<String>> {
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
                    self.lowlink
                        .insert(node.to_string(), node_low.min(successor_low));
                } else if *self.on_stack.get(&successor).unwrap_or(&false) {
                    let successor_index = self.indices[&successor];
                    let node_low = self.lowlink[node];
                    self.lowlink
                        .insert(node.to_string(), node_low.min(successor_index));
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
        .filter(|scc| scc.len() > 1 || graph.edges_from(&scc[0]).any(|e| e.to == scc[0]))
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

        let findings = ImportCycleChecker.check(&graph, &ArchitectureConfig::default());

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

        assert!(
            ImportCycleChecker
                .check(&graph, &ArchitectureConfig::default())
                .is_empty()
        );
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

        let findings = ImportCycleChecker.check(&graph, &ArchitectureConfig::default());

        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn lookup_finds_layering_and_coupling_checkers() {
        assert!(lookup("layering").is_some());
        assert!(lookup("coupling").is_some());
    }

    #[test]
    fn layering_flags_a_reverse_dependency() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/infra".to_string());
        graph.nodes.insert("app/domain".to_string());
        graph.edges.push(edge("app/infra", "app/domain"));
        let config = ArchitectureConfig {
            layers: vec!["domain".to_string(), "infra".to_string()],
        };

        let findings = LayeringChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("layering violation"));
    }

    #[test]
    fn layering_allows_a_forward_dependency() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/domain".to_string());
        graph.nodes.insert("app/infra".to_string());
        graph.edges.push(edge("app/domain", "app/infra"));
        let config = ArchitectureConfig {
            layers: vec!["domain".to_string(), "infra".to_string()],
        };

        assert!(LayeringChecker.check(&graph, &config).is_empty());
    }

    #[test]
    fn layering_ignores_packages_outside_the_declared_layers() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/vendor/lib".to_string());
        graph.nodes.insert("app/other/lib".to_string());
        graph.edges.push(edge("app/vendor/lib", "app/other/lib"));
        let config = ArchitectureConfig {
            layers: vec!["domain".to_string(), "infra".to_string()],
        };

        assert!(LayeringChecker.check(&graph, &config).is_empty());
    }

    #[test]
    fn layering_with_no_declared_layers_has_no_findings() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("a".to_string());
        graph.nodes.insert("b".to_string());
        graph.edges.push(edge("a", "b"));

        assert!(
            LayeringChecker
                .check(&graph, &ArchitectureConfig::default())
                .is_empty()
        );
    }

    #[test]
    fn coupling_flags_high_fan_out() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("hub".to_string());
        for n in 0..(MAX_FAN_OUT + 1) {
            let dep = format!("dep{n}");
            graph.nodes.insert(dep.clone());
            graph.edges.push(edge("hub", &dep));
        }

        let findings = CouplingChecker.check(&graph, &ArchitectureConfig::default());

        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("hub") && f.message.contains("imports"))
        );
    }

    #[test]
    fn coupling_flags_high_fan_in() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("core".to_string());
        for n in 0..(MAX_FAN_IN + 1) {
            let dep = format!("dep{n}");
            graph.nodes.insert(dep.clone());
            graph.edges.push(edge(&dep, "core"));
        }

        let findings = CouplingChecker.check(&graph, &ArchitectureConfig::default());

        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("core") && f.message.contains("imported by"))
        );
    }

    #[test]
    fn coupling_below_threshold_has_no_findings() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("a".to_string());
        graph.nodes.insert("b".to_string());
        graph.edges.push(edge("a", "b"));

        assert!(
            CouplingChecker
                .check(&graph, &ArchitectureConfig::default())
                .is_empty()
        );
    }
}
