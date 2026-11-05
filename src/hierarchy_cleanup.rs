//! Fowler's inheritance-chapter cleanups, over resolved `Extends` edges (Java, Kotlin, TS/JS):
//! `collapse-hierarchy`, `extract-superclass`, `remove-subclass`. Go embedding, `Implements`
//! and unresolved targets are ignored. See docs/hierarchy-cleanup.md for thresholds and backtest.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::arch_model::{ArchModel, SymbolKind, SymbolNode, TypeRelationKind};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::checker::Language;
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;
use crate::symbol_extract::type_extends_capable;

/// A subtype adding at most this many methods (overrides included) barely differs.
pub(crate) const COLLAPSE_MAX_SUBTYPE_METHODS: usize = 1;
pub(crate) const EXTRACT_MIN_SHARED_SIGNATURES: usize = 4;
pub(crate) const EXTRACT_MIN_JACCARD: f64 = 0.7;

/// Methods every class has; sharing them says nothing about a missing superclass.
const UNIVERSAL_METHODS: [&str; 5] = ["constructor", "toString", "equals", "hashCode", "init"];
/// Marker subtypes (exceptions) exist for their name, not their body.
const MARKER_SUFFIXES: [&str; 2] = ["Exception", "Error"];

struct Hierarchy<'a> {
    repo_root: &'a Path,
    /// type id -> (package path, symbol)
    types: HashMap<String, (&'a str, &'a SymbolNode)>,
    /// type id -> own methods
    methods: HashMap<String, Vec<&'a SymbolNode>>,
    /// subtype id -> supertype id (resolved `Extends` to an in-repo `Type`)
    parent: HashMap<&'a str, &'a str>,
    /// supertype id -> subtype ids
    children: HashMap<&'a str, Vec<&'a str>>,
    /// type id -> raw `Extends` target text (resolved or not); lets two types naming the same
    /// external or unresolved base count as related.
    extends_text: HashMap<&'a str, &'a str>,
    /// type id -> every `Implements` target (resolved or not)
    implements: HashMap<&'a str, HashSet<&'a str>>,
}

impl<'a> Hierarchy<'a> {
    fn new(model: &'a ArchModel) -> Self {
        // Types are keyed `{pkg}::{name}` and nested types are not qualified by their outer
        // class, so two `Builder`s collide and their methods merge under one key; drop such ids
        // entirely (only ids in `types` are ever used to look up methods).
        let mut types: HashMap<String, (&str, &SymbolNode)> = HashMap::new();
        let mut ambiguous: HashSet<&str> = HashSet::new();
        let mut methods: HashMap<String, Vec<&SymbolNode>> = HashMap::new();
        for pkg in model.packages.values() {
            for sym in &pkg.symbols {
                match (sym.kind, sym.parent.as_deref()) {
                    (SymbolKind::Type, _) => {
                        if types
                            .insert(sym.id.clone(), (pkg.path.as_str(), sym))
                            .is_some()
                        {
                            ambiguous.insert(sym.id.as_str());
                        }
                    }
                    (SymbolKind::Method, Some(parent)) if sym.name != "constructor" => {
                        methods
                            .entry(format!("{}::{parent}", pkg.path))
                            .or_default()
                            .push(sym);
                    }
                    _ => {}
                }
            }
        }
        types.retain(|id, _| !ambiguous.contains(id.as_str()));
        let mut parent = HashMap::new();
        let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut extends_text = HashMap::new();
        let mut implements: HashMap<&str, HashSet<&str>> = HashMap::new();
        for edge in &model.type_edges {
            if edge.kind == Some(TypeRelationKind::Extends) {
                extends_text.insert(edge.from.as_str(), edge.to.as_str());
            }
            match edge.kind {
                Some(TypeRelationKind::Extends)
                    if edge.resolved
                        && edge.from != edge.to
                        && types
                            .get(&edge.from)
                            .is_some_and(|t| Self::is_extends_capable(t.1))
                        && types.contains_key(&edge.to) =>
                {
                    parent.insert(edge.from.as_str(), edge.to.as_str());
                    children
                        .entry(edge.to.as_str())
                        .or_default()
                        .push(edge.from.as_str());
                }
                Some(TypeRelationKind::Implements) => {
                    implements
                        .entry(edge.from.as_str())
                        .or_default()
                        .insert(edge.to.as_str());
                }
                _ => {}
            }
        }
        Hierarchy {
            repo_root: &model.repo_root,
            types,
            methods,
            parent,
            children,
            extends_text,
            implements,
        }
    }

    fn is_test(&self, sym: &SymbolNode) -> bool {
        is_test_file(sym.file.strip_prefix(self.repo_root).unwrap_or(&sym.file))
    }

    fn own_methods(&self, type_id: &str) -> &[&'a SymbolNode] {
        self.methods.get(type_id).map_or(&[], Vec::as_slice)
    }

    fn ancestors(&self, type_id: &str) -> HashSet<&str> {
        let mut out = HashSet::new();
        let mut cur = type_id;
        while let Some(&p) = self.parent.get(cur) {
            if !out.insert(p) {
                break;
            }
            cur = p;
        }
        out
    }

    fn is_extends_capable(sym: &SymbolNode) -> bool {
        !sym.file.to_string_lossy().ends_with(".d.ts")
            && Language::for_path(&sym.file).is_some_and(type_extends_capable)
    }

    fn sorted_types(&self) -> Vec<(&String, &(&'a str, &'a SymbolNode))> {
        let mut all: Vec<_> = self.types.iter().collect();
        all.sort_by(|a, b| (&a.1.1.file, a.1.1.line, a.0).cmp(&(&b.1.1.file, b.1.1.line, b.0)));
        all
    }
}

#[derive(Default)]
struct Findings {
    collapse: Vec<ArchFinding>,
    remove: Vec<ArchFinding>,
    extract: Vec<ArchFinding>,
}

/// Identity of the model a cached result belongs to (address + cheap shape counts).
type ModelKey = (usize, usize, usize);
type CachedPass = (ModelKey, Arc<Findings>);

/// One `Hierarchy` pass shared by the three checkers: the first `check` computes all three
/// result sets, the others reuse them. Each registry call builds a fresh instance, so a cache
/// never outlives the model it was computed for in practice; the key guards reuse anyway.
#[derive(Clone, Default)]
struct SharedFindings(Arc<Mutex<Option<CachedPass>>>);

impl SharedFindings {
    fn get(&self, model: &ArchModel) -> Arc<Findings> {
        let key = (
            std::ptr::from_ref(model) as usize,
            model.type_edges.len(),
            model.packages.values().map(|p| p.symbols.len()).sum(),
        );
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((k, f)) = slot.as_ref()
            && *k == key
        {
            return Arc::clone(f);
        }
        let h = Hierarchy::new(model);
        let sorted = h.sorted_types();
        let findings = Arc::new(Findings {
            collapse: sorted
                .iter()
                .filter_map(|&(id, &(_, sym))| collapse_finding(&h, id, sym))
                .collect(),
            remove: sorted
                .iter()
                .filter_map(|&(id, &(_, sym))| remove_subclass_finding(&h, id, sym))
                .collect(),
            extract: extract_findings(&h),
        });
        *slot = Some((key, Arc::clone(&findings)));
        findings
    }
}

/// The three registered checkers, sharing one hierarchy pass per model.
pub fn checkers() -> Vec<Box<dyn ArchModelChecker>> {
    let shared = SharedFindings::default();
    vec![
        Box::new(CollapseHierarchyChecker {
            shared: shared.clone(),
        }),
        Box::new(ExtractSuperclassChecker {
            shared: shared.clone(),
        }),
        Box::new(RemoveSubclassChecker { shared }),
    ]
}

fn finding(sym: &SymbolNode, message: String) -> ArchFinding {
    ArchFinding {
        file: Some(sym.file.clone()),
        line: Some(sym.line),
        message,
        severity_override: None,
        operation: None,
    }
}

fn is_marker_name(name: &str) -> bool {
    MARKER_SUFFIXES.iter().any(|s| name.ends_with(s))
}

// --- collapse-hierarchy ---------------------------------------------------------

#[derive(Default)]
pub struct CollapseHierarchyChecker {
    shared: SharedFindings,
}

impl ArchModelChecker for CollapseHierarchyChecker {
    fn name(&self) -> &str {
        "collapse-hierarchy"
    }

    /// Package-private classes and private helper methods count toward "barely differ".
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        self.shared.get(model).collapse.clone()
    }
}

/// Guards shared by `collapse-hierarchy` and `remove-subclass`: the subtype exists for its
/// name, its interface, or a test, so folding it into the supertype is not the right advice.
fn subtype_is_exempt(h: &Hierarchy, sub_id: &str, sub: &SymbolNode, sup: &SymbolNode) -> bool {
    is_marker_name(&sub.name)
        || sub.is_abstract
        || h.implements.contains_key(sub_id)
        || h.is_test(sub)
        || h.is_test(sup)
}

fn collapse_finding(h: &Hierarchy, sub_id: &str, sub: &SymbolNode) -> Option<ArchFinding> {
    let sup_id = *h.parent.get(sub_id)?;
    let (_, sup) = h.types.get(sup_id)?;
    let only_child = h.children.get(sup_id).is_some_and(|c| c.len() == 1);
    let barely_differs = h.own_methods(sub_id).len() <= COLLAPSE_MAX_SUBTYPE_METHODS
        && !sub.declares_fields
        && !sup.is_abstract
        && !h.children.contains_key(sub_id);
    if !only_child || !barely_differs || subtype_is_exempt(h, sub_id, sub, sup) {
        return None;
    }
    let n = h.own_methods(sub_id).len();
    Some(finding(
        sub,
        format!(
            "[collapse-hierarchy] {sub_id} is the only subtype of {sup_id} and declares {n} \
             method(s) and no fields — merge it into the supertype (Fowler, Collapse Hierarchy)"
        ),
    ))
}

// --- remove-subclass ------------------------------------------------------------

#[derive(Default)]
pub struct RemoveSubclassChecker {
    shared: SharedFindings,
}

impl ArchModelChecker for RemoveSubclassChecker {
    fn name(&self) -> &str {
        "remove-subclass"
    }

    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        self.shared.get(model).remove.clone()
    }
}

fn remove_subclass_finding(h: &Hierarchy, sub_id: &str, sub: &SymbolNode) -> Option<ArchFinding> {
    let sup_id = *h.parent.get(sub_id)?;
    let (_, sup) = h.types.get(sup_id)?;
    let own = h.own_methods(sub_id);
    let sup_names: HashSet<&str> = h
        .own_methods(sup_id)
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    let is_leaf_field_free = !h.children.contains_key(sub_id) && !sub.declares_fields;
    let only_constant_overrides = !own.is_empty()
        && own.iter().all(|m| {
            m.returns_constant && !m.name.is_empty() && sup_names.contains(m.name.as_str())
        });
    if !is_leaf_field_free || !only_constant_overrides || subtype_is_exempt(h, sub_id, sub, sup) {
        return None;
    }
    let names: BTreeSet<&str> = own.iter().map(|m| m.name.as_str()).collect();
    let names = names.into_iter().collect::<Vec<_>>().join(", ");
    Some(finding(
        sub,
        format!(
            "[remove-subclass] {sub_id} only overrides {names} of {sup_id} to return constants \
             and declares no fields — replace it with a field on the supertype set by a \
             factory (Fowler, Remove Subclass)"
        ),
    ))
}

// --- extract-superclass ---------------------------------------------------------

#[derive(Default)]
pub struct ExtractSuperclassChecker {
    shared: SharedFindings,
}

impl ArchModelChecker for ExtractSuperclassChecker {
    fn name(&self) -> &str {
        "extract-superclass"
    }

    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        self.shared.get(model).extract.clone()
    }
}

fn extract_findings(h: &Hierarchy) -> Vec<ArchFinding> {
    let mut by_pkg: HashMap<&str, Vec<Candidate>> = HashMap::new();
    for (id, &(pkg, sym)) in h.sorted_types() {
        if !Hierarchy::is_extends_capable(sym) || h.is_test(sym) {
            continue;
        }
        let sigs = signatures(h.own_methods(id));
        if sigs.len() >= EXTRACT_MIN_SHARED_SIGNATURES {
            by_pkg.entry(pkg).or_default().push((id, sym, sigs));
        }
    }
    let mut findings: Vec<ArchFinding> = by_pkg
        .values()
        .flat_map(|types| clusters(h, types))
        .collect();
    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    findings
}

/// `(type id, symbol, method signatures)`
type Candidate<'a> = (&'a str, &'a SymbolNode, BTreeSet<String>);

/// `name(param types)`; Java/Kotlin carry no param info in the model, so overloads merge.
fn signatures(methods: &[&SymbolNode]) -> BTreeSet<String> {
    methods
        .iter()
        .filter(|m| !UNIVERSAL_METHODS.contains(&m.name.as_str()))
        .map(|m| {
            let types: Vec<&str> = m.params.iter().map(|(_, t)| t.as_str()).collect();
            format!("{}({})", m.name, types.join(","))
        })
        .collect()
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> (usize, f64) {
    let shared = a.intersection(b).count();
    let union = a.len() + b.len() - shared;
    (shared, shared as f64 / union as f64)
}

fn unrelated(h: &Hierarchy, a: &str, b: &str) -> bool {
    let (anc_a, anc_b) = (h.ancestors(a), h.ancestors(b));
    let shares_interface = match (h.implements.get(a), h.implements.get(b)) {
        (Some(x), Some(y)) => !x.is_disjoint(y),
        _ => false,
    };
    let same_base = match (h.extends_text.get(a), h.extends_text.get(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    };
    !same_base
        && !anc_a.contains(b)
        && !anc_b.contains(a)
        && anc_a.is_disjoint(&anc_b)
        && !shares_interface
}

/// Greedy groups in which every pair is similar and unrelated (pairwise, not transitive, so a
/// finding's "no hierarchy between them" holds for every member); one finding per group.
fn clusters(h: &Hierarchy, types: &[Candidate]) -> Vec<ArchFinding> {
    let linked = |i: usize, j: usize| {
        let (small, large) = (
            types[i].2.len().min(types[j].2.len()),
            types[i].2.len().max(types[j].2.len()),
        );
        // Jaccard <= min/max, so a lopsided pair can't reach the threshold.
        if (small as f64 / large as f64) < EXTRACT_MIN_JACCARD {
            return false;
        }
        let (shared, score) = jaccard(&types[i].2, &types[j].2);
        shared >= EXTRACT_MIN_SHARED_SIGNATURES
            && score >= EXTRACT_MIN_JACCARD
            && unrelated(h, types[i].0, types[j].0)
    };
    let mut assigned = vec![false; types.len()];
    let mut findings = Vec::new();
    for i in 0..types.len() {
        if assigned[i] {
            continue;
        }
        let mut group = vec![i];
        for (j, &taken) in assigned.iter().enumerate().skip(i + 1) {
            if !taken && group.iter().all(|&m| linked(m, j)) {
                group.push(j);
            }
        }
        if group.len() > 1 {
            group.iter().for_each(|&m| assigned[m] = true);
            findings.push(cluster_finding(types, &group));
        }
    }
    findings
}

fn cluster_finding(types: &[Candidate], members: &[usize]) -> ArchFinding {
    let first = types[members[0]].1;
    let names: Vec<&str> = members.iter().map(|&i| types[i].1.name.as_str()).collect();
    let common = members
        .iter()
        .skip(1)
        .fold(types[members[0]].2.clone(), |acc, &i| {
            acc.intersection(&types[i].2).cloned().collect()
        });
    finding(
        first,
        format!(
            "[extract-superclass] {} share a similar method structure ({} signatures common to \
             all, pairwise Jaccard >= {EXTRACT_MIN_JACCARD}) with no hierarchy between them — \
             extract a common superclass (Fowler, Extract Superclass)",
            names.join(", "),
            common.len()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::PruneConfig;
    use std::path::PathBuf;

    fn model_from(files: Vec<(&str, &str)>) -> ArchModel {
        let files: Vec<(PathBuf, String)> = files
            .into_iter()
            .map(|(path, src)| (PathBuf::from(path), src.to_string()))
            .collect();
        crate::arch_model::build_model(
            &PathBuf::from("/repo"),
            &files,
            &crate::import_graph::ImportGraph::default(),
            &PruneConfig {
                include_private: true,
            },
        )
        .unwrap()
    }

    fn run(checker: &dyn ArchModelChecker, files: Vec<(&str, &str)>) -> Vec<ArchFinding> {
        checker.check(&model_from(files), &ArchitectureConfig::default())
    }

    // --- collapse-hierarchy ---

    const BASE: &str = "class Base { int a() { return compute(); } void b() { } }\n";

    #[test]
    fn collapse_flags_lone_thin_subtype() {
        let src = format!("{BASE}class Sub extends Base {{ void extra() {{ }} }}\n");
        let found = run(
            &CollapseHierarchyChecker::default(),
            vec![("/repo/p/A.java", &src)],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("Sub"), "{}", found[0].message);
        assert!(found[0].message.contains("Collapse Hierarchy"));
    }

    #[test]
    fn collapse_ignores_two_subtypes() {
        let src = format!("{BASE}class S1 extends Base {{ }}\nclass S2 extends Base {{ }}\n");
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_subtype_with_many_methods() {
        let src = format!("{BASE}class Sub extends Base {{ void x() {{ }} void y() {{ }} }}\n");
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_subtype_with_fields() {
        let src = format!("{BASE}class Sub extends Base {{ int n; }}\n");
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_abstract_supertype() {
        let src =
            "abstract class Base { abstract void a(); }\nclass Sub extends Base { void a() { } }\n";
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_exception_marker_subtype() {
        let src = "class BaseException extends RuntimeException { void a() { } }\n\
                   class FooException extends BaseException { }\n";
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_subtype_that_implements_an_interface() {
        let src = format!("{BASE}interface I {{ }}\nclass Sub extends Base implements I {{ }}\n");
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_subtype_with_own_subtypes() {
        let src = format!(
            "{BASE}class Mid extends Base {{ }}\nclass Leaf extends Mid {{ void z() {{ }} void y() {{ }} }}\n"
        );
        // Mid is Base's only child but has a child itself; Leaf is Mid's only child but has 2 methods.
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_test_files() {
        let src = format!("{BASE}class Sub extends Base {{ }}\n");
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/ATest.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_unresolved_supertype_and_go_embedding() {
        let java = "class Sub extends External { }\n";
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", java)]
            )
            .is_empty()
        );
        let go = "package p\n\ntype Base struct{}\n\ntype Sub struct{ Base }\n";
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/a.go", go)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_works_for_typescript() {
        let src = "class Base { a() { return 1 + this.b() } b() { return 2 } }\nclass Sub extends Base { }\n";
        assert_eq!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/a.ts", src)]
            )
            .len(),
            1
        );
    }

    // --- remove-subclass ---

    const SHAPE: &str = "class Shape { String name() { return n(); } int sides() { return 0; } }\n";

    #[test]
    fn remove_subclass_flags_constant_overrides() {
        let src = format!(
            "{SHAPE}class Tri extends Shape {{ String name() {{ return \"tri\"; }} int sides() {{ return 3; }} }}\n"
        );
        let found = run(
            &RemoveSubclassChecker::default(),
            vec![("/repo/p/A.java", &src)],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("name, sides"),
            "{}",
            found[0].message
        );
    }

    #[test]
    fn remove_subclass_ignores_computed_override() {
        let src =
            format!("{SHAPE}class Tri extends Shape {{ int sides() {{ return count(); }} }}\n");
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn remove_subclass_ignores_mix_of_constant_and_computed() {
        let src = format!(
            "{SHAPE}class Tri extends Shape {{ int sides() {{ return 3; }} String name() {{ return n() + \"x\"; }} }}\n"
        );
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn remove_subclass_ignores_subclass_with_fields() {
        let src =
            format!("{SHAPE}class Tri extends Shape {{ int k; int sides() {{ return 3; }} }}\n");
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn remove_subclass_ignores_non_leaf() {
        let src = format!(
            "{SHAPE}class Tri extends Shape {{ int sides() {{ return 3; }} }}\nclass Eq extends Tri {{ }}\n"
        );
        let found = run(
            &RemoveSubclassChecker::default(),
            vec![("/repo/p/A.java", &src)],
        );
        assert!(
            found.iter().all(|f| !f.message.contains("p::Tri ")),
            "{found:?}"
        );
    }

    #[test]
    fn remove_subclass_ignores_constant_method_that_is_not_an_override() {
        let src = format!("{SHAPE}class Tri extends Shape {{ int extra() {{ return 3; }} }}\n");
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn remove_subclass_ignores_parameterised_constant_method() {
        let src =
            format!("{SHAPE}class Tri extends Shape {{ int sides(int x) {{ return 3; }} }}\n");
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn remove_subclass_works_for_typescript_and_kotlin() {
        let ts = "class Shape { sides() { return n() } }\nclass Tri extends Shape { sides() { return 3 } }\n";
        assert_eq!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/a.ts", ts)]
            )
            .len(),
            1
        );
        let kt = "open class Shape {\n  open fun sides(): Int { return n() }\n}\n\
                  class Tri : Shape() {\n  override fun sides() = 3\n}\n";
        assert_eq!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/a.kt", kt)]
            )
            .len(),
            1
        );
    }

    #[test]
    fn remove_subclass_ignores_computed_property_names() {
        let src = "class Base { get [Symbol.toStringTag]() { return n() } }\n\
                   class Sub extends Base { get [Symbol.toStringTag]() { return 'Sub' } }\n";
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/a.js", src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn declaration_files_are_ignored() {
        let src = "declare class Base { a(): void }\ndeclare class Sub extends Base { }\n";
        let model = model_from(vec![("/repo/p/a.d.ts", src)]);
        let names: Vec<&str> = model
            .packages
            .values()
            .flat_map(|p| &p.symbols)
            .filter(|s| s.kind == SymbolKind::Type)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            names.contains(&"Base") && names.contains(&"Sub"),
            "declare classes must reach the model for this test to mean anything: {names:?}"
        );
        assert!(
            model.type_edges.iter().any(|e| e.resolved),
            "{:?}",
            model.type_edges
        );
        let cfg = ArchitectureConfig::default();
        assert!(
            CollapseHierarchyChecker::default()
                .check(&model, &cfg)
                .is_empty()
        );
        // The same shape in a plain .ts file is flagged, so only the extension differs.
        let ts = "class Base { a() { return 1 + this.b() } b() { return 2 } }\nclass Sub extends Base { }\n";
        assert_eq!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/a.ts", ts)]
            )
            .len(),
            1
        );
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/a.d.ts", ts)]
            )
            .is_empty()
        );
        let two = format!("{}{}", ts_class("One", &SIX), ts_class("Two", &SIX));
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.d.ts", &two)]
            )
            .is_empty()
        );
        assert_eq!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &two)]
            )
            .len(),
            1
        );
    }

    #[test]
    fn remove_subclass_ignores_test_files() {
        let src = format!("{SHAPE}class Tri extends Shape {{ int sides() {{ return 3; }} }}\n");
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/TriTest.java", &src)]
            )
            .is_empty()
        );
    }

    // --- extract-superclass ---

    fn ts_class(name: &str, methods: &[&str]) -> String {
        let body: String = methods
            .iter()
            .map(|m| format!("{m}(x: string) {{ }}\n"))
            .collect();
        format!("class {name} {{\n{body}}}\n")
    }

    const SIX: [&str; 6] = ["a", "b", "c", "d", "e", "f"];

    #[test]
    fn extract_flags_similar_unrelated_siblings() {
        let src = format!("{}{}", ts_class("One", &SIX), ts_class("Two", &SIX));
        let found = run(
            &ExtractSuperclassChecker::default(),
            vec![("/repo/p/a.ts", &src)],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("One, Two"),
            "{}",
            found[0].message
        );
    }

    #[test]
    fn extract_clusters_three_siblings_into_one_finding() {
        let src = format!(
            "{}{}{}",
            ts_class("One", &SIX),
            ts_class("Two", &SIX),
            ts_class("Three", &SIX)
        );
        assert_eq!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .len(),
            1
        );
    }

    #[test]
    fn extract_ignores_dissimilar_siblings() {
        let src = format!(
            "{}{}",
            ts_class("One", &SIX),
            ts_class("Two", &["a", "b", "c", "d", "x", "y", "z"])
        );
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_ignores_too_few_shared_signatures() {
        let src = format!(
            "{}{}",
            ts_class("One", &SIX[..3]),
            ts_class("Two", &SIX[..3])
        );
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_ignores_types_with_hierarchy_edge() {
        let src = format!(
            "{}{}",
            ts_class("One", &SIX),
            ts_class("Two", &SIX).replace("class Two", "class Two extends One")
        );
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_ignores_siblings_sharing_a_parent() {
        let src = format!(
            "class P {{ }}\n{}{}",
            ts_class("One", &SIX).replace("class One", "class One extends P"),
            ts_class("Two", &SIX).replace("class Two", "class Two extends P")
        );
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_groups_are_pairwise_not_transitive() {
        let src = format!(
            "class P {{ }}\n{}{}{}",
            ts_class("One", &SIX).replace("class One", "class One extends P"),
            ts_class("Two", &SIX).replace("class Two", "class Two extends P"),
            ts_class("Three", &SIX)
        );
        let found = run(
            &ExtractSuperclassChecker::default(),
            vec![("/repo/p/a.ts", &src)],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("One, Three"),
            "{}",
            found[0].message
        );
        assert!(!found[0].message.contains("Two"), "{}", found[0].message);
    }

    #[test]
    fn extract_ignores_siblings_extending_the_same_external_base() {
        let src = format!(
            "{}{}",
            ts_class("One", &SIX).replace("class One", "class One extends External"),
            ts_class("Two", &SIX).replace("class Two", "class Two extends External")
        );
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_ignores_siblings_implementing_the_same_interface() {
        let src = format!(
            "interface I {{ }}\n{}{}",
            ts_class("One", &SIX).replace("class One", "class One implements I"),
            ts_class("Two", &SIX).replace("class Two", "class Two implements I")
        );
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_flags_siblings_implementing_different_interfaces() {
        let src = format!(
            "interface I {{ }}\ninterface J {{ }}\n{}{}",
            ts_class("One", &SIX).replace("class One", "class One implements I"),
            ts_class("Two", &SIX).replace("class Two", "class Two implements J")
        );
        assert_eq!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .len(),
            1
        );
    }

    #[test]
    fn extract_ignores_different_param_types() {
        let one = ts_class("One", &SIX);
        let two = ts_class("Two", &SIX).replace("x: string", "x: number");
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &format!("{one}{two}"))]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_ignores_universal_methods() {
        let m = ["toString", "equals", "hashCode", "init", "a", "b"];
        let src = format!("{}{}", ts_class("One", &m), ts_class("Two", &m));
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_ignores_go_and_test_files() {
        let go = "package p\n\ntype A struct{}\ntype B struct{}\n\
                  func (A) a() {}\nfunc (A) b() {}\nfunc (A) c() {}\nfunc (A) d() {}\n\
                  func (B) a() {}\nfunc (B) b() {}\nfunc (B) c() {}\nfunc (B) d() {}\n";
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.go", go)]
            )
            .is_empty()
        );
        let src = format!("{}{}", ts_class("One", &SIX), ts_class("Two", &SIX));
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.test.ts", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_ignores_similar_types_in_different_packages() {
        let (one, two) = (ts_class("One", &SIX), ts_class("Two", &SIX));
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &one), ("/repo/q/b.ts", &two)]
            )
            .is_empty()
        );
    }

    // --- review regressions ---

    #[test]
    fn same_named_nested_classes_do_not_merge_their_methods() {
        let src = "class A { static class Builder { void a() { } void b() { } } }\n\
                   class B { static class Builder { void c() { } void d() { } } }\n\
                   class Z { void a() { } void b() { } void c() { } void d() { } }\n";
        let found = run(
            &ExtractSuperclassChecker::default(),
            vec![("/repo/p/A.java", src)],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn ambiguous_type_ids_are_not_hierarchy_nodes() {
        let src = format!(
            "{BASE}class A {{ static class Sub extends Base {{ }} }}\nclass B {{ static class Sub {{ }} }}\n"
        );
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_and_remove_subclass_work_for_cjs() {
        let src = "class Base { a() { return 1 + this.b() } b() { return 2 } }\nclass Sub extends Base { }\n";
        assert_eq!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/a.cjs", src)]
            )
            .len(),
            1
        );
    }

    #[test]
    fn collapse_ignores_abstract_subtype() {
        let src = format!("{BASE}abstract class Sub extends Base {{ }}\n");
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_error_marker_subtype() {
        let src = "class BaseError { void a() { } }\nclass FooError extends BaseError { }\n";
        assert!(
            run(
                &CollapseHierarchyChecker::default(),
                vec![("/repo/p/A.java", src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn collapse_ignores_test_subtype_of_production_base() {
        let found = run(
            &CollapseHierarchyChecker::default(),
            vec![
                ("/repo/p/A.java", BASE),
                ("/repo/p/SubTest.java", "class Sub extends Base { }\n"),
            ],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn collapse_ignores_production_subtype_of_test_base() {
        let found = run(
            &CollapseHierarchyChecker::default(),
            vec![
                ("/repo/p/ATest.java", BASE),
                ("/repo/p/Sub.java", "class Sub extends Base { }\n"),
            ],
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn remove_subclass_ignores_abstract_subclass() {
        let src =
            format!("{SHAPE}abstract class Tri extends Shape {{ int sides() {{ return 3; }} }}\n");
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .is_empty()
        );
    }

    #[test]
    fn remove_subclass_shares_collapse_guards() {
        let iface = format!(
            "{SHAPE}interface I {{ }}\nclass Tri extends Shape implements I {{ int sides() {{ return 3; }} }}\n"
        );
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &iface)]
            )
            .is_empty()
        );
        let tri = "class Tri extends Shape { int sides() { return 3; } }\n";
        let found = run(
            &RemoveSubclassChecker::default(),
            vec![("/repo/p/ATest.java", SHAPE), ("/repo/p/Tri.java", tri)],
        );
        assert!(found.is_empty(), "test supertype: {found:?}");
        let marker = "class BaseError { int code() { return n(); } }\n\
                      class FooError extends BaseError { int code() { return 1; } }\n";
        assert!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", marker)]
            )
            .is_empty()
        );
    }

    #[test]
    fn remove_subclass_accepts_negative_constants() {
        let src = format!("{SHAPE}class Tri extends Shape {{ int sides() {{ return -1; }} }}\n");
        assert_eq!(
            run(
                &RemoveSubclassChecker::default(),
                vec![("/repo/p/A.java", &src)]
            )
            .len(),
            1
        );
    }

    #[test]
    fn remove_subclass_lists_override_names_sorted() {
        let src = format!(
            "{SHAPE}class Tri extends Shape {{ int sides() {{ return 3; }} String name() {{ return \"tri\"; }} }}\n"
        );
        let found = run(
            &RemoveSubclassChecker::default(),
            vec![("/repo/p/A.java", &src)],
        );
        assert!(
            found[0].message.contains("name, sides"),
            "{}",
            found[0].message
        );
    }

    #[test]
    fn extract_group_is_exactly_the_free_class_and_its_first_partner() {
        let src = format!(
            "{}class P {{ }}\n{}{}",
            ts_class("Three", &SIX),
            ts_class("One", &SIX).replace("class One", "class One extends P"),
            ts_class("Two", &SIX).replace("class Two", "class Two extends P"),
        );
        let found = run(
            &ExtractSuperclassChecker::default(),
            vec![("/repo/p/a.ts", &src)],
        );
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("Three, One"),
            "{}",
            found[0].message
        );
        assert!(!found[0].message.contains("Two"), "{}", found[0].message);
    }

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| format!("{s}(string)")).collect()
    }

    #[test]
    fn extract_jaccard_threshold_boundary() {
        let seven = ["a", "b", "c", "d", "e", "f", "g"];
        let ten = ["a", "b", "c", "d", "e", "f", "g", "x", "y", "z"];
        let (shared, score) = jaccard(&names(&seven), &names(&ten));
        assert_eq!(shared, 7);
        assert!(
            score >= EXTRACT_MIN_JACCARD && (score - 0.7).abs() < 1e-12,
            "{score}"
        );
        let at = format!("{}{}", ts_class("One", &seven), ts_class("Two", &ten));
        assert_eq!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &at)]
            )
            .len(),
            1
        );

        let a = ["a", "b", "c", "d", "e", "f", "g"];
        let b = ["a", "b", "c", "d", "e", "f", "x", "y"];
        let (shared, score) = jaccard(&names(&a), &names(&b));
        assert_eq!(shared, 6);
        assert!(score < EXTRACT_MIN_JACCARD, "{score}");
        let below = format!("{}{}", ts_class("One", &a), ts_class("Two", &b));
        assert!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &below)]
            )
            .is_empty()
        );
    }

    #[test]
    fn extract_flags_exactly_four_shared_signatures() {
        let src = format!(
            "{}{}",
            ts_class("One", &SIX[..4]),
            ts_class("Two", &SIX[..4])
        );
        assert_eq!(
            run(
                &ExtractSuperclassChecker::default(),
                vec![("/repo/p/a.ts", &src)]
            )
            .len(),
            1
        );
    }

    fn extract_with_heritage(one: &str, two: &str, decls: &str) -> Vec<ArchFinding> {
        let src = format!(
            "{decls}{}{}",
            ts_class("One", &SIX).replace("class One", &format!("class One{one}")),
            ts_class("Two", &SIX).replace("class Two", &format!("class Two{two}")),
        );
        run(
            &ExtractSuperclassChecker::default(),
            vec![("/repo/p/a.ts", &src)],
        )
    }

    #[test]
    fn extract_ignores_cousins_sharing_a_distant_ancestor() {
        let found = extract_with_heritage(
            " extends P",
            " extends Q",
            "class P { }\nclass Q extends P { }\n",
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn extract_flags_types_with_disjoint_ancestors() {
        let found = extract_with_heritage(" extends P", " extends Q", "class P { }\nclass Q { }\n");
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn extract_ignores_first_declared_type_extending_the_second() {
        let found = extract_with_heritage(" extends Two", "", "");
        assert!(found.is_empty(), "{found:?}");
        let found = extract_with_heritage("", " extends One", "");
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn extract_flags_siblings_extending_different_external_bases() {
        let found = extract_with_heritage(" extends ExtA", " extends ExtB", "");
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn registry_exposes_the_three_rule_names() {
        let names: Vec<String> = checkers().iter().map(|c| c.name().to_string()).collect();
        assert_eq!(
            names,
            [
                "collapse-hierarchy",
                "extract-superclass",
                "remove-subclass"
            ]
        );
    }

    #[test]
    fn shared_checkers_reuse_one_pass_per_model() {
        let src = format!("{BASE}class Sub extends Base {{ }}\n");
        let model = model_from(vec![("/repo/p/A.java", &src)]);
        let cfg = ArchitectureConfig::default();
        let all = checkers();
        let first = all[0].check(&model, &cfg);
        assert_eq!(first.len(), 1);
        assert_eq!(all[0].check(&model, &cfg), first);
        let other = model_from(vec![("/repo/p/A.java", BASE)]);
        assert!(all[0].check(&other, &cfg).is_empty());
    }
}
