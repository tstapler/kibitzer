//! `rust-ignored-error`: a discarded or panicking error in non-test Rust.
//!
//! Two shapes: `let _ = <call>;` (a `Result` silently dropped) and a bare `.unwrap()` (a panic
//! standing in for error handling). `.expect("msg")` is deliberately not flagged: the message
//! documents the invariant. Test code (`#[cfg(test)]`, `#[test]`, `tests/`, `benches/`,
//! `examples/`) is skipped. See docs/rust-ignored-error-false-positives.md.

use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::rust_common::{in_test_code, is_test_path, one_line, text};
use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::tree_walk::walk_preorder;

const NAME: &str = "rust-ignored-error";

/// `.unwrap()` on these receivers cannot fail except by poisoning/propagated panic, which is
/// the idiomatic response (`std::sync` guards, thread joins).
const IDIOMATIC_UNWRAP_RECEIVERS: &[&str] =
    &["lock", "read", "write", "join", "try_lock", "borrow"];
/// `let _ = <call>` left alone: callees returning a non-`Result`, and channel `send`, whose only
/// error is a dropped receiver (the dominant, intentional pattern in the backtest corpus).
const INFALLIBLE_DISCARDS: &[&str] = &[
    "drop",
    "clone",
    "take",
    "replace",
    "swap",
    "insert",
    "push",
    "remove",
    "pop",
    "len",
    "get",
    "entry",
    "or_insert",
    "set",
    "store",
    "load",
    "fetch_add",
    "fetch_sub",
    "spawn",
    "join",
    "send",
    "try_send",
    "recv",
    "try_recv",
    "next",
    "from_raw",
];

pub struct RustIgnoredErrorChecker;

impl Checker for RustIgnoredErrorChecker {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        "flags `let _ = <call>` and bare `.unwrap()` in non-test Rust"
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
            .context("rust-ignored-error requires a parsed tree")?;
        let mut findings = Vec::new();
        walk_preorder(tree.root_node(), &mut |node| {
            let finding = match node.kind() {
                "let_declaration" => ignored_let(node, ctx.source),
                "call_expression" => bare_unwrap(node, ctx.source),
                _ => None,
            };
            if let Some(message) = finding
                && !in_test_code(node, ctx.source)
            {
                findings.push(Finding {
                    line: node.start_position().row + 1,
                    message,
                });
            }
            true
        });
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(RustIgnoredErrorChecker)])
}

/// Last path/method segment of a call's callee (`a::b::f` -> `f`, `x.send` -> `send`).
fn callee_name<'a>(call: Node, source: &'a str) -> &'a str {
    let callee = call
        .child_by_field_name("function")
        .map_or("", |f| text(f, source));
    callee
        .rsplit(['.', ':'])
        .next()
        .unwrap_or(callee)
        .split('<')
        .next()
        .unwrap_or("")
}

fn ignored_let(decl: Node, source: &str) -> Option<String> {
    if text(decl.child_by_field_name("pattern")?, source) != "_" {
        return None;
    }
    let value = decl.child_by_field_name("value")?;
    // `.await` and `?` wrap the real call; look through them.
    let mut call = value;
    while matches!(call.kind(), "await_expression" | "try_expression") {
        call = call.named_child(0)?;
    }
    if value.kind() == "try_expression" || call.kind() != "call_expression" {
        return None;
    }
    let name = callee_name(call, source);
    // `ExceptionHandling::Report`: the callee already reports the exception itself.
    if INFALLIBLE_DISCARDS.contains(&name)
        || text(call, source).contains("ExceptionHandling::Report")
    {
        return None;
    }
    Some(format!(
        "`let _ = {}` discards a possible error — handle it, log it, or comment why it is safe to ignore",
        one_line(text(value, source), 60)
    ))
}

fn bare_unwrap(call: Node, source: &str) -> Option<String> {
    let func = call.child_by_field_name("function")?;
    if func.kind() != "field_expression"
        || text(func.child_by_field_name("field")?, source) != "unwrap"
    {
        return None;
    }
    if call.child_by_field_name("arguments")?.named_child_count() != 0 {
        return None;
    }
    let receiver = func.child_by_field_name("value")?;
    if is_infallible_by_construction(receiver, source) {
        return None;
    }
    if receiver.kind() == "call_expression"
        && IDIOMATIC_UNWRAP_RECEIVERS.contains(&callee_name(receiver, source))
    {
        return None;
    }
    Some("bare `.unwrap()` panics on error — propagate with `?`, or use `.expect(\"why this cannot fail\")`".to_string())
}

/// `write!`/`writeln!` into a `String`/`Vec` cannot fail, and a call whose arguments are all
/// literals (`Regex::new("a+")`, `Url::parse("http://x")`) is a startup invariant that
/// `.unwrap()` states as well as `.expect()` would.
fn is_infallible_by_construction(receiver: Node, source: &str) -> bool {
    match receiver.kind() {
        "macro_invocation" => {
            let name = receiver
                .child_by_field_name("macro")
                .map_or("", |m| text(m, source));
            matches!(name, "write" | "writeln")
        }
        "call_expression" => receiver
            .child_by_field_name("arguments")
            .is_some_and(|args| {
                let mut cursor = args.walk();
                let mut named = args.named_children(&mut cursor);
                let mut any = false;
                let all_literal = named.all(|a| {
                    any = true;
                    matches!(
                        a.kind(),
                        "string_literal"
                            | "raw_string_literal"
                            | "integer_literal"
                            | "float_literal"
                            | "boolean_literal"
                    )
                });
                any && all_literal
            }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<Finding> {
        crate::test_support::check_rust_source(&RustIgnoredErrorChecker, src).unwrap()
    }

    #[test]
    fn flags_let_underscore_of_a_call() {
        let f = run("fn f() { let _ = std::fs::remove_file(\"x\"); }");
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("let _ ="));
    }

    #[test]
    fn flags_method_call_discard() {
        assert_eq!(run("fn f(w: W) { let _ = w.flush(); }").len(), 1);
    }

    #[test]
    fn recv_next_from_raw_and_reported_exceptions_are_not_flagged() {
        assert!(
            run("fn f(r: R) { let _ = r.recv(); let _ = it.next(); let _ = Box::from_raw(p); }")
                .is_empty()
        );
        assert!(run("fn f(cb: C) { let _ = cb.Call_(cx, ExceptionHandling::Report); }").is_empty());
    }

    #[test]
    fn channel_send_discard_is_not_flagged() {
        assert!(run("fn f(tx: Tx) { let _ = tx.send(1); }").is_empty());
    }

    #[test]
    fn plain_value_and_infallible_discards_are_not_flagged() {
        assert!(run("fn f(x: i32) { let _ = x; }").is_empty());
        assert!(run("fn f(v: V) { let _ = v.clone(); let _ = drop(v); }").is_empty());
    }

    #[test]
    fn named_underscore_bindings_are_not_flagged() {
        assert!(run("fn f() { let _guard = g(); }").is_empty());
    }

    #[test]
    fn flags_bare_unwrap_in_main() {
        let f = run("fn main() { let x = parse().unwrap(); }");
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("unwrap"));
    }

    #[test]
    fn expect_and_lock_unwrap_are_not_flagged() {
        assert!(run("fn f() { let x = parse().expect(\"valid\"); }").is_empty());
        assert!(run("fn f(m: M) { let g = m.lock().unwrap(); }").is_empty());
    }

    #[test]
    fn write_macro_and_literal_argument_unwraps_are_not_flagged() {
        assert!(run("fn f(out: &mut String) { writeln!(out, \"x\").unwrap(); }").is_empty());
        assert!(run("fn f() { let r = Regex::new(\"a+\").unwrap(); }").is_empty());
        assert_eq!(
            run("fn f(p: &str) { let r = Regex::new(p).unwrap(); }").len(),
            1
        );
    }

    #[test]
    fn test_code_is_skipped() {
        assert!(run("#[test]\nfn t() { let x = parse().unwrap(); let _ = f(); }").is_empty());
        assert!(run("#[cfg(test)]\nmod t { fn h() { parse().unwrap(); } }").is_empty());
    }

    #[test]
    fn tests_directory_is_skipped() {
        let src = "fn f() { parse().unwrap(); }";
        let tree = crate::checker::GrammarCache::new()
            .parse(Language::Rust, src)
            .unwrap();
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        assert!(
            RustIgnoredErrorChecker
                .check(Path::new("crate/tests/it.rs"), &ctx)
                .unwrap()
                .is_empty()
        );
    }
}
