//! ISP fat-interface detection — issue #38's "expensive" tier item 2, buildable now that
//! #39's field-access data (and, for this checker, the syntactically-typed-local
//! machinery `god_class.rs` built for ATFD) exist. Interface Segregation says a fat
//! interface with many methods is a design smell when its consumers only ever need small,
//! non-overlapping subsets — clients get coupled to methods they never call.
//!
//! Design: for each package-local interface with enough declared methods, find every
//! syntactically-typed local (parameter, or `var`/`:=`-declared local — reusing
//! `god_class::collect_typed_locals`) whose type resolves to that interface, and record
//! which of the interface's methods each such "consumer" call site actually invokes. An
//! interface where every observed consumer uses only a small slice of its full method set
//! is the ISP signal — no single consumer needs the whole interface.
//!
//! Go-only, same-package-only, declared-methods-only (embedded interfaces' promoted
//! methods aren't walked into — same "embedding introduces promoted members this v1
//! extraction doesn't follow" ceiling `go_struct_fields` already documents for struct
//! field embedding).

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::arch_model::{ArchModel, PackageNode, SymbolKind, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;
use crate::god_class::{
    FileCache, collect_typed_locals, find_method_node, is_call_target, node_text,
};

/// An interface needs at least this many declared methods before a "no consumer uses all
/// of it" observation is worth flagging — same "too small to matter" status as
/// `GOD_CLASS_MIN_METHODS`/`LCOM_MIN_METHODS`. A 2-method interface where one consumer
/// uses only 1 of them isn't a design smell, it's just how interfaces are used.
const ISP_MIN_INTERFACE_METHODS: usize = 5;

/// The largest fraction of an interface's methods any single observed consumer may use
/// before the interface no longer reads as "fat" — if some consumer already needs most of
/// it, splitting the interface wouldn't actually decouple that consumer from anything.
const ISP_MAX_USAGE_FRACTION: f64 = 0.5;

/// Minimum number of *distinct* same-package consumers observed before "the heaviest one
/// uses only M of N methods" is worth reporting at all. Confirmed as a real gap
/// backtesting against `kubernetes/kubernetes`: consumers here are same-package-only (see
/// this checker's doc comment), which undercounts a widely-shared, cross-package-exported
/// interface badly — `metav1.Object` showed only 5 same-package consumers even though
/// `grep -rl 'metav1\.Object\b'` across the whole repo finds 100 files referencing it, and
/// several other findings had exactly 1 observed consumer, which is not a pattern, it's a
/// single data point. Requiring several observed consumers before flagging doesn't fix the
/// same-package undercounting itself (that's the documented "known ceiling, not fixed for
/// v1" above), but it does stop the checker from asserting a design smell off evidence too
/// thin to support it. No stronger literature citation than "a sample of one proves
/// nothing," same status as `ISP_MIN_INTERFACE_METHODS`/`GOD_CLASS_MIN_METHODS`.
const ISP_MIN_CONSUMERS: usize = 3;

/// ISP fat-interface detector. See this module's doc comment for the full design and its
/// scope (Go-only, same-package-only, declared-methods-only).
///
/// **Known ceiling, not fixed for v1**: consumers are found via the same
/// syntactically-typed-local technique `god_class.rs` uses for ATFD — a parameter or
/// `var`/`:=`-declared local whose type is written out explicitly. A consumer that
/// receives the interface some other way (a struct field of interface type, a slice/map
/// of the interface, a value returned from a function and used inline without ever being
/// bound to a locally-typed variable) is invisible to this detector. An interface with
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
        model
            .packages
            .values()
            .flat_map(isp_findings_for_package)
            .collect()
    }
}

fn isp_findings_for_package(pkg: &PackageNode) -> Vec<ArchFinding> {
    let mut files = FileCache::new();
    let interfaces: HashMap<&str, &SymbolNode> = pkg
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Interface)
        .map(|s| (s.name.as_str(), s))
        .collect();
    if interfaces.is_empty() {
        return Vec::new();
    }

    let mut declared_methods: HashMap<&str, HashSet<String>> = HashMap::new();
    for (&name, sym) in &interfaces {
        if let Some(methods) = interface_declared_methods(sym, &mut files) {
            declared_methods.insert(name, methods);
        }
    }

    // interface_name -> one entry per caller observed using it, each the count of that
    // interface's methods that one caller invoked through a locally-typed variable.
    let mut usage: HashMap<&str, Vec<usize>> = HashMap::new();
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
        for interface_name in declared_methods.keys() {
            let used =
                methods_called_through_locals_of_type(node, source, interface_name, &typed_locals);
            if !used.is_empty() {
                usage.entry(interface_name).or_default().push(used.len());
            }
        }
    }

    declared_methods
        .into_iter()
        .filter_map(|(interface_name, methods)| {
            if methods.len() < ISP_MIN_INTERFACE_METHODS {
                return None;
            }
            let usages = usage.get(interface_name)?;
            if usages.len() < ISP_MIN_CONSUMERS {
                return None;
            }
            let max_used = *usages.iter().max()?;
            let fraction = max_used as f64 / methods.len() as f64;
            if fraction > ISP_MAX_USAGE_FRACTION {
                return None;
            }
            Some(ArchFinding {
                file: None,
                line: None,
                message: format!(
                    "[isp-fat-interface] {}::{interface_name} declares {} methods; of {} \
                     observed same-package consumers, the heaviest used only {max_used} — \
                     likely means no consumer needs the whole interface, though this only \
                     sees consumers in the same package (see the checker's own documented \
                     ceiling); worth a second look, not proof — consider splitting it by \
                     usage",
                    pkg.path,
                    methods.len(),
                    usages.len()
                ),
                severity_override: None,
            })
        })
        .collect()
}

/// Set of method names actually invoked, within `caller`'s body, on any local whose type
/// (per `typed_locals`) is `interface_name` — reuses the same call-target exclusion
/// (`is_call_target`) god_class's ATFD walker uses, but the opposite way around: ATFD
/// excludes calls to keep only field accesses, this keeps only calls.
fn methods_called_through_locals_of_type(
    caller: Node,
    source: &str,
    interface_name: &str,
    typed_locals: &HashMap<String, String>,
) -> HashSet<String> {
    let mut out = HashSet::new();
    walk_calls_through_locals(caller, source, interface_name, typed_locals, &mut out);
    out
}

fn walk_calls_through_locals(
    node: Node,
    source: &str,
    interface_name: &str,
    typed_locals: &HashMap<String, String>,
    out: &mut HashSet<String>,
) {
    if node.kind() == "selector_expression"
        && is_call_target(node)
        && let Some(operand) = node.child_by_field_name("operand")
        && operand.kind() == "identifier"
        && typed_locals
            .get(node_text(operand, source))
            .is_some_and(|t| t == interface_name)
        && let Some(field) = node.child_by_field_name("field")
    {
        out.insert(node_text(field, source).to_string());
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_calls_through_locals(child, source, interface_name, typed_locals, out);
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
    if interface_type.kind() != "interface_type" {
        return None;
    }
    let mut methods = HashSet::new();
    let mut cursor = interface_type.walk();
    for child in interface_type
        .children(&mut cursor)
        .filter(|c| c.kind() == "method_elem")
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
    if node.kind() == "type_spec"
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
    use crate::arch_model::PruningSummary;
    use std::path::{Path, PathBuf};

    fn write_go_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
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
        let findings = isp_findings_for_package(&pkg);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("declares 6 methods"));
        assert!(findings[0].message.contains("heaviest used only 2"));
        assert!(
            findings[0]
                .message
                .contains("3 observed same-package consumers")
        );
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
            isp_findings_for_package(&pkg).is_empty(),
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
            isp_findings_for_package(&pkg).is_empty(),
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
            isp_findings_for_package(&pkg).is_empty(),
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
        assert!(isp_findings_for_package(&pkg).is_empty());
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
        assert!(isp_findings_for_package(&pkg).is_empty());
    }

    #[allow(dead_code)]
    fn _unused_pruning_summary_anchor() -> PruningSummary {
        PruningSummary::default()
    }
}
