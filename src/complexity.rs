use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// A function/method whose cyclomatic complexity exceeds this counts toward
/// `file-complexity`'s per-file aggregate. McCabe's original paper treats 10 as the
/// point past which a function becomes hard to test exhaustively; industry linters
/// (`gocyclo`, ESLint's `complexity` rule) commonly default in the 10-20 range — 10
/// matches the stricter end, consistent with this repo's other advisory thresholds
/// erring toward catching more real signal (see #32).
const CYCLOMATIC_COMPLEXITY_THRESHOLD: usize = 10;

/// `file-complexity` only fires once a file has at least this many over-threshold
/// functions — one complex function means that function has a problem (a narrower,
/// per-function check, not built here — see #32's "keeping both distinct" scope note),
/// not that the whole file's responsibilities need splitting. Mirrors
/// `duplicate-code`'s `MIN_OCCURRENCES` precedent: a single occurrence is ordinary,
/// several is the real signal.
const MIN_COMPLEX_FUNCTIONS: usize = 3;

/// Flags a Go file containing several functions whose McCabe cyclomatic complexity
/// crosses [`CYCLOMATIC_COMPLEXITY_THRESHOLD`] — a file-level "this file has taken on
/// too many complex responsibilities, consider splitting it" signal, distinct from (and
/// a level above) any single function being complex. v1 scope is Go-only, matching this
/// repo's established rollout precedent (`primitive_obsession.rs`, `duplicate_code.rs`
/// both started Go-first); see #32.
pub struct FileComplexityChecker;

impl Checker for FileComplexityChecker {
    fn name(&self) -> &str {
        "file-complexity"
    }

    fn description(&self) -> &str {
        "flags a file with several functions over a cyclomatic-complexity threshold, suggesting its responsibilities should be split across files"
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
            .context("file-complexity checker requires a parsed tree")?;
        let complex = complex_functions(tree.root_node());
        if complex.len() < MIN_COMPLEX_FUNCTIONS {
            return Ok(Vec::new());
        }
        Ok(vec![aggregate_finding(&complex)])
    }
}

fn aggregate_finding(complex: &[(usize, usize)]) -> Finding {
    let locations: Vec<String> = complex
        .iter()
        .map(|(line, complexity)| format!("{line} (complexity {complexity})"))
        .collect();
    Finding {
        line: complex.last().map_or(1, |(line, _)| *line),
        message: format!(
            "{} functions exceed cyclomatic complexity {CYCLOMATIC_COMPLEXITY_THRESHOLD} \
             (lines {}) — consider splitting responsibilities into separate files",
            complex.len(),
            locations.join(", "),
        ),
    }
}

/// `(declaration line, complexity)` for every Go function/method in `root` whose
/// cyclomatic complexity exceeds [`CYCLOMATIC_COMPLEXITY_THRESHOLD`], in source order.
fn complex_functions(root: Node) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    collect_complex_functions(root, &mut out);
    out
}

fn collect_complex_functions(node: Node, out: &mut Vec<(usize, usize)>) {
    if matches!(node.kind(), "function_declaration" | "method_declaration") {
        let complexity = cyclomatic_complexity(node);
        if complexity > CYCLOMATIC_COMPLEXITY_THRESHOLD {
            out.push((node.start_position().row + 1, complexity));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_complex_functions(child, out);
    }
}

/// McCabe cyclomatic complexity: one path through the function to start with, plus one
/// more for every decision point in its body — an `if`, a `for`, each `case`/`type
/// case`/`communication case` clause (a `default` clause adds no new condition to
/// evaluate, so isn't counted), and each short-circuiting `&&`/`||`. Node kinds verified
/// against `tree-sitter-go`'s own parse output, matching this codebase's existing
/// verification discipline (see `rules.rs`'s `LangRuleConfig` doc comment) rather than
/// guessed by analogy with another grammar.
///
/// Nested closures (`func_literal`) are walked into, not skipped or treated as their
/// own unit — same precedent `rules.rs`'s `max_nesting_depth` already set for this
/// exact grammar: a closure's own branching is inseparable from the complexity of the
/// function that defines it.
pub(crate) fn cyclomatic_complexity(decl: Node) -> usize {
    1 + count_decision_points(decl)
}

fn count_decision_points(node: Node) -> usize {
    let mut count = match node.kind() {
        "if_statement" | "for_statement" | "expression_case" | "type_case"
        | "communication_case" => 1,
        "binary_expression" => usize::from(is_short_circuit(node)),
        _ => 0,
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_points(child);
    }
    count
}

fn is_short_circuit(binary_expression: Node) -> bool {
    binary_expression
        .child_by_field_name("operator")
        .is_some_and(|op| matches!(op.kind(), "&&" | "||"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checker::GrammarCache;

    fn check_source(src: &str) -> Vec<Finding> {
        let cache = GrammarCache::new();
        let tree = cache.parse(Language::Go, src).unwrap();
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        FileComplexityChecker
            .check(Path::new("<source>"), &ctx)
            .unwrap()
    }

    fn simple_func(name: &str) -> String {
        format!("func {name}() {{\n\tfmt.Println(\"hi\")\n}}\n")
    }

    /// `n` chained `if`/`else if` branches — each adds one decision point, so `n`
    /// branches (plus the base path) gives complexity `n + 1`.
    fn complex_func(name: &str, branches: usize) -> String {
        let mut body = String::new();
        for i in 0..branches {
            if i == 0 {
                body.push_str(&format!("\tif x == {i} {{\n\t\treturn\n\t}}"));
            } else {
                body.push_str(&format!(" else if x == {i} {{\n\t\treturn\n\t}}"));
            }
        }
        format!("func {name}(x int) {{\n{body}\n}}\n")
    }

    #[test]
    fn does_not_flag_a_file_with_no_complex_functions() {
        let src = format!(
            "package main\n\n{}{}{}",
            simple_func("a"),
            simple_func("b"),
            simple_func("c")
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_a_file_with_only_two_complex_functions() {
        let src = format!(
            "package main\n\n{}{}",
            complex_func("a", 11),
            complex_func("b", 11)
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn flags_a_file_with_three_or_more_complex_functions() {
        let src = format!(
            "package main\n\n{}{}{}",
            complex_func("a", 11),
            complex_func("b", 11),
            complex_func("c", 11)
        );
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3 functions"));
        assert!(findings[0].message.contains("exceed cyclomatic complexity"));
    }

    #[test]
    fn does_not_flag_functions_at_or_under_the_threshold() {
        // 10 branches -> complexity 11 (over 10); 9 branches -> complexity 10 (at, not over).
        let src = format!(
            "package main\n\n{}{}{}",
            complex_func("a", 9),
            complex_func("b", 9),
            complex_func("c", 9)
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn counts_for_switch_select_and_short_circuit_operators_as_decision_points() {
        let src = "package main\n\nfunc f(x int, a, b bool) int {\n\
                    \tif a && b {\n\t\treturn 1\n\t}\n\
                    \tfor i := 0; i < x; i++ {\n\t\tx++\n\t}\n\
                    \tswitch x {\n\tcase 1:\n\t\treturn 1\n\tcase 2:\n\t\treturn 2\n\tdefault:\n\t\treturn 0\n\t}\n\
                    \treturn x\n}\n";
        // 1 (base) + 1 (if) + 1 (&&) + 1 (for) + 2 (case, case; default doesn't count) = 6.
        let cache = GrammarCache::new();
        let tree = cache.parse(Language::Go, src).unwrap();
        let mut cursor = tree.root_node().walk();
        let decl = tree
            .root_node()
            .children(&mut cursor)
            .find(|n| n.kind() == "function_declaration")
            .unwrap();
        assert_eq!(cyclomatic_complexity(decl), 6);
    }
}
