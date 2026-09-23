//! ISP fat-interface detection — issue #38's "expensive" tier item 2, buildable now that
//! #39's field-access data (and, for this checker, the syntactically-typed-local
//! machinery `god_class.rs` built for ATFD) exist. Interface Segregation says a fat
//! interface with many methods is a design smell when its consumers only ever need small,
//! non-overlapping subsets — clients get coupled to methods they never call.
//!
//! Design: for every interface across the whole model with enough declared methods, scan
//! every function/method body across every package for a syntactically-typed local
//! (parameter, or `var`/`:=`-declared local — reusing `god_class::collect_typed_locals`)
//! whose type resolves to that interface — same-package by bare name, or cross-package by
//! resolving a `pkg.Type`-qualified local through that file's `ArchModel::
//! file_import_aliases` entry (`god_class::resolve_qualified_type`) — and record which of
//! the interface's methods each such "consumer" call site actually invokes. An interface
//! where every observed consumer uses only a small slice of its full method set is the ISP
//! signal — no single consumer needs the whole interface.
//!
//! Go-only, declared-methods-only (embedded interfaces' promoted methods aren't walked
//! into — same "embedding introduces promoted members this v1 extraction doesn't follow"
//! ceiling `go_struct_fields` already documents for struct field embedding).

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::arch_model::{ArchModel, SymbolKind, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::god_class::{
    FileCache, collect_typed_locals, find_method_node, is_call_target, node_text,
    resolve_qualified_type,
};
use crate::node_kind::GoKind;

/// An interface needs at least this many declared methods before a "no consumer uses all
/// of it" observation is worth flagging — same "too small to matter" status as
/// `GOD_CLASS_MIN_METHODS`/`LCOM_MIN_METHODS`. A 2-method interface where one consumer
/// uses only 1 of them isn't a design smell, it's just how interfaces are used.
const ISP_MIN_INTERFACE_METHODS: usize = 5;

/// The largest fraction of an interface's methods any single observed consumer may use
/// before the interface no longer reads as "fat" — if some consumer already needs most of
/// it, splitting the interface wouldn't actually decouple that consumer from anything.
const ISP_MAX_USAGE_FRACTION: f64 = 0.5;

/// Minimum number of *distinct* consumers observed before "the heaviest one uses only M
/// of N methods" is worth reporting at all. Confirmed as a real gap backtesting against
/// `kubernetes/kubernetes` while this checker was still same-package-only: several
/// findings had exactly 1 observed consumer, which is not a pattern, it's a single data
/// point. Cross-package resolution (`ArchModel::file_import_aliases`) narrows the
/// remaining undercounting a lot, but doesn't eliminate it (a consumer that receives the
/// interface via a struct field, a slice, or a function return rather than a directly
/// syntactically-typed local is still invisible — see [`IspFatInterfaceChecker`]'s doc
/// comment), so this gate stays: requiring several observed consumers before flagging
/// stops the checker from asserting a design smell off evidence too thin to support it.
/// No stronger literature citation than "a sample of one proves nothing," same status as
/// `ISP_MIN_INTERFACE_METHODS`/`GOD_CLASS_MIN_METHODS`.
const ISP_MIN_CONSUMERS: usize = 3;

/// ISP fat-interface detector. See this module's doc comment for the full design and its
/// scope (Go-only, declared-methods-only).
///
/// **Known ceiling, not fixed for v1**: consumers are found via the same
/// syntactically-typed-local technique `god_class.rs` uses for ATFD — a parameter or
/// `var`/`:=`-declared local whose type is written out explicitly, resolved cross-package
/// via `ArchModel::file_import_aliases` when it's a `pkg.Type`-qualified name. A consumer
/// that receives the interface some other way (a struct field of interface type, a
/// slice/map of the interface, a value returned from a function and used inline without
/// ever being bound to a locally-typed variable, or a dot-import `file_import_aliases`
/// never populates an entry for) is still invisible to this detector. An interface with
/// zero observed consumers is skipped entirely (no evidence, not "zero usage" — the same
/// "unknown vs. zero" distinction `LcomChecker`'s zero-signal handling draws), so this
/// checker's silence is not proof an interface is well-used, only that it found nothing
/// to measure.
pub struct IspFatInterfaceChecker;

impl ArchModelChecker for IspFatInterfaceChecker {
    fn name(&self) -> &str {
        "isp-fat-interface"
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        isp_findings_for_model(model)
    }
}

/// One interface candidate, keyed by its `SymbolNode::id` (already `"{package}::{name}"`
/// — the same scheme `god_class::resolve_qualified_type` produces for a resolved
/// cross-package reference, so a consumer's resolved key and an interface's own id are
/// directly comparable without a separate lookup table).
struct InterfaceCandidate {
    pkg_path: String,
    name: String,
    methods: HashSet<String>,
}

fn isp_findings_for_model(model: &ArchModel) -> Vec<ArchFinding> {
    let mut files = FileCache::new();

    let interfaces = collect_interface_candidates(model, &mut files);
    if interfaces.is_empty() {
        return Vec::new();
    }
    let usage = collect_interface_usage(model, &mut files, &interfaces);
    build_findings(&interfaces, &usage)
}

/// Every interface across the whole model with at least `ISP_MIN_INTERFACE_METHODS`
/// directly-declared methods, keyed by `SymbolNode::id`.
fn collect_interface_candidates(
    model: &ArchModel,
    files: &mut FileCache,
) -> HashMap<String, InterfaceCandidate> {
    let mut interfaces = HashMap::new();
    for pkg in model.packages.values() {
        for sym in pkg
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
        {
            let Some(methods) = interface_declared_methods(sym, files) else {
                continue;
            };
            if methods.len() < ISP_MIN_INTERFACE_METHODS {
                continue;
            }
            interfaces.insert(
                sym.id.clone(),
                InterfaceCandidate {
                    pkg_path: pkg.path.clone(),
                    name: sym.name.clone(),
                    methods,
                },
            );
        }
    }
    interfaces
}

/// Interface id -> one entry per caller observed using it, each the count of that
/// interface's methods one caller invoked through a locally-typed variable. Scans every
/// function/method across every package (not just an interface's own) since a consumer
/// can now resolve cross-package.
fn collect_interface_usage<'a>(
    model: &ArchModel,
    files: &mut FileCache,
    interfaces: &'a HashMap<String, InterfaceCandidate>,
) -> HashMap<&'a str, Vec<usize>> {
    let mut usage: HashMap<&str, Vec<usize>> = HashMap::new();
    for pkg in model.packages.values() {
        for sym in &pkg.symbols {
            if sym.kind != SymbolKind::Function && sym.kind != SymbolKind::Method {
                continue;
            }
            let Some((source, tree)) = files.get(&sym.file) else {
                continue;
            };
            let Some(node) = find_method_node(tree.root_node(), sym.line, &sym.name, source) else {
                continue;
            };
            let mut typed_locals = HashMap::new();
            collect_typed_locals(node, source, &mut typed_locals);
            if typed_locals.is_empty() {
                continue;
            }
            let ctx = InterfaceUsageCtx {
                consumer_pkg: &pkg.path,
                file_aliases: model.file_import_aliases.get(&sym.file),
                typed_locals: &typed_locals,
                interfaces,
            };
            let mut used_by_interface: HashMap<&str, HashSet<String>> = HashMap::new();
            walk_calls_resolving_interfaces(node, source, &ctx, &mut used_by_interface);
            for (interface_id, methods) in used_by_interface {
                usage.entry(interface_id).or_default().push(methods.len());
            }
        }
    }
    usage
}

fn build_findings(
    interfaces: &HashMap<String, InterfaceCandidate>,
    usage: &HashMap<&str, Vec<usize>>,
) -> Vec<ArchFinding> {
    interfaces
        .iter()
        .filter_map(|(interface_id, candidate)| {
            let usages = usage.get(interface_id.as_str())?;
            if usages.len() < ISP_MIN_CONSUMERS {
                return None;
            }
            let max_used = *usages.iter().max()?;
            let fraction = max_used as f64 / candidate.methods.len() as f64;
            if fraction > ISP_MAX_USAGE_FRACTION {
                return None;
            }
            Some(ArchFinding {
                file: None,
                line: None,
                message: format!(
                    "[isp-fat-interface] {}::{} declares {} methods; of {} observed \
                     consumers, the heaviest used only {max_used} — likely means no \
                     consumer needs the whole interface, though a consumer reached other \
                     than through a directly-typed local is still invisible to this check \
                     (see the checker's own documented ceiling); worth a second look, not \
                     proof — consider splitting it by usage",
                    candidate.pkg_path,
                    candidate.name,
                    candidate.methods.len(),
                    usages.len()
                ),
                severity_override: None,
            })
        })
        .collect()
}

/// Resolves `local_type` (as stored by `god_class::collect_typed_locals` — a bare name or
/// a `pkg.Name` qualified one) to the `SymbolNode::id`-shaped key an interface candidate
/// is registered under: `"{consumer_pkg}::{local_type}"` for a bare name (the local's
/// declaring package is its own, same-package interface), or `god_class::
/// resolve_qualified_type`'s cross-package resolution for a qualified one.
fn resolve_interface_key(
    local_type: &str,
    consumer_pkg: &str,
    file_aliases: Option<&HashMap<String, String>>,
) -> Option<String> {
    if local_type.contains('.') {
        resolve_qualified_type(local_type, file_aliases)
    } else {
        Some(format!("{consumer_pkg}::{local_type}"))
    }
}

/// Threaded context for `walk_calls_resolving_interfaces`'s recursive walk — bundled
/// rather than passed as separate parameters, same rationale as `god_class`'s
/// `ForeignAccessCtx`. Two lifetimes, not one: `'s` covers the short-lived per-call
/// borrows (`typed_locals` is rebuilt fresh for every consumer method walked), while `'i`
/// covers `interfaces`, which outlives the whole scan and is what `out`'s keys borrow
/// from — collapsing both into one lifetime would force `typed_locals` to live as long as
/// `interfaces`, which it can't.
struct InterfaceUsageCtx<'s, 'i> {
    consumer_pkg: &'s str,
    file_aliases: Option<&'s HashMap<String, String>>,
    typed_locals: &'s HashMap<String, String>,
    interfaces: &'i HashMap<String, InterfaceCandidate>,
}

/// Single walk collecting, per interface actually called through a locally-typed
/// variable in `node`'s body, the set of method names invoked on it — one pass resolves
/// every local's type once rather than re-walking the whole body once per known
/// interface (which would be O(interfaces × consumer methods) across a whole-model scan).
/// Reuses the same call-target exclusion (`is_call_target`) `god_class`'s ATFD walker
/// uses, but the opposite way around: ATFD excludes calls to keep only field accesses,
/// this keeps only calls.
fn walk_calls_resolving_interfaces<'i>(
    node: Node,
    source: &str,
    ctx: &InterfaceUsageCtx<'_, 'i>,
    out: &mut HashMap<&'i str, HashSet<String>>,
) {
    if GoKind::of(node) == GoKind::SelectorExpression
        && is_call_target(node)
        && let Some(operand) = node.child_by_field_name("operand")
        && GoKind::of(operand) == GoKind::Identifier
        && let Some(local_type) = ctx.typed_locals.get(node_text(operand, source))
        && let Some(key) = resolve_interface_key(local_type, ctx.consumer_pkg, ctx.file_aliases)
        && let Some((interface_id, _)) = ctx.interfaces.get_key_value(&key)
        && let Some(field) = node.child_by_field_name("field")
    {
        out.entry(interface_id.as_str())
            .or_default()
            .insert(node_text(field, source).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls_resolving_interfaces(child, source, ctx, out);
    }
}

/// The method names an interface declares directly on its own `interface_type` node —
/// `method_elem` children only. `type_elem` children (an embedded interface, `io.Closer`)
/// are skipped: resolving an embedded interface's own promoted methods (possibly from
/// another package entirely) is real cross-type/cross-package work, deferred rather than
/// guessed at, same as struct field embedding in `go_struct_fields`.
fn interface_declared_methods(sym: &SymbolNode, files: &mut FileCache) -> Option<HashSet<String>> {
    let (source, tree) = files.get(&sym.file)?;
    let node = find_interface_node(tree.root_node(), sym.line, &sym.name, source)?;
    let interface_type = node.child_by_field_name("type")?;
    if GoKind::of(interface_type) != GoKind::InterfaceType {
        return None;
    }
    let mut methods = HashSet::new();
    let mut cursor = interface_type.walk();
    for child in interface_type
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::MethodElem)
    {
        if let Some(name) = child.child_by_field_name("name") {
            methods.insert(node_text(name, source).to_string());
        }
    }
    Some(methods)
}

/// Finds the `type_spec` node (not `type_declaration` — a grouped `type (...)` block can
/// hold several specs on different lines) whose name and 1-indexed start line match, then
/// returns the `type_spec` itself so the caller can read its `type` field.
fn find_interface_node<'a>(
    node: Node<'a>,
    line: usize,
    name: &str,
    source: &str,
) -> Option<Node<'a>> {
    if GoKind::of(node) == GoKind::TypeSpec
        && node.start_position().row + 1 == line
        && node
            .child_by_field_name("name")
            .is_some_and(|n| node_text(n, source) == name)
    {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| find_interface_node(child, line, name, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{PackageNode, PruningSummary};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn write_go_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// Wraps a single `PackageNode` in a minimal `ArchModel` (no cross-package import
    /// aliases) — the shape every pre-existing single-package test in this module needs
    /// now that `isp_findings_for_model` scans the whole model, not one package.
    fn model_of(pkg: PackageNode) -> ArchModel {
        model_of_packages(vec![pkg], BTreeMap::new())
    }

    fn model_of_packages(
        pkgs: Vec<PackageNode>,
        file_import_aliases: BTreeMap<PathBuf, HashMap<String, String>>,
    ) -> ArchModel {
        let mut packages = BTreeMap::new();
        for pkg in pkgs {
            packages.insert(pkg.path.clone(), pkg);
        }
        ArchModel {
            repo_root: PathBuf::new(),
            packages,
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![],
            file_import_aliases,
            pruning: PruningSummary::default(),
        }
    }

    fn interface_symbol(pkg: &str, name: &str, file: &Path, line: usize) -> SymbolNode {
        SymbolNode {
            id: format!("{pkg}::{name}"),
            name: name.to_string(),
            kind: SymbolKind::Interface,
            file: file.to_path_buf(),
            line,
            exported: true,
            parent: None,
        }
    }

    fn function_symbol(pkg: &str, name: &str, file: &Path, line: usize) -> SymbolNode {
        SymbolNode {
            id: format!("{pkg}::{name}"),
            name: name.to_string(),
            kind: SymbolKind::Function,
            file: file.to_path_buf(),
            line,
            exported: true,
            parent: None,
        }
    }

    fn package(path: &str, symbols: Vec<SymbolNode>, files: Vec<PathBuf>) -> PackageNode {
        PackageNode {
            path: path.to_string(),
            files,
            symbols,
        }
    }

    #[test]
    fn interface_declared_methods_collects_direct_methods_only() {
        let dir = crate::test_support::unique_temp_dir("isp-declared");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype Store interface {\n\tGet(k string) string\n\tSet(k, v string)\n\tDelete(k string)\n\tio.Closer\n}\n",
        );
        let sym = interface_symbol("pkg", "Store", &file, 3);
        let mut files = FileCache::new();
        let methods = interface_declared_methods(&sym, &mut files).unwrap();
        assert_eq!(
            methods,
            HashSet::from(["Get".to_string(), "Set".to_string(), "Delete".to_string()]),
            "embedded io.Closer's promoted method must not be counted"
        );
    }

    #[test]
    fn flags_a_fat_interface_with_no_consumer_using_more_than_half() {
        let dir = crate::test_support::unique_temp_dir("isp-fat");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\n\
             type Store interface {\n\tA()\n\tB()\n\tC()\n\tD()\n\tE()\n\tF()\n}\n\n\
             func UseAB(s Store) {\n\ts.A()\n\ts.B()\n}\n\n\
             func UseC(s Store) {\n\ts.C()\n}\n\n\
             func UseD(s Store) {\n\ts.D()\n}\n",
        );
        let pkg = package(
            "pkg",
            vec![
                interface_symbol("pkg", "Store", &file, 3),
                function_symbol("pkg", "UseAB", &file, 12),
                function_symbol("pkg", "UseC", &file, 17),
                function_symbol("pkg", "UseD", &file, 21),
            ],
            vec![file],
        );
        let findings = isp_findings_for_model(&model_of(pkg));
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("declares 6 methods"));
        assert!(findings[0].message.contains("heaviest used only 2"));
        assert!(findings[0].message.contains("3 observed consumers"));
    }

    #[test]
    fn does_not_flag_below_the_minimum_consumer_count() {
        // Same shape as the "flags" test above but with only 2 consumers, one below
        // ISP_MIN_CONSUMERS - a small same-package sample must not be reported as a
        // design smell, regardless of what fraction it happens to show.
        let dir = crate::test_support::unique_temp_dir("isp-too-few-consumers");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\n\
             type Store interface {\n\tA()\n\tB()\n\tC()\n\tD()\n\tE()\n\tF()\n}\n\n\
             func UseAB(s Store) {\n\ts.A()\n\ts.B()\n}\n\n\
             func UseC(s Store) {\n\ts.C()\n}\n",
        );
        let pkg = package(
            "pkg",
            vec![
                interface_symbol("pkg", "Store", &file, 3),
                function_symbol("pkg", "UseAB", &file, 12),
                function_symbol("pkg", "UseC", &file, 17),
            ],
            vec![file],
        );
        assert!(
            isp_findings_for_model(&model_of(pkg)).is_empty(),
            "2 observed consumers is below ISP_MIN_CONSUMERS"
        );
    }

    #[test]
    fn does_not_flag_when_a_consumer_uses_more_than_half() {
        let dir = crate::test_support::unique_temp_dir("isp-not-fat");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\n\
             type Store interface {\n\tA()\n\tB()\n\tC()\n\tD()\n\tE()\n\tF()\n}\n\n\
             func UseAlmostAll(s Store) {\n\ts.A()\n\ts.B()\n\ts.C()\n\ts.D()\n}\n\n\
             func UseA(s Store) {\n\ts.A()\n}\n\n\
             func UseB(s Store) {\n\ts.B()\n}\n",
        );
        let pkg = package(
            "pkg",
            vec![
                interface_symbol("pkg", "Store", &file, 3),
                function_symbol("pkg", "UseAlmostAll", &file, 12),
                function_symbol("pkg", "UseA", &file, 19),
                function_symbol("pkg", "UseB", &file, 23),
            ],
            vec![file],
        );
        assert!(
            isp_findings_for_model(&model_of(pkg)).is_empty(),
            "3 consumers clears ISP_MIN_CONSUMERS, but the heaviest uses 4/6 (over the \
             fraction threshold) - this must fail on the fraction check, not the count \
             gate, to actually exercise what this test is named for"
        );
    }

    #[test]
    fn does_not_flag_an_interface_with_no_observed_consumers() {
        let dir = crate::test_support::unique_temp_dir("isp-no-consumers");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype Store interface {\n\tA()\n\tB()\n\tC()\n\tD()\n\tE()\n\tF()\n}\n",
        );
        let pkg = package(
            "pkg",
            vec![interface_symbol("pkg", "Store", &file, 3)],
            vec![file],
        );
        assert!(
            isp_findings_for_model(&model_of(pkg)).is_empty(),
            "no evidence must not be treated as a violation"
        );
    }

    #[test]
    fn does_not_flag_a_small_interface_below_the_method_threshold() {
        let dir = crate::test_support::unique_temp_dir("isp-small");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype Pair interface {\n\tA()\n\tB()\n}\n\nfunc UseA(p Pair) {\n\tp.A()\n}\n",
        );
        let pkg = package(
            "pkg",
            vec![
                interface_symbol("pkg", "Pair", &file, 3),
                function_symbol("pkg", "UseA", &file, 8),
            ],
            vec![file],
        );
        assert!(isp_findings_for_model(&model_of(pkg)).is_empty());
    }

    #[test]
    fn no_interfaces_in_package_short_circuits_without_touching_disk() {
        let pkg = package(
            "pkg",
            vec![function_symbol(
                "pkg",
                "F",
                Path::new("/does/not/exist.go"),
                1,
            )],
            vec![],
        );
        assert!(isp_findings_for_model(&model_of(pkg)).is_empty());
    }

    #[test]
    fn resolves_a_cross_package_consumer_via_import_alias() {
        // Interface and consumer live in different packages; the consumer's file has no
        // explicit alias, so this also exercises the implicit-alias-via-declared-package-
        // name path. Three separate consumer functions, each reusing an explicit alias
        // set up per-file below, to clear ISP_MIN_CONSUMERS without needing three
        // separate packages.
        let iface_dir = crate::test_support::unique_temp_dir("isp-cross-pkg-iface");
        let iface_file = write_go_file(
            &iface_dir,
            "store.go",
            "package store\n\ntype Store interface {\n\tA()\n\tB()\n\tC()\n\tD()\n\tE()\n\tF()\n}\n",
        );
        let consumer_dir = crate::test_support::unique_temp_dir("isp-cross-pkg-consumer");
        let consumer_file = write_go_file(
            &consumer_dir,
            "use.go",
            "package consumer\n\n\
             func UseAB(s store.Store) {\n\ts.A()\n\ts.B()\n}\n\n\
             func UseC(s store.Store) {\n\ts.C()\n}\n\n\
             func UseD(s store.Store) {\n\ts.D()\n}\n",
        );

        let iface_pkg = package(
            "store",
            vec![interface_symbol("store", "Store", &iface_file, 3)],
            vec![iface_file],
        );
        let consumer_pkg = package(
            "consumer",
            vec![
                function_symbol("consumer", "UseAB", &consumer_file, 3),
                function_symbol("consumer", "UseC", &consumer_file, 8),
                function_symbol("consumer", "UseD", &consumer_file, 12),
            ],
            vec![consumer_file.clone()],
        );

        let mut aliases = BTreeMap::new();
        aliases.insert(
            consumer_file,
            HashMap::from([("store".to_string(), "store".to_string())]),
        );
        let model = model_of_packages(vec![iface_pkg, consumer_pkg], aliases);

        let findings = isp_findings_for_model(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("store::Store"));
        assert!(findings[0].message.contains("declares 6 methods"));
        assert!(findings[0].message.contains("heaviest used only 2"));
        assert!(findings[0].message.contains("3 observed consumers"));
    }

    #[test]
    fn does_not_resolve_a_cross_package_consumer_with_no_import_alias_entry() {
        // Same shape as the passing cross-package test, but with no
        // `file_import_aliases` entry for the consumer file at all — must not fabricate
        // a resolution from the raw "store.Store" text.
        let iface_dir = crate::test_support::unique_temp_dir("isp-cross-pkg-iface-noalias");
        let iface_file = write_go_file(
            &iface_dir,
            "store.go",
            "package store\n\ntype Store interface {\n\tA()\n\tB()\n\tC()\n\tD()\n\tE()\n\tF()\n}\n",
        );
        let consumer_dir = crate::test_support::unique_temp_dir("isp-cross-pkg-consumer-noalias");
        let consumer_file = write_go_file(
            &consumer_dir,
            "use.go",
            "package consumer\n\n\
             func UseAB(s store.Store) {\n\ts.A()\n\ts.B()\n}\n\n\
             func UseC(s store.Store) {\n\ts.C()\n}\n\n\
             func UseD(s store.Store) {\n\ts.D()\n}\n",
        );

        let iface_pkg = package(
            "store",
            vec![interface_symbol("store", "Store", &iface_file, 3)],
            vec![iface_file],
        );
        let consumer_pkg = package(
            "consumer",
            vec![
                function_symbol("consumer", "UseAB", &consumer_file, 3),
                function_symbol("consumer", "UseC", &consumer_file, 8),
                function_symbol("consumer", "UseD", &consumer_file, 12),
            ],
            vec![consumer_file],
        );

        let model = model_of_packages(vec![iface_pkg, consumer_pkg], BTreeMap::new());
        assert!(
            isp_findings_for_model(&model).is_empty(),
            "no alias info for the consumer file means no resolvable consumer at all"
        );
    }

    #[allow(dead_code)]
    fn _unused_pruning_summary_anchor() -> PruningSummary {
        PruningSummary::default()
    }
}
