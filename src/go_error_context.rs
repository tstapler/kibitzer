use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Flags `if err != nil { return err }` — a bare passthrough that discards the call
/// site's context — but only in files that already demonstrate an `fmt.Errorf(...,
/// "%w", err)` wrapping convention elsewhere. Without that signal we can't tell
/// whether the project wraps errors at all, so staying silent avoids false positives
/// in codebases that intentionally propagate errors unwrapped.
///
/// Advisory by default (wire it up with `"severity": "advisory"` in `.claude/inspect.json`):
/// this is a style nudge, not a correctness bug, and the heuristic is intentionally
/// narrow — see `docs/go-error-context-false-positives.md` for the documented scope
/// gaps (sentinel comparisons, errors.Is/As chains, defer-based handling, named
/// returns) that this checker deliberately does not flag.
pub struct ErrorContextChecker;

impl Checker for ErrorContextChecker {
    fn name(&self) -> &str {
        "go-error-context"
    }

    fn description(&self) -> &str {
        "flags bare `if err != nil { return err }` passthroughs in files that already use fmt.Errorf(\"%w\", ...) elsewhere"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("go-error-context checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();
        let root = tree.root_node();

        if !has_wrapping_convention(root, src) {
            return Ok(Vec::new());
        }

        let mut findings = Vec::new();
        collect_bare_passthroughs(root, src, &mut findings);
        Ok(findings)
    }
}

/// True if `fmt.Errorf(...)` is called anywhere in the file with a `%w` verb in its
/// format string — the signal that this codebase wraps errors on purpose.
fn has_wrapping_convention(node: Node, src: &[u8]) -> bool {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && function.kind() == "selector_expression"
        && let Some(operand) = function.child_by_field_name("operand")
        && let Some(field) = function.child_by_field_name("field")
        && operand.utf8_text(src) == Ok("fmt")
        && field.utf8_text(src) == Ok("Errorf")
        && let Some(args) = node.child_by_field_name("arguments")
    {
        let mut cursor = args.walk();
        for arg in args.children(&mut cursor) {
            if arg.kind() == "interpreted_string_literal"
                && arg
                    .utf8_text(src)
                    .map(|s| s.contains("%w"))
                    .unwrap_or(false)
            {
                return true;
            }
        }
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| has_wrapping_convention(child, src))
}

fn collect_bare_passthroughs(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    if node.kind() == "if_statement" && is_bare_err_passthrough(node, src) {
        findings.push(Finding {
            line: node.start_position().row + 1,
            message: "returns the error unwrapped despite this file's fmt.Errorf(\"%w\", ...) \
                      convention — consider wrapping with context here too"
                .to_string(),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_bare_passthroughs(child, src, findings);
    }
}

/// Matches exactly `if <id> != nil { return <id> }` — a condition comparing one
/// identifier against `nil` with `!=`, and a consequence block whose only statement
/// bounces that same identifier back unchanged. Anything else (sentinel comparisons,
/// `errors.Is`/`errors.As` calls, extra statements, bare `return` with no
/// expression) falls through untouched — see the module doc comment.
fn is_bare_err_passthrough(if_stmt: Node, src: &[u8]) -> bool {
    let Some(condition) = if_stmt.child_by_field_name("condition") else {
        return false;
    };
    if condition.kind() != "binary_expression" {
        return false;
    }
    let Some(left) = condition.child_by_field_name("left") else {
        return false;
    };
    let Some(right) = condition.child_by_field_name("right") else {
        return false;
    };
    if left.kind() != "identifier" || right.kind() != "nil" {
        return false;
    }
    let mut cursor = condition.walk();
    if !condition.children(&mut cursor).any(|c| c.kind() == "!=") {
        return false;
    }
    let Ok(err_name) = left.utf8_text(src) else {
        return false;
    };

    let Some(consequence) = if_stmt.child_by_field_name("consequence") else {
        return false;
    };
    if consequence.kind() != "block" {
        return false;
    }
    let mut cursor = consequence.walk();
    let statements: Vec<Node> = consequence
        .children(&mut cursor)
        .filter(|n| n.kind() == "statement_list")
        .flat_map(|list| {
            let mut inner_cursor = list.walk();
            list.children(&mut inner_cursor)
                .collect::<Vec<_>>()
                .into_iter()
        })
        .collect();
    if statements.len() != 1 {
        return false;
    }
    let only = statements[0];
    if only.kind() != "return_statement" {
        return false;
    }
    let Some(expr_list) = only.child_by_field_name("child") else {
        // return_statement's returned expressions aren't a named field in this
        // grammar; fall back to scanning direct children for the expression_list.
        let mut inner_cursor = only.walk();
        let exprs: Vec<Node> = only
            .children(&mut inner_cursor)
            .filter(|n| n.kind() == "expression_list")
            .collect();
        return exprs.len() == 1 && single_identifier_matches(exprs[0], src, err_name);
    };
    single_identifier_matches(expr_list, src, err_name)
}

fn single_identifier_matches(expr_list: Node, src: &[u8], name: &str) -> bool {
    let mut cursor = expr_list.walk();
    let idents: Vec<Node> = expr_list
        .children(&mut cursor)
        .filter(|n| n.kind() == "identifier")
        .collect();
    idents.len() == 1 && idents[0].utf8_text(src) == Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .context("loading tree-sitter-go grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing Go source with tree-sitter")?;
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        ErrorContextChecker.check(Path::new("<source>"), &ctx)
    }

    const WRAPPING_CONVENTION: &str =
        "func wrap(err error) error {\n\treturn fmt.Errorf(\"wrap: %w\", err)\n}\n";

    #[test]
    fn flags_bare_passthrough_when_wrapping_convention_exists() {
        let src = format!(
            "package main\nimport \"fmt\"\n{WRAPPING_CONVENTION}func g() error {{\n\terr := doThing()\n\tif err != nil {{\n\t\treturn err\n\t}}\n\treturn nil\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn does_not_flag_when_no_wrapping_convention_in_file() {
        let src = "package main\nfunc g() error {\n\terr := doThing()\n\tif err != nil {\n\t\treturn err\n\t}\n\treturn nil\n}\n";
        let findings = check_source(src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_sentinel_comparison() {
        let src = format!(
            "package main\nimport (\n\t\"fmt\"\n\t\"io\"\n)\n{WRAPPING_CONVENTION}func g() error {{\n\terr := doThing()\n\tif err == io.EOF {{\n\t\treturn err\n\t}}\n\treturn nil\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_errors_is_chain() {
        let src = format!(
            "package main\nimport (\n\t\"errors\"\n\t\"fmt\"\n)\n{WRAPPING_CONVENTION}func g() error {{\n\terr := doThing()\n\tif errors.Is(err, ErrNotFound) {{\n\t\treturn err\n\t}}\n\treturn nil\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_defer_based_handling() {
        let src = format!(
            "package main\nimport \"fmt\"\n{WRAPPING_CONVENTION}func g() (err error) {{\n\tdefer func() {{\n\t\tif err != nil {{\n\t\t\terr = fmt.Errorf(\"deferred: %w\", err)\n\t\t}}\n\t}}()\n\treturn doThing()\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_named_return_bare_return() {
        let src = format!(
            "package main\nimport \"fmt\"\n{WRAPPING_CONVENTION}func g() (result int, err error) {{\n\terr = doThing()\n\tif err != nil {{\n\t\treturn\n\t}}\n\treturn\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_wrapped_return() {
        let src = format!(
            "package main\nimport \"fmt\"\n{WRAPPING_CONVENTION}func g() error {{\n\terr := doThing()\n\tif err != nil {{\n\t\treturn fmt.Errorf(\"g: %w\", err)\n\t}}\n\treturn nil\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn empty_file_produces_no_findings() {
        let findings = check_source("package main\n").unwrap();
        assert!(findings.is_empty());
    }
}
