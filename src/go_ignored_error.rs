use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::{Node, Tree};

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::go_call_resolution::{self, GoModule};

/// Flags Go short variable declarations that discard the *last* value of a
/// multi-value call via `_` (e.g. `result, _ := f()`) — by Go convention the last
/// return value is the error, so this shape usually means an error is being
/// silently dropped. Deliberately does NOT flag `_, err := f()`: there the
/// blank identifier discards a non-last value and the error is kept, which is
/// the exact conflation this check must avoid. Also requires the RHS to be a
/// single `call_expression`, so a comma-ok type assertion (`v[1].(string)`) or
/// map index (`m[k]`) — same LHS shape, but the trailing value is a bool, not
/// an error — isn't flagged. When the call is a package-qualified free function
/// (`pkg.Func(...)`) resolvable to a real declaration under this file's own Go
/// module (see `go_call_resolution`), a non-`error` last return type suppresses
/// the finding; anything unresolvable (stdlib, vendor, another module, or a
/// method call on a local variable) falls back to flagging, same as before —
/// see docs/go-ignored-error-false-positives.md for the remaining known gap.
pub struct IgnoredErrorChecker;

impl Checker for IgnoredErrorChecker {
    fn name(&self) -> &str {
        "go-ignored-error"
    }

    fn description(&self) -> &str {
        "flags `result, _ := f()`-shaped discards of a call's last (conventionally error) return value"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("go-ignored-error checker requires a parsed tree")?;
        let module = resolve_module(file);
        let mut findings = Vec::new();
        walk(
            tree.root_node(),
            tree,
            ctx.source.as_bytes(),
            module.as_ref(),
            &mut findings,
        );
        Ok(findings)
    }
}

struct ResolvedModule {
    root: std::path::PathBuf,
    path: String,
}

fn resolve_module(file: &Path) -> Option<ResolvedModule> {
    let root = go_call_resolution::find_module_root(file)?;
    let go_mod_src = std::fs::read_to_string(root.join("go.mod")).ok()?;
    let path = go_call_resolution::parse_module_path(&go_mod_src)?;
    Some(ResolvedModule { root, path })
}

/// Returns the RHS `call_expression` when a `short_var_declaration`'s `right`
/// expression list is a single call. By Go convention the "last value is the
/// error" heuristic only makes sense for a multi-value function/method call — a
/// comma-ok type assertion (`v[1].(string)`) or map index (`m[k]`) produces the
/// same two-identifier LHS shape but its trailing value is a plain bool, not an
/// error, so those RHS node kinds must not be flagged.
fn rhs_call_expression(node: Node) -> Option<Node> {
    let right = node.child_by_field_name("right")?;
    let mut cursor = right.walk();
    let mut exprs = right.named_children(&mut cursor);
    match (exprs.next(), exprs.next()) {
        (Some(only), None) if only.kind() == "call_expression" => Some(only),
        _ => None,
    }
}

/// When `call`'s callee is `pkg_alias.func_name` (a package-qualified free function,
/// not a method call on a local variable), attempts to resolve it to a real
/// declaration under `module` and reports whether the discard is safe to suppress
/// (the real last return type isn't `error`). `false` for every unresolvable case —
/// not a package-qualified selector, no module found, or resolution failed — so the
/// caller's default stays "flag it," never "guess it's fine."
fn safe_to_suppress(call: Node, tree: &Tree, src: &[u8], module: Option<&ResolvedModule>) -> bool {
    let Some(module) = module else {
        return false;
    };
    let Some(function) = call.child_by_field_name("function") else {
        return false;
    };
    if function.kind() != "selector_expression" {
        return false;
    }
    let Some(operand) = function.child_by_field_name("operand") else {
        return false;
    };
    if operand.kind() != "identifier" {
        return false;
    }
    let Some(field) = function.child_by_field_name("field") else {
        return false;
    };
    let (Ok(pkg_alias), Ok(func_name)) = (operand.utf8_text(src), field.utf8_text(src)) else {
        return false;
    };

    let go_module = GoModule {
        root: &module.root,
        path: &module.path,
    };
    go_call_resolution::resolve_last_return_is_error(&go_module, tree, src, pkg_alias, func_name)
        == Some(false)
}

fn walk(
    node: Node,
    tree: &Tree,
    src: &[u8],
    module: Option<&ResolvedModule>,
    findings: &mut Vec<Finding>,
) {
    if node.kind() == "short_var_declaration"
        && let Some(call) = rhs_call_expression(node)
        && let Some(left) = node.child_by_field_name("left")
    {
        let mut cursor = left.walk();
        let names: Vec<Node> = left
            .children(&mut cursor)
            .filter(|n| n.kind() == "identifier")
            .collect();
        if names.len() >= 2
            && let Some(last) = names.last()
            && last.utf8_text(src) == Ok("_")
            && !safe_to_suppress(call, tree, src, module)
        {
            findings.push(Finding {
                line: node.start_position().row + 1,
                message: "discards the last return value via `_` — by convention that's \
                          the error; if `f()` can fail, handle or explicitly justify \
                          ignoring it"
                    .to_string(),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, tree, src, module, findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Result<Vec<Finding>> {
        crate::test_support::check_go_source(&IgnoredErrorChecker, src)
    }

    #[test]
    fn flags_discarded_last_value() {
        let findings =
            check_source("package main\nfunc f() (int, error) { return 0, nil }\nfunc g() {\n\tresult, _ := f()\n\t_ = result\n}\n")
                .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn does_not_flag_discarded_first_value_keeping_error() {
        // The exact conflation risk this check must avoid: blank in a non-last
        // position discards a value, not the error.
        let findings = check_source(
            "package main\nfunc f() (int, error) { return 0, nil }\nfunc g() {\n\t_, err := f()\n\tif err != nil {\n\t\treturn\n\t}\n}\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_single_value_declaration() {
        let findings = check_source(
            "package main\nfunc f() int { return 0 }\nfunc g() {\n\tx := f()\n\t_ = x\n}\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_declaration_with_no_blank() {
        let findings = check_source(
            "package main\nfunc f() (int, error) { return 0, nil }\nfunc g() {\n\tx, err := f()\n\t_ = x\n\t_ = err\n}\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_all_blank_declaration() {
        let findings = check_source(
            "package main\nfunc f() (int, error) { return 0, nil }\nfunc g() {\n\t_, _ := f()\n}\n",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn empty_file_produces_no_findings() {
        let findings = check_source("package main\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_comma_ok_type_assertion() {
        // Real shape from vendor/k8s.io/klog/v2/klogr.go:80 — the discarded
        // value is the comma-ok bool from a type assertion, not an error.
        let findings = check_source(
            "package main\nfunc g(v []interface{}) {\n\tprefix, _ := v[1].(string)\n\t_ = prefix\n}\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_comma_ok_map_index() {
        // Same fixable mechanism as the type-assertion case: `m[k]` is an
        // index_expression, not a call, so its comma-ok bool isn't an error.
        let findings = check_source(
            "package main\nfunc g(m map[string]int) {\n\tv, ok := m[\"k\"]\n\t_ = v\n\t_ = ok\n}\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_ignored_error_from_real_call() {
        // Genuine true positive: RHS is a call_expression and the last
        // return value (by convention, the error) is discarded. Uses `:=`
        // (not `=`) because plain-assignment re-declarations are a separate,
        // already-documented scope gap this checker doesn't cover at all
        // (see "Documented scope gaps" in go-ignored-error-false-positives.md) —
        // confirmed by running this test against `_, _ = f()` first, which
        // produced 0 findings rather than the expected 1.
        let findings = check_source(
            "package main\nfunc f() (int, error) { return 0, nil }\nfunc g() {\n\t_, _ := f()\n}\n",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    /// Builds a tiny two-package Go module on disk under a unique temp directory
    /// (same manual-tempdir pattern as `duplicate_cross_file_checker.rs`'s tests —
    /// no `tempfile` crate dependency): a `go.mod` declaring `module_path`, and a
    /// `helper` package containing exactly one function, `helper_func_src` (its full
    /// text, e.g. `"func F(s string) (string, string) { return s, s }"`). Returns the
    /// module root and the path of a `main.go` file (not yet written) the caller
    /// should check against.
    struct TempModule {
        root: std::path::PathBuf,
    }

    impl TempModule {
        fn new(module_path: &str, helper_func_src: &str) -> Self {
            let root = crate::test_support::unique_temp_dir("go-ignored-error-module");
            let helper_dir = root.join("helper");
            std::fs::create_dir_all(&helper_dir).unwrap();
            std::fs::write(
                root.join("go.mod"),
                format!("module {module_path}\n\ngo 1.22\n"),
            )
            .unwrap();
            std::fs::write(
                helper_dir.join("helper.go"),
                format!("package helper\n\n{helper_func_src}\n"),
            )
            .unwrap();
            TempModule { root }
        }

        /// Checks `main_src` as if it were `main.go` at the module root, importing
        /// `helper` under `module_path/helper`.
        fn check_main(&self, module_path: &str, main_src: &str) -> Vec<Finding> {
            let src = format!("package main\n\nimport \"{module_path}/helper\"\n\n{main_src}\n");
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&tree_sitter_go::LANGUAGE.into())
                .unwrap();
            let tree = parser.parse(&src, None).unwrap();
            let ctx = CheckContext {
                source: &src,
                tree: Some(&tree),
            };
            IgnoredErrorChecker
                .check(&self.root.join("main.go"), &ctx)
                .unwrap()
        }
    }

    impl Drop for TempModule {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    #[test]
    fn does_not_flag_same_module_call_whose_real_last_return_is_not_error() {
        // Recreates the readwriter.go:64 shape: a same-module, cross-package free
        // function returning (string, string) — the discarded value is a second
        // string, never an error, and this is resolvable purely from source on disk.
        let module = TempModule::new(
            "example.com/mod",
            "func PathsForCertAndKey(dir, name string) (string, string) { return dir, name }",
        );
        let findings = module.check_main(
            "example.com/mod",
            "func g() {\n\ta, _ := helper.PathsForCertAndKey(\"d\", \"n\")\n\t_ = a\n}",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn still_flags_same_module_call_whose_real_last_return_is_error() {
        // True-positive guard: resolution must not suppress a genuine ignored error
        // just because the call happens to be cross-package and resolvable.
        let module = TempModule::new(
            "example.com/mod",
            "func DoThing() (int, error) { return 0, nil }",
        );
        let findings = module.check_main(
            "example.com/mod",
            "func g() {\n\t_, _ := helper.DoThing()\n}",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn still_flags_when_the_import_is_not_under_this_module() {
        // Conservative-fallback guard: an import path that doesn't resolve under the
        // current module (stdlib, vendor, or another module entirely) must not be
        // treated as resolved-and-safe — it must fall back to flagging, unchanged.
        // Doesn't need `sync.Frobnicate` to be a real API — the checker only inspects
        // shape, and resolution must fail (and stay conservative) before ever reaching
        // real-signature-checking territory.
        let module = TempModule::new("example.com/mod", "func Unused() {}");
        let src = "package main\n\nimport \"sync\"\n\nfunc g() {\n\t_, _ := sync.Frobnicate()\n}\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        let findings = IgnoredErrorChecker
            .check(&module.root.join("main.go"), &ctx)
            .unwrap();
        assert_eq!(findings.len(), 1);
    }
}
