//! Fowler's **Inline Function**: a function/method whose entire body is one delegating
//! statement (a bare call, or `return <call>`) and which has exactly one resolved call site
//! in the whole repo (issue #48). The AST-shape judgment (`is_pure_delegation`) happens
//! upstream in `symbol_extract.rs`, not here — `ArchModel` drops source/AST after the
//! initial scan, so a whole-repo checker can't re-derive it at check time.
//!
//! Scope mirrors `unreferenced_symbols`'s precedent for the same `call_edges`-derived
//! signal: exported symbols and zero-caller symbols (dead code, a different smell) are
//! never candidates, and only `symbol_extract::call_graph_supports` languages apply.
//! No interface-implementation carve-out yet (a single call site today may not be the
//! only one once a second implementer/caller appears) — `ArchModel` has no
//! type-implements-interface edge to detect it with; tracked as #117.

use std::collections::HashMap;

use crate::arch_model::{ArchModel, SymbolKind, SymbolNode};
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

/// Number of distinct resolved call sites per callee `SymbolNode::id` (distinct by caller,
/// file and line). The finding says "inline it at that call site", so a callee called twice
/// from one caller has two sites, not one: inlining would duplicate its body.
fn call_site_counts(model: &ArchModel) -> HashMap<&str, usize> {
    // A self-edge (a wrapper calling a same-named method on another receiver, which the
    // receiver-blind resolver maps back to itself) is not a call site of the wrapper.
    let mut sites_by_target: HashMap<
        &str,
        std::collections::HashSet<(&str, &std::path::Path, usize)>,
    > = HashMap::new();
    for edge in model
        .call_edges
        .iter()
        .filter(|e| e.resolved && e.from != e.to)
    {
        sites_by_target
            .entry(edge.to.as_str())
            .or_default()
            .insert((edge.from.as_str(), edge.file.as_path(), edge.line));
    }
    sites_by_target
        .into_iter()
        .map(|(target, sites)| (target, sites.len()))
        .collect()
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
             another call and has exactly one call site in the repo — consider inlining it \
             at that call site (Fowler, Inline Function)",
            symbol.name
        ),
        severity_override: None,
    }
}

fn single_call_site_delegation_findings(model: &ArchModel) -> Vec<ArchFinding> {
    let counts = call_site_counts(model);
    let mut findings: Vec<ArchFinding> = model
        .packages
        .values()
        .flat_map(|pkg| pkg.symbols.iter())
        .filter(|symbol| is_candidate(symbol))
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
    fn does_not_count_a_self_edge_as_the_sole_caller() {
        // `Stat` forwards to a same-named method on another receiver; the receiver-blind
        // resolver maps that call back to `Stat` itself, which is not a caller.
        let model = model_from(vec![(
            "/repo/pkg/a.ts",
            "class P {\n  stat(r: string) { return this.inner.stat(r); }\n}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "a self-edge must not make `stat` its own one caller"
        );
    }

    #[test]
    fn a_module_level_callback_counts_as_a_second_caller() {
        let model = model_from(vec![(
            "/repo/pkg/a.ts",
            "function helper(x: number) { return x; }\n\n\
             function thin(x: number) { return helper(x); }\n\n\
             function named() { thin(1); }\n\n\
             describe('suite', () => { it('works', () => { thin(2); }); });\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "`thin` is called from `named` and from a describe() callback"
        );
    }

    #[test]
    fn does_not_flag_a_wrapper_around_a_closure() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nimport \"net/http\"\n\n\
             func auth() http.Handler {\n\treturn http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {\n\t\tw.WriteHeader(401)\n\t})\n}\n\n\
             func Live() {\n\tauth()\n}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "a body whose one call wraps a closure registers logic, it doesn't forward"
        );
    }

    #[test]
    fn does_not_flag_a_wrapper_called_only_from_several_module_level_callbacks() {
        let model = model_from(vec![(
            "/repo/pkg/a.ts",
            "function helper(x: number) { return x; }\n\n\
             function thin(x: number) { return helper(x); }\n\n\
             describe('a', () => { thin(1); });\n\
             describe('b', () => { thin(2); });\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "`thin` is called from two separate callbacks"
        );
    }

    #[test]
    fn does_not_flag_a_wrapper_called_twice_from_the_same_function() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc thin() {\n\thelper()\n}\n\nfunc helper() {}\n\n\
             func Live() {\n\tthin()\n\tthin()\n}\n",
        )]);
        assert!(
            single_call_site_delegation_findings(&model).is_empty(),
            "two call sites in one caller are still two call sites"
        );
    }
}
