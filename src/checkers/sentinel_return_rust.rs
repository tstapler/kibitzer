//! `sentinel-return-rust`: an explicit `return None` / `return Err(..)` that hand-propagates a
//! failure (inside an `Err(..)`/`None` match arm, a `let .. else` block, or an `if let Err(..)` /
//! `.is_none()` / `.is_err()` branch) in a function that also uses `?`.
//!
//! Mixing the two exit styles makes the error path hard to follow: the `?` operator expresses
//! these returns (`clippy::question_mark` flags the same shape). Plain guard returns that
//! originate a new error (`if n > max { return Err(TooBig) }`) are deliberately not flagged:
//! the first backtest of "any explicit return" found them idiomatic (docs/sentinel-return-rust-false-positives.md).
//! Only functions returning `Option<_>`/`Result<_, _>` count. Constructor-like names (`new`,
//! `try_new`, `from_*`, `try_from*`, `default`) are skipped, as are test code and closures'
//! own `?`/returns (a closure is its own exit scope). See docs/sentinel-return-rust-false-positives.md.

use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::rust_common::{in_test_code, is_test_path, text};
use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::tree_walk::walk_preorder;

const NAME: &str = "sentinel-return-rust";

pub struct SentinelReturnRustChecker;

impl Checker for SentinelReturnRustChecker {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        "flags explicit `return None`/`return Err(..)` in a Rust function that also uses `?`"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Rust)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.rs"]
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if is_test_path(file) {
            return Ok(Vec::new());
        }
        let tree = ctx
            .tree
            .context("sentinel-return-rust requires a parsed tree")?;
        let mut findings = Vec::new();
        walk_preorder(tree.root_node(), &mut |node| {
            if node.kind() == "function_item" && !in_test_code(node, ctx.source) {
                check_fn(node, ctx.source, &mut findings);
            }
            true
        });
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(SentinelReturnRustChecker)])
}

fn is_constructor_name(name: &str) -> bool {
    matches!(name, "new" | "try_new" | "default")
        || name.starts_with("from_")
        || name.starts_with("try_from")
}

fn returns_option_or_result(func: Node, source: &str) -> bool {
    func.child_by_field_name("return_type").is_some_and(|t| {
        let ty = text(t, source);
        ty.contains("Option<")
            || ty.contains("Result<")
            || ty.starts_with("Result")
            || ty.contains("::Result")
    })
}

fn check_fn(func: Node, source: &str, findings: &mut Vec<Finding>) {
    let name = func
        .child_by_field_name("name")
        .map_or("", |n| text(n, source));
    if is_constructor_name(name) || !returns_option_or_result(func, source) {
        return;
    }
    let Some(body) = func.child_by_field_name("body") else {
        return;
    };
    let mut has_try = false;
    let mut sentinels = Vec::new();
    scan(body, source, &mut has_try, &mut sentinels);
    if !has_try {
        return;
    }
    for (line, what) in sentinels {
        findings.push(Finding {
            line,
            message: format!(
                "explicit `return {what}` in `{name}`, which also propagates with `?` — use one exit style (e.g. `.ok_or(..)?`) so the error path reads uniformly"
            ),
        });
    }
}

/// Collects this function's own `?` and explicit sentinel returns, not descending into nested
/// closures or items (each is its own exit scope).
fn scan(node: Node, source: &str, has_try: &mut bool, sentinels: &mut Vec<(usize, &'static str)>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "closure_expression" | "function_item" | "impl_item" | "mod_item" => {}
            "try_expression" => {
                *has_try = true;
                scan(child, source, has_try, sentinels);
            }
            "return_expression" => {
                if let Some(what) =
                    sentinel_kind(child, source).filter(|_| is_propagation_site(child, source))
                {
                    sentinels.push((child.start_position().row + 1, what));
                }
                scan(child, source, has_try, sentinels);
            }
            _ => scan(child, source, has_try, sentinels),
        }
    }
}

/// The return is the whole arm value or the only statement of its block: with a log line or
/// cleanup before it, `?` could not replace the branch.
fn is_sole_statement(ret: Node) -> bool {
    let Some(parent) = ret.parent() else {
        return false;
    };
    match parent.kind() {
        "match_arm" => true,
        "expression_statement" => parent.parent().is_some_and(|block| {
            block.kind() == "block"
                && block
                    .named_children(&mut block.walk())
                    .filter(|c| !matches!(c.kind(), "line_comment" | "block_comment"))
                    .count()
                    == 1
        }),
        "block" => {
            parent
                .named_children(&mut parent.walk())
                .filter(|c| !matches!(c.kind(), "line_comment" | "block_comment"))
                .count()
                == 1
        }
        _ => false,
    }
}

/// The return sits in the failure branch of a fallible value: an `Err(..)`/`None` match arm, a
/// `let .. else` block, or an `if` testing `let Err(..)`/`let None`/`.is_none()`/`.is_err()`.
fn is_propagation_site(ret: Node, source: &str) -> bool {
    if !is_sole_statement(ret) {
        return false;
    }
    let mut cur = ret.parent();
    while let Some(n) = cur {
        match n.kind() {
            "closure_expression" | "function_item" => return false,
            "match_arm" => {
                let pat = n
                    .child_by_field_name("pattern")
                    .map_or("", |p| text(p, source));
                return pat.starts_with("Err(") || pat.trim() == "None" || pat.trim() == "Err(_)";
            }
            "let_declaration" => {
                let in_else = n
                    .child_by_field_name("alternative")
                    .is_some_and(|a| a.byte_range().contains(&ret.start_byte()));
                let pat = n
                    .child_by_field_name("pattern")
                    .map_or("", |p| text(p, source));
                return in_else && (pat.starts_with("Some(") || pat.starts_with("Ok("));
            }
            "if_expression" => {
                let in_consequence = n
                    .child_by_field_name("consequence")
                    .is_some_and(|c| c.byte_range().contains(&ret.start_byte()));
                if in_consequence {
                    let cond = n
                        .child_by_field_name("condition")
                        .map_or("", |c| text(c, source));
                    if cond.contains("&&") || cond.contains("||") {
                        return false;
                    }
                    return cond.contains("Err(") && cond.contains("let ")
                        || cond.contains("let None")
                        || cond.contains(".is_none()")
                        || cond.contains(".is_err()");
                }
            }
            _ => {}
        }
        cur = n.parent();
    }
    false
}

fn sentinel_kind(ret: Node, source: &str) -> Option<&'static str> {
    let value = ret.named_child(0)?;
    let t = text(value, source).trim();
    if t == "None" {
        Some("None")
    } else if value.kind() == "call_expression" && t.starts_with("Err(") {
        Some("Err(..)")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<Finding> {
        crate::test_support::check_rust_source(&SentinelReturnRustChecker, src).unwrap()
    }

    #[test]
    fn flags_explicit_err_in_a_function_that_also_uses_question_mark() {
        let f = run(
            "fn f(x: &str) -> Result<i32, E> {\n    let a = g(x)?;\n    let b = match h(a) {\n        Ok(b) => b,\n        Err(e) => return Err(E::from(e)),\n    };\n    Ok(b)\n}",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 5);
    }

    #[test]
    fn flags_explicit_none_in_option_fn_with_question_mark() {
        assert_eq!(run("fn f(x: &str) -> Option<i32> {\n    let a = g(x)?;\n    let Some(b) = h(a) else { return None };\n    Some(b)\n}").len(), 1);
        assert_eq!(run("fn f(x: &str) -> Option<i32> {\n    let a = g(x)?;\n    if h(a).is_none() { return None; }\n    Some(a)\n}").len(), 1);
    }

    #[test]
    fn guard_returns_that_originate_an_error_are_not_flagged() {
        assert!(run("fn f(x: &str) -> Result<i32, E> { let a = g(x)?; if a > 1 { return Err(E::Big); } Ok(a) }").is_empty());
    }

    #[test]
    fn no_question_mark_means_no_finding() {
        assert!(run("fn f(x: i32) -> Result<i32, E> { match h(x) { Ok(v) => Ok(v), Err(e) => return Err(e) } }").is_empty());
    }

    #[test]
    fn side_effect_before_return_and_non_option_let_else_are_not_flagged() {
        assert!(run("fn f() -> Option<i32> { let a = g()?; let Some(b) = h(a) else { log(); return None }; Some(b) }").is_empty());
        assert!(run("fn f(t: T) -> Option<i32> { let a = g()?; let Tok::Lit(c) = t else { return None }; Some(c) }").is_empty());
    }

    #[test]
    fn ok_returns_are_never_flagged() {
        assert!(run("fn f(x: &str) -> Result<i32, E> { let a = g(x)?; return Ok(a); }").is_empty());
    }

    #[test]
    fn constructors_are_skipped() {
        assert!(run("fn new(x: &str) -> Option<Self> { let a = g(x)?; let Some(b) = h(a) else { return None }; Some(S(b)) }").is_empty());
        assert!(run("fn from_str(x: &str) -> Result<Self, E> { let a = g(x)?; match h(a) { Ok(b) => Ok(S(b)), Err(e) => return Err(e) } }").is_empty());
    }

    #[test]
    fn non_fallible_return_type_is_skipped() {
        assert!(
            run(
                "fn f(x: &str) -> i32 { let a = g(x)?; let Some(b) = h(a) else { return None }; b }"
            )
            .is_empty()
        );
    }

    #[test]
    fn closure_scope_is_separate() {
        assert!(run("fn f() -> Result<(), E> { let c = || -> Option<i32> { let a = g()?; Some(a) }; Ok(()) }").is_empty());
    }

    #[test]
    fn test_code_is_skipped() {
        assert!(run("#[cfg(test)]\nmod t { fn f() -> Result<i32, E> { let a = g()?; match h() { Ok(b) => Ok(b), Err(e) => return Err(e) } } }").is_empty());
    }
}
