use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::node_kind::GoKind;

/// Go's built-in primitive types worth flagging when two or more parameters share one.
/// Deliberately wider than the old ast-grep rule (which only covered
/// string/int/int32/int64/float32/float64/bool) — see
/// .claude/rules/primitive-obsession-checklist.md for the newtype/value-object fix.
const PRIMITIVE_TYPES: &[&str] = &[
    "string", "int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32",
    "uint64", "float32", "float64", "bool", "byte", "rune",
];

/// Flags Go parameters that pile up the same primitive type (see
/// .claude/rules/primitive-obsession-checklist.md).
pub struct PrimitiveObsessionChecker;

impl Checker for PrimitiveObsessionChecker {
    fn name(&self) -> &str {
        "primitive-obsession"
    }

    fn description(&self) -> &str {
        "flags Go parameters that pile up the same primitive type"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if file
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with("_test.go"))
        {
            return Ok(Vec::new());
        }
        let tree = ctx
            .tree
            .context("primitive-obsession checker requires a parsed tree")?;
        let mut findings = Vec::new();
        walk(tree.root_node(), ctx.source.as_bytes(), &mut findings);
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(PrimitiveObsessionChecker)])
}

/// Detects two shapes of same-typed-parameter piles in Go function signatures:
///   (a) a single `parameter_declaration` naming ≥2 identifiers of one primitive
///       type, e.g. `func f(a, b string)`
///   (b) a run of ≥2 consecutive single-name `parameter_declaration`s that share
///       one primitive type, e.g. `func f(a string, b string)`
///
/// Thin wrapper over [`PrimitiveObsessionChecker`] kept for the unit tests below, which
/// predate the `Checker`/registry machinery and check a Go source string directly.
#[cfg(test)]
fn check_source(src: &str) -> Result<Vec<Finding>> {
    crate::test_support::check_go_source(&PrimitiveObsessionChecker, src)
}

/// Stdlib and `slices` package functions whose closure parameters follow a mandated
/// signature — the caller cannot change these names without breaking the stdlib contract.
/// Matched against the `field` child of the selector_expression (e.g. `"Slice"` for
/// `sort.Slice`), so any package with a same-named function is also exempted (accepted
/// false-negative for user-defined wrappers — see ADR-001).
const STDLIB_CALLBACK_ALLOWLIST: &[&str] = &[
    "Slice",
    "SliceStable",
    "Search",
    "SortFunc",
    "SortStableFunc",
    "BinarySearchFunc",
    "IndexFunc",
    "ContainsFunc",
    "EqualFunc",
    "CompareFunc",
];

/// Returns `true` when `param_list` belongs to a `func_literal` passed directly as an
/// argument to a call whose selector field is in `STDLIB_CALLBACK_ALLOWLIST`.
///
/// Parent chain: parameter_list → func_literal → argument_list → call_expression.
/// Modeled on `is_run_subtest_closure` in `complexity.rs`.
fn is_stdlib_callback_param_list(param_list: Node, src: &[u8]) -> bool {
    let Some(func_lit) = param_list.parent() else {
        return false;
    };
    if GoKind::of(func_lit) != GoKind::FuncLiteral {
        return false;
    }
    let Some(args) = func_lit.parent() else {
        return false;
    };
    if GoKind::of(args) != GoKind::ArgumentList {
        return false;
    }
    let Some(call) = args.parent() else {
        return false;
    };
    if GoKind::of(call) != GoKind::CallExpression {
        return false;
    }
    call.child_by_field_name("function")
        .filter(|f| GoKind::of(*f) == GoKind::SelectorExpression)
        .and_then(|f| f.child_by_field_name("field"))
        .and_then(|f| f.utf8_text(src).ok())
        .is_some_and(|name| STDLIB_CALLBACK_ALLOWLIST.contains(&name))
}

fn walk(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    crate::tree_walk::walk_preorder(node, &mut |n| {
        if GoKind::of(n) == GoKind::ParameterList && !is_named_return_list(n) {
            check_parameter_list(n, src, findings);
        }
        true
    });
}

/// True when `node` is the `result` field of its parent signature node.
///
/// `tree-sitter-go`'s grammar reuses the `parameter_list` node kind for named
/// return-value lists (`function_declaration`/`method_declaration`/`func_literal`/
/// `function_type`/`method_elem` all mark it via a `result` field — see
/// `tree-sitter-go`'s `grammar.js`). A named return list carries none of the
/// call-site-mixup risk this check targets, so it's excluded regardless of which
/// of those parent kinds it hangs off of.
fn is_named_return_list(node: Node) -> bool {
    node.parent()
        .and_then(|parent| parent.child_by_field_name("result"))
        .is_some_and(|result| result == node)
}

struct Param<'a> {
    node: Node<'a>,
    /// Total names in the declaration, blank (`_`) identifiers included — used
    /// to detect single-name declarations for the run check (case b).
    name_count: usize,
    /// Names excluding blank (`_`) identifiers — used to detect a real
    /// multi-name declaration for case (a). A blank identifier can never be
    /// confused with another variable in the function body, so it carries none
    /// of the naming-mixup risk that branch targets.
    real_name_count: usize,
    primitive_type: Option<&'a str>,
}

// kibitzer:ignore long-function -- two distinct detection cases (multi-name declaration and consecutive-run) require the combined body; splitting would hurt locality
fn check_parameter_list<'a>(list: Node<'a>, src: &'a [u8], findings: &mut Vec<Finding>) {
    if is_stdlib_callback_param_list(list, src) {
        return;
    }
    let mut cursor = list.walk();
    let params: Vec<Param<'a>> = list
        .children(&mut cursor)
        .filter(|n| GoKind::of(*n) == GoKind::ParameterDeclaration)
        .map(|decl| describe_param(decl, src))
        .collect();

    // Case (a): one declaration naming multiple identifiers of one primitive type.
    for param in &params {
        if param.real_name_count >= 2
            && let Some(ty) = param.primitive_type
        {
            findings.push(Finding {
                line: param.node.start_position().row + 1,
                message: format!(
                    "parameter declaration names {} identifiers of primitive type `{ty}` — consider a newtype/value object instead",
                    param.real_name_count
                ),
            });
        }
    }

    // Case (b): runs of ≥2 consecutive single-name declarations sharing a primitive type.
    let mut run_start = 0;
    while run_start < params.len() {
        let Some(ty) = single_name_primitive(&params[run_start]) else {
            run_start += 1;
            continue;
        };
        let mut run_end = run_start + 1;
        while run_end < params.len() && single_name_primitive(&params[run_end]) == Some(ty) {
            run_end += 1;
        }
        if run_end - run_start >= 2 {
            findings.push(Finding {
                line: params[run_start].node.start_position().row + 1,
                message: format!(
                    "{} consecutive parameters share primitive type `{ty}` — consider a newtype/value object instead",
                    run_end - run_start
                ),
            });
        }
        run_start = run_end;
    }
}

fn single_name_primitive<'a>(param: &Param<'a>) -> Option<&'a str> {
    if param.name_count == 1 {
        param.primitive_type
    } else {
        None
    }
}

fn describe_param<'a>(decl: Node<'a>, src: &'a [u8]) -> Param<'a> {
    let mut cursor = decl.walk();
    let names: Vec<Node<'a>> = decl.children_by_field_name("name", &mut cursor).collect();
    let name_count = names.len();
    let real_name_count = names
        .iter()
        .filter(|n| n.utf8_text(src).unwrap_or("") != "_")
        .count();

    let primitive_type = decl.child_by_field_name("type").and_then(|ty| {
        if GoKind::of(ty) == GoKind::TypeIdentifier {
            let text = ty.utf8_text(src).unwrap_or("");
            PRIMITIVE_TYPES.iter().find(|&&p| p == text).copied()
        } else {
            None
        }
    });

    Param {
        node: decl,
        name_count,
        real_name_count,
        primitive_type,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_multi_name_declaration() {
        let findings = check_source("package main\nfunc f(a, b string) {}\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("2 identifiers"));
    }

    #[test]
    fn flags_consecutive_single_name_declarations() {
        let findings = check_source("package main\nfunc f(a string, b string) {}\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("2 consecutive parameters"));
    }

    #[test]
    fn flags_run_of_three() {
        let findings = check_source("package main\nfunc f(x int, y int, z int) {}\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3 consecutive parameters"));
    }

    #[test]
    fn allows_distinct_types() {
        let findings = check_source("package main\nfunc f(a string, b int) {}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_single_primitive_param() {
        let findings = check_source("package main\nfunc f(a string) {}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_non_primitive_types() {
        let findings = check_source("package main\nfunc f(a MyType, b MyType) {}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_double_count_across_run_and_multi_name() {
        // a, b string is one declaration (case a); c string is a separate single-name
        // declaration immediately after — together they'd also look like a run, but
        // case (a) already flagged the pair, so case (b) should not also fire since the
        // multi-name declaration itself isn't single-name and breaks the run.
        let findings = check_source("package main\nfunc f(a, b string, c string) {}\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("2 identifiers"));
    }

    #[test]
    fn flags_real_multi_named_parameter_list() {
        // True-positive regression: a genuine call-site parameter list should
        // still fire even after excluding named-return `parameter_list`s.
        let findings = check_source("package main\nfunc newUser(name, email string) {}\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("2 identifiers"));
    }

    #[test]
    fn ignores_named_return_list_on_function_declaration() {
        // Regression for the 2026-09-12 false positive: tree-sitter-go gives a
        // named return list the same `parameter_list` node kind as a call-site
        // parameter list (e.g. kubernetes/kubernetes's generated
        // `(major, minor int)` shape).
        let findings =
            check_source("package main\nfunc f() (major, minor int) { return 1, 0 }\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_named_return_list_on_method_declaration() {
        // Same shape as stapler-squad's `session/ent/mutation.go:12536`:
        // `Acknowledged() (r bool, exists bool)`.
        let findings = check_source(
            "package main\ntype T struct{}\nfunc (t T) M() (r bool, exists bool) { return false, false }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_named_return_list_on_func_literal() {
        let findings = check_source(
            "package main\nfunc f() { g := func() (major, minor int) { return 1, 0 }; _ = g }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn still_flags_parameters_when_named_return_list_present() {
        // A real parameter-list primitive pile-up shouldn't get a pass just
        // because the same function also has a named return list.
        let findings =
            check_source("package main\nfunc f(a, b string) (x, y string) { return a, b }\n")
                .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("2 identifiers"));
    }

    #[test]
    fn ignores_all_blank_names() {
        // Regression for stapler-squad's `session/autonomous_driver_test.go:28`:
        // `CallBlocking(_, _ string, ...)` — both names discarded, so there's no
        // real identifier to mix up.
        let findings = check_source("package main\nfunc f(_, _ string) {}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_mixed_blank_and_real_name() {
        // Only one real (non-blank) name in the declaration — nothing in the
        // function body can confuse `x` for `_`, so this shouldn't fire either.
        let findings = check_source("package main\nfunc f(x, _ string) {}\n").unwrap();
        assert!(findings.is_empty());
    }

    // --- stdlib callback suppression (AC1, AC2) ---

    #[test]
    fn suppresses_sort_slice_callback() {
        // AC1: sort.Slice comparator has mandated (i, j int) signature
        let src = r#"package main
import "sort"
func f(s []int) {
    sort.Slice(s, func(i, j int) bool { return s[i] < s[j] })
}"#;
        assert!(check_source(src).unwrap().is_empty());
    }

    #[test]
    fn suppresses_slices_sort_func_callback() {
        // AC2: slices.SortFunc with primitive element type
        let src = r#"package main
import "slices"
func f(s []int) {
    slices.SortFunc(s, func(a, b int) int { return a - b })
}"#;
        assert!(check_source(src).unwrap().is_empty());
    }

    #[test]
    fn suppresses_slices_sort_stable_func_callback() {
        // AC2: slices.SortStableFunc with string element type
        let src = r#"package main
import "slices"
func f(s []string) {
    slices.SortStableFunc(s, func(a, b string) int {
        if a < b { return -1 }
        return 1
    })
}"#;
        assert!(check_source(src).unwrap().is_empty());
    }

    #[test]
    fn suppresses_sort_search_callback() {
        // AC2: sort.Search with single-param callback (confirms allowlist match on different shape)
        let src = r#"package main
import "sort"
func f(n int) int {
    return sort.Search(n, func(i int) bool { return i >= 0 })
}"#;
        assert!(check_source(src).unwrap().is_empty());
    }

    // --- test-file suppression (AC3) and true-positive guard (AC4) ---

    #[test]
    fn suppresses_func_literal_in_test_file() {
        // AC3: same-typed multi-param func literal in a _test.go file → zero findings
        use crate::checker::{CheckContext, Checker};
        let checker = PrimitiveObsessionChecker;
        let src = r#"package main_test
func TestSomething(t *testing.T) {
    write := func(name, body string) {}
    write("x", "y")
}"#;
        let tree = crate::test_support::parse_go(src).unwrap();
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        let findings = checker
            .check(std::path::Path::new("foo_test.go"), &ctx)
            .unwrap();
        assert!(
            findings.is_empty(),
            "expected no findings in _test.go, got: {findings:?}"
        );
    }

    #[test]
    fn still_flags_exported_multi_primitive_in_non_test_file() {
        // AC4: exported func with same-type primitive pile-up in a non-test file still fires
        let findings = check_source("package main\nfunc PublicAPI(a, b string) {}\n").unwrap();
        assert_eq!(findings.len(), 1);
    }
}
