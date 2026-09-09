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
        Ok(aggregate_findings(&complex))
    }
}

/// One finding per complex function, all sharing the same aggregate message — not a
/// single finding anchored at just one of them. `PostToolUse`'s diff-scoping filters
/// findings to whichever lines an edit actually touched, so anchoring at only e.g. the
/// last complex function in source order would silently swallow this finding whenever
/// the edit that crossed `MIN_COMPLEX_FUNCTIONS` touched a different one.
fn aggregate_findings(complex: &[(usize, usize)]) -> Vec<Finding> {
    let locations: Vec<String> = complex
        .iter()
        .map(|(line, complexity)| format!("{line} (complexity {complexity})"))
        .collect();
    let message = format!(
        "{} functions exceed cyclomatic complexity {CYCLOMATIC_COMPLEXITY_THRESHOLD} \
         (lines {}) — consider splitting responsibilities into separate files",
        complex.len(),
        locations.join(", "),
    );
    complex
        .iter()
        .map(|(line, _)| Finding {
            line: *line,
            message: message.clone(),
        })
        .collect()
}

/// `(declaration line, complexity)` for every Go function/method in `root` whose
/// cyclomatic complexity exceeds [`CYCLOMATIC_COMPLEXITY_THRESHOLD`], in source order.
fn complex_functions(root: Node) -> Vec<(usize, usize)> {
    // Reuses `rules.rs`'s already-verified Go `function_kinds` table instead of an
    // independently-declared literal of the same two strings, so the two can't
    // silently drift apart if Go's grammar node names are ever revisited there.
    let function_kinds = crate::rules::lang_config(Language::Go).function_kinds;
    let mut out = Vec::new();
    collect_complex_functions(root, function_kinds, &mut out);
    out
}

fn collect_complex_functions(node: Node, function_kinds: &[&str], out: &mut Vec<(usize, usize)>) {
    if function_kinds.contains(&node.kind()) {
        let complexity = cyclomatic_complexity(node);
        if complexity > CYCLOMATIC_COMPLEXITY_THRESHOLD {
            out.push((node.start_position().row + 1, complexity));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_complex_functions(child, function_kinds, out);
    }
}

/// McCabe cyclomatic complexity: 1 plus one per decision point — `if`, `for`, each
/// non-default `case`/`type case`/`communication case`, and each short-circuiting
/// `&&`/`||`. Closures (`func_literal`) are walked into, not treated as their own unit,
/// matching `rules.rs`'s `max_nesting_depth` precedent for this grammar. Go-specific
/// today (node kinds are hardcoded, not threaded through a `LangRuleConfig`-style table)
/// — #49/#55 should design their own multi-language shape rather than inherit this one.
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

    /// The 1-indexed declaration line of the named top-level function/method in `src`,
    /// found via the parse tree itself rather than hand-derived from `complex_func`'s
    /// string-building — the exact line a chained `if`/`else if` block lands on isn't
    /// worth re-deriving by formula when the tree already knows it authoritatively.
    fn function_line(src: &str, name: &str) -> usize {
        let cache = GrammarCache::new();
        let tree = cache.parse(Language::Go, src).unwrap();
        let mut cursor = tree.root_node().walk();
        tree.root_node()
            .children(&mut cursor)
            .find(|n| {
                n.kind() == "function_declaration"
                    && n.child_by_field_name("name")
                        .and_then(|id| id.utf8_text(src.as_bytes()).ok())
                        == Some(name)
            })
            .map(|n| n.start_position().row + 1)
            .unwrap()
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

        // One finding per complex function (not one aggregate anchored at just the
        // last), so diff-scoping surfaces it regardless of which one an edit touched.
        assert_eq!(findings.len(), 3);
        let mut lines: Vec<usize> = findings.iter().map(|f| f.line).collect();
        lines.sort_unstable();
        let mut expected = vec![
            function_line(&src, "a"),
            function_line(&src, "b"),
            function_line(&src, "c"),
        ];
        expected.sort_unstable();
        assert_eq!(lines, expected);
        for finding in &findings {
            assert!(finding.message.contains("3 functions"));
            assert!(finding.message.contains("exceed cyclomatic complexity"));
        }
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

    /// Cyclomatic complexity of the first top-level `function_declaration` in `src`.
    fn complexity_of(src: &str) -> usize {
        let cache = GrammarCache::new();
        let tree = cache.parse(Language::Go, src).unwrap();
        let mut cursor = tree.root_node().walk();
        let decl = tree
            .root_node()
            .children(&mut cursor)
            .find(|n| n.kind() == "function_declaration")
            .unwrap();
        cyclomatic_complexity(decl)
    }

    #[test]
    fn counts_for_switch_select_and_short_circuit_operators_as_decision_points() {
        let src = "package main\n\nfunc f(x int, a, b bool) int {\n\
                    \tif a && b {\n\t\treturn 1\n\t}\n\
                    \tfor i := 0; i < x; i++ {\n\t\tx++\n\t}\n\
                    \tswitch x {\n\tcase 1:\n\t\treturn 1\n\tcase 2:\n\t\treturn 2\n\tdefault:\n\t\treturn 0\n\t}\n\
                    \treturn x\n}\n";
        // 1 (base) + 1 (if) + 1 (&&) + 1 (for) + 2 (case, case; default doesn't count) = 6.
        assert_eq!(complexity_of(src), 6);
    }

    #[test]
    fn counts_type_switch_cases_but_not_its_default() {
        let src = "package main\n\nfunc f(x interface{}) int {\n\
                    \tswitch v := x.(type) {\n\
                    \tcase int:\n\t\treturn v\n\
                    \tcase string:\n\t\treturn 0\n\
                    \tdefault:\n\t\treturn -1\n\
                    \t}\n}\n";
        // 1 (base) + 2 (two type cases; default doesn't count) = 3.
        assert_eq!(complexity_of(src), 3);
    }

    #[test]
    fn counts_select_communication_cases_but_not_its_default() {
        let src = "package main\n\nfunc f(ch chan int) int {\n\
                    \tselect {\n\
                    \tcase v := <-ch:\n\t\treturn v\n\
                    \tdefault:\n\t\treturn -1\n\
                    \t}\n}\n";
        // 1 (base) + 1 (one communication case; default doesn't count) = 2.
        assert_eq!(complexity_of(src), 2);
    }

    #[test]
    fn a_closures_branching_contributes_to_the_enclosing_functions_complexity() {
        let with_closure = "package main\n\nfunc f() {\n\
                             \tg := func() {\n\
                             \t\tif true {\n\t\t\tif true {\n\t\t\t\tif true {\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\
                             \t}\n\
                             \tg()\n}\n";
        let without_closure = "package main\n\nfunc f() {\n}\n";
        // The closure's three nested `if`s aren't their own declaration (no
        // `function_declaration`/`method_declaration` node for a `func_literal`), so
        // they must show up in `f`'s own count instead of being silently dropped.
        assert_eq!(
            complexity_of(with_closure),
            complexity_of(without_closure) + 3
        );

        // And the closure must never be reported as its own separate entry alongside
        // the enclosing function — only one function-like declaration exists in this
        // source (`outer`), so `complex_functions` finding two entries here would mean
        // the closure was incorrectly treated as its own reportable unit.
        let mut branches = String::new();
        for i in 0..15 {
            branches.push_str(&format!("\t\tif x == {i} {{\n\t\t\treturn\n\t\t}}\n"));
        }
        let src = format!(
            "package main\n\nfunc outer() {{\n\tg := func(x int) {{\n{branches}\t}}\n\tg(0)\n}}\n"
        );
        let cache = GrammarCache::new();
        let tree = cache.parse(Language::Go, &src).unwrap();
        assert_eq!(complex_functions(tree.root_node()).len(), 1);
    }
}
