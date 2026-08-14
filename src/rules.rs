use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::config::Severity;

/// A function/method body spanning more lines than this is flagged by `long-function`.
const LONG_FUNCTION_LINES: usize = 40;
/// A function/method body whose control-flow nesting depth exceeds this is flagged by
/// `deep-nesting`. Depth 1 is the function body itself; each nested
/// if/for/switch/select/func_literal adds one.
const MAX_NESTING_DEPTH: usize = 4;
/// A function/method parameter list naming more identifiers than this is flagged by
/// `long-parameter-list`.
const LONG_PARAM_LIST_COUNT: usize = 5;

/// Metadata for one rule in the catalog. Thresholds above are fixed for now —
/// per-rule configurability is a natural follow-up, not required for the initial
/// catalog.
#[allow(dead_code)]
pub struct RuleMeta {
    pub id: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub default_severity: Severity,
}

/// Self-documentation for future consumers (e.g. a `kibitzer rules list` command) —
/// not read by the checker logic itself, hence the blanket allow above.
#[allow(dead_code)]
pub const CATALOG: &[RuleMeta] = &[
    RuleMeta {
        id: "long-function",
        category: "complexity",
        description: "Function/method body spans more than 40 lines.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "deep-nesting",
        category: "complexity",
        description: "Function/method body nests if/for/switch/select/func_literal more than 4 levels deep.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "long-parameter-list",
        category: "style",
        description: "Function/method parameter list names more than 5 identifiers.",
        default_severity: Severity::Advisory,
    },
];

pub struct SyntaxRulesChecker;

impl Checker for SyntaxRulesChecker {
    fn name(&self) -> &str {
        "syntax-rules"
    }

    fn description(&self) -> &str {
        "native syntactic rule catalog: long-function, deep-nesting, long-parameter-list (see docs/syntax-rules.md)"
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
            .context("syntax-rules checker requires a parsed tree")?;
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &mut findings);
        Ok(findings)
    }
}

fn walk_declarations(node: Node, findings: &mut Vec<Finding>) {
    if node.kind() == "function_declaration" || node.kind() == "method_declaration" {
        check_declaration(node, findings);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_declarations(child, findings);
    }
}

fn check_declaration(decl: Node, findings: &mut Vec<Finding>) {
    let line = decl.start_position().row + 1;

    if let Some(body) = decl.child_by_field_name("body") {
        let body_lines = body.end_position().row - body.start_position().row + 1;
        if body_lines > LONG_FUNCTION_LINES {
            findings.push(Finding {
                line,
                message: format!(
                    "[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider splitting it up"
                ),
            });
        }

        let depth = max_nesting_depth(body, 1);
        if depth > MAX_NESTING_DEPTH {
            findings.push(Finding {
                line,
                message: format!(
                    "[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider extracting a function or inverting a condition"
                ),
            });
        }
    }

    if let Some(params) = decl.child_by_field_name("parameters") {
        let count = param_identifier_count(params);
        if count > LONG_PARAM_LIST_COUNT {
            findings.push(Finding {
                line,
                message: format!(
                    "[long-parameter-list] parameter list names {count} identifiers (over {LONG_PARAM_LIST_COUNT}) — consider a config struct"
                ),
            });
        }
    }
}

/// Depth of `node` itself (as passed in via `current_depth`), taking the max over all
/// descendants. Each if/for/switch/select/func_literal body nested inside adds one —
/// except a chained `else if`, which Go's grammar represents as an `if_statement`
/// nested in the prior one's `alternative` field. That's a flat branch, not real
/// nesting, so it stays at the current depth rather than adding one.
fn max_nesting_depth(node: Node, current_depth: usize) -> usize {
    let else_if_id = (node.kind() == "if_statement")
        .then(|| node.child_by_field_name("alternative"))
        .flatten()
        .filter(|alt| alt.kind() == "if_statement")
        .map(|alt| alt.id());

    let mut max_depth = current_depth;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_depth = if Some(child.id()) == else_if_id {
            current_depth
        } else if is_nesting_construct(child.kind()) {
            current_depth + 1
        } else {
            current_depth
        };
        max_depth = max_depth.max(max_nesting_depth(child, child_depth));
    }
    max_depth
}

fn is_nesting_construct(kind: &str) -> bool {
    matches!(
        kind,
        "if_statement"
            | "for_statement"
            | "expression_switch_statement"
            | "type_switch_statement"
            | "select_statement"
            | "func_literal"
    )
}

fn param_identifier_count(params: Node) -> usize {
    let mut count = 0;
    let mut cursor = params.walk();
    for decl in params.children(&mut cursor) {
        if decl.kind() != "parameter_declaration" && decl.kind() != "variadic_parameter_declaration"
        {
            continue;
        }
        let mut name_cursor = decl.walk();
        let named = decl
            .children_by_field_name("name", &mut name_cursor)
            .count();
        // A parameter_declaration with no name field still names one anonymous
        // parameter (e.g. `func f(string)` in an interface method set).
        count += named.max(1);
    }
    count
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

        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &mut findings);
        Ok(findings)
    }

    #[test]
    fn allows_short_function() {
        let findings = check_source("package main\nfunc f() {\n\tprintln(\"ok\")\n}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_long_function() {
        let mut src = String::from("package main\nfunc f() {\n");
        for _ in 0..45 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn allows_shallow_nesting() {
        let findings = check_source(
            "package main\nfunc f(x int) {\n\tif x > 0 {\n\t\tprintln(\"pos\")\n\t}\n}\n",
        )
        .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn flags_deep_nesting() {
        let src = "package main\n\
             func f(x int) {\n\
             \tif x > 0 {\n\
             \t\tfor i := 0; i < x; i++ {\n\
             \t\t\tswitch i {\n\
             \t\t\tcase 0:\n\
             \t\t\t\tif i == 0 {\n\
             \t\t\t\t\tprintln(\"deep\")\n\
             \t\t\t\t}\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n\
             }\n";
        let findings = check_source(src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn allows_short_parameter_list() {
        let findings = check_source("package main\nfunc f(a, b string) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn flags_long_parameter_list() {
        let findings = check_source("package main\nfunc f(a, b, c, d, e, f int) {}\n").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn rules_fire_independently_on_one_function() {
        let mut src = String::from("package main\nfunc f(a, b, c, d, e, g int) {\n");
        for _ in 0..45 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn flat_else_if_chain_does_not_count_as_nesting() {
        let src = "package main\n\
             func f(x int) {\n\
             \tif x == 0 {\n\
             \t\tprintln(\"a\")\n\
             \t} else if x == 1 {\n\
             \t\tprintln(\"b\")\n\
             \t} else if x == 2 {\n\
             \t\tprintln(\"c\")\n\
             \t} else if x == 3 {\n\
             \t\tprintln(\"d\")\n\
             \t} else if x == 4 {\n\
             \t\tprintln(\"e\")\n\
             \t} else {\n\
             \t\tprintln(\"f\")\n\
             \t}\n\
             }\n";
        let findings = check_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn empty_file_has_no_findings() {
        let findings = check_source("package main\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn checks_method_declarations_too() {
        let findings =
            check_source("package main\nfunc (r *T) f(a, b, c, d, e, g int) {}\n").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn all_three_rules_can_fire_on_one_function() {
        let mut src = String::from("package main\nfunc f(a, b, c, d, e, g int) {\n");
        src.push_str(
            "\tif a > 0 {\n\t\tfor i := 0; i < a; i++ {\n\t\t\tswitch i {\n\t\t\tcase 0:\n\t\t\t\tif i == 0 {\n\t\t\t\t\tprintln(\"deep\")\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n",
        );
        for _ in 0..45 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn function_at_exactly_forty_lines_is_not_long() {
        let mut src = String::from("package main\nfunc f() {\n");
        for _ in 0..38 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn function_at_forty_one_lines_is_long() {
        let mut src = String::from("package main\nfunc f() {\n");
        for _ in 0..39 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn five_params_is_not_long() {
        let findings = check_source("package main\nfunc f(a, b, c, d, e int) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn catalog_ids_are_unique_and_documented() {
        let ids: Vec<&str> = CATALOG.iter().map(|r| r.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len());
        assert!(CATALOG.iter().all(|r| !r.description.is_empty()));
    }
}
