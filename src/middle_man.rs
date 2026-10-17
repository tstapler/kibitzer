//! Fowler's **Remove Middle Man**: a type whose methods almost all do nothing but forward
//! to one of its own fields adds indirection without behavior. The AST-shape judgment
//! (`SymbolNode::delegation_field`) happens upstream in `symbol_extract.rs`, since
//! `ArchModel` drops source/AST after the initial scan.
//!
//! Go and TS/JS only (`symbol_extract::call_graph_supports`): other languages never get a
//! `delegation_field`, so they can't reach the ratio. Types under `MIDDLE_MAN_MIN_METHODS`
//! methods are skipped — a one- or two-method forwarder is usually an intentional
//! Adapter/Decorator. The carve-outs below are fixed constants, not `ArchitectureConfig`
//! options. Skipped: types named `*Adapter`/`*Wrapper`/`*Proxy`/`*Decorator`, methods in test
//! files, and types with an explicit `implements` edge (TS/Java-style; Go's structural
//! satisfaction is not tracked). Backtesting `kubernetes/kubernetes` and `microsoft/vscode`
//! showed those were the intentional-delegation majority of raw hits
//! (see `project_plans/remove-middle-man/decisions/validation.md`).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::arch_model::{ArchModel, PackageNode, SymbolKind, SymbolNode, TypeRelationKind};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;

const MIDDLE_MAN_MIN_METHODS: usize = 3;
const MIDDLE_MAN_DELEGATION_RATIO: f64 = 0.8;
const INTENTIONAL_DELEGATION_SUFFIXES: [&str; 4] = ["Adapter", "Wrapper", "Proxy", "Decorator"];

pub struct MiddleManChecker;

impl ArchModelChecker for MiddleManChecker {
    fn name(&self) -> &str {
        "remove-middle-man"
    }

    /// Unexported methods count toward both sides of the ratio; without this
    /// `build_arch_model_for_check` prunes them before `check` runs.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        // Resolved or not: an `implements Foo` whose interface lives outside the repo is
        // still an explicit adapter declaration.
        let implementers: HashSet<&str> = model
            .type_edges
            .iter()
            .filter(|e| e.kind == Some(TypeRelationKind::Implements))
            .map(|e| e.from.as_str())
            .collect();
        let mut findings: Vec<ArchFinding> = model
            .packages
            .values()
            .flat_map(|pkg| findings_for_package(pkg, &model.repo_root, &implementers))
            .collect();
        findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        findings
    }
}

fn findings_for_package(
    pkg: &PackageNode,
    repo_root: &Path,
    implementers: &HashSet<&str>,
) -> Vec<ArchFinding> {
    let mut methods_by_parent: BTreeMap<&str, Vec<&SymbolNode>> = BTreeMap::new();
    for sym in &pkg.symbols {
        if sym.kind == SymbolKind::Method
            && !is_test_file(sym.file.strip_prefix(repo_root).unwrap_or(&sym.file))
            && let Some(parent) = &sym.parent
            && !implements_declared_interface(sym, implementers)
        {
            methods_by_parent.entry(parent).or_default().push(sym);
        }
    }

    methods_by_parent
        .into_iter()
        .filter_map(|(type_name, methods)| finding_for_type(&pkg.path, type_name, &methods))
        .collect()
}

/// Method ids are `{pkg}::{type}.{name}`, the same shape `type_edges.from` uses for the type.
fn implements_declared_interface(method: &SymbolNode, implementers: &HashSet<&str>) -> bool {
    method
        .id
        .strip_suffix(method.name.as_str())
        .and_then(|rest| rest.strip_suffix('.'))
        .is_some_and(|type_id| implementers.contains(type_id))
}

fn finding_for_type(
    pkg_path: &str,
    type_name: &str,
    methods: &[&SymbolNode],
) -> Option<ArchFinding> {
    if INTENTIONAL_DELEGATION_SUFFIXES
        .iter()
        .any(|suffix| type_name.ends_with(suffix))
    {
        return None;
    }
    let total = methods.len();
    if total < MIDDLE_MAN_MIN_METHODS {
        return None;
    }

    let mut by_field: BTreeMap<&str, Vec<&SymbolNode>> = BTreeMap::new();
    for method in methods {
        if let Some(field) = method.delegation_field.as_deref() {
            by_field.entry(field).or_default().push(method);
        }
    }
    let (field, delegating) = by_field
        .into_iter()
        .min_by_key(|(field, delegating)| (std::cmp::Reverse(delegating.len()), *field))?;

    if (delegating.len() as f64) / (total as f64) < MIDDLE_MAN_DELEGATION_RATIO {
        return None;
    }

    let first = delegating
        .iter()
        .min_by_key(|m| (&m.file, m.line))
        .expect("delegating is non-empty");
    Some(ArchFinding {
        file: Some(first.file.clone()),
        line: Some(first.line),
        message: format!(
            "[remove-middle-man] {pkg_path}::{type_name} delegates {}/{total} methods \
             through field '{field}' — consider removing this middle man and letting \
             callers talk to '{field}' directly (Fowler, Remove Middle Man)",
            delegating.len()
        ),
        severity_override: None,
    })
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

    fn findings(model: &ArchModel) -> Vec<ArchFinding> {
        MiddleManChecker.check(model, &ArchitectureConfig::default())
    }

    fn go_type(methods: &[&str]) -> String {
        format!(
            "package pkg\n\ntype S struct {{ inner T; other T }}\n\ntype T struct{{}}\n\n{}",
            methods.join("\n")
        )
    }

    #[test]
    fn flags_go_type_with_majority_same_field_delegation() {
        let src = go_type(&[
            "func (s *S) A() { s.inner.A() }",
            "func (s *S) B() int { return s.inner.B() }",
            "func (s *S) C() { s.inner.C() }",
            "func (s *S) D() { s.inner.D() }",
            "func (s *S) E() { s.work(); s.work() }",
        ]);
        let model = model_from(vec![("/repo/pkg/a.go", &src)]);
        let found = findings(&model);
        assert_eq!(found.len(), 1, "got: {found:?}");
        assert!(
            found[0]
                .message
                .contains("delegates 4/5 methods through field 'inner'"),
            "{}",
            found[0].message
        );
        assert!(found[0].message.contains("(Fowler, Remove Middle Man)"));
        assert_eq!(found[0].file, Some(PathBuf::from("/repo/pkg/a.go")));
        assert_eq!(found[0].line, Some(7), "first delegating method is A");
    }

    #[test]
    fn flags_typescript_type_with_majority_same_field_delegation() {
        let model = model_from(vec![(
            "/repo/src/a.ts",
            "class S {\n  private inner: any;\n  a() { return this.inner.a(); }\n  b() { this.inner.b(); }\n  c(x: number) { return this.inner.c(x); }\n}\n",
        )]);
        let found = findings(&model);
        assert_eq!(found.len(), 1, "got: {found:?}");
        assert!(
            found[0]
                .message
                .contains("delegates 3/3 methods through field 'inner'")
        );
    }

    #[test]
    fn does_not_flag_mixed_field_delegation() {
        let src = go_type(&[
            "func (s *S) A() { s.inner.A() }",
            "func (s *S) B() { s.inner.B() }",
            "func (s *S) C() { s.other.C() }",
            "func (s *S) D() { s.other.D() }",
        ]);
        let model = model_from(vec![("/repo/pkg/a.go", &src)]);
        assert!(findings(&model).is_empty());
    }

    #[test]
    fn does_not_flag_type_below_min_method_count() {
        let src = go_type(&[
            "func (s *S) A() { s.inner.A() }",
            "func (s *S) B() { s.inner.B() }",
        ]);
        let model = model_from(vec![("/repo/pkg/a.go", &src)]);
        assert!(findings(&model).is_empty());
    }

    #[test]
    fn does_not_flag_type_below_ratio_threshold() {
        let src = go_type(&[
            "func (s *S) A() { s.inner.A() }",
            "func (s *S) B() { s.inner.B() }",
            "func (s *S) C() { s.inner.C() }",
            "func (s *S) D() { s.work(); s.work() }",
            "func (s *S) E() { s.work(); s.work() }",
        ]);
        let model = model_from(vec![("/repo/pkg/a.go", &src)]);
        assert!(findings(&model).is_empty(), "3/5 = 0.6 is under 0.8");
    }

    #[test]
    fn methods_without_a_delegation_field_do_not_count_as_delegating() {
        // Two of four delegate through `inner`; the other two are pure delegations that
        // route through a call chain (`delegation_field = None`), not a field.
        let src = go_type(&[
            "func (s *S) A() { s.inner.A() }",
            "func (s *S) B() { s.inner.B() }",
            "func (s *S) C() { s.Get().C() }",
            "func (s *S) D() { helper() }",
            "func helper() {}",
        ]);
        let model = model_from(vec![("/repo/pkg/a.go", &src)]);
        assert!(findings(&model).is_empty(), "2/4 = 0.5 is under 0.8");
    }

    #[test]
    fn does_not_flag_types_named_as_an_intentional_delegation_pattern() {
        let src = "package pkg\n\ntype T struct{}\ntype streamAdapter struct{ delegate T }\n\n\
                   func (s *streamAdapter) A() { s.delegate.A() }\n\
                   func (s *streamAdapter) B() { s.delegate.B() }\n\
                   func (s *streamAdapter) C() { s.delegate.C() }\n";
        let model = model_from(vec![("/repo/pkg/a.go", src)]);
        assert!(findings(&model).is_empty());
    }

    #[test]
    fn does_not_flag_types_in_test_files() {
        let src = go_type(&[
            "func (s *S) A() { s.inner.A() }",
            "func (s *S) B() { s.inner.B() }",
            "func (s *S) C() { s.inner.C() }",
        ]);
        let model = model_from(vec![("/repo/pkg/a_test.go", &src)]);
        assert!(findings(&model).is_empty());
    }

    #[test]
    fn a_checkout_under_a_tests_directory_is_still_checked() {
        let src = go_type(&[
            "func (s *S) A() { s.inner.A() }",
            "func (s *S) B() { s.inner.B() }",
            "func (s *S) C() { s.inner.C() }",
        ]);
        let files = vec![(PathBuf::from("/work/tests/repo/pkg/a.go"), src)];
        let model = crate::arch_model::build_model(
            &PathBuf::from("/work/tests/repo"),
            &files,
            &crate::import_graph::ImportGraph::default(),
            &PruneConfig {
                include_private: true,
            },
        )
        .unwrap();
        assert_eq!(findings(&model).len(), 1);
    }

    #[test]
    fn does_not_flag_a_typescript_type_that_declares_implements() {
        let model = model_from(vec![(
            "/repo/src/a.ts",
            "interface IFoo { a(): void; b(): void; c(): void; }\nclass S implements IFoo {\n  private inner: any;\n  a() { this.inner.a(); }\n  b() { this.inner.b(); }\n  c() { this.inner.c(); }\n}\n",
        )]);
        assert!(findings(&model).is_empty());
    }
}
