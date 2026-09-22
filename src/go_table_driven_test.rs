use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::duplicate_code::MIN_OCCURRENCES;

/// Literal-value node kinds treated as wildcards when comparing two test bodies — the
/// "inputs/expected values" issue #31 describes near-identical table-driven-test
/// candidates as differing only by.
const LITERAL_KINDS: &[&str] = &[
    "int_literal",
    "float_literal",
    "imaginary_literal",
    "rune_literal",
    "interpreted_string_literal",
    "raw_string_literal",
];

/// Minimum normalized-token count a test body must reach to be considered. A near-empty
/// body (`t.Skip()`) repeated verbatim is boilerplate, not a table-driven-test signal.
const MIN_NORMALIZED_TOKENS: usize = 8;

/// Flags 3+ `func TestXxx(t *testing.T)` functions in the same file whose bodies are
/// identical once literal values are normalized away — the classic copy-pasted
/// table-driven-test candidate (see the Go Wiki
/// <https://go.dev/wiki/TableDrivenTests>). Comparing whole normalized bodies, not
/// `duplicate-code`'s line-window overlap, is what keeps this from flagging tests that
/// only coincidentally share a setup prefix (e.g. `t.Run` subtest boilerplate) but
/// diverge afterward — those normalize to different strings and never group together.
pub struct TableDrivenTestChecker;

impl Checker for TableDrivenTestChecker {
    fn name(&self) -> &str {
        "go-table-driven-test-candidate"
    }

    fn description(&self) -> &str {
        "flags 3+ TestXxx functions in the same file whose bodies are identical except \
         for literal values — a table-driven test candidate"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*_test.go"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("go-table-driven-test-candidate checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();

        let mut groups: HashMap<String, Vec<(&str, usize)>> = HashMap::new();
        crate::tree_walk::walk_preorder(tree.root_node(), &mut |n| {
            if n.kind() == "function_declaration"
                && let Some((name, line, signature)) = test_function_signature(n, src)
            {
                groups.entry(signature).or_default().push((name, line));
            }
            true
        });

        let mut findings: Vec<Finding> = groups
            .into_values()
            .filter(|occurrences| occurrences.len() >= MIN_OCCURRENCES)
            .map(to_finding)
            .collect();
        findings.sort_by_key(|f| f.line);
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(TableDrivenTestChecker)])
}

fn to_finding(mut occurrences: Vec<(&str, usize)>) -> Finding {
    occurrences.sort_by_key(|(_, line)| *line);
    let last_line = occurrences.last().expect("filtered to non-empty groups").1;
    let names: Vec<&str> = occurrences.iter().map(|(name, _)| *name).collect();
    Finding {
        line: last_line,
        message: format!(
            "{} near-identical test functions look like table-driven test candidates: {} \
             — consider consolidating into one `tests := []struct{{...}}{{...}}` table",
            occurrences.len(),
            names.join(", ")
        ),
    }
}

/// If `decl` is a `func TestXxx(t *testing.T) {...}`-shaped declaration whose body
/// clears [`MIN_NORMALIZED_TOKENS`], returns its name, 1-indexed line, and a normalized
/// signature of its body (literal values replaced by a placeholder) for grouping
/// against other test functions in the same file.
fn test_function_signature<'a>(decl: Node<'a>, src: &'a [u8]) -> Option<(&'a str, usize, String)> {
    let name = decl.child_by_field_name("name")?.utf8_text(src).ok()?;
    if !is_go_test_name(name) {
        return None;
    }
    let params = decl.child_by_field_name("parameters")?;
    if !has_single_testing_t_param(params, src) {
        return None;
    }
    let body = decl.child_by_field_name("body")?;
    let tokens = normalize_body_tokens(body, src);
    if tokens.len() < MIN_NORMALIZED_TOKENS {
        return None;
    }
    Some((
        name,
        decl.start_position().row + 1,
        // Tokens can't themselves contain whitespace (identifiers/operators/keywords,
        // plus the literal placeholder), so a plain space join is a safe, simple
        // signature — no separate delimiter needed.
        tokens.join(" "),
    ))
}

/// Go's `go test` convention: `Test` followed immediately by an uppercase letter (or
/// nothing) — the same "prefix + capitalized rest" shape
/// `go_bulk_fetch_linear_scan.rs`'s `is_bulk_fetch_name` uses for its own name-prefix
/// check.
fn is_go_test_name(name: &str) -> bool {
    name == "Test" || (name.starts_with("Test") && name.as_bytes()[4].is_ascii_uppercase())
}

/// True if `params` (a function's own `parameter_list`) declares exactly one parameter,
/// bound to exactly one name, of type `*testing.T`.
fn has_single_testing_t_param(params: Node, src: &[u8]) -> bool {
    let mut cursor = params.walk();
    let decls: Vec<Node> = params
        .children(&mut cursor)
        .filter(|n| n.kind() == "parameter_declaration")
        .collect();
    let [decl] = decls.as_slice() else {
        return false;
    };
    let mut name_cursor = decl.walk();
    if decl
        .children_by_field_name("name", &mut name_cursor)
        .count()
        != 1
    {
        return false;
    }
    decl.child_by_field_name("type")
        .is_some_and(|ty| is_testing_t_pointer(ty, src))
}

/// True for a `pointer_type` wrapping the qualified type `testing.T` — i.e. `*testing.T`.
fn is_testing_t_pointer(ty: Node, src: &[u8]) -> bool {
    ty.kind() == "pointer_type"
        && ty
            .named_child(0)
            .and_then(|inner| inner.utf8_text(src).ok())
            == Some("testing.T")
}

/// Leaf tokens of `body` in document order, with each literal-value node (see
/// [`LITERAL_KINDS`]) collapsed to a single placeholder token and comments dropped —
/// identifiers, keywords, and operators are kept verbatim, so two bodies normalize
/// equal only when they call the same things in the same shape and differ solely in
/// literal values.
///
/// A literal node stops the walk from descending into it (e.g. an
/// `interpreted_string_literal` wraps its quotes and an `interpreted_string_literal_content`
/// child rather than being a leaf itself) — otherwise its content would be emitted as an
/// ordinary token instead of collapsed away.
fn normalize_body_tokens<'a>(body: Node<'a>, src: &'a [u8]) -> Vec<&'a str> {
    let mut tokens = Vec::new();
    crate::tree_walk::walk_preorder(body, &mut |n| {
        if LITERAL_KINDS.contains(&n.kind()) {
            tokens.push("<lit>");
            return false;
        }
        if n.kind() != "comment"
            && n.child_count() == 0
            && let Ok(text) = n.utf8_text(src)
        {
            tokens.push(text);
        }
        true
    });
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Vec<Finding> {
        crate::test_support::check_go_source(&TableDrivenTestChecker, src).unwrap()
    }

    fn test_fn(name: &str, arg: &str, want: &str) -> String {
        format!(
            "func {name}(t *testing.T) {{\n\
             \tresult := doSomething(\"{arg}\")\n\
             \tif result != \"{want}\" {{\n\
             \t\tt.Fatalf(\"got %q, want %q\", result, \"{want}\")\n\
             \t}}\n\
             }}\n"
        )
    }

    #[test]
    fn flags_three_near_identical_tests_differing_only_in_literals() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            test_fn("TestFoo", "a", "A"),
            test_fn("TestBar", "b", "B"),
            test_fn("TestBaz", "c", "C"),
        );
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("TestFoo, TestBar, TestBaz"));
        assert!(findings[0].message.contains("3 near-identical"));
    }

    #[test]
    fn does_not_flag_only_two_near_identical_tests() {
        let src = format!(
            "package main\n\n{}\n{}",
            test_fn("TestFoo", "a", "A"),
            test_fn("TestBar", "b", "B"),
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_tests_with_different_shapes() {
        let src = "package main\n\n\
                    func TestFoo(t *testing.T) {\n\
                    \tresult := doSomething(\"a\")\n\
                    \tif result != \"A\" {\n\
                    \t\tt.Fatalf(\"got %q, want %q\", result, \"A\")\n\
                    \t}\n\
                    }\n\n\
                    func TestBar(t *testing.T) {\n\
                    \tconn := openConnection()\n\
                    \tdefer conn.Close()\n\
                    \tif err := conn.Ping(); err != nil {\n\
                    \t\tt.Fatal(err)\n\
                    \t}\n\
                    }\n\n\
                    func TestBaz(t *testing.T) {\n\
                    \tif 1+1 != 2 {\n\
                    \t\tt.Fatal(\"math is broken\")\n\
                    \t}\n\
                    }\n";
        assert!(check_source(src).is_empty());
    }

    #[test]
    fn does_not_flag_non_test_functions() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            test_fn("doWork", "a", "A").replace("t *testing.T", "t int"),
            test_fn("helperTwo", "b", "B").replace("t *testing.T", "t int"),
            test_fn("helperThree", "c", "C").replace("t *testing.T", "t int"),
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_trivial_near_empty_test_bodies() {
        let block = "func {name}(t *testing.T) {\n\tt.Skip()\n}\n";
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            block.replace("{name}", "TestFoo"),
            block.replace("{name}", "TestBar"),
            block.replace("{name}", "TestBaz"),
        );
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_tests_that_diverge_after_a_shared_setup_prefix() {
        // Same `t.Run` setup boilerplate, but each subtest body does something
        // different — the false-positive concern flagged in #31 for line-window-based
        // duplicate detection. Whole-body normalization must not group these: they
        // diverge past the shared prefix, so their signatures differ.
        let src = "package main\n\n\
                    func TestFoo(t *testing.T) {\n\
                    \tt.Run(\"case\", func(t *testing.T) {\n\
                    \t\tresult := doSomething(\"a\")\n\
                    \t\tassertEqual(t, result, \"A\")\n\
                    \t})\n\
                    }\n\n\
                    func TestBar(t *testing.T) {\n\
                    \tt.Run(\"case\", func(t *testing.T) {\n\
                    \t\tconn := openConnection()\n\
                    \t\tdefer conn.Close()\n\
                    \t\tassertNoError(t, conn.Ping())\n\
                    \t})\n\
                    }\n\n\
                    func TestBaz(t *testing.T) {\n\
                    \tt.Run(\"case\", func(t *testing.T) {\n\
                    \t\tvalue := compute(1, 2)\n\
                    \t\tassertEqual(t, value, 3)\n\
                    \t})\n\
                    }\n";
        assert!(check_source(src).is_empty());
    }

    #[test]
    fn reports_last_occurrence_line_and_all_names() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            test_fn("TestFoo", "a", "A"),
            test_fn("TestBar", "b", "B"),
            test_fn("TestBaz", "c", "C"),
        );
        let findings = check_source(&src);
        let expected_line = src
            .lines()
            .position(|l| l.contains("func TestBaz"))
            .map(|i| i + 1)
            .unwrap();
        assert_eq!(findings[0].line, expected_line);
    }
}
