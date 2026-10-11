//! Fowler's **Inline Function**: a function/method whose entire body is one delegating
//! statement (a bare call, or `return <call>`) and which has exactly one resolved caller
//! in the whole repo (issue #48). The AST-shape judgment (`is_pure_delegation`) happens
//! upstream in `symbol_extract.rs`, not here — `ArchModel` drops source/AST after the
//! initial scan, so a whole-repo checker can't re-derive it at check time.
//!
//! Scope mirrors `unreferenced_symbols`'s precedent for the same `call_edges`-derived
//! signal: exported symbols and zero-caller symbols (dead code, a different smell) are
//! never candidates, and only `symbol_extract::call_graph_supports` languages apply.
//! Interface-method carve-out: a method whose owning type has a resolved `Implements`
//! edge in `ArchModel.type_edges` is skipped when any interface it implements has
//! another implementer (a second caller may appear through that interface) — but still
//! flagged when the type is the sole implementer of every interface it declares.
//! Only explicit `implements` syntax is covered; Go structural interface satisfaction
//! (method-set matching) is not in `type_edges` v1 (`arch_model.rs:226-230`), so a Go
//! method that implicitly satisfies an interface can still be a false positive.

use std::collections::{HashMap, HashSet};

use crate::arch_model::{ArchModel, SymbolKind, SymbolNode, TypeRelationKind};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;

pub struct SingleCallSiteDelegationChecker;

impl ArchModelChecker for SingleCallSiteDelegationChecker {
    fn name(&self) -> &str {
        "single-call-site-delegation"
    }

    /// Every candidate is unexported by definition (`is_candidate`) — without this,
    /// `build_arch_model_for_check` would prune them out of `model.packages` before
    /// `check` ever sees them, same requirement `UnreferencedPrivateSymbolChecker`
    /// documents for the identical reason.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        single_call_site_delegation_findings(model)
    }
}

/// Number of distinct resolved callers per callee `SymbolNode::id` — a callee called
/// twice from the same caller still has exactly one *caller*, so this counts distinct
/// `from` values, not edge count.
fn caller_counts(model: &ArchModel) -> HashMap<&str, usize> {
    let mut callers_by_target: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in model.call_edges.iter().filter(|e| e.resolved) {
        callers_by_target
            .entry(edge.to.as_str())
            .or_default()
            .insert(edge.from.as_str());
    }
    callers_by_target
        .into_iter()
        .map(|(target, callers)| (target, callers.len()))
        .collect()
}

/// Resolved `Implements` edges, deduplicated by (type, interface) pair.
struct ImplementsIndex<'a> {
    interfaces_of: HashMap<&'a str, Vec<&'a str>>,
    implementer_counts: HashMap<&'a str, usize>,
}

fn implements_index(model: &ArchModel) -> ImplementsIndex<'_> {
    let pairs: HashSet<(&str, &str)> = model
        .type_edges
        .iter()
        .filter(|e| e.resolved && e.kind == Some(TypeRelationKind::Implements))
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    let mut interfaces_of: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut implementer_counts: HashMap<&str, usize> = HashMap::new();
    for (from, to) in pairs {
        interfaces_of.entry(from).or_default().push(to);
        *implementer_counts.entry(to).or_default() += 1;
    }
    ImplementsIndex {
        interfaces_of,
        implementer_counts,
    }
}

/// True when `symbol`'s owning type implements an interface that has another
/// implementer. A type that is the sole implementer of everything it implements is not
/// suppressed — the interface is effectively private to it.
fn implements_shared_interface(symbol: &SymbolNode, index: &ImplementsIndex<'_>) -> bool {
    // Method ids are `{pkg}::{type}.{name}`, the same shape `type_edges.from` uses for the type.
    let Some(type_id) = symbol
        .id
        .strip_suffix(symbol.name.as_str())
        .and_then(|rest| rest.strip_suffix('.'))
    else {
        return false;
    };
    index.interfaces_of.get(type_id).is_some_and(|interfaces| {
        interfaces
            .iter()
            .any(|iface| index.implementer_counts.get(iface).is_some_and(|&n| n > 1))
    })
}

fn is_candidate(symbol: &SymbolNode) -> bool {
    !symbol.exported
        && matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
        && symbol.is_pure_delegation
}

fn finding_for(symbol: &SymbolNode) -> ArchFinding {
    let kind = symbol.kind.function_or_method_word();
    ArchFinding {
        file: Some(symbol.file.clone()),
        line: Some(symbol.line),
        message: format!(
            "[single-call-site-delegation] {kind} `{}` does nothing but delegate to \
             another call and has exactly one caller in the repo — consider inlining it \
             at that call site (Fowler, Inline Function)",
            symbol.name
        ),
        severity_override: None,
    }
}

fn single_call_site_delegation_findings(model: &ArchModel) -> Vec<ArchFinding> {
    let counts = caller_counts(model);
    let implements = implements_index(model);
    let mut findings: Vec<ArchFinding> = model
        .packages
        .values()
        .flat_map(|pkg| pkg.symbols.iter())
        .filter(|symbol| is_candidate(symbol))
        .filter(|symbol| !implements_shared_interface(symbol, &implements))
        .filter(|symbol| counts.get(symbol.id.as_str()).copied().unwrap_or(0) == 1)
        .map(finding_for)
        .collect();
    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    findings
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

    #[test]
    fn flags_a_bare_call_delegating_function_with_one_caller() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc thin() {\n\thelper()\n}\n\nfunc helper() {}\n\nfunc Live() {\n\tthin()\n}\n",
        )]);
        let findings = single_call_site_delegation_findings(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("`thin`"));
    }

    #[test]
    fn flags_a_return_call_delegating_function_with_one_caller() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc thin(x int) int {\n\treturn helper(x)\n}\n\n\
             func helper(x int) int { return x }\n\nfunc Live() {\n\tthin(1)\n}\n",
        )]);
        let findings = single_call_site_delegation_findings(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("`thin`"));
    }

    #[test]
    fn does_not_flag_a_delegating_function_with_two_callers() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc thin() {\n\thelper()\n}\n\nfunc helper() {}\n\n\
             func Live() {\n\tthin()\n}\n\nfunc Live2() {\n\tthin()\n}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "thin() has two callers and must not be flagged"
        );
    }

    #[test]
    fn does_not_flag_a_delegating_function_with_zero_callers() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc thin() {\n\thelper()\n}\n\nfunc helper() {}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "thin() has no callers at all — that's dead code, not a delegation smell"
        );
    }

    #[test]
    fn does_not_flag_an_exported_delegating_function() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc Thin() {\n\thelper()\n}\n\nfunc helper() {}\n\n\
             func Live() {\n\tThin()\n}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "exported symbols may have callers outside this repo"
        );
    }

    #[test]
    fn does_not_flag_a_function_with_a_multi_statement_body() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc notThin() {\n\tlogSomething()\n\thelper()\n}\n\n\
             func logSomething() {}\nfunc helper() {}\n\nfunc Live() {\n\tnotThin()\n}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "a two-statement body isn't pure delegation"
        );
    }

    #[test]
    fn flags_a_typescript_function_delegating_to_a_call_with_one_caller() {
        let model = model_from(vec![(
            "/repo/src/a.ts",
            "function helper(x: number) { return x; }\n\n\
             function thin(x: number) { return helper(x); }\n\n\
             function live() { thin(1); }\n",
        )]);
        let findings = single_call_site_delegation_findings(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("`thin`"));
    }

    #[test]
    fn flags_a_go_method_delegating_to_a_call_with_one_caller() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\ntype T struct{}\n\nfunc (t T) thin() {\n\thelper()\n}\n\n\
             func helper() {}\n\nfunc Live() {\n\tvar t T\n\tt.thin()\n}\n",
        )]);
        let findings = single_call_site_delegation_findings(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("method `thin`"));
    }

    #[test]
    fn does_not_flag_a_function_with_an_empty_body() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc thin() {}\n\nfunc Live() {\n\tthin()\n}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "an empty body has no delegating statement to flag"
        );
    }

    #[test]
    fn does_not_flag_an_unsupported_language_even_with_one_caller() {
        // Rust isn't in `symbol_extract::call_graph_supports`, so `is_pure_delegation`
        // is never set `true` for it — this must be silent, not a false positive from
        // treating "no call graph" as "zero callers is fine to report anyway."
        let model = model_from(vec![(
            "/repo/src/a.rs",
            "fn helper(x: i32) -> i32 { x }\n\nfn thin(x: i32) -> i32 { helper(x) }\n\n\
             fn live() { thin(1); }\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "Rust has no call-graph coverage — is_pure_delegation must stay false"
        );
    }

    #[test]
    fn does_not_flag_ts_method_on_type_sharing_an_interface_with_another_implementer() {
        let model = model_from(vec![(
            "/repo/src/a.ts",
            "interface Shape { area(): number; }\n\
             class Sq implements Shape {\n  private thin() { return this.compute(); }\n  compute() { return 1; }\n  area() { return this.thin(); }\n}\n\
             class Circ implements Shape {\n  area() { return 2; }\n}\n",
        )]);
        let findings = single_call_site_delegation_findings(&model);
        assert!(
            !findings.iter().any(|f| f.message.contains("`thin`")),
            "Sq implements Shape alongside Circ: {findings:?}"
        );
    }

    #[test]
    fn flags_ts_method_on_sole_implementer_of_its_interface() {
        let model = model_from(vec![(
            "/repo/src/a.ts",
            "interface Shape { area(): number; }\n\
             class Sq implements Shape {\n  private thin() { return this.compute(); }\n  compute() { return 1; }\n  area() { return this.thin(); }\n}\n",
        )]);
        let findings = single_call_site_delegation_findings(&model);
        assert!(
            findings.iter().any(|f| f.message.contains("`thin`")),
            "sole implementer is not exempted from the finding: {findings:?}"
        );
    }

    #[test]
    fn go_method_without_implements_edges_is_still_flagged() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\ntype I interface{ thin() }\n\ntype T struct{}\n\n\
             func (t T) thin() {\n\thelper()\n}\n\nfunc helper() {}\n\n\
             func Live() {\n\tvar t T\n\tt.thin()\n}\n",
        )]);
        assert!(
            !model
                .type_edges
                .iter()
                .any(|e| e.kind == Some(TypeRelationKind::Implements)),
            "Go structural satisfaction produces no Implements edge"
        );
        let findings = single_call_site_delegation_findings(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("method `thin`"));
    }
}
