//! `rust-error-context`: a `?` on a known-foreign fallible call (fs, parse, process) inside an
//! anyhow-returning function with no `.context(..)`/`.map_err(..)`, so the error carries no hint of
//! what was attempted. Rules, limits and backtest: docs/rust-error-context-false-positives.md.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::rust_common::{
    SNIPPET_MAX, callee_last_segment, in_test_code, is_test_path, one_line, strip_await, text,
};
use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::tree_walk::walk_preorder;

const NAME: &str = "rust-error-context";
/// Calls whose errors come from the OS or a parser and carry no operand (path, input): the `?`
/// that most needs `.context(..)`. A project-internal call is not flagged: its error is built
/// with a message, and per-file analysis cannot tell which internal calls are bare.
const FOREIGN_FALLIBLE: &[&str] = &[
    "read",
    "read_to_string",
    "read_to_end",
    "read_line",
    "read_dir",
    "read_exact",
    "write",
    "write_all",
    "flush",
    "open",
    "create",
    "create_dir",
    "create_dir_all",
    "remove_file",
    "remove_dir",
    "remove_dir_all",
    "copy",
    "rename",
    "metadata",
    "canonicalize",
    "current_dir",
    "current_exe",
    "set_nonblocking",
    "parse",
    "from_str",
    "from_utf8",
    "connect",
    "bind",
    "accept",
    "spawn",
    "output",
    "status",
    "wait",
    "set_permissions",
    "from_reader",
    "from_slice",
    "to_string_pretty",
    "to_vec",
    "to_writer",
    "send_to",
    "recv_from",
    "try_from",
    "try_into",
    "from_env",
    "var",
];
const CONTEXT_METHODS: &[&str] = &[
    "context",
    "with_context",
    "wrap_err",
    "wrap_err_with",
    "map_err",
    "ok_or",
    "ok_or_else",
    "ok_or_eyre",
];

pub struct RustErrorContextChecker;

impl Checker for RustErrorContextChecker {
    fn name(&self) -> &str {
        NAME
    }

    fn description(&self) -> &str {
        "flags a `?` in an anyhow-returning Rust function that adds no `.context()`"
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
            .context("rust-error-context requires a parsed tree")?;
        let imports_result = imports_anyhow_result(ctx.source);
        let anyhow_fns = same_file_anyhow_fns(tree.root_node(), ctx.source, imports_result);
        let mut findings = Vec::new();
        walk_preorder(tree.root_node(), &mut |node| {
            if node.kind() == "try_expression"
                && let Some(f) = check_try(node, ctx.source, imports_result, &anyhow_fns)
            {
                findings.push(f);
            }
            true
        });
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(RustErrorContextChecker)])
}

fn imports_anyhow_result(source: &str) -> bool {
    source.lines().any(|l| {
        let l = l.trim();
        l.starts_with("use ")
            && (l.contains("anyhow::Result")
                || l.contains("eyre::Result")
                || (l.contains("anyhow::{") && l.contains("Result")))
    })
}

fn is_anyhow_return(func: Node, source: &str, imports_result: bool) -> bool {
    let Some(ty) = func.child_by_field_name("return_type") else {
        return false;
    };
    let ty = text(ty, source);
    if ty.contains("anyhow::") || ty.contains("eyre::") {
        return true;
    }
    // `Result<T>` with a single type argument and an imported anyhow `Result`.
    imports_result && ty.starts_with("Result<") && !top_level_comma(&ty["Result<".len()..])
}

fn top_level_comma(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            ',' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

fn check_try(
    node: Node,
    source: &str,
    imports_result: bool,
    anyhow_fns: &HashSet<String>,
) -> Option<Finding> {
    let mut cur = node.parent();
    let func = loop {
        let n = cur?;
        match n.kind() {
            "closure_expression" | "async_block" => return None,
            "function_item" => break n,
            _ => cur = n.parent(),
        }
    };
    if !is_anyhow_return(func, source, imports_result) || in_test_code(node, source) {
        return None;
    }
    let operand = node.named_child(0)?;
    if has_context(operand, source)
        || calls_same_file_anyhow_fn(operand, source, anyhow_fns)
        || !is_foreign_fallible(operand, source)
    {
        return None;
    }
    Some(Finding {
        line: node.start_position().row + 1,
        message: format!(
            "`{}?` propagates an anyhow error with no context — add `.context(\"what failed\")` or `.with_context(|| ..)`",
            one_line(text(operand, source), SNIPPET_MAX)
        ),
    })
}

/// Names of this file's functions that already return an anyhow error: their errors are built
/// with a message, so `?` on a call to one is not a context-free foreign error.
fn same_file_anyhow_fns(root: Node, source: &str, imports_result: bool) -> HashSet<String> {
    let mut names = HashSet::new();
    walk_preorder(root, &mut |node| {
        if node.kind() == "function_item"
            && is_anyhow_return(node, source, imports_result)
            && let Some(n) = node.child_by_field_name("name")
        {
            names.insert(text(n, source).to_string());
        }
        true
    });
    names
}

fn calls_same_file_anyhow_fn(operand: Node, source: &str, names: &HashSet<String>) -> bool {
    let operand = strip_await(operand);
    let Some(func) = operand
        .child_by_field_name("function")
        .filter(|_| operand.kind() == "call_expression")
    else {
        return false;
    };
    let callee = text(func, source);
    names.contains(callee_last_segment(callee))
}

fn is_foreign_fallible(operand: Node, source: &str) -> bool {
    let operand = strip_await(operand);
    let Some(func) = operand
        .child_by_field_name("function")
        .filter(|_| operand.kind() == "call_expression")
    else {
        return false;
    };
    let callee = text(func, source);
    let last = callee_last_segment(callee);
    let foreign_path = [
        "std::",
        "serde_json::",
        "toml::",
        "serde_yaml::",
        "tokio::fs::",
        "tokio::process::",
    ]
    .iter()
    .any(|p| callee.starts_with(p));
    let is_method = func.kind() == "field_expression";
    // A bare path call like `index::read(..)` is project code that merely shares a name.
    let known_name = FOREIGN_FALLIBLE.contains(&last) && (is_method || !callee.contains("::"));
    // Failing to write to the terminal needs no "while doing X" context.
    let to_terminal = is_method && {
        let recv = callee.to_ascii_lowercase();
        recv.contains("stdout") || recv.contains("stderr")
    };
    (known_name || foreign_path) && !to_terminal
}

/// The chain ends in a context-adding method (looking through a trailing `.await`).
fn has_context(operand: Node, source: &str) -> bool {
    let operand = strip_await(operand);
    if operand.kind() != "call_expression" {
        return false;
    }
    let Some(func) = operand.child_by_field_name("function") else {
        return false;
    };
    func.kind() == "field_expression"
        && func
            .child_by_field_name("field")
            .is_some_and(|f| CONTEXT_METHODS.contains(&text(f, source)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<Finding> {
        crate::test_support::check_rust_source(&RustErrorContextChecker, src).unwrap()
    }

    #[test]
    fn flags_bare_question_mark_in_anyhow_fn() {
        let f = run("fn f() -> anyhow::Result<()> { std::fs::read(\"x\")?; Ok(()) }");
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("context"));
    }

    /// Each suppressor must beat a `std::fs::read("x")?` that would otherwise fire; the bare
    /// control proves the operand really is one the checker flags.
    #[test]
    fn context_adding_chains_suppress_a_flagged_operand() {
        let wrap = |op: &str| format!("fn f() -> anyhow::Result<()> {{ {op}?; Ok(()) }}");
        assert_eq!(run(&wrap("std::fs::read(\"x\")")).len(), 1, "control");
        for op in [
            "std::fs::read(\"x\").context(\"read\")",
            "std::fs::read(\"x\").with_context(|| \"read\")",
            "std::fs::read(\"x\").map_err(|e| MyErr::from(e))",
            "std::fs::read(\"x\").ok_or(MyErr)",
        ] {
            assert!(run(&wrap(op)).is_empty(), "{op} should be accepted");
        }
    }

    #[test]
    fn awaited_context_is_accepted() {
        let wrap = |op: &str| format!("async fn f() -> anyhow::Result<()> {{ {op}?; Ok(()) }}");
        assert_eq!(
            run(&wrap("tokio::fs::read(\"x\").await")).len(),
            1,
            "control"
        );
        assert!(run(&wrap("tokio::fs::read(\"x\").await.context(\"x\")")).is_empty());
        assert!(run(&wrap("tokio::fs::read(\"x\").context(\"y\").await")).is_empty());
    }

    #[test]
    fn calls_to_same_file_anyhow_functions_are_skipped() {
        assert!(run("fn g() -> anyhow::Result<()> { bail!(\"x\") }\nfn f() -> anyhow::Result<()> { g()?; Ok(()) }").is_empty());
    }

    #[test]
    fn project_internal_calls_are_not_flagged() {
        assert!(
            run("fn f() -> anyhow::Result<()> { args.searcher()?; helper(x)?; Ok(()) }").is_empty()
        );
    }

    #[test]
    fn same_named_project_paths_and_terminal_writes_are_not_flagged() {
        assert!(run("fn f() -> anyhow::Result<()> { index::read(&a)?; stdout.write_all(b\"x\")?; Ok(()) }").is_empty());
        assert_eq!(
            run("fn f() -> anyhow::Result<()> { out.write_all(b\"x\")?; Ok(()) }").len(),
            1
        );
    }

    #[test]
    fn typed_error_functions_are_skipped() {
        assert!(run("fn f() -> Result<(), MyError> { read()?; Ok(()) }").is_empty());
    }

    #[test]
    fn bare_result_alias_counts_only_with_an_anyhow_import() {
        let body = "fn f() -> Result<()> { read()?; Ok(()) }";
        assert!(run(body).is_empty());
        assert_eq!(run(&format!("use anyhow::Result;\n{body}")).len(), 1);
    }

    #[test]
    fn closures_and_tests_are_skipped() {
        assert!(
            run(
                "fn f() -> anyhow::Result<()> { let c = || -> Result<()> { read()?; Ok(()) }; c() }"
            )
            .is_empty()
        );
        assert!(run("#[test]\nfn t() -> anyhow::Result<()> { read()?; Ok(()) }").is_empty());
    }
}
