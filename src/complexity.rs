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

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if crate::file_size::is_generated(ctx.source) {
            return Ok(Vec::new());
        }
        let tree = ctx
            .tree
            .context("file-complexity checker requires a parsed tree")?;
        let is_test_file = file
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_test.go"));
        let complex = complex_functions(tree.root_node(), ctx.source.as_bytes(), is_test_file);
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
/// `is_test_file` gates the `t.Run`/`b.Run`/`m.Run` subtest exclusion (see
/// [`is_run_subtest_closure`]) to `_test.go` files only.
fn complex_functions(root: Node, source: &[u8], is_test_file: bool) -> Vec<(usize, usize)> {
    // Reuses `rules.rs`'s already-verified Go `function_kinds` table instead of an
    // independently-declared literal of the same two strings, so the two can't
    // silently drift apart if Go's grammar node names are ever revisited there.
    let function_kinds = crate::rules::lang_config(Language::Go).function_kinds;
    let mut out = Vec::new();
    collect_complex_functions(root, function_kinds, source, is_test_file, &mut out);
    out
}

fn collect_complex_functions(
    node: Node,
    function_kinds: &[&str],
    source: &[u8],
    is_test_file: bool,
    out: &mut Vec<(usize, usize)>,
) {
    if function_kinds.contains(&node.kind()) {
        let subtests = subtest_handling_for(is_test_file, node, source);
        let complexity = cyclomatic_complexity(node, source, subtests);
        if complexity > CYCLOMATIC_COMPLEXITY_THRESHOLD {
            out.push((node.start_position().row + 1, complexity));
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_complex_functions(child, function_kinds, source, is_test_file, out);
    }
}

/// Only a `Test`/`Benchmark`/`Fuzz`-named function in a `_test.go` file can plausibly
/// be Go's idiomatic table-test/subtest pattern — narrower than just "any
/// `.Run(closure)` call" so a same-named `.Run` method on a production type (e.g. a
/// retry runner) never has its closure's branching dropped.
fn subtest_handling_for(is_test_file: bool, decl: Node, source: &[u8]) -> SubtestHandling {
    if is_test_file && is_test_like_name(decl, source) {
        SubtestHandling::ExcludeRunSubtests
    } else {
        SubtestHandling::IncludeAll
    }
}

/// True if `decl` (a `function_declaration` or `method_declaration`) has a name
/// starting with `Test`, `Benchmark`, or `Fuzz` — Go's convention for functions the
/// `go test` tool invokes directly (<https://pkg.go.dev/testing>), the only place
/// `t.Run`/`b.Run`/`m.Run`-style subtests appear.
fn is_test_like_name(decl: Node, source: &[u8]) -> bool {
    decl.child_by_field_name("name")
        .and_then(|n| n.utf8_text(source).ok())
        .is_some_and(|name| {
            name.starts_with("Test") || name.starts_with("Benchmark") || name.starts_with("Fuzz")
        })
}

/// Whether `count_decision_points` should treat a `.Run(closure)` subtest's branching
/// as its own independent test case (excluded from the enclosing function's sum) or
/// fold it in as ordinary nested control flow. An enum rather than a bare `bool`
/// parameter: `IncludeAll` and `ExcludeRunSubtests` name the two behaviors at every
/// call site instead of leaving a reader to infer what `true`/`false` mean.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubtestHandling {
    IncludeAll,
    ExcludeRunSubtests,
}

/// McCabe cyclomatic complexity: 1 plus one per decision point — `if`, `for`, each
/// non-default `case`/`type case`/`communication case`, and each short-circuiting
/// `&&`/`||`. Closures (`func_literal`) are walked into and summed into the enclosing
/// function, matching `rules.rs`'s `max_nesting_depth` precedent for this grammar —
/// except a closure passed directly to a `.Run(...)` call inside a `Test`/`Benchmark`/
/// `Fuzz` function (`SubtestHandling::ExcludeRunSubtests`), which is Go's idiomatic
/// subtest pattern: each closure is its own independent test case, not control flow
/// entangled with its siblings, so summing them overstates the function's real
/// branching. Go-specific today (node kinds are hardcoded, not threaded through a
/// `LangRuleConfig`-style table) — #49/#55 should design their own multi-language
/// shape rather than inherit this one.
pub(crate) fn cyclomatic_complexity(decl: Node, source: &[u8], subtests: SubtestHandling) -> usize {
    1 + count_decision_points(decl, source, subtests)
}

fn count_decision_points(node: Node, source: &[u8], subtests: SubtestHandling) -> usize {
    if subtests == SubtestHandling::ExcludeRunSubtests
        && node.kind() == "func_literal"
        && is_run_subtest_closure(node, source)
    {
        return 0;
    }
    let mut count = match node.kind() {
        "if_statement" | "for_statement" | "expression_case" | "type_case"
        | "communication_case" => 1,
        "binary_expression" => usize::from(is_short_circuit(node)),
        _ => 0,
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decision_points(child, source, subtests);
    }
    count
}

fn is_short_circuit(binary_expression: Node) -> bool {
    binary_expression
        .child_by_field_name("operator")
        .is_some_and(|op| matches!(op.kind(), "&&" | "||"))
}

/// True if `func_literal` is passed as an argument to a call whose callee is a
/// selector ending in `.Run` — the shape of `t.Run(name, func(t *testing.T) {...})`,
/// `b.Run(...)`, and `m.Run(...)`. Scoped to exactly this call shape (not "any closure
/// in a test file") so a closure doing real work unrelated to subtests — e.g. one built
/// inline and invoked immediately, or passed to a helper that isn't named `Run` — is
/// unaffected.
fn is_run_subtest_closure(func_literal: Node, source: &[u8]) -> bool {
    let Some(args) = func_literal.parent() else {
        return false;
    };
    if args.kind() != "argument_list" {
        return false;
    }
    let Some(call) = args.parent() else {
        return false;
    };
    if call.kind() != "call_expression" {
        return false;
    }
    call.child_by_field_name("function")
        .filter(|f| f.kind() == "selector_expression")
        .and_then(|f| f.child_by_field_name("field"))
        .and_then(|f| f.utf8_text(source).ok())
        == Some("Run")
}

#[cfg(test)]
#[path = "complexity_tests.rs"]
mod tests;
