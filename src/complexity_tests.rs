use super::*;
use crate::checker::GrammarCache;

fn check_source(src: &str) -> Vec<Finding> {
    check_source_at(Path::new("<source>"), src)
}

fn check_source_at(path: &Path, src: &str) -> Vec<Finding> {
    let cache = GrammarCache::new();
    let tree = cache.parse(Language::Go, src).unwrap();
    let ctx = CheckContext {
        source: src,
        tree: Some(&tree),
    };
    FileComplexityChecker.check(path, &ctx).unwrap()
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
    cyclomatic_complexity(decl, src.as_bytes(), SubtestHandling::IncludeAll)
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
    assert_eq!(
        complex_functions(tree.root_node(), src.as_bytes(), false).len(),
        1
    );
}

#[test]
fn skips_generated_file_even_with_a_complex_function() {
    let mut src = "// Code generated by validation-gen. DO NOT EDIT.\n".to_string();
    src.push_str(&complex_func("validate", 20));
    assert!(check_source(&src).is_empty());
}

/// `n` independent `t.Run(name, func(t *testing.T) {...})` subtests, each with one
/// `if err != nil { return }` — the idiomatic Go table-test/subtest shape from
/// `docs/file-complexity-false-positives.md`'s `TestGitProviderGetChangedFiles`
/// example.
fn test_func_with_run_subtests(name: &str, subtests: usize) -> String {
    let mut body = String::new();
    for i in 0..subtests {
        body.push_str(&format!(
            "\tt.Run(\"case{i}\", func(t *testing.T) {{\n\
                 \t\tif err != nil {{\n\t\t\treturn\n\t\t}}\n\
                 \t}})\n"
        ));
    }
    format!("func {name}(t *testing.T) {{\n{body}}}\n")
}

#[test]
fn does_not_sum_independent_t_run_subtests_in_a_test_file() {
    // 5 subtests, each contributing 1 decision point if summed naively -> complexity
    // 1 (base) + 5 = 6 without the fix. With the fix, none of the t.Run closures'
    // branching should count, leaving complexity at the base case: 1.
    let src = format!(
        "package foo\n\nimport \"testing\"\n\n{}",
        test_func_with_run_subtests("TestGitProviderGetChangedFiles", 5)
    );
    let cache = GrammarCache::new();
    let tree = cache.parse(Language::Go, &src).unwrap();
    let mut cursor = tree.root_node().walk();
    let decl = tree
        .root_node()
        .children(&mut cursor)
        .find(|n| n.kind() == "function_declaration")
        .unwrap();
    assert_eq!(
        cyclomatic_complexity(decl, src.as_bytes(), SubtestHandling::ExcludeRunSubtests),
        1
    );

    // And end to end via the checker: a _test.go path with enough t.Run-heavy Test
    // functions to have crossed MIN_COMPLEX_FUNCTIONS under the old (unfixed) sum
    // must no longer fire.
    let src = format!(
        "package foo\n\nimport \"testing\"\n\n{}{}{}",
        test_func_with_run_subtests("TestA", 15),
        test_func_with_run_subtests("TestB", 15),
        test_func_with_run_subtests("TestC", 15)
    );
    assert!(check_source_at(Path::new("git_provider_test.go"), &src).is_empty());
}

#[test]
fn still_sums_a_run_closures_branching_outside_a_test_file() {
    // Same `.Run(func(){...})` shape, but not in a _test.go file and not inside a
    // Test/Benchmark/Fuzz-named function (e.g. a retry runner's `r.Run(...)`) — its
    // branching must still count, since this is real control flow, not a subtest.
    let src = "package foo\n\nfunc process(r Runner) {\n\
                    \tr.Run(func() {\n\
                    \t\tif true {\n\t\t\tif true {\n\t\t\t\tif true {\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\
                    \t})\n}\n";
    // 1 (base) + 3 (nested ifs inside the closure, summed as before the fix).
    assert_eq!(complexity_of(src), 4);
}

#[test]
fn still_sums_a_run_closures_branching_in_a_non_test_named_function_in_a_test_file() {
    // `.Run(closure)` inside a _test.go file, but the enclosing function isn't
    // Test/Benchmark/Fuzz-named (a plain helper) -> not the go test subtest
    // convention, so it must still be summed. Uses `complex_functions` (not
    // `cyclomatic_complexity` directly) so the `is_test_like_name` gate that
    // `collect_complex_functions` applies is actually exercised, not bypassed.
    let mut body = String::new();
    for i in 0..11 {
        if i == 0 {
            body.push_str(&format!("\t\tif x == {i} {{\n\t\t\treturn\n\t\t}}"));
        } else {
            body.push_str(&format!(" else if x == {i} {{\n\t\t\treturn\n\t\t}}"));
        }
    }
    let src = format!(
        "package foo\n\nfunc setupRunner(r Runner) {{\n\tr.Run(func(x int) {{\n{body}\n\t}})\n}}\n"
    );
    let cache = GrammarCache::new();
    let tree = cache.parse(Language::Go, &src).unwrap();
    // is_test_file: true (as if read from helper_test.go), but "setupRunner" isn't
    // Test/Benchmark/Fuzz-named, so the closure's 11 branches must still be summed:
    // 1 (base) + 11 = 12, over threshold, so it shows up here.
    let complex = complex_functions(tree.root_node(), src.as_bytes(), true);
    assert_eq!(complex, vec![(3, 12)]);
}

#[test]
fn a_genuinely_complex_hand_written_function_still_flags() {
    // Not generated, not a test file, not a t.Run pattern — a real
    // over-threshold function must still fire (true positive preserved).
    let src = format!(
        "package main\n\n{}{}{}",
        complex_func("a", 11),
        complex_func("b", 11),
        complex_func("c", 11)
    );
    let findings = check_source_at(Path::new("server.go"), &src);
    assert_eq!(findings.len(), 3);
}
