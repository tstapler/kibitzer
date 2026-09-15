use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::file_size::is_generated;

/// Minimum number of `case` clauses on a single `switch x.(type)` before it's flagged —
/// #38's "OCP proxy: type-switch density" item. No stronger literature citation than
/// "a 2-case type switch is an ordinary branch, a growing one is a type-code smell,"
/// same status as `LCOM_MIN_METHODS`/`MAX_FAN_OUT`. A `default:` clause doesn't count
/// toward this total — it isn't branching on a type.
const MIN_TYPE_SWITCH_CASES: usize = 4;

/// AST pattern count, no cross-file graph work: a `switch v := x.(type) { case A: ...
/// case B: ... }` with `MIN_TYPE_SWITCH_CASES` or more type cases is a deterministic OCP
/// proxy — heavy branching on a type enum is exactly the shape Replace Conditional with
/// Polymorphism / Replace Type Code with Subclasses target. This checker only counts;
/// it does not know whether the same type set recurs across other methods of the same
/// type (the signal that would turn this into an actual refactoring candidate) — that
/// clustering step is a stated dependency, tracked separately (see
/// docs/refactoring-catalog-analysis.md and issue #60).
///
/// Go-only for v1: Go's mechanism (`switch x.(type)`) is a single, unambiguous AST shape
/// to match. A chain of repeated `if _, ok := x.(A); ok { ... } else if _, ok :=
/// x.(B); ok { ... }` type-assertion `if`/`else if` chains is the same smell but a much
/// messier pattern to detect reliably (no single enclosing node groups the chain the way
/// `type_switch_statement` does) — deliberately out of scope for this pass rather than
/// guessed at with a fragile heuristic. Other languages' `instanceof`/downcast chains
/// (Java, TS) are the same kind of further, separate scope-widening, not attempted here.
pub struct TypeSwitchDensityChecker;

impl Checker for TypeSwitchDensityChecker {
    fn name(&self) -> &str {
        "go-type-switch-density"
    }

    fn description(&self) -> &str {
        "flags a `switch x.(type)` with many type cases — an Open/Closed Principle proxy for a growing type-code branch"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if is_generated(ctx.source) {
            return Ok(Vec::new());
        }
        let tree = ctx
            .tree
            .context("go-type-switch-density checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();

        let mut findings = Vec::new();
        collect_dense_type_switches(tree.root_node(), src, &mut findings);
        Ok(findings)
    }
}

fn collect_dense_type_switches(node: Node, src: &[u8], out: &mut Vec<Finding>) {
    if node.kind() == "type_switch_statement" {
        let case_count = type_case_count(node);
        if case_count >= MIN_TYPE_SWITCH_CASES {
            let subject = node
                .child_by_field_name("value")
                .map(|v| node_text(v, src))
                .unwrap_or_else(|| "<expr>".to_string());
            out.push(Finding {
                line: node.start_position().row + 1,
                message: format!(
                    "[go-type-switch-density] type switch on `{subject}` branches over \
                     {case_count} types — a growing type-code switch is an Open/Closed \
                     Principle proxy; if this same type set recurs across other methods \
                     of the same type, it's a Replace Conditional with Polymorphism \
                     candidate"
                ),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_dense_type_switches(child, src, out);
    }
}

/// Number of `type_case` clauses on a `type_switch_statement` — a `case A, B:` clause
/// with more than one listed type is still one clause/one branch, so this counts
/// `type_case` child nodes, not the total number of `type:` fields across them.
/// `default_case` is a distinct node kind and is never counted here.
fn type_case_count(type_switch: Node) -> usize {
    let mut cursor = type_switch.walk();
    type_switch
        .children(&mut cursor)
        .filter(|c| c.kind() == "type_case")
        .count()
}

fn node_text(node: Node, src: &[u8]) -> String {
    String::from_utf8_lossy(&src[node.start_byte()..node.end_byte()]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go_findings(src: &str) -> Vec<Finding> {
        crate::test_support::check_go_source(&TypeSwitchDensityChecker, src).unwrap()
    }

    #[test]
    fn flags_a_type_switch_at_the_threshold() {
        let src = "package p\n\nfunc F(x interface{}) {\n\tswitch x.(type) {\n\tcase int:\n\tcase string:\n\tcase bool:\n\tcase float64:\n\t}\n}\n";
        let findings = go_findings(src);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("branches over 4 types"));
    }

    #[test]
    fn does_not_flag_a_type_switch_below_the_threshold() {
        let src = "package p\n\nfunc F(x interface{}) {\n\tswitch x.(type) {\n\tcase int:\n\tcase string:\n\tcase bool:\n\t}\n}\n";
        assert!(go_findings(src).is_empty());
    }

    #[test]
    fn default_case_does_not_count_toward_the_threshold() {
        let src = "package p\n\nfunc F(x interface{}) {\n\tswitch x.(type) {\n\tcase int:\n\tcase string:\n\tcase bool:\n\tdefault:\n\t}\n}\n";
        assert!(go_findings(src).is_empty());
    }

    #[test]
    fn a_multi_type_case_clause_counts_as_one_branch_not_two() {
        // `case string, bool:` is one clause listing two types — still one branch.
        let src = "package p\n\nfunc F(x interface{}) {\n\tswitch x.(type) {\n\tcase int:\n\tcase string, bool:\n\tcase float64:\n\t}\n}\n";
        assert!(
            go_findings(src).is_empty(),
            "3 clauses (one multi-type) is below the 4-clause threshold"
        );
    }

    #[test]
    fn subject_expression_is_named_in_the_message() {
        let src = "package p\n\ntype T struct{ Kind interface{} }\n\nfunc F(t T) {\n\tswitch t.Kind.(type) {\n\tcase int:\n\tcase string:\n\tcase bool:\n\tcase float64:\n\t}\n}\n";
        let findings = go_findings(src);
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].message.contains("`t.Kind`"),
            "got: {}",
            findings[0].message
        );
    }

    #[test]
    fn generated_file_is_skipped() {
        let src = "// Code generated by mockgen. DO NOT EDIT.\npackage p\n\nfunc F(x interface{}) {\n\tswitch x.(type) {\n\tcase int:\n\tcase string:\n\tcase bool:\n\tcase float64:\n\t}\n}\n";
        assert!(go_findings(src).is_empty());
    }
}
