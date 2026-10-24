//! Advisory Go package-focus check: does a package hold two or more groups of types that
//! never reference each other? Evidence is the intra-package reference graph from
//! `ArchModel`, never size alone (see `docs/go-package-focus.md`, ADR-001).
//!
//! Every method collapses into its receiver type's node (a "super-node"), so a type whose
//! methods split into groups is Extract Class territory (`extract_class.rs`, `lcom`), not a
//! package-split signal. Types and free functions are then joined by resolved calls,
//! cross-type field accesses, embedding/`implements` edges and parameter types; a package
//! qualifies when it has at least two components that each hold enough types.
//!
//! Go only. Package-level shared state (`sync`/`atomic`/`chan` vars) and `init()`
//! registration are contrary evidence, not connections.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::arch_model::{ArchModel, PackageNode, SymbolKind, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::checkers::is_test_file;
use crate::config::{ArchitectureConfig, Severity};

/// Provisional thresholds; `docs/go-package-focus.md` ("Threshold decisions") records why.
const MIN_TYPES_PER_COMPONENT: usize = 2;
const MIN_TYPE_NODES: usize = 4;
const MAX_TYPE_NODES: usize = 60;
/// Size-only mode: packages at or above either bound are reported as informational leads.
const SIZE_ONLY_MIN_FILES: usize = 15;
const SIZE_ONLY_MIN_SYMBOLS: usize = 120;
/// A `main` package or a `cmd/` path with at least this many distinct imports is wiring.
const ENTRY_POINT_MIN_FAN_OUT: usize = 5;

/// Which evidence a run uses; compared in `docs/go-package-focus.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceMode {
    /// Package size only; always informational.
    SizeOnly,
    /// Reference-graph components with no package-size floor and one type per component.
    GraphOnly,
    /// Reference graph plus the size and per-component floors (the registered default).
    Combined,
}

pub struct GoPackageFocusChecker {
    mode: EvidenceMode,
}

impl GoPackageFocusChecker {
    pub fn new(mode: EvidenceMode) -> Self {
        Self { mode }
    }
}

impl Default for GoPackageFocusChecker {
    fn default() -> Self {
        Self::new(EvidenceMode::Combined)
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Advisory,
    Informational,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Coverage {
    pub private_symbols_skipped: usize,
    pub generated_files_skipped: usize,
    pub files_with_parse_errors: usize,
    pub test_import_edges_excluded: usize,
    pub unresolved_edges: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Component {
    pub label: String,
    pub types: Vec<String>,
    pub symbols: Vec<String>,
    pub state_fields: Vec<String>,
    pub external_calls: Vec<String>,
}

/// The JSON-serializable form of one finding; documented in `docs/go-package-focus.md`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PackageFocusReport {
    pub package: String,
    pub level: Level,
    pub hypothesis: String,
    pub components: Vec<Component>,
    pub cross_boundary_edges: Vec<String>,
    pub import_direction: String,
    pub required_new_exports: Vec<String>,
    pub cycle_risks: Vec<String>,
    pub contrary_evidence: Vec<String>,
    pub coverage: Coverage,
    #[serde(skip)]
    anchor: Option<(std::path::PathBuf, usize)>,
}

impl ArchModelChecker for GoPackageFocusChecker {
    fn name(&self) -> &str {
        "go-package-focus"
    }

    /// Clustering needs unexported types and functions; pruned, they would look unconnected.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        analyze(model, self.mode)
            .into_iter()
            .map(|report| ArchFinding {
                file: report.anchor.as_ref().map(|(f, _)| f.clone()),
                line: report.anchor.as_ref().map(|(_, l)| *l),
                message: render::render(&report),
                severity_override: Some(Severity::Advisory),
            })
            .collect()
    }
}

/// One report per qualifying package, ordered by package path.
pub fn analyze(model: &ArchModel, mode: EvidenceMode) -> Vec<PackageFocusReport> {
    model
        .packages
        .values()
        .filter_map(|pkg| analyze_package(model, pkg, mode))
        .collect()
}

fn go_symbols<'a>(model: &ArchModel, pkg: &'a PackageNode) -> Vec<&'a SymbolNode> {
    pkg.symbols
        .iter()
        .filter(|s| {
            s.file.extension().is_some_and(|e| e == "go")
                && s.name != "init"
                && !is_test_file(s.file.strip_prefix(&model.repo_root).unwrap_or(&s.file))
        })
        .collect()
}

fn analyze_package(
    model: &ArchModel,
    pkg: &PackageNode,
    mode: EvidenceMode,
) -> Option<PackageFocusReport> {
    let symbols = go_symbols(model, pkg);
    if symbols.is_empty() {
        return None;
    }
    let source = PackageSource::read(pkg);
    let fan_out = distinct_imports(model, &pkg.path);
    let level = if is_entry_point(pkg, &symbols, fan_out) {
        Level::Informational
    } else {
        Level::Advisory
    };

    if mode == EvidenceMode::SizeOnly {
        return size_only_report(model, pkg, &symbols, &source);
    }

    let mut graph = TypeGraph::build(model, pkg, &symbols);
    if graph.type_count() > MAX_TYPE_NODES {
        return Some(too_large_report(model, pkg, &graph, &source));
    }
    let (min_types, min_total) = match mode {
        EvidenceMode::GraphOnly => (1, 2),
        _ => (MIN_TYPES_PER_COMPONENT, MIN_TYPE_NODES),
    };
    if graph.type_count() < min_total {
        return None;
    }
    let groups = graph.components(min_types);
    if groups.len() < 2 {
        return None;
    }
    // Shared package-level state overrides the graph: splitting would expose it.
    if source.shared_state {
        return None;
    }
    Some(split_report(model, pkg, &graph, groups, &source, level))
}

// ---------------------------------------------------------------------------------
// Reference graph
// ---------------------------------------------------------------------------------

struct TypeGraph<'a> {
    nodes: Vec<Node<'a>>,
    parent: Vec<usize>,
}

struct Node<'a> {
    name: String,
    is_type: bool,
    symbols: Vec<&'a SymbolNode>,
}

impl<'a> TypeGraph<'a> {
    fn build(model: &ArchModel, pkg: &'a PackageNode, symbols: &[&'a SymbolNode]) -> Self {
        let mut nodes: Vec<Node<'a>> = Vec::new();
        let mut by_type: HashMap<String, usize> = HashMap::new();
        let mut by_id: HashMap<&str, usize> = HashMap::new();

        for sym in symbols {
            if matches!(sym.kind, SymbolKind::Type | SymbolKind::Interface) {
                let idx = *by_type.entry(sym.name.clone()).or_insert_with(|| {
                    nodes.push(Node {
                        name: sym.name.clone(),
                        is_type: true,
                        symbols: vec![],
                    });
                    nodes.len() - 1
                });
                nodes[idx].symbols.push(sym);
                by_id.insert(&sym.id, idx);
            }
        }
        for sym in symbols {
            match sym.kind {
                SymbolKind::Method => {
                    let Some(owner) = sym.parent.as_deref() else {
                        continue;
                    };
                    let idx = *by_type.entry(owner.to_string()).or_insert_with(|| {
                        nodes.push(Node {
                            name: owner.to_string(),
                            is_type: true,
                            symbols: vec![],
                        });
                        nodes.len() - 1
                    });
                    nodes[idx].symbols.push(sym);
                    by_id.insert(&sym.id, idx);
                }
                SymbolKind::Function => {
                    nodes.push(Node {
                        name: sym.name.clone(),
                        is_type: false,
                        symbols: vec![sym],
                    });
                    by_id.insert(&sym.id, nodes.len() - 1);
                }
                _ => {}
            }
        }

        let mut graph = TypeGraph {
            parent: (0..nodes.len()).collect(),
            nodes,
        };
        let prefix = symbol_prefix(pkg);
        let owner_node = |id: &str| -> Option<usize> {
            let rest = id.strip_prefix(&prefix)?;
            by_type.get(rest.split('.').next()?).copied()
        };

        for edge in model.call_edges.iter().filter(|e| e.resolved) {
            if let (Some(&a), Some(&b)) =
                (by_id.get(edge.from.as_str()), by_id.get(edge.to.as_str()))
            {
                graph.join(a, b);
            }
        }
        for access in &model.field_accesses {
            if let (Some(&a), Some(b)) = (by_id.get(access.from.as_str()), owner_node(&access.to)) {
                graph.join(a, b);
            }
        }
        for edge in model.type_edges.iter().filter(|e| e.resolved) {
            if let (Some(a), Some(b)) = (owner_node(&edge.from), owner_node(&edge.to)) {
                graph.join(a, b);
            }
        }
        for sym in symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
        {
            let Some(&a) = by_id.get(sym.id.as_str()) else {
                continue;
            };
            for (_, type_text) in &sym.params {
                for ident in unqualified_idents(type_text) {
                    if let Some(&b) = by_type.get(ident) {
                        graph.join(a, b);
                    }
                }
            }
        }
        graph
    }

    fn join(&mut self, a: usize, b: usize) {
        crate::union_find::union(&mut self.parent, a, b);
    }

    fn type_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_type).count()
    }

    /// Components holding at least `min_types` type nodes, ordered by their first type name.
    fn components(&mut self, min_types: usize) -> Vec<Vec<usize>> {
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for idx in 0..self.nodes.len() {
            let root = crate::union_find::root(&mut self.parent, idx);
            groups.entry(root).or_default().push(idx);
        }
        let mut qualified: Vec<Vec<usize>> = groups
            .into_values()
            .filter(|g| g.iter().filter(|&&i| self.nodes[i].is_type).count() >= min_types)
            .collect();
        qualified.sort_by_key(|g| self.first_type_name(g));
        qualified
    }

    fn first_type_name(&self, group: &[usize]) -> String {
        group
            .iter()
            .filter(|&&i| self.nodes[i].is_type)
            .map(|&i| self.nodes[i].name.clone())
            .min()
            .unwrap_or_default()
    }
}

/// Identifiers in a Go type expression that are not package-qualified (`io.Reader` → none).
fn unqualified_idents(type_text: &str) -> Vec<&str> {
    let bytes = type_text.as_bytes();
    let mut idents = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let qualified = (start > 0 && bytes[start - 1] == b'.') || bytes.get(i) == Some(&b'.');
            if !qualified {
                idents.push(&type_text[start..i]);
            }
        } else {
            i += 1;
        }
    }
    idents
}

// ---------------------------------------------------------------------------------
// Package source facts the model drops
// ---------------------------------------------------------------------------------

/// Symbol ids are `{package}::{name}`.
fn symbol_prefix(pkg: &PackageNode) -> String {
    format!("{}::", pkg.path)
}

fn distinct_imports(model: &ArchModel, pkg_path: &str) -> usize {
    model
        .import_edges
        .iter()
        .filter(|e| e.from == pkg_path)
        .map(|e| e.to.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

fn is_entry_point(pkg: &PackageNode, symbols: &[&SymbolNode], fan_out: usize) -> bool {
    let is_main = symbols
        .iter()
        .any(|s| s.kind == SymbolKind::Function && s.name == "main");
    let in_cmd = pkg.path.split('/').any(|seg| seg == "cmd");
    (is_main || in_cmd) && fan_out >= ENTRY_POINT_MIN_FAN_OUT
}

// ---------------------------------------------------------------------------------
// Report assembly
// ---------------------------------------------------------------------------------

fn coverage_for(model: &ArchModel, pkg: &PackageNode, source: &PackageSource) -> Coverage {
    let prefix = symbol_prefix(pkg);
    let unresolved_edges = model
        .call_edges
        .iter()
        .filter(|e| !e.resolved && e.from.starts_with(&prefix))
        .count();
    let test_import_edges_excluded = model
        .import_edges
        .iter()
        .filter(|e| e.from == pkg.path && e.file.to_string_lossy().ends_with("_test.go"))
        .count();
    let mut notes = Vec::new();
    if source.has_generics {
        notes.push("generics may inflate or hide reference edges".to_string());
    }
    if source.has_build_tags {
        notes.push("build-tagged files: only one configuration is modeled".to_string());
    }
    if !model.pruning.files_with_parse_errors.is_empty() {
        notes.push("files that failed to parse are not modeled".to_string());
    }
    if source.unreadable > 0 {
        notes.push(format!(
            "{} source files unreadable for shared-state scan",
            source.unreadable
        ));
    }
    notes.push("callers outside this repository are unknown".to_string());
    Coverage {
        private_symbols_skipped: model.pruning.private_symbols_skipped,
        generated_files_skipped: model.pruning.generated_files_skipped,
        files_with_parse_errors: model.pruning.files_with_parse_errors.len(),
        test_import_edges_excluded,
        unresolved_edges,
        notes,
    }
}

fn split_report(
    model: &ArchModel,
    pkg: &PackageNode,
    graph: &TypeGraph,
    groups: Vec<Vec<usize>>,
    source: &PackageSource,
    level: Level,
) -> PackageFocusReport {
    let components: Vec<Component> = groups
        .iter()
        .map(|g| component_for(model, pkg, graph, g))
        .collect();
    let labels: Vec<&str> = components.iter().map(|c| c.label.as_str()).collect();
    let coverage = coverage_for(model, pkg, source);

    let mut contrary = vec![
        "no resolved call, field-access, embedding or parameter-type edge links the components, \
         but return types, interface satisfaction and dynamic calls are not tracked"
            .to_string(),
    ];
    if coverage.unresolved_edges > 0 {
        contrary.push(format!(
            "{} unresolved call edges from this package may connect the components",
            coverage.unresolved_edges
        ));
    }
    if level == Level::Informational {
        contrary.push(
            "package looks like wiring/entry-point code; wide fan-out is expected here".to_string(),
        );
    }
    contrary.push(
        "alternatives: keep together, reorganize files, or refactor types in place".to_string(),
    );

    let test_note = if coverage.test_import_edges_excluded > 0 {
        vec![format!(
            "{} import edges from _test.go files ignored; test-only cycles are not reported",
            coverage.test_import_edges_excluded
        )]
    } else {
        vec![]
    };

    PackageFocusReport {
        package: pkg.path.clone(),
        level,
        hypothesis: format!("separate {}", labels.join(" from ")),
        anchor: first_anchor(&groups, graph),
        cross_boundary_edges: vec![],
        import_direction: "none required: no resolved reference crosses the component boundary"
            .to_string(),
        required_new_exports: vec![],
        cycle_risks: test_note,
        contrary_evidence: contrary,
        coverage,
        components,
    }
}

fn component_for(
    model: &ArchModel,
    pkg: &PackageNode,
    graph: &TypeGraph,
    group: &[usize],
) -> Component {
    let mut types: Vec<String> = group
        .iter()
        .filter(|&&i| graph.nodes[i].is_type)
        .map(|&i| graph.nodes[i].name.clone())
        .collect();
    types.sort();
    let mut symbols: Vec<String> = group
        .iter()
        .flat_map(|&i| graph.nodes[i].symbols.iter().map(|s| s.id.clone()))
        .collect();
    symbols.sort();

    let prefix = symbol_prefix(pkg);
    let state_fields: BTreeSet<String> = model
        .field_accesses
        .iter()
        .filter(|a| symbols.binary_search(&a.from).is_ok())
        .map(|a| a.to.strip_prefix(&prefix).unwrap_or(&a.to).to_string())
        .collect();
    let external_calls: BTreeSet<String> = model
        .call_edges
        .iter()
        .filter(|e| !e.resolved && symbols.binary_search(&e.from).is_ok())
        .map(|e| e.to.clone())
        .collect();

    Component {
        label: label_for(&types),
        types,
        symbols,
        state_fields: state_fields.into_iter().collect(),
        external_calls: external_calls.into_iter().collect(),
    }
}

/// Longest common type-name prefix of at least 3 characters, else the first type name.
fn label_for(types: &[String]) -> String {
    let first = types.first().cloned().unwrap_or_default();
    let prefix_len = types.iter().skip(1).fold(first.len(), |len, t| {
        first
            .chars()
            .zip(t.chars())
            .take(len)
            .take_while(|(a, b)| a == b)
            .count()
    });
    if prefix_len >= 3 {
        first[..prefix_len].to_string()
    } else {
        first
    }
}

fn first_anchor(groups: &[Vec<usize>], graph: &TypeGraph) -> Option<(std::path::PathBuf, usize)> {
    groups
        .iter()
        .flatten()
        .flat_map(|&i| graph.nodes[i].symbols.iter())
        .map(|s| (s.file.clone(), s.line))
        .min()
}

fn size_only_report(
    model: &ArchModel,
    pkg: &PackageNode,
    symbols: &[&SymbolNode],
    source: &PackageSource,
) -> Option<PackageFocusReport> {
    if pkg.files.len() < SIZE_ONLY_MIN_FILES && symbols.len() < SIZE_ONLY_MIN_SYMBOLS {
        return None;
    }
    Some(PackageFocusReport {
        package: pkg.path.clone(),
        level: Level::Informational,
        hypothesis: format!(
            "large package ({} files, {} symbols); size alone is not evidence of lost focus",
            pkg.files.len(),
            symbols.len()
        ),
        anchor: symbols.iter().map(|s| (s.file.clone(), s.line)).min(),
        components: vec![],
        cross_boundary_edges: vec![],
        import_direction: "not evaluated".to_string(),
        required_new_exports: vec![],
        cycle_risks: vec![],
        contrary_evidence: vec![
            "size-only mode ignores the reference graph; a large cohesive package is fine"
                .to_string(),
        ],
        coverage: coverage_for(model, pkg, source),
    })
}

fn too_large_report(
    model: &ArchModel,
    pkg: &PackageNode,
    graph: &TypeGraph,
    source: &PackageSource,
) -> PackageFocusReport {
    PackageFocusReport {
        package: pkg.path.clone(),
        level: Level::Informational,
        hypothesis: format!(
            "package too large to cluster ({} type nodes > {MAX_TYPE_NODES})",
            graph.type_count()
        ),
        anchor: pkg.files.first().map(|f| (f.clone(), 1)),
        components: vec![],
        cross_boundary_edges: vec![],
        import_direction: "not evaluated".to_string(),
        required_new_exports: vec![],
        cycle_risks: vec![],
        contrary_evidence: vec![
            "clustering skipped, so no evidence for or against a split".to_string(),
        ],
        coverage: coverage_for(model, pkg, source),
    }
}

mod render;
mod source;
use source::PackageSource;
#[cfg(test)]
mod tests;
