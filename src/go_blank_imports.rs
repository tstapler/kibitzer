use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Flags Go blank imports (`import _ "pkg"`) with no adjacent comment explaining the
/// side effect being relied on. A blank import that isn't explained reads as dead code
/// to the next person who touches the file.
pub struct BlankImportsChecker;

impl Checker for BlankImportsChecker {
    fn name(&self) -> &str {
        "go-blank-imports"
    }

    fn description(&self) -> &str {
        "flags Go blank imports (`import _ \"pkg\"`) with no adjacent justification comment"
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
            .context("go-blank-imports checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();

        let mut comment_rows = std::collections::HashSet::new();
        collect_comment_rows(tree.root_node(), &mut comment_rows);
        let mut import_spec_rows = std::collections::HashSet::new();
        collect_import_spec_rows(tree.root_node(), &mut import_spec_rows);
        // A comment sharing a row with an import spec is that spec's trailing
        // comment, not a leading comment for whatever import follows it — exclude
        // those rows before using row-1 lookups to detect leading comments.
        let leading_comment_rows: std::collections::HashSet<usize> = comment_rows
            .difference(&import_spec_rows)
            .copied()
            .collect();

        let mut findings = Vec::new();
        collect_blank_imports(
            tree.root_node(),
            src,
            &comment_rows,
            &leading_comment_rows,
            &mut findings,
        );
        Ok(findings)
    }
}

fn collect_comment_rows(node: Node, rows: &mut std::collections::HashSet<usize>) {
    if node.kind() == "comment" {
        rows.insert(node.start_position().row);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_comment_rows(child, rows);
    }
}

fn collect_import_spec_rows(node: Node, rows: &mut std::collections::HashSet<usize>) {
    if node.kind() == "import_spec" {
        rows.insert(node.start_position().row);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_import_spec_rows(child, rows);
    }
}

fn collect_blank_imports(
    node: Node,
    src: &[u8],
    comment_rows: &std::collections::HashSet<usize>,
    leading_comment_rows: &std::collections::HashSet<usize>,
    findings: &mut Vec<Finding>,
) {
    if node.kind() == "import_spec"
        && let Some(name) = node.child_by_field_name("name")
        && name.kind() == "blank_identifier"
    {
        let row = node.start_position().row;
        // Justified if a comment sits on the same line (trailing) or the line
        // immediately above (leading) — either reads as "explaining this import".
        let justified =
            comment_rows.contains(&row) || (row > 0 && leading_comment_rows.contains(&(row - 1)));
        if !justified {
            let path = node
                .child_by_field_name("path")
                .and_then(|p| p.utf8_text(src).ok())
                .unwrap_or("<unknown>");
            findings.push(Finding {
                line: row + 1,
                message: format!(
                    "blank import {path} has no adjacent comment explaining the side effect it relies on"
                ),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_blank_imports(child, src, comment_rows, leading_comment_rows, findings);
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
        BlankImportsChecker.check(Path::new("<source>"), &ctx)
    }

    #[test]
    fn flags_unjustified_blank_import_in_group() {
        let findings =
            check_source("package main\n\nimport (\n\t_ \"database/sql/driver\"\n)\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("database/sql/driver"));
    }

    #[test]
    fn flags_unjustified_single_blank_import() {
        let findings = check_source("package main\n\nimport _ \"unsafe\"\n").unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn allows_blank_import_with_trailing_comment() {
        let findings = check_source(
            "package main\n\nimport (\n\t_ \"net/http/pprof\" // for side effect: pprof endpoints\n)\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_blank_import_with_leading_comment() {
        let findings = check_source(
            "package main\n\nimport (\n\t// for side effect X\n\t_ \"database/sql/driver\"\n)\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_normal_named_import() {
        let findings = check_source("package main\n\nimport \"fmt\"\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn does_not_flag_aliased_import() {
        let findings = check_source("package main\n\nimport f \"fmt\"\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn mixed_group_only_flags_unjustified_entry() {
        let findings = check_source(
            "package main\n\nimport (\n\t_ \"database/sql/driver\" // for the driver registration\n\t_ \"unjustified/pkg\"\n)\n",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("unjustified/pkg"));
    }

    #[test]
    fn empty_file_produces_no_findings() {
        let findings = check_source("package main\n").unwrap();
        assert!(findings.is_empty());
    }
}
