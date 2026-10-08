//! Anchor conformance: for every default per-file check, a directive placed where the anchor
//! convention says (the row above the finding, the finding's own row, or the file head for
//! file-scope rules) drops the real checker's finding. Adding a default check without a fixture
//! or an exemption fails `anchor_conformance_should_HaveFixtureOrExemption_When_DefaultCheckerAdded`.
#![allow(non_snake_case)]

use std::path::Path;

use crate::checker::{Finding, run_checker_configured};
use crate::config::{Check, default_checks};
use crate::inline_ignores::IgnoreTarget;
use crate::inline_ignores::{
    FILE_SCOPE_RULES, InlineIgnoreContext, anchor_rule, apply_inline_ignores,
};

type Source = fn() -> String;

/// (check name, fixture path, source) where the real checker fires.
const ANCHOR_FIXTURES: &[(&str, &str, Source)] = &[
    ("markdown-link-integrity", "n.md", md_undefined_link),
    ("primitive-obsession", "x.go", go_primitive_obsession),
    ("file-complexity", "x.go", go_three_complex_functions),
    ("duplicate-code", "x.go", go_triplicated_block),
    ("duplicate-code-cross-file", "c.go", cross_file_block),
    ("go-ignored-error", "x.go", go_ignored_error),
    ("go-error-context", "x.go", go_error_context),
    ("go-type-switch-density", "x.go", go_type_switch),
    ("go-encapsulate-collection", "x.go", go_slice_getter),
    ("go-bulk-fetch-linear-scan", "x.go", go_bulk_fetch),
    (
        "go-table-driven-test-candidate",
        "x_test.go",
        go_near_identical_tests,
    ),
    ("java-ignored-error", "C.java", java_ignored_error),
    ("java-error-context", "C.java", java_error_context),
    (
        "java-swallowed-interrupt",
        "C.java",
        java_swallowed_interrupt,
    ),
    ("java-lost-exception-cause", "C.java", java_lost_cause),
    ("go-file-size", "x.go", || long_file("//")),
    ("typescript-file-size", "x.ts", || long_file("//")),
    ("tsx-file-size", "x.tsx", || long_file("//")),
    ("javascript-file-size", "x.js", || long_file("//")),
    ("python-file-size", "x.py", || long_file("#")),
    ("java-file-size", "C.java", || long_file("//")),
    ("kotlin-file-size", "x.kt", || long_file("//")),
    ("rust-file-size", "x.rs", || long_file("//")),
    ("syntax-rules-go", "x.go", || {
        "package main\n\nfunc f() {\n\ta := 86400\n\tb := 86400\n\tc := 86400\n}\n".into()
    }),
    ("syntax-rules-typescript", "x.ts", ts_magic_literal),
    ("syntax-rules-tsx", "x.tsx", ts_magic_literal),
    ("syntax-rules-javascript", "x.js", ts_magic_literal),
    ("syntax-rules-python", "x.py", || {
        "def f():\n    print(42)\n    print(42)\n    print(42)\n".into()
    }),
    ("syntax-rules-java", "C.java", || {
        "class C {\n  void f() {\n    System.out.println(42);\n    System.out.println(42);\n    System.out.println(42);\n  }\n}\n".into()
    }),
    ("syntax-rules-kotlin", "x.kt", || {
        "fun f() {\n  println(42)\n  println(42)\n  println(42)\n}\n".into()
    }),
    ("syntax-rules-rust", "x.rs", || {
        "fn f() {\n    use_it(42);\n    use_it(42);\n    use_it(42);\n}\n".into()
    }),
    ("comment-quality-go", "x.go", || {
        "package main\n\n// This seamlessly handles requests.\nfunc handle() {}\n".into()
    }),
    ("comment-quality-typescript", "x.ts", slash_comment_function),
    ("comment-quality-tsx", "x.tsx", slash_comment_function),
    ("comment-quality-javascript", "x.js", slash_comment_function),
    ("comment-quality-python", "x.py", || {
        "# This seamlessly handles requests.\ndef handle():\n    pass\n".into()
    }),
    ("comment-quality-java", "C.java", || {
        "class C {\n    // This seamlessly handles requests.\n    void handle() {}\n}\n".into()
    }),
    ("comment-quality-kotlin", "x.kt", || {
        "// This seamlessly handles requests.\nfun handle() {}\n".into()
    }),
    ("comment-quality-rust", "x.rs", || {
        "// This seamlessly handles requests.\nfn handle() {}\n".into()
    }),
    ("repetitive-sentence-structure", "n.md", || {
        "# T\n\nThis is bad. This is worse. This is worst.\n".into()
    }),
    ("missing-paragraph-break", "n.md", || {
        "# T\n\nOne. Two. Three. Four. Five. Six. However, seven changes topic.\n".into()
    }),
];

/// Default checks with no per-file anchor to place a directive on, each with the reason.
const ANCHOR_EXEMPT: &[(&str, &str)] = &[
    (
        "inline-ignore",
        "meta checker: emits only meta rules, which no directive can cover",
    ),
    (
        "go-package-size",
        "whole-repo architecture check, reports packages rather than file rows",
    ),
    (
        "go-blank-imports",
        "an adjacent comment of any kind counts as the justification, so a directive above the \
         import clears the finding by construction rather than by suppression",
    ),
];

/// Checks where only the whole-line placement conforms: a trailing comment on the reported row
/// becomes part of the duplicated block's text, so the block stops matching and the finding
/// re-anchors one row lower.
const TRAILING_EXEMPT: &[(&str, &str)] = &[
    (
        "duplicate-code",
        "trailing comment alters the block text and drifts the anchor to the next row",
    ),
    (
        "duplicate-code-cross-file",
        "trailing comment alters the block text and drifts the anchor to the next row",
    ),
];

/// Default checks whose fixture is still to write; each needs a tracking note.
const ANCHOR_PENDING: &[(&str, &str)] = &[];

fn md_undefined_link() -> String {
    "# T\n\nSee [foo] here.\n".into()
}

fn go_primitive_obsession() -> String {
    "package main\n\nfunc f(a string, b string) {}\n".into()
}

fn complex_func(name: &str) -> String {
    let mut body = String::new();
    for i in 0..11 {
        let lead = if i == 0 { "\tif" } else { " else if" };
        body.push_str(&format!("{lead} x == {i} {{\n\t\treturn\n\t}}"));
    }
    format!("func {name}(x int) {{\n{body}\n}}\n")
}

fn go_three_complex_functions() -> String {
    format!(
        "package main\n\n{}\n{}\n{}",
        complex_func("a"),
        complex_func("b"),
        complex_func("c")
    )
}

const DO_WORK: &str = "func doWork(id string) error {\n\
    \tconn := openConnection(id)\n\
    \tdefer conn.Close()\n\
    \tresult := conn.Fetch(id)\n\
    \tlog.Printf(\"fetched %v\", result)\n\
    \treturn conn.Validate(result)\n}\n";

fn go_triplicated_block() -> String {
    format!("package main\n\n{DO_WORK}\n{DO_WORK}\n{DO_WORK}")
}

fn cross_file_block() -> String {
    format!("package other\n\n{DO_WORK}")
}

fn go_ignored_error() -> String {
    "package main\n\nfunc f() (int, error) { return 0, nil }\n\nfunc g() {\n\tresult, _ := f()\n\t_ = result\n}\n".into()
}

fn go_error_context() -> String {
    "package main\n\nimport \"fmt\"\n\nfunc wrap(err error) error {\n\treturn fmt.Errorf(\"wrap: %w\", err)\n}\n\nfunc g() error {\n\terr := doThing()\n\tif err != nil {\n\t\treturn err\n\t}\n\treturn nil\n}\n".into()
}

fn go_type_switch() -> String {
    "package p\n\nfunc F(x interface{}) {\n\tswitch x.(type) {\n\tcase int:\n\tcase string:\n\tcase bool:\n\tcase float64:\n\t}\n}\n".into()
}

fn go_slice_getter() -> String {
    "package p\n\ntype S struct {\n\titems []int\n}\n\nfunc (s *S) Items() []int {\n\treturn s.items\n}\n".into()
}

fn go_bulk_fetch() -> String {
    "package main\n\nfunc (s *Store) FindInstanceDataByID(id string) (*Data, error) {\n\tall, err := s.ListInstanceData()\n\tif err != nil {\n\t\treturn nil, err\n\t}\n\tfor _, item := range all {\n\t\tif item.ID == id {\n\t\t\treturn &item, nil\n\t\t}\n\t}\n\treturn nil, ErrNotFound\n}\n".into()
}

fn go_near_identical_tests() -> String {
    let test_fn = |name: &str, arg: &str, want: &str| {
        format!(
            "func {name}(t *testing.T) {{\n\tresult := doSomething(\"{arg}\")\n\tif result != \"{want}\" {{\n\t\tt.Fatalf(\"got %q, want %q\", result, \"{want}\")\n\t}}\n}}\n"
        )
    };
    format!(
        "package main\n\n{}\n{}\n{}",
        test_fn("TestFoo", "a", "A"),
        test_fn("TestBar", "b", "B"),
        test_fn("TestBaz", "c", "C")
    )
}

fn java_ignored_error() -> String {
    "class Foo {\n  void bar() {\n    try { doThing(); } catch (IOException e) { }\n  }\n}\n".into()
}

fn java_error_context() -> String {
    "class Foo {\n  void wrap() { try { doThing(); } catch (IOException e) { throw new RuntimeException(\"wrap failed\", e); } }\n  void g() {\n    try { doThing(); } catch (IOException e) { throw e; }\n  }\n}\n".into()
}

fn java_swallowed_interrupt() -> String {
    "class Foo {\n  void bar() {\n    try { a(); } catch (InterruptedException e) { }\n  }\n}\n"
        .into()
}

fn java_lost_cause() -> String {
    "class Foo {\n  void bar() {\n    try { a(); } catch (IOException e) { throw new RuntimeException(\"boom\"); }\n  }\n}\n".into()
}

/// A file one line over the 500-line threshold, built from comment lines in the leader's syntax.
fn long_file(leader: &str) -> String {
    format!("{leader} filler\n").repeat(501)
}

fn ts_magic_literal() -> String {
    "function f() {\n  console.log(42);\n  console.log(42);\n  console.log(42);\n}\n".into()
}

fn slash_comment_function() -> String {
    "// This seamlessly handles requests.\nfunction handle() {}\n".into()
}

fn real_findings(check: &Check, path: &str, source: &str) -> Vec<Finding> {
    if check.name == "duplicate-code-cross-file" {
        let sibling = cross_file_block();
        return crate::checkers::duplicate_cross_file_checker::findings_with_siblings(
            &[("a.go", sibling.as_str()), ("b.go", sibling.as_str())],
            path,
            source,
        );
    }
    let checker = check.checker.as_deref().expect("per-file check");
    run_checker_configured(checker, Path::new(path), source, check.options.as_ref())
        .unwrap_or_else(|e| panic!("{}: {e:#}", check.name))
}

fn checker_name(check: &Check) -> &str {
    check.checker.as_deref().unwrap_or(&check.name)
}

fn comment_syntax(path: &str) -> (&'static str, &'static str) {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("md") => ("<!-- ", " -->"),
        Some("py") => ("# ", ""),
        _ => ("// ", ""),
    }
}

fn directive(path: &str, rule: &str) -> String {
    let (open, close) = comment_syntax(path);
    format!("{open}kibitzer:ignore {rule} -- conformance fixture{close}")
}

fn insert_line_before(source: &str, row: usize, text: &str) -> String {
    let mut lines: Vec<&str> = source.split('\n').collect();
    lines.insert(row - 1, text);
    lines.join("\n")
}

fn append_trailing(source: &str, row: usize, text: &str) -> String {
    let mut lines: Vec<String> = source.split('\n').map(str::to_string).collect();
    lines[row - 1] = format!("{} {text}", lines[row - 1]);
    lines.join("\n")
}

fn is_comment_row(line: &str) -> bool {
    let t = line.trim_start();
    ["//", "#", "/*", "*", "<!--"]
        .iter()
        .any(|l| t.starts_with(l))
}

/// Reruns the real checker on `source` and requires the `rule` finding at `line` to be dropped.
fn expect_dropped(
    check: &Check,
    path: &str,
    source: &str,
    rule: &str,
    line: usize,
) -> Result<(), String> {
    let checker = checker_name(check);
    let applied = apply_inline_ignores(
        real_findings(check, path, source),
        IgnoreTarget {
            file: Path::new(path),
            source,
            checker_name: checker,
            severity: check.severity,
        },
        &InlineIgnoreContext::default(),
    );
    let dropped = applied
        .dropped
        .iter()
        .any(|d| d.rule.as_str() == rule && d.finding_line.get() == line);
    let remains = applied
        .kept
        .iter()
        .any(|f| f.line == line && anchor_rule(checker, f).as_str() == rule);
    if dropped && !remains {
        return Ok(());
    }
    Err(format!(
        "{}: expected [{rule}] at line {line} dropped; dropped={:?} kept={:?}",
        check.name,
        applied
            .dropped
            .iter()
            .map(|d| d.finding_line.get())
            .collect::<Vec<_>>(),
        applied
            .kept
            .iter()
            .map(|f| (f.line, &f.message))
            .collect::<Vec<_>>(),
    ))
}

fn conformance_failures(check: &Check, path: &str, source: &str) -> Vec<String> {
    let checker = checker_name(check);
    let original = real_findings(check, path, source);
    if original.is_empty() {
        return vec![format!(
            "{}: fixture does not make the checker fire",
            check.name
        )];
    }
    let rows: Vec<&str> = source.split('\n').collect();
    let mut failures = Vec::new();
    for finding in &original {
        let rule = anchor_rule(checker, finding);
        let rule = rule.as_str();
        let text = directive(path, rule);
        if FILE_SCOPE_RULES.contains(&rule) {
            let head = insert_line_before(source, 1, &text);
            failures.extend(expect_dropped(check, path, &head, rule, finding.line + 1).err());
            continue;
        }
        let above = insert_line_before(source, finding.line, &text);
        failures.extend(expect_dropped(check, path, &above, rule, finding.line + 1).err());
        // A trailing directive on a comment row would stop being a comment-start directive.
        let trailing_ok = !TRAILING_EXEMPT.iter().any(|(n, _)| *n == check.name);
        if trailing_ok && !is_comment_row(rows[finding.line - 1]) {
            let (open, close) = comment_syntax(path);
            let trailing = append_trailing(
                source,
                finding.line,
                &format!("{open}kibitzer:ignore {rule} -- conformance fixture{close}"),
            );
            failures.extend(expect_dropped(check, path, &trailing, rule, finding.line).err());
        }
    }
    failures
}

fn fixture_for(name: &str) -> Option<&'static (&'static str, &'static str, Source)> {
    ANCHOR_FIXTURES.iter().find(|(n, _, _)| *n == name)
}

#[test]
fn anchor_conformance_should_DropFinding_When_DirectiveOnReportedLineOrRowAbove() {
    let defaults = default_checks();
    let mut failures = Vec::new();
    for check in defaults.iter().filter(|c| fixture_for(&c.name).is_some()) {
        let (_, path, source) = fixture_for(&check.name).unwrap();
        failures.extend(conformance_failures(check, path, &source()));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn anchor_conformance_should_HaveFixtureOrExemption_When_DefaultCheckerAdded() {
    let defaults = default_checks();
    let names: Vec<&str> = defaults.iter().map(|c| c.name.as_str()).collect();
    let uncovered: Vec<&&str> = names
        .iter()
        .filter(|n| {
            fixture_for(n).is_none()
                && !ANCHOR_EXEMPT.iter().any(|(e, _)| e == *n)
                && !ANCHOR_PENDING.iter().any(|(p, _)| p == *n)
        })
        .collect();
    assert!(
        uncovered.is_empty(),
        "default checks without an ANCHOR_FIXTURES, ANCHOR_EXEMPT or ANCHOR_PENDING entry: {uncovered:?}"
    );
    let listed = ANCHOR_FIXTURES
        .iter()
        .map(|(n, _, _)| *n)
        .chain(ANCHOR_EXEMPT.iter().map(|(n, _)| *n))
        .chain(ANCHOR_PENDING.iter().map(|(n, _)| *n));
    let stale: Vec<&str> = listed.filter(|n| !names.contains(n)).collect();
    assert!(
        stale.is_empty(),
        "entries naming no default check: {stale:?}"
    );
    assert!(ANCHOR_EXEMPT.iter().all(|(_, why)| !why.is_empty()));
    assert!(ANCHOR_PENDING.iter().all(|(_, note)| !note.is_empty()));
}

// Explicit rows for the awkward anchors (multi-line comments, block comments, file-head rules).

fn check_named(name: &str) -> Check {
    default_checks()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no default check {name}"))
}

/// Findings left after applying inline ignores, as `(line, message)`.
fn kept_after_apply(
    check: &Check,
    path: &str,
    source: &str,
    findings: Vec<Finding>,
) -> Vec<(usize, String)> {
    apply_inline_ignores(
        findings,
        IgnoreTarget {
            file: Path::new(path),
            source,
            checker_name: checker_name(check),
            severity: check.severity,
        },
        &InlineIgnoreContext::default(),
    )
    .kept
    .into_iter()
    .map(|f| (f.line, f.message))
    .collect()
}

fn kept_for_real_run(name: &str, path: &str, source: &str) -> Vec<(usize, String)> {
    let check = check_named(name);
    let findings = real_findings(&check, path, source);
    kept_after_apply(&check, path, source, findings)
}

fn ignore_line(rule: &str) -> String {
    format!("// kibitzer:ignore {rule} -- generated tables\n")
}

#[test]
fn file_size_ignore_should_Suppress_When_DirectiveInFirstTenLines() {
    let body = "// filler\n".repeat(899);
    let at_row = |row: usize| {
        let mut lines: Vec<&str> = body.lines().collect();
        let directive = ignore_line("file-size");
        lines.insert(row - 1, directive.trim_end());
        format!("{}\n", lines.join("\n"))
    };
    assert!(kept_for_real_run("go-file-size", "x.go", &at_row(1)).is_empty());
    assert!(kept_for_real_run("go-file-size", "x.go", &at_row(10)).is_empty());
}

#[test]
fn file_size_ignore_should_NotSuppress_When_DirectiveBelowRowTen() {
    let mut lines: Vec<String> = vec!["// filler".to_string(); 899];
    lines.insert(10, ignore_line("file-size").trim_end().to_string());
    let source = format!("{}\n", lines.join("\n"));
    let kept = kept_for_real_run("go-file-size", "x.go", &source);
    assert_eq!(kept.len(), 1, "{kept:?}");
}

#[test]
fn file_complexity_ignore_should_DropOnlyOneFinding_When_DirectiveAboveFirstFunction() {
    let base = go_three_complex_functions();
    let all = kept_for_real_run("file-complexity", "x.go", &base);
    assert_eq!(all.len(), 3, "{all:?}");
    let first_func_row = all[0].0;
    let above_first = insert_line_before(
        &base,
        first_func_row,
        ignore_line("file-complexity").trim_end(),
    );
    // The directive sits below row 10 only when the function does; this fixture's first function is at row 3.
    assert!(first_func_row <= 10);
    assert!(kept_for_real_run("file-complexity", "x.go", &above_first).is_empty());
    let late = format!("{}{}", "// pad\n".repeat(20), base);
    let late_rows = kept_for_real_run("file-complexity", "x.go", &late);
    assert_eq!(late_rows.len(), 3, "{late_rows:?}");
    let above_late_first = insert_line_before(
        &late,
        late_rows[0].0,
        ignore_line("file-complexity").trim_end(),
    );
    assert_eq!(
        kept_for_real_run("file-complexity", "x.go", &above_late_first).len(),
        2
    );
    let head = insert_line_before(&late, 1, ignore_line("file-complexity").trim_end());
    assert!(kept_for_real_run("file-complexity", "x.go", &head).is_empty());
}

#[test]
fn duplicate_code_ignore_should_Suppress_When_AboveLastOccurrence() {
    let base = go_triplicated_block();
    let last = real_findings(&check_named("duplicate-code"), "x.go", &base)[0].line;
    let above_last = insert_line_before(&base, last, ignore_line("duplicate-code").trim_end());
    assert!(kept_for_real_run("duplicate-code", "x.go", &above_last).is_empty());
    let above_first = insert_line_before(&base, 3, ignore_line("duplicate-code").trim_end());
    assert_eq!(
        kept_for_real_run("duplicate-code", "x.go", &above_first).len(),
        1
    );
}

#[test]
fn duplicate_cross_file_ignore_should_CoverOnlyOwnFile_When_IgnoreInAGoOnly() {
    let check = check_named("duplicate-code-cross-file");
    let plain = cross_file_block();
    let with_ignore = insert_line_before(
        &plain,
        3,
        ignore_line("duplicate-code-cross-file").trim_end(),
    );
    let kept = |target: &str, source: &str, siblings: [(&str, &str); 2]| {
        let findings = crate::checkers::duplicate_cross_file_checker::findings_with_siblings(
            &siblings, target, source,
        );
        kept_after_apply(&check, target, source, findings)
    };
    let a_clean = kept(
        "a.go",
        &with_ignore,
        [("b.go", plain.as_str()), ("c.go", plain.as_str())],
    );
    let b_reports = kept(
        "b.go",
        &plain,
        [("a.go", with_ignore.as_str()), ("c.go", plain.as_str())],
    );
    assert!(a_clean.is_empty(), "{a_clean:?}");
    assert_eq!(b_reports.len(), 1, "{b_reports:?}");
}

fn leading_comment_block(lines: usize) -> String {
    let comments: String = (0..lines)
        .map(|n| format!("// note number {n} about the function below\n"))
        .collect();
    format!("package main\n\n{comments}func f() {{}}\n")
}

#[test]
fn over_commented_ignore_should_Suppress_When_AboveFirstLeadingCommentRow() {
    let base = leading_comment_block(8);
    let check = check_named("comment-quality-go");
    let findings = real_findings(&check, "x.go", &base);
    let over: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.message.starts_with("[over-commented]"))
        .collect();
    assert_eq!(
        over.len(),
        1,
        "{:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    let ignored = insert_line_before(
        &base,
        over[0].line,
        ignore_line("over-commented").trim_end(),
    );
    let kept = kept_for_real_run("comment-quality-go", "x.go", &ignored);
    assert!(
        kept.iter().all(|(_, m)| !m.starts_with("[over-commented]")),
        "{kept:?}"
    );
}

#[test]
fn commented_out_code_ignore_should_Suppress_When_AboveThatRow() {
    let base = "package main\n\nfunc f() {\n\t// x = doSomething(1, 2);\n}\n";
    let check = check_named("comment-quality-go");
    let findings = real_findings(&check, "x.go", base);
    let dead: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.message.starts_with("[commented-out-code]"))
        .collect();
    assert_eq!(
        dead.len(),
        1,
        "{:?}",
        findings.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    let ignored = insert_line_before(
        base,
        dead[0].line,
        ignore_line("commented-out-code").trim_end(),
    );
    let kept = kept_for_real_run("comment-quality-go", "x.go", &ignored);
    assert!(
        kept.iter()
            .all(|(_, m)| !m.starts_with("[commented-out-code]")),
        "{kept:?}"
    );
}

fn md_ignore(rule: &str) -> String {
    format!("<!-- kibitzer:ignore {rule} -- placeholder reference -->")
}

#[test]
fn markdown_link_integrity_ignore_should_Suppress_When_NamedByCheckerName() {
    let sources = [
        ("used but never defined", "# T\n\nSee [foo] here.\n"),
        (
            "defined but never used",
            "# T\n\nText.\n\n[foo]: https://example.com\n",
        ),
        (
            "file does not exist",
            "# T\n\nSee [foo] here.\n\n[foo]: ./no-such-file-anywhere.md\n",
        ),
    ];
    for (variant, source) in sources {
        let findings = real_findings(&check_named("markdown-link-integrity"), "n.md", source);
        assert!(!findings.is_empty(), "{variant}: no finding");
        let row = findings[0].line;
        assert!(
            findings[0].message.contains(variant),
            "{variant}: {}",
            findings[0].message
        );
        let by_checker = insert_line_before(source, row, &md_ignore("markdown-link-integrity"));
        assert!(
            kept_for_real_run("markdown-link-integrity", "n.md", &by_checker).is_empty(),
            "{variant}"
        );
        let by_ref_id = insert_line_before(source, row, &md_ignore("foo"));
        assert_eq!(
            kept_for_real_run("markdown-link-integrity", "n.md", &by_ref_id).len(),
            findings.len(),
            "{variant}: a directive naming the ref id must not match"
        );
    }
}
