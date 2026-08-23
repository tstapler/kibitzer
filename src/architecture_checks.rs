use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{ArchitectureConfig, DependencyRule, Severity, component_of};
use crate::glob::matches_scope;
use crate::import_graph::ImportGraph;

/// A finding produced by a whole-repo architecture checker. Carries a file+line where
/// the underlying import graph could attribute one (e.g. the specific import statement
/// that closes a cycle), so output still fits the `{file}:{line}: {message}` convention
/// every other checker follows — `None` when a finding is graph-wide rather than tied
/// to one edge (not needed yet, but `ImportCycleChecker` always has an edge to point at).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchFinding {
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    pub message: String,
    /// Narrows this finding's effective severity below the enclosing `Check.severity` —
    /// e.g. a zero-match-component advisory must never render `[blocking]` just because
    /// the `Check` that surfaced it is. `None` means "use `Check.severity` as-is"; this
    /// field only ever narrows toward `Advisory`, never overrides a real violation's
    /// severity upward or downward. `#[serde(default)]` so an `ArchFinding` cached to
    /// `cache.json` before this field existed still deserializes (same additive-field
    /// precedent as `CheckResult.command`, `src/check.rs:26-32`).
    #[serde(default)]
    pub severity_override: Option<Severity>,
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
        Box::new(ComponentDependencyChecker),
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
                    severity_override: None,
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
                        severity_override: None,
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
                        severity_override: None,
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
                    severity_override: None,
                })
            } else {
                None
            }
        }));
        findings
    }
}

/// Builds the `[component-deps]` violation message for an edge crossing from `from_comp`
/// into `to_comp`, given the `DependencyRule` declared for `from_comp` (`None` if no rule
/// references it at all). Returns `None` when the edge is allowed. Implements the
/// deny-wins / closed-world-allow-list / deny-by-default precedence from plan.md's Pattern
/// Decisions table:
/// - `to_comp` present in `deny_depend_on` → always a violation (deny wins), even if
///   `to_comp` is also present in `may_depend_on`.
/// - Otherwise, `may_depend_on: Some(list)` is a closed-world allow-list — `to_comp` not in
///   `list` is a violation.
/// - `may_depend_on: None` is open-world (deny-list-only, depguard-style) — allowed unless
///   caught by `deny_depend_on` above. (`may_depend_on`'s `None` vs. `Some(vec![])` distinction
///   isn't pinned by any Story 1.1.1 acceptance criterion; this reading matches Story
///   7.2.1/7.2.2's dogfooding examples, which rely on `may_depend_on: None` +
///   `deny_depend_on: [...]` to express a deny-list-only rule without enumerating every
///   allowed component.)
/// - No `DependencyRule` at all for `from_comp` → deny-by-default, always a violation.
fn component_dependency_finding(
    from_comp: &str,
    from_node: &str,
    to_comp: &str,
    to_node: &str,
    rule: Option<&DependencyRule>,
) -> Option<String> {
    let base = format!("[component-deps] {from_comp} ({from_node}) imports {to_comp} ({to_node})");
    match rule {
        None => Some(format!(
            "{base} — '{from_comp}' has no declared dependency rule (deny-by-default)"
        )),
        Some(rule) => {
            if rule.deny_depend_on.iter().any(|d| d == to_comp) {
                return Some(format!(
                    "{base} — '{from_comp}' may not depend on: {}",
                    rule.deny_depend_on.join(", ")
                ));
            }
            match &rule.may_depend_on {
                Some(allowed) => {
                    if allowed.iter().any(|a| a == to_comp) {
                        None
                    } else {
                        Some(format!(
                            "{base} — '{from_comp}' may depend on: {}",
                            allowed.join(", ")
                        ))
                    }
                }
                None => None,
            }
        }
    }
}

/// Returns an advisory `ArchFinding` when none of `declared` satisfies `is_matched` — a
/// declared component/rule glob that matches nothing in the current scan, surfaced so a
/// stale or typo'd pattern is noticed instead of silently never firing. `describe` renders
/// the subject of the message (e.g. `"component 'domain' (glob '**/domain')"`); `noun`
/// names what was searched for a match (e.g. `"nodes in the import graph"`,
/// `"declarations"`), so Phase 2/3's `ContentChecker`/`NamingChecker` can reuse this same
/// helper with their own wording (plan.md Story 1.1.3's shared-helper requirement).
/// `severity_override` is always `Some(Severity::Advisory)` — this never masquerades as a
/// blocking finding, regardless of the enclosing `Check.severity`.
fn zero_match_advisory<T>(
    declared: &[T],
    is_matched: impl Fn(&T) -> bool,
    describe: impl Fn(&T) -> String,
    noun: &str,
) -> Option<ArchFinding> {
    if declared.is_empty() || declared.iter().any(is_matched) {
        return None;
    }
    Some(ArchFinding {
        file: None,
        line: None,
        message: format!(
            "[component] {} matched 0 {noun} — rules referencing it will never fire",
            describe(&declared[0])
        ),
        severity_override: Some(Severity::Advisory),
    })
}

/// Arbitrary named-component allow/deny dependency rules — `component-deps`. Expresses
/// rules an ordered `layers` list can't (e.g. "services must not import `net/http`"),
/// registered alongside (not replacing) `ImportCycleChecker`/`LayeringChecker`/`CouplingChecker`.
/// `layers` continues to work unchanged via `ArchitectureConfig::effective_components()`/
/// `effective_dependency_rules()`, which desugar `layers` into this same `Component`/
/// `DependencyRule` shape — see the golden-regression tests below proving the two checkers
/// agree on every existing `LayeringChecker` fixture.
pub struct ComponentDependencyChecker;

impl ArchitectureChecker for ComponentDependencyChecker {
    fn name(&self) -> &str {
        "component-deps"
    }

    fn check(&self, graph: &ImportGraph, config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let components = config.effective_components();
        let rules = config.effective_dependency_rules();

        let mut findings: Vec<ArchFinding> = graph
            .edges
            .iter()
            // Graph-membership guard (pre-mortem.md P1 #3(i)): an edge's endpoints must
            // themselves be real `graph.nodes` entries — not just strings a component glob
            // happens to textually match — before either side is resolved to a component.
            // Defense-in-depth: today's `build_go`/`build_js` already guarantee this by
            // construction, but a future language extractor shouldn't be trusted implicitly.
            .filter(|edge| graph.nodes.contains(&edge.from) && graph.nodes.contains(&edge.to))
            .filter_map(|edge| {
                let from_comp = component_of(&edge.from, &components)?;
                let to_comp = component_of(&edge.to, &components)?;
                if from_comp == to_comp {
                    return None;
                }
                let rule = rules.iter().find(|r| r.component == from_comp);
                let message =
                    component_dependency_finding(from_comp, &edge.from, to_comp, &edge.to, rule)?;
                Some(ArchFinding {
                    file: Some(edge.file.clone()),
                    line: Some(edge.line),
                    message,
                    severity_override: None,
                })
            })
            .collect();

        findings.extend(components.iter().filter_map(|component| {
            zero_match_advisory(
                std::slice::from_ref(component),
                |c| graph.nodes.iter().any(|n| matches_scope(n, &c.paths)),
                |c| format!("component '{}' (glob '{}')", c.name, c.paths.join(", ")),
                "nodes in the import graph",
            )
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
    use crate::config::Component;
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

    fn edge_at(from: &str, to: &str, file: &str, line: usize) -> ImportEdge {
        ImportEdge {
            from: from.to_string(),
            to: to.to_string(),
            file: PathBuf::from(file),
            line,
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
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

        assert!(findings.iter().any(|f| f.message.contains("hub") && f.message.contains("imports")));
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

        assert!(findings.iter().any(|f| f.message.contains("core") && f.message.contains("imported by")));
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

    // --- Story 1.1.1: ComponentDependencyChecker ---

    #[test]
    fn lookup_finds_component_deps_checker() {
        assert!(lookup("component-deps").is_some());
    }

    #[test]
    fn component_deps_flags_disallowed_edge() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("dogfood.example/app/domain".to_string());
        graph.nodes.insert("dogfood.example/app/infra".to_string());
        graph.edges.push(edge_at(
            "dogfood.example/app/domain",
            "dogfood.example/app/infra",
            "domain/domain.go",
            5,
        ));
        let config = ArchitectureConfig {
            components: vec![
                Component {
                    name: "domain".into(),
                    paths: vec!["**/domain".into()],
                },
                Component {
                    name: "infra".into(),
                    paths: vec!["**/infra".into()],
                },
            ],
            dependency_rules: vec![DependencyRule {
                component: "domain".into(),
                may_depend_on: Some(vec!["domain".into()]),
                deny_depend_on: vec![],
            }],
            ..Default::default()
        };

        let findings = ComponentDependencyChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, Some(PathBuf::from("domain/domain.go")));
        assert_eq!(findings[0].line, Some(5));
        assert_eq!(
            findings[0].message,
            "[component-deps] domain (dogfood.example/app/domain) imports infra \
             (dogfood.example/app/infra) — 'domain' may depend on: domain"
        );
        assert_eq!(findings[0].severity_override, None);
    }

    #[test]
    fn component_deps_deny_wins_over_allow() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("dogfood.example/app/domain".to_string());
        graph.nodes.insert("dogfood.example/app/infra".to_string());
        graph.edges.push(edge_at(
            "dogfood.example/app/domain",
            "dogfood.example/app/infra",
            "domain/domain.go",
            5,
        ));
        let config = ArchitectureConfig {
            components: vec![
                Component {
                    name: "domain".into(),
                    paths: vec!["**/domain".into()],
                },
                Component {
                    name: "infra".into(),
                    paths: vec!["**/infra".into()],
                },
            ],
            dependency_rules: vec![DependencyRule {
                component: "domain".into(),
                may_depend_on: Some(vec!["infra".into()]),
                deny_depend_on: vec!["infra".into()],
            }],
            ..Default::default()
        };

        let findings = ComponentDependencyChecker.check(&graph, &config);

        // Deny wins even though "infra" also appears in `may_depend_on`.
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn component_deps_denies_by_default_with_no_rule() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("dogfood.example/app/domain".to_string());
        graph.nodes.insert("dogfood.example/app/infra".to_string());
        graph.edges.push(edge_at(
            "dogfood.example/app/domain",
            "dogfood.example/app/infra",
            "domain/domain.go",
            5,
        ));
        let config = ArchitectureConfig {
            components: vec![
                Component {
                    name: "domain".into(),
                    paths: vec!["**/domain".into()],
                },
                Component {
                    name: "infra".into(),
                    paths: vec!["**/infra".into()],
                },
            ],
            dependency_rules: vec![],
            ..Default::default()
        };

        let findings = ComponentDependencyChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn component_deps_ignores_same_component_edges() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("dogfood.example/app/domain".to_string());
        graph
            .nodes
            .insert("dogfood.example/app/domain/sub".to_string());
        graph.edges.push(edge_at(
            "dogfood.example/app/domain",
            "dogfood.example/app/domain/sub",
            "domain/domain.go",
            5,
        ));
        let config = ArchitectureConfig {
            components: vec![Component {
                name: "domain".into(),
                paths: vec!["**/domain".into(), "**/domain/**".into()],
            }],
            dependency_rules: vec![],
            ..Default::default()
        };

        // Both endpoints resolve to the same "domain" component regardless of any
        // declared rule.
        assert!(ComponentDependencyChecker.check(&graph, &config).is_empty());
    }

    #[test]
    fn component_deps_ignores_unmapped_nodes() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("dogfood.example/app/domain".to_string());
        graph.nodes.insert("fmt".to_string());
        graph.edges.push(edge_at(
            "dogfood.example/app/domain",
            "fmt",
            "domain/domain.go",
            5,
        ));
        let config = ArchitectureConfig {
            components: vec![Component {
                name: "domain".into(),
                paths: vec!["**/domain".into()],
            }],
            dependency_rules: vec![DependencyRule {
                component: "domain".into(),
                may_depend_on: Some(vec!["domain".into()]),
                deny_depend_on: vec![],
            }],
            ..Default::default()
        };

        assert!(ComponentDependencyChecker.check(&graph, &config).is_empty());
    }

    /// pre-mortem.md P1 #3(i): an edge's target must resolve to an actual `graph.nodes`
    /// entry, not just any string a component glob syntactically matches.
    #[test]
    fn component_deps_ignores_glob_matching_external_import_not_in_graph_nodes() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("dogfood.example/app/domain".to_string());
        // A real graph node the "infra" component's glob legitimately matches, so this
        // test isolates the graph-membership guard rather than confounding it with the
        // (correct, but separate) zero-match-component advisory from Story 1.1.3.
        graph
            .nodes
            .insert("dogfood.example/app/infra/client".to_string());
        graph.edges.push(edge_at(
            "dogfood.example/app/domain",
            "some-vendor/infra-client",
            "domain/domain.go",
            5,
        ));
        let config = ArchitectureConfig {
            components: vec![Component {
                name: "infra".into(),
                paths: vec!["**/infra/**".into()],
            }],
            dependency_rules: vec![DependencyRule {
                component: "domain".into(),
                may_depend_on: Some(vec!["domain".into()]),
                deny_depend_on: vec![],
            }],
            ..Default::default()
        };

        // "some-vendor/infra-client" textually matches "**/infra/**" but was never a
        // walked repo-local file (`build_go`/`build_js` would never have produced this
        // edge), so it must never resolve to a component or a violation.
        assert!(ComponentDependencyChecker.check(&graph, &config).is_empty());
    }

    // --- Story 1.1.2: `layers`-desugar golden regression tests ---

    /// Compares two checkers' findings by `(file, line)` only — message text differs by
    /// design (`[component-deps]` vs. `layering violation:`). Excludes
    /// `component-deps`-only zero-match-component advisories (Story 1.1.3), which have no
    /// `LayeringChecker` analog; this comparison is about violation-finding parity.
    fn assert_same_findings_by_location(layering: &[ArchFinding], component_deps: &[ArchFinding]) {
        let mut layering_locations: Vec<(Option<PathBuf>, Option<usize>)> =
            layering.iter().map(|f| (f.file.clone(), f.line)).collect();
        let mut component_deps_locations: Vec<(Option<PathBuf>, Option<usize>)> = component_deps
            .iter()
            .filter(|f| f.severity_override.is_none())
            .map(|f| (f.file.clone(), f.line))
            .collect();
        layering_locations.sort();
        component_deps_locations.sort();
        assert_eq!(
            layering_locations, component_deps_locations,
            "layering findings {layering:?} vs component-deps findings {component_deps:?}"
        );
    }

    #[test]
    fn layers_desugar_matches_layering_for_reverse_dependency() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/infra".to_string());
        graph.nodes.insert("app/domain".to_string());
        graph.edges.push(edge("app/infra", "app/domain"));
        let config = ArchitectureConfig {
            layers: vec!["domain".to_string(), "infra".to_string()],
            ..Default::default()
        };

        let layering_findings = LayeringChecker.check(&graph, &config);
        let component_deps_findings = ComponentDependencyChecker.check(&graph, &config);

        assert_eq!(layering_findings.len(), 1);
        assert_same_findings_by_location(&layering_findings, &component_deps_findings);
    }

    #[test]
    fn layers_desugar_matches_layering_for_forward_dependency() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/domain".to_string());
        graph.nodes.insert("app/infra".to_string());
        graph.edges.push(edge("app/domain", "app/infra"));
        let config = ArchitectureConfig {
            layers: vec!["domain".to_string(), "infra".to_string()],
            ..Default::default()
        };

        let layering_findings = LayeringChecker.check(&graph, &config);
        let component_deps_findings = ComponentDependencyChecker.check(&graph, &config);

        assert!(layering_findings.is_empty());
        assert_same_findings_by_location(&layering_findings, &component_deps_findings);
    }

    #[test]
    fn layers_desugar_matches_layering_ignoring_outside_packages() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/vendor/lib".to_string());
        graph.nodes.insert("app/other/lib".to_string());
        graph.edges.push(edge("app/vendor/lib", "app/other/lib"));
        let config = ArchitectureConfig {
            layers: vec!["domain".to_string(), "infra".to_string()],
            ..Default::default()
        };

        let layering_findings = LayeringChecker.check(&graph, &config);
        let component_deps_findings = ComponentDependencyChecker.check(&graph, &config);

        assert!(layering_findings.is_empty());
        assert_same_findings_by_location(&layering_findings, &component_deps_findings);
    }

    #[test]
    fn layers_desugar_matches_layering_with_no_declared_layers() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("a".to_string());
        graph.nodes.insert("b".to_string());
        graph.edges.push(edge("a", "b"));
        let config = ArchitectureConfig::default();

        let layering_findings = LayeringChecker.check(&graph, &config);
        let component_deps_findings = ComponentDependencyChecker.check(&graph, &config);

        assert!(layering_findings.is_empty());
        assert_same_findings_by_location(&layering_findings, &component_deps_findings);
    }

    #[test]
    fn layers_desugar_does_not_substring_match_segment_names() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/domain2".to_string());
        graph.nodes.insert("app/infra".to_string());
        graph.edges.push(edge("app/domain2", "app/infra"));
        let config = ArchitectureConfig {
            layers: vec!["domain".to_string(), "infra".to_string()],
            ..Default::default()
        };

        let layering_findings = LayeringChecker.check(&graph, &config);
        let component_deps_findings = ComponentDependencyChecker.check(&graph, &config);

        // "domain2" matches neither `layer_of()`'s exact-segment match nor the desugared
        // `**/domain`/`**/domain/**` globs (glob anchors on the full segment).
        assert!(layering_findings.is_empty());
        assert_same_findings_by_location(&layering_findings, &component_deps_findings);
    }

    // --- Story 1.1.3: zero-match component glob -> advisory finding ---

    #[test]
    fn component_deps_flags_zero_match_component_as_advisory_even_when_check_is_blocking() {
        let mut graph = ImportGraph::default();
        graph.nodes.insert("app/handlers".to_string());
        graph.nodes.insert("app/infra".to_string());
        let config = ArchitectureConfig {
            components: vec![Component {
                name: "domain".into(),
                paths: vec!["**/domain".into()],
            }],
            ..Default::default()
        };

        let findings = ComponentDependencyChecker.check(&graph, &config);

        let advisory = findings
            .iter()
            .find(|f| f.severity_override == Some(Severity::Advisory))
            .expect("zero-match advisory finding present");
        assert_eq!(advisory.file, None);
        assert_eq!(advisory.line, None);
        assert_eq!(
            advisory.message,
            "[component] component 'domain' (glob '**/domain') matched 0 nodes in the \
             import graph — rules referencing it will never fire"
        );
        // `severity_override` is unconditionally `Some(Advisory)` here — it never depends
        // on what severity an enclosing `Check` would use (`Blocking` or `Advisory`); the
        // `check()` signature carries no `Check`, so the actual "still renders `[advisory]`
        // under a blocking Check" wiring lives in mcp.rs/check.rs (Tasks 1.1.3b/c/f),
        // outside this file's scope.
    }

    #[test]
    fn zero_match_advisory_helper_ignores_matched_items() {
        let matched = vec!["a", "b"];
        assert_eq!(
            zero_match_advisory(&matched, |_| true, |s| s.to_string(), "things"),
            None
        );
    }

    #[test]
    fn zero_match_advisory_helper_ignores_empty_declared_list() {
        let declared: Vec<&str> = vec![];
        assert_eq!(
            zero_match_advisory(&declared, |_| false, |s| s.to_string(), "things"),
            None
        );
    }
}
