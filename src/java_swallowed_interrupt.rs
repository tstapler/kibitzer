use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Flags a `catch (InterruptedException ...)` (bare or as part of a multi-catch) whose
/// body neither restores the interrupt status (a call to `.interrupt()`, conventionally
/// `Thread.currentThread().interrupt()`) nor rethrows anything. This is a distinct,
/// well-established Java idiom violation from generic exception-swallowing — see
/// SonarQube rule S2142 ("InterruptedException should not be ignored") — because
/// *logging or otherwise handling it is not enough*: clearing the JVM's interrupt flag
/// without restoring it can make a thread's cancellation/shutdown request silently
/// vanish. `java-ignored-error` only catches the *empty*-body case; this checker fires
/// even when the catch body does real work, as long as that work isn't one of the two
/// accepted fixes.
///
/// A comment anywhere directly in the catch body is accepted as a third way out — the
/// same "explained deliberately" convention `java-ignored-error` uses — since a
/// `docs/backtest-repos.md` run against `apache/cassandra` (2026-09-13) turned up real,
/// deliberate cases (e.g. `// ignore interruptions, retry and rely on being shut down by
/// requestClosure`) that genuinely rely on a different shutdown mechanism instead of the
/// two structural fixes this checker otherwise requires.
pub struct SwallowedInterruptChecker;

impl Checker for SwallowedInterruptChecker {
    fn name(&self) -> &str {
        "java-swallowed-interrupt"
    }

    fn description(&self) -> &str {
        "flags `catch (InterruptedException e)` blocks that neither call `.interrupt()` nor rethrow"
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
            .context("java-swallowed-interrupt checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();
        let mut findings = Vec::new();
        walk(tree.root_node(), src, &mut findings);
        Ok(findings)
    }
}

fn catches_interrupted_exception(catch_clause: Node, src: &[u8]) -> bool {
    let Some(param) = catch_clause
        .named_child(0)
        .filter(|n| n.kind() == "catch_formal_parameter")
    else {
        return false;
    };
    let Some(catch_type) = param
        .children(&mut param.walk())
        .find(|n| n.kind() == "catch_type")
    else {
        return false;
    };
    let mut cursor = catch_type.walk();
    catch_type
        .named_children(&mut cursor)
        .any(|t| t.utf8_text(src) == Ok("InterruptedException"))
}

/// True if `node`'s subtree contains a `throw_statement` or a call whose method name is
/// `interrupt` — either is an accepted way to not silently lose the interrupt signal.
fn restores_or_propagates_interrupt(node: Node, src: &[u8]) -> bool {
    if node.kind() == "throw_statement" {
        return true;
    }
    if node.kind() == "method_invocation"
        && let Some(name) = node.child_by_field_name("name")
        && name.utf8_text(src) == Ok("interrupt")
    {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| restores_or_propagates_interrupt(child, src))
}

fn has_justifying_comment(body: Node) -> bool {
    let mut cursor = body.walk();
    body.named_children(&mut cursor)
        .any(|child| matches!(child.kind(), "line_comment" | "block_comment"))
}

fn walk(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    crate::tree_walk::walk_preorder(node, &mut |n| {
        check_catch_clause(n, src, findings);
        true
    });
}

fn check_catch_clause(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    if node.kind() == "catch_clause"
        && catches_interrupted_exception(node, src)
        && let Some(body) = node.child_by_field_name("body")
        && !restores_or_propagates_interrupt(body, src)
        && !has_justifying_comment(body)
    {
        findings.push(Finding {
            line: node.start_position().row + 1,
            message: "catches InterruptedException without calling Thread.currentThread()\
                      .interrupt() or rethrowing — this silently discards the thread's \
                      interrupt/cancellation signal (SonarQube S2142)"
                .to_string(),
        });
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(SwallowedInterruptChecker)])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Result<Vec<Finding>> {
        crate::test_support::check_java_source(&SwallowedInterruptChecker, src)
    }

    #[test]
    fn flags_empty_catch_of_interrupted_exception() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (InterruptedException e) { } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn flags_catch_that_only_logs() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (InterruptedException e) { log.warn(\"interrupted\", e); } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn allows_catch_justified_by_a_comment() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (InterruptedException e) { \
             // ignore interruptions, retry and rely on being shut down elsewhere\n \
             retry(); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_catch_that_restores_interrupt_status() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (InterruptedException e) { Thread.currentThread().interrupt(); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_catch_that_rethrows() {
        let findings = check_source(
            "class Foo { void bar() throws InterruptedException { try { a(); } catch (InterruptedException e) { throw e; } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_catch_that_wraps_and_rethrows_unchecked() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (InterruptedException e) { throw new RuntimeException(e); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_multi_catch_including_interrupted_exception() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException | InterruptedException e) { } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn does_not_flag_unrelated_exception_type() {
        let findings = check_source(
            "class Foo { void bar() { try { a(); } catch (IOException e) { } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn empty_file_produces_no_findings() {
        let findings = check_source("class Foo { }").unwrap();
        assert!(findings.is_empty());
    }
}
