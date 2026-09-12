use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::file_size::is_generated;

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
        // Generated files always emit whatever wrapping shape their template
        // hard-codes — boilerplate, not a chosen convention — and nobody hand-edits
        // generated code to "fix" its wrapping style anyway.
        if is_generated(ctx.source) {
            return Ok(Vec::new());
        }

        let tree = ctx
            .tree
            .context("go-error-context checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();
        let root = tree.root_node();

        let wrap_count = count_wrapping_occurrences(root, src);
        if wrap_count == 0 {
            return Ok(Vec::new());
        }

        let mut findings = Vec::new();
        collect_bare_passthroughs(root, src, &mut findings);
        if !wrapping_convention_established(wrap_count, findings.len()) {
            return Ok(Vec::new());
        }
        Ok(findings)
    }
}

/// Counts calls to `fmt.Errorf(...)` anywhere in the file that carry a `%w` verb in
/// their format string — each is independent signal that this codebase wraps errors
/// on purpose.
fn count_wrapping_occurrences(node: Node, src: &[u8]) -> usize {
    let mut count = if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && function.kind() == "selector_expression"
        && let Some(operand) = function.child_by_field_name("operand")
        && let Some(field) = function.child_by_field_name("field")
        && operand.utf8_text(src) == Ok("fmt")
        && field.utf8_text(src) == Ok("Errorf")
        && let Some(args) = node.child_by_field_name("arguments")
    {
        let mut cursor = args.walk();
        args.children(&mut cursor).any(|arg| {
            matches!(
                arg.kind(),
                "interpreted_string_literal" | "raw_string_literal"
            ) && arg
                .utf8_text(src)
                .map(|s| s.contains("%w"))
                .unwrap_or(false)
        }) as usize
    } else {
        0
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_wrapping_occurrences(child, src);
    }
    count
}

/// One `%w` call is weak evidence next to many untouched bare passthroughs (a lone
/// wrapper in a 3000-line file full of `if err != nil { return err }`); two or more
/// independent sites are unlikely to be accidental regardless of file size.
fn wrapping_convention_established(wrap_count: usize, bare_count: usize) -> bool {
    wrap_count >= 2 || bare_count <= wrap_count
}

fn collect_bare_passthroughs(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    if node.kind() == "if_statement"
        && let Some(err_name) = bare_err_passthrough_name(node, src)
        && !wrapped_earlier_in_enclosing_function(node, err_name, src)
    {
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
/// expression) falls through untouched — see the module doc comment. Returns the
/// identifier's name (needed by `wrapped_earlier_in_enclosing_function`) rather than a
/// bare bool.
fn bare_err_passthrough_name<'a>(if_stmt: Node, src: &'a [u8]) -> Option<&'a str> {
    let mut condition = if_stmt.child_by_field_name("condition")?;
    while condition.kind() == "parenthesized_expression" {
        let mut cursor = condition.walk();
        condition = condition
            .children(&mut cursor)
            .find(|c| c.kind() != "(" && c.kind() != ")")?;
    }
    if condition.kind() != "binary_expression" {
        return None;
    }
    let left = condition.child_by_field_name("left")?;
    let right = condition.child_by_field_name("right")?;
    if left.kind() != "identifier" || right.kind() != "nil" {
        return None;
    }
    let mut cursor = condition.walk();
    if !condition.children(&mut cursor).any(|c| c.kind() == "!=") {
        return None;
    }
    let err_name = left.utf8_text(src).ok()?;
    let consequence = if_stmt.child_by_field_name("consequence")?;
    consequence_returns_only(consequence, src, err_name).then_some(err_name)
}

/// True when `consequence` is a `block` whose only statement is `return <name>`.
fn consequence_returns_only(consequence: Node, src: &[u8], name: &str) -> bool {
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
    let [only] = statements.as_slice() else {
        return false;
    };
    if only.kind() != "return_statement" {
        return false;
    }
    // return_statement's returned expressions aren't exposed as a named field in
    // this grammar, so scan direct children for the expression_list instead.
    let mut cursor = only.walk();
    let exprs: Vec<Node> = only
        .children(&mut cursor)
        .filter(|n| n.kind() == "expression_list")
        .collect();
    exprs.len() == 1 && single_identifier_matches(exprs[0], src, name)
}

/// True when `err_name` was already reassigned, earlier in the *same* enclosing
/// function/method/closure body (anywhere textually before `if_stmt`, regardless of
/// intervening block nesting — not just the immediately preceding statement), via a
/// `fmt.Errorf(..., "%w", ...)` call that references `err_name` as one of its
/// arguments. Deliberately narrow and purely textual/positional — it cannot see a wrap
/// that happens behind a callee or a passed closure (a call like `err =
/// doThing(func() error { ... return fmt.Errorf(...) ... })` looks, from here, just
/// like `err = <call>`), which is why this does not resolve the two originally-logged
/// false positives (`pkg/kubelet/cm/dra/manager.go`, `session/workspace_peers.go`) —
/// both wrap inside a called closure/function, a case genuinely out of reach without
/// real interprocedural analysis. This only ever narrows (suppresses), never widens,
/// so it can't introduce a new false positive.
fn wrapped_earlier_in_enclosing_function(if_stmt: Node, err_name: &str, src: &[u8]) -> bool {
    let Some(body) = enclosing_function_body(if_stmt) else {
        return false;
    };
    let mut found = false;
    find_prior_wrap(body, err_name, src, if_stmt.start_byte(), &mut found);
    found
}

fn enclosing_function_body(node: Node) -> Option<Node> {
    let mut current = node.parent()?;
    loop {
        if matches!(
            current.kind(),
            "function_declaration" | "method_declaration" | "func_literal"
        ) {
            return current.child_by_field_name("body");
        }
        current = current.parent()?;
    }
}

fn find_prior_wrap(node: Node, err_name: &str, src: &[u8], before: usize, found: &mut bool) {
    if *found || node.start_byte() >= before {
        return;
    }
    if is_wrap_reassignment(node, err_name, src) {
        *found = true;
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        find_prior_wrap(child, err_name, src, before, found);
        if *found {
            return;
        }
    }
}

/// True when `node` is `<err_name> = fmt.Errorf(..., "%w", ..., <err_name>, ...)` or
/// `<err_name> := fmt.Errorf(...)` (an `assignment_statement` or
/// `short_var_declaration` whose LHS is exactly `err_name` and whose RHS is a
/// `%w`-wrapping `fmt.Errorf` call referencing `err_name` among its arguments).
fn is_wrap_reassignment(node: Node, err_name: &str, src: &[u8]) -> bool {
    let (left, right) = match node.kind() {
        "assignment_statement" => (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ),
        "short_var_declaration" => (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ),
        _ => return false,
    };
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    if !single_identifier_matches(left, src, err_name) {
        return false;
    }
    let mut cursor = right.walk();
    right
        .named_children(&mut cursor)
        .any(|expr| is_wrapf_call_referencing(expr, err_name, src))
}

fn is_wrapf_call_referencing(node: Node, err_name: &str, src: &[u8]) -> bool {
    node.kind() == "call_expression"
        && is_fmt_errorf_callee(node, src)
        && node
            .child_by_field_name("arguments")
            .is_some_and(|args| errorf_args_wrap_and_reference(args, err_name, src))
}

fn is_fmt_errorf_callee(call: Node, src: &[u8]) -> bool {
    call.child_by_field_name("function")
        .is_some_and(|function| {
            function.kind() == "selector_expression"
                && function
                    .child_by_field_name("operand")
                    .is_some_and(|o| o.utf8_text(src) == Ok("fmt"))
                && function
                    .child_by_field_name("field")
                    .is_some_and(|f| f.utf8_text(src) == Ok("Errorf"))
        })
}

fn errorf_args_wrap_and_reference(args: Node, err_name: &str, src: &[u8]) -> bool {
    let mut cursor = args.walk();
    let mut has_wrap_verb = false;
    let mut has_name = false;
    for arg in args.children(&mut cursor) {
        if matches!(
            arg.kind(),
            "interpreted_string_literal" | "raw_string_literal"
        ) && arg
            .utf8_text(src)
            .map(|s| s.contains("%w"))
            .unwrap_or(false)
        {
            has_wrap_verb = true;
        }
        if arg.kind() == "identifier" && arg.utf8_text(src) == Ok(err_name) {
            has_name = true;
        }
    }
    has_wrap_verb && has_name
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

    #[test]
    fn detects_wrapping_convention_in_raw_string_literal() {
        let src = "package main\nimport \"fmt\"\nfunc wrap(err error) error {\n\treturn fmt.Errorf(`wrap: %w`, err)\n}\nfunc g() error {\n\terr := doThing()\n\tif err != nil {\n\t\treturn err\n\t}\n\treturn nil\n}\n";
        let findings = check_source(src).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn flags_parenthesized_condition() {
        let src = format!(
            "package main\nimport \"fmt\"\n{WRAPPING_CONVENTION}func g() error {{\n\terr := doThing()\n\tif (err != nil) {{\n\t\treturn err\n\t}}\n\treturn nil\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn flags_multi_value_bare_passthrough() {
        let src = format!(
            "package main\nimport \"fmt\"\n{WRAPPING_CONVENTION}func g() (int, error) {{\n\terr := doThing()\n\tif err != nil {{\n\t\treturn 0, err\n\t}}\n\treturn 0, nil\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert_eq!(findings.len(), 1);
    }

    /// Regression for a false positive found backtesting `stapler-squad`'s
    /// `session/ent/claudesession_query.go`: an ent-generated file's boilerplate `%w`
    /// helper isn't evidence anyone chose a wrapping convention for the file.
    #[test]
    fn does_not_flag_generated_file() {
        let src = format!(
            "// Code generated by ent, DO NOT EDIT.\n\npackage main\nimport \"fmt\"\n{WRAPPING_CONVENTION}func g() error {{\n\terr := doThing()\n\tif err != nil {{\n\t\treturn err\n\t}}\n\treturn nil\n}}\n"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    /// Regression for a false positive found backtesting kubernetes/kubernetes'
    /// vendored `vendor/golang.org/x/net/http2/transport.go`: one incidental `%w`
    /// call lost among many bare passthroughs isn't a chosen convention. Not fixed
    /// via a `vendor/`-path exclusion since `kibitzer run`'s normal walk already
    /// skips `vendor/` via `check::SKIP_DIRS` — this covers direct `check native`
    /// invocation, which bypasses that walk.
    #[test]
    fn does_not_flag_lone_wrap_among_many_bare_passthroughs() {
        let mut src = format!("package main\nimport \"fmt\"\n{WRAPPING_CONVENTION}");
        for i in 0..4 {
            src.push_str(&format!(
                "func g{i}() error {{\n\terr := doThing()\n\tif err != nil {{\n\t\treturn err\n\t}}\n\treturn nil\n}}\n"
            ));
        }
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    /// Two or more independent wrap sites are unlikely to be accidental, so a
    /// straggler is still flagged even though bare passthroughs outnumber wraps.
    #[test]
    fn flags_genuine_inconsistency_with_multiple_wrap_sites() {
        let src = "package main\nimport \"fmt\"\n\
                    func wrap1(err error) error {\n\treturn fmt.Errorf(\"wrap1: %w\", err)\n}\n\
                    func wrap2(err error) error {\n\treturn fmt.Errorf(\"wrap2: %w\", err)\n}\n\
                    func g() error {\n\terr := doThing()\n\tif err != nil {\n\t\treturn err\n\t}\n\treturn nil\n}\n";
        let findings = check_source(src).unwrap();
        assert_eq!(findings.len(), 1);
    }

    /// `err` is wrapped inside an earlier `if` block (not the immediately preceding
    /// statement — there's an intervening block boundary) before the bare
    /// `if err != nil { return err }` a few lines down. This is the narrow,
    /// same-function backward scan `wrapped_earlier_in_enclosing_function` covers —
    /// deliberately does NOT close the two real corpus false positives this checker
    /// still has open (`pkg/kubelet/cm/dra/manager.go`, `session/workspace_peers.go`),
    /// both of which wrap behind a called closure/function, not a textual
    /// reassignment visible in the same function body — see
    /// docs/go-error-context-false-positives.md's `## Log`.
    #[test]
    fn does_not_flag_when_wrapped_earlier_in_the_same_function() {
        let src = "package main\nimport \"fmt\"\n\
                    func wrap1(err error) error {\n\treturn fmt.Errorf(\"wrap1: %w\", err)\n}\n\
                    func wrap2(err error) error {\n\treturn fmt.Errorf(\"wrap2: %w\", err)\n}\n\
                    func g() error {\n\
                    \terr := doThing()\n\
                    \tif err != nil {\n\
                    \t\terr = fmt.Errorf(\"context: %w\", err)\n\
                    \t}\n\
                    \tif err != nil {\n\
                    \t\treturn err\n\
                    \t}\n\
                    \treturn nil\n\
                    }\n";
        let findings = check_source(src).unwrap();
        assert!(findings.is_empty());
    }

    /// True-positive guard: a *different*, never-wrapped identifier in the same
    /// function must still flag — the backward scan must not suppress every bare
    /// passthrough in a function just because some other variable was wrapped.
    #[test]
    fn still_flags_a_different_unwrapped_identifier_in_the_same_function() {
        let src = "package main\nimport \"fmt\"\n\
                    func wrap1(err error) error {\n\treturn fmt.Errorf(\"wrap1: %w\", err)\n}\n\
                    func wrap2(err error) error {\n\treturn fmt.Errorf(\"wrap2: %w\", err)\n}\n\
                    func g() error {\n\
                    \terr := doThing()\n\
                    \tif err != nil {\n\
                    \t\terr = fmt.Errorf(\"context: %w\", err)\n\
                    \t}\n\
                    \tif err != nil {\n\
                    \t\treturn err\n\
                    \t}\n\
                    \terr2 := doOtherThing()\n\
                    \tif err2 != nil {\n\
                    \t\treturn err2\n\
                    \t}\n\
                    \treturn nil\n\
                    }\n";
        let findings = check_source(src).unwrap();
        assert_eq!(findings.len(), 1);
    }
}
