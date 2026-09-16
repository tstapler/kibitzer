use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Flags `catch (X e) { throw e; }` — a bare rethrow that discards the catch site's
/// context — but only in files that already demonstrate a `throw new
/// SomeException("...", e)` wrapping convention elsewhere. The Java shape of
/// `go-error-context`'s `if err != nil { return err }` heuristic: without a
/// same-file signal that the project wraps causes with a message, we can't tell
/// whether bare rethrows are intentional, so staying silent avoids false positives
/// in codebases that propagate exceptions unwrapped on purpose.
///
/// Advisory by default (wire it up with `"severity": "advisory"` in
/// `.claude/inspect.json`): a style nudge, not a correctness bug, with the same
/// deliberately narrow scope as `go-error-context` — see that checker's module doc
/// for the class of gaps (multi-statement catch blocks, exception chaining via
/// `initCause`, logging-then-rethrow) this one does not attempt to cover either.
pub struct ErrorContextChecker;

impl Checker for ErrorContextChecker {
    fn name(&self) -> &str {
        "java-error-context"
    }

    fn description(&self) -> &str {
        "flags bare `catch (X e) { throw e; }` rethrows in files that already use `throw new X(\"...\", e)` elsewhere"
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
            .context("java-error-context checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();
        let root = tree.root_node();

        if !has_wrapping_convention(root, src) {
            return Ok(Vec::new());
        }

        let mut findings = Vec::new();
        collect_bare_rethrows(root, src, &mut findings);
        Ok(findings)
    }
}

/// True if `throw new X(...)` is called anywhere in the file with both a string
/// literal and an identifier among its constructor arguments — the signal that this
/// codebase wraps a caught exception with a message on purpose.
fn has_wrapping_convention(node: Node, src: &[u8]) -> bool {
    if node.kind() == "throw_statement"
        && let Some(expr) = node.named_child(0)
        && expr.kind() == "object_creation_expression"
        && let Some(args) = expr.child_by_field_name("arguments")
    {
        let mut cursor = args.walk();
        let mut has_message = false;
        let mut has_cause = false;
        for arg in args.named_children(&mut cursor) {
            match arg.kind() {
                "string_literal" => has_message = true,
                "identifier" => has_cause = true,
                _ => {}
            }
        }
        if has_message && has_cause {
            return true;
        }
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| has_wrapping_convention(child, src))
}

fn collect_bare_rethrows(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    if node.kind() == "catch_clause" && is_bare_rethrow(node, src) {
        findings.push(Finding {
            line: node.start_position().row + 1,
            message: "rethrows the caught exception unwrapped despite this file's \
                      `throw new X(\"...\", e)` convention — consider wrapping with context here too"
                .to_string(),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_bare_rethrows(child, src, findings);
    }
}

/// Matches exactly `catch (X <name>) { throw <name>; }` — a catch body whose only
/// statement is a `throw` of the same identifier the catch parameter bound. Anything
/// else (extra statements, logging, wrapping, a different identifier) falls through
/// untouched — see the module doc comment.
fn is_bare_rethrow(catch_clause: Node, src: &[u8]) -> bool {
    let Some(param) = catch_clause
        .named_child(0)
        .filter(|n| n.kind() == "catch_formal_parameter")
    else {
        return false;
    };
    let Some(name_node) = param.child_by_field_name("name") else {
        return false;
    };
    let Ok(param_name) = name_node.utf8_text(src) else {
        return false;
    };

    let Some(body) = catch_clause.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let statements: Vec<Node> = body
        .named_children(&mut cursor)
        .filter(|n| !matches!(n.kind(), "line_comment" | "block_comment"))
        .collect();
    if statements.len() != 1 || statements[0].kind() != "throw_statement" {
        return false;
    }
    let Some(thrown) = statements[0].named_child(0) else {
        return false;
    };
    thrown.kind() == "identifier" && thrown.utf8_text(src) == Ok(param_name)
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
        ErrorContextChecker.check(Path::new("<source>"), &ctx)
    }

    const WRAPPING_CONVENTION: &str =
        "void wrap() { try { doThing(); } catch (IOException e) { throw new RuntimeException(\"wrap failed\", e); } }";

    #[test]
    fn flags_bare_rethrow_when_wrapping_convention_exists() {
        let src = format!(
            "class Foo {{ {WRAPPING_CONVENTION} void g() {{ try {{ doThing(); }} catch (IOException e) {{ throw e; }} }} }}"
        );
        let findings = check_source(&src).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn does_not_flag_when_no_wrapping_convention_in_file() {
        let src =
            "class Foo { void g() { try { doThing(); } catch (IOException e) { throw e; } } }";
        let findings = check_source(src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_catch_that_wraps() {
        let src = format!(
            "class Foo {{ {WRAPPING_CONVENTION} void g() {{ try {{ doThing(); }} catch (IOException e) {{ throw new RuntimeException(\"g failed\", e); }} }} }}"
        );
        let findings = check_source(&src).unwrap();
        // Neither catch here bare-rethrows (both wrap) — WRAPPING_CONVENTION's own
        // catch must not self-trigger either.
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_catch_with_extra_statements() {
        let src = format!(
            "class Foo {{ {WRAPPING_CONVENTION} void g() {{ try {{ doThing(); }} catch (IOException e) {{ log.warn(\"oops\"); throw e; }} }} }}"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_rethrow_of_a_different_identifier() {
        let src = format!(
            "class Foo {{ {WRAPPING_CONVENTION} void g() {{ try {{ doThing(); }} catch (IOException e) {{ throw other; }} }} }}"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_empty_catch() {
        let src = format!(
            "class Foo {{ {WRAPPING_CONVENTION} void g() {{ try {{ doThing(); }} catch (IOException e) {{ }} }} }}"
        );
        let findings = check_source(&src).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn empty_file_produces_no_findings() {
        let findings = check_source("class Foo { }").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_multiple_bare_rethrows() {
        let src = format!(
            "class Foo {{ {WRAPPING_CONVENTION} \
             void g() {{ try {{ a(); }} catch (IOException e) {{ throw e; }} }} \
             void h() {{ try {{ b(); }} catch (SQLException e) {{ throw e; }} }} \
             }}"
        );
        let findings = check_source(&src).unwrap();
        assert_eq!(findings.len(), 2);
    }
}
