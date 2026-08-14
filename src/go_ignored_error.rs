use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Flags Go short variable declarations that discard the *last* value of a
/// multi-value call via `_` (e.g. `result, _ := f()`) — by Go convention the last
/// return value is the error, so this shape usually means an error is being
/// silently dropped. Deliberately does NOT flag `_, err := f()`: there the
/// blank identifier discards a non-last value and the error is kept, which is
/// the exact conflation this check must avoid.
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

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("go-ignored-error checker requires a parsed tree")?;
        let mut findings = Vec::new();
        walk(tree.root_node(), ctx.source.as_bytes(), &mut findings);
        Ok(findings)
    }
}

fn walk(node: Node, src: &[u8], findings: &mut Vec<Finding>) {
    if node.kind() == "short_var_declaration"
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
        walk(child, src, findings);
    }
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
        IgnoredErrorChecker.check(Path::new("<source>"), &ctx)
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
}
