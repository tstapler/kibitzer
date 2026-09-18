use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Flags a Java `catch` block with no statements and no comment explaining why the
/// exception is being swallowed — the Java shape of the same "silently ignoring an
/// error signal" problem `go-ignored-error` targets for Go's `result, _ := f()`.
/// Deliberately narrow: only a truly empty catch body is flagged (matching this
/// codebase's other narrow-heuristic checkers, e.g. `go-error-context`'s doc comment
/// on avoiding false positives) — a catch block that logs, rethrows, or does anything
/// else is out of scope, even if that handling is itself questionable.
///
/// A catch parameter named (case-insensitively) `ignore`/`ignored`/`unused`/
/// `expected`/`suppressed` is treated as self-documenting, the same way Go's blank
/// `_` identifier signals "deliberately discarded" without needing a comment too —
/// this convention alone accounted for 44% of raw hits in a `docs/backtest-repos.md`
/// run against `apache/cassandra` (2026-09-11), so it's load-bearing for precision,
/// not a cosmetic nicety.
pub struct IgnoredErrorChecker;

const JUSTIFIED_NAMES: &[&str] = &["ignore", "ignored", "unused", "expected", "suppressed"];

impl Checker for IgnoredErrorChecker {
    fn name(&self) -> &str {
        "java-ignored-error"
    }

    fn description(&self) -> &str {
        "flags empty Java catch blocks with no comment explaining why the exception is swallowed"
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
            .context("java-ignored-error checker requires a parsed tree")?;
        let mut findings = Vec::new();
        walk(tree.root_node(), ctx.source.as_bytes(), &mut findings);
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(IgnoredErrorChecker)])
}

fn is_comment(node: Node) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}

/// True if `catch_clause`'s bound exception variable is named (case-insensitively)
/// one of `JUSTIFIED_NAMES` — see this module's doc comment for why that counts as
/// justification on its own, without a comment.
fn has_justified_name(catch_clause: Node, src: &[u8]) -> bool {
    catch_clause
        .named_child(0)
        .filter(|n| n.kind() == "catch_formal_parameter")
        .and_then(|param| param.child_by_field_name("name"))
        .and_then(|name| name.utf8_text(src).ok())
        .is_some_and(|name| {
            JUSTIFIED_NAMES
                .iter()
                .any(|justified| name.eq_ignore_ascii_case(justified))
        })
}

fn walk(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    if node.kind() == "catch_clause"
        && let Some(body) = node.child_by_field_name("body")
    {
        let mut cursor = body.walk();
        let mut statement_count = 0usize;
        let mut has_comment = false;
        for child in body.named_children(&mut cursor) {
            if is_comment(child) {
                has_comment = true;
            } else {
                statement_count += 1;
            }
        }
        if statement_count == 0 && !has_comment && !has_justified_name(node, src) {
            findings.push(Finding {
                line: node.start_position().row + 1,
                message: "empty catch block silently swallows the exception — handle it, log \
                          it, or add a comment explaining why it's safe to ignore"
                    .to_string(),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, findings);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Result<Vec<Finding>> {
        crate::test_support::check_java_source(&IgnoredErrorChecker, src)
    }

    #[test]
    fn flags_empty_catch_block() {
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException e) { } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn allows_empty_catch_block_with_ignore_named_parameter() {
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException ignore) { } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_empty_catch_block_with_ignored_named_parameter_case_insensitively() {
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException Ignored) { } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn still_flags_empty_catch_block_with_unrelated_short_name() {
        // "ie" isn't in JUSTIFIED_NAMES — a short variable name alone isn't a
        // recognized justification convention, only the specific listed names are.
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException ie) { } } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn allows_empty_catch_block_with_interior_comment() {
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException e) { \
             // intentionally ignored: best-effort cleanup\n } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_empty_catch_block_with_trailing_comment() {
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException e) { // ignored\n } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_catch_that_handles_the_exception() {
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException e) { log.warn(e); } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_catch_that_rethrows() {
        let findings = check_source(
            "class Foo { void bar() { try { doThing(); } catch (IOException e) { throw e; } } }",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_multiple_empty_catch_blocks() {
        let findings = check_source(
            "class Foo { void bar() { \
             try { a(); } catch (IOException e) { } \
             try { b(); } catch (SQLException e) { } \
             } }",
        )
        .unwrap();
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn empty_file_produces_no_findings() {
        let findings = check_source("class Foo { }").unwrap();
        assert!(findings.is_empty());
    }
}
