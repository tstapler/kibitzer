use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Flags `throw new SomeException("message: " + e.getMessage())`-shaped constructions
/// inside a `catch` block: a *new* `...Exception`/`...Error` is thrown with at least one
/// argument, but the caught exception (`e`) is never passed as a direct constructor
/// argument — so its cause chain and original stack trace are silently dropped even
/// though the code clearly means to describe what went wrong. This is the mirror-image
/// bug from `java-error-context`'s bare-rethrow check (which flags *not adding* context)
/// — see SonarQube S1166 ("Exception handlers should preserve the original exceptions")
/// and PMD's `PreserveStackTrace`.
///
/// Deliberately narrow, matching this codebase's other exception-shaped checkers:
/// "preserved" means `e` (or `e.getCause()`, optionally cast — unwrapping one level,
/// e.g. rethrowing an `IOError`'s wrapped `IOException`, is also a legitimate chain) is
/// one of the new exception's own arguments (e.g. `new FooException("...", e)`) — not
/// merely referenced somewhere in the expression tree (`e.getMessage()`), since a
/// message-only reference is exactly the failure mode this checker targets. A
/// `docs/backtest-repos.md` run against `apache/cassandra` (2026-09-13) found the
/// `e.getCause()` shape accounted for 13 of 306 raw hits before this carve-out — a real,
/// if minority, false-positive source. Constructor-type matching is by simple name ending
/// in `Exception`/`Error` (no static type resolution), so a type aliased or imported under
/// an unrelated-looking name won't be caught — see this codebase's other native
/// checkers' doc comments for the same textual-heuristic tradeoff.
pub struct LostExceptionCauseChecker;

impl Checker for LostExceptionCauseChecker {
    fn name(&self) -> &str {
        "java-lost-exception-cause"
    }

    fn description(&self) -> &str {
        "flags `throw new XException(\"...\")` inside a catch block that never passes the caught exception as a cause"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Java)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.java"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("java-lost-exception-cause checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();
        let mut findings = Vec::new();
        walk(tree.root_node(), src, None, &mut findings);
        Ok(findings)
    }
}

fn catch_param_name<'a>(catch_clause: Node, src: &'a [u8]) -> Option<&'a str> {
    catch_clause
        .named_child(0)
        .filter(|n| n.kind() == "catch_formal_parameter")
        .and_then(|param| param.child_by_field_name("name"))
        .and_then(|name| name.utf8_text(src).ok())
}

fn looks_like_throwable_type(type_node: Node, src: &[u8]) -> bool {
    type_node
        .utf8_text(src)
        .map(|t| t.ends_with("Exception") || t.ends_with("Error"))
        .unwrap_or(false)
}

/// True if `node` is `<caught_name>.getCause()` — unwrapping one level to chain the
/// caught exception's own cause instead of the exception itself, e.g. rethrowing an
/// `IOError`'s wrapped `IOException`. Recognized as cause-preserving alongside a bare
/// `caught_name` reference (see `preserves_cause`).
fn is_get_cause_call(node: Node, src: &[u8], caught_name: &str) -> bool {
    node.kind() == "method_invocation"
        && node
            .child_by_field_name("object")
            .and_then(|o| o.utf8_text(src).ok())
            == Some(caught_name)
        && node
            .child_by_field_name("name")
            .and_then(|n| n.utf8_text(src).ok())
            == Some("getCause")
}

/// True if `caught_name` (or `caught_name.getCause()`, optionally cast) appears as one
/// of `object_creation_expression`'s own constructor arguments — shapes that actually
/// chain the original exception. A reference buried inside a larger expression
/// (`e.getMessage()`, `"x: " + e`) does not count; that's exactly the drop this checker
/// exists to catch.
fn preserves_cause(object_creation: Node, src: &[u8], caught_name: &str) -> bool {
    let Some(args) = object_creation.child_by_field_name("arguments") else {
        return false;
    };
    let mut cursor = args.walk();
    args.named_children(&mut cursor).any(|arg| {
        if arg.kind() == "identifier" && arg.utf8_text(src) == Ok(caught_name) {
            return true;
        }
        if is_get_cause_call(arg, src, caught_name) {
            return true;
        }
        if arg.kind() == "cast_expression"
            && let Some(value) = arg.child_by_field_name("value")
            && is_get_cause_call(value, src, caught_name)
        {
            return true;
        }
        false
    })
}

/// Walks the tree tracking `enclosing_catch_name`: the nearest enclosing catch clause's
/// bound exception variable, shadowed on entering a nested `catch_clause`'s own body
/// (so a `throw new X(...)` is only checked against the catch it's actually lexically
/// inside).
fn walk<'a>(
    node: Node<'a>,
    src: &'a [u8],
    enclosing_catch_name: Option<&'a str>,
    findings: &mut Vec<Finding>,
) {
    if node.kind() == "catch_clause" {
        let name = catch_param_name(node, src);
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                walk(child, src, name, findings);
            }
        }
        return;
    }

    if node.kind() == "throw_statement"
        && let Some(caught_name) = enclosing_catch_name
        && let Some(expr) = node.named_child(0)
        && expr.kind() == "object_creation_expression"
        && let Some(type_node) = expr.child_by_field_name("type")
        && looks_like_throwable_type(type_node, src)
        && let Some(args) = expr.child_by_field_name("arguments")
        && args.named_child_count() > 0
        && !preserves_cause(expr, src, caught_name)
    {
        findings.push(Finding {
            line: node.start_position().row + 1,
            message: format!(
                "throws a new exception without passing `{caught_name}` as its cause — the \
                 original stack trace is lost (SonarQube S1166)"
            ),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, enclosing_catch_name, findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .context("loading tree-sitter-java grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing Java source with tree-sitter")?;
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        LostExceptionCauseChecker.check(Path::new("<source>"), &ctx)
    }

    #[test]
    fn flags_message_only_wrap_via_get_message() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { throw new RuntimeException(\"wrap: \" + e.getMessage()); } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn flags_message_only_wrap_with_no_reference_to_caught_exception() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { throw new RuntimeException(\"boom\"); } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn allows_wrap_that_passes_caught_exception_as_cause() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { throw new RuntimeException(\"wrap\", e); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_cause_only_constructor() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { throw new RuntimeException(e); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_wrap_that_passes_caught_exceptions_get_cause() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOError e) { throw new RuntimeException(\"wrap\", e.getCause()); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_wrap_that_passes_a_cast_get_cause() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOError e) { throw new CorruptSSTableException((Exception) e.getCause(), filename); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn still_flags_get_cause_get_message_double_drop() {
        // e.getCause().getMessage() stringifies the underlying cause too — worse than
        // the single-level e.getMessage() case, not a preserved chain.
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOError e) { throw new RuntimeException(\"wrap: \" + e.getCause().getMessage()); } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn allows_bare_rethrow() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { throw e; } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_new_exception_outside_a_catch_block() {
        let findings = check_source(
            "class Foo { void bar() { if (bad) { throw new RuntimeException(\"boom\"); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_zero_arg_constructor() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { throw new RuntimeException(); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_non_exception_named_type() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { throw new Widget(\"boom\"); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn uses_the_correct_shadowed_catch_variable_in_nested_try() {
        let findings = check_source(
            "class Foo { void bar() { \
             try { a(); } catch (IOException e) { \
                 try { b(); } catch (SQLException inner) { throw new RuntimeException(\"wrap\", inner); } \
                 throw new RuntimeException(\"outer wrap\", e); \
             } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_when_nested_catch_uses_outer_catch_variable_instead_of_its_own() {
        let findings = check_source(
            "class Foo { void bar() { \
             try { a(); } catch (IOException e) { \
                 try { b(); } catch (SQLException inner) { throw new RuntimeException(\"wrap\", e); } \
             } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn empty_file_produces_no_findings() {
        let findings = check_source("class Foo { }").unwrap();
        assert!(findings.is_empty());
    }
}
