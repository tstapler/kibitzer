//! `run_checks_for_trigger` end to end: the post-pass fed by the real check pipeline.
//! The post-pass's own unit tests live beside it and use an injected rerun instead.

#![allow(non_snake_case)]

use super::*;
use crate::plugin::Registry;
use std::sync::atomic::Ordering;

#[test]
fn run_checks_for_trigger_should_EmitBlockingSuppressedAdvisory_When_AddedDirectiveSilencesBlocking()
 {
    let dir = std::env::temp_dir().join(format!("kibitzer-post-pass-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("notes.md");
    std::fs::write(
        &file,
        "# T\n\n<!-- kibitzer:ignore markdown-link-integrity -- placeholder for later -->\nSee [foo] here.\n",
    )
    .unwrap();
    let results = crate::check::run_checks_for_trigger(
        &crate::config::default_checks(),
        "PostToolUse",
        &dir,
        &file,
        Some(&[(3, 3)]),
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let advisory = results
        .iter()
        .find(|r| r.output.contains("[blocking-suppressed]"))
        .unwrap_or_else(|| panic!("no advisory in {results:#?}"));
    assert_eq!(advisory.severity, Severity::Advisory);
    assert!(
        advisory
            .output
            .contains("notes.md:3: [blocking-suppressed] markdown-link-integrity"),
        "{}",
        advisory.output
    );
    // The suppressed finding itself must not fail the run.
    assert!(
        results
            .iter()
            .filter(|r| r.severity == Severity::Blocking)
            .all(|r| r.passed)
    );
}

const GO_IGNORE: &str = "// kibitzer:ignore flag-argument -- legacy API, callers pinned\n";
const GO_FUNC: &str = "func f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n";

/// `package main`, then `directive` on `directive_row` and `func f(b bool)` (a
/// `[flag-argument]` finding) on row 20; every other row is blank.
fn go_with_directive_at(directive_row: usize, directive: &str) -> String {
    let mut rows = vec![String::new(); 19];
    rows[0] = "package main".to_string();
    rows[directive_row - 1] = directive.trim_end().to_string();
    format!("{}\n{GO_FUNC}", rows.join("\n"))
}

struct Hook {
    results: Vec<CheckResult>,
    run_ctx: RunContext,
}

fn run_hook(name: &str, source: &str, changed: Option<&[(usize, usize)]>) -> Hook {
    let dir =
        std::env::temp_dir().join(format!("kibitzer-post-pass-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("x.go");
    std::fs::write(&file, source).unwrap();
    let run_ctx = RunContext::default();
    let results = crate::check::run_checks_for_trigger(
        &crate::config::default_checks(),
        "PostToolUse",
        &dir,
        &file,
        changed,
        &Registry::default(),
        &run_ctx,
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    Hook { results, run_ctx }
}

fn unused_lines(h: &Hook) -> Vec<&str> {
    h.results
        .iter()
        .filter(|r| r.output.contains("[unused-ignore]"))
        .map(|r| r.output.as_str())
        .collect()
}

fn reruns(h: &Hook) -> usize {
    h.run_ctx
        .inline
        .scan_memo
        .raw_reruns
        .load(Ordering::Relaxed)
}

#[test]
fn run_checks_for_trigger_should_EmitUnusedIgnoreWithNearestRow_When_AddedDirectiveOnWrongRow() {
    let h = run_hook(
        "wrong",
        &go_with_directive_at(12, GO_IGNORE),
        Some(&[(12, 12)]),
    );
    let lines = unused_lines(&h);
    assert_eq!(lines.len(), 1, "{:#?}", h.results);
    assert!(lines[0].ends_with(
        "x.go:12: [unused-ignore] kibitzer:ignore flag-argument matches no finding at line 12 or 13; the finding is at line 20. Move the comment to the line directly above line 20 (or the end of line 20)"
    ), "{}", lines[0]);
    assert!(!lines[0].contains("line 19"));
}

#[test]
fn run_checks_for_trigger_should_EmitNoUnusedIgnore_When_DirectiveCoversFinding() {
    let h = run_hook(
        "right",
        &go_with_directive_at(19, GO_IGNORE),
        Some(&[(19, 19)]),
    );
    assert!(unused_lines(&h).is_empty(), "{:#?}", h.results);
    assert_eq!(reruns(&h), 1);
}

#[test]
fn post_pass_should_NameRowTwenty_When_ChangedLinesIsRowTwelveOnly() {
    let h = run_hook(
        "scoped",
        &go_with_directive_at(12, GO_IGNORE),
        Some(&[(12, 12)]),
    );
    // Diff-scoping hid row 20 from the first pass, yet the advisory still names it.
    assert!(
        h.results
            .iter()
            .filter(|r| r.check_name == "syntax-rules-go")
            .all(|r| !r.output.contains("x.go:20:")),
        "{:#?}",
        h.results
    );
    assert!(unused_lines(&h)[0].contains("the finding is at line 20"));
}

#[test]
fn run_checks_for_trigger_should_SkipRawRerun_When_NoDirectiveRowInChangedLines() {
    let h = run_hook(
        "norow",
        &go_with_directive_at(12, GO_IGNORE),
        Some(&[(1, 3)]),
    );
    assert_eq!(reruns(&h), 0);
    assert!(unused_lines(&h).is_empty());
}

#[test]
fn run_checks_for_trigger_should_EmitNoUnusedIgnore_When_ChangedLinesScoped() {
    let source = format!("package main\n{}\n{GO_IGNORE}{GO_FUNC}", "\n".repeat(47));
    let h = run_hook("far", &source, Some(&[(1, 3)]));
    assert!(
        h.results
            .iter()
            .all(|r| !r.output.contains("[unused-ignore]"))
    );
    assert_eq!(reruns(&h), 0);
}

#[test]
fn run_checks_for_trigger_should_SkipRawRerun_When_FileLacksMarker() {
    let h = run_hook("nomarker", &go_with_directive_at(12, ""), Some(&[(12, 12)]));
    assert_eq!(reruns(&h), 0);
}

#[test]
fn run_checks_for_trigger_should_PointToHead_When_FileScopeDirectiveBelowRowTen() {
    let ignore = "// kibitzer:ignore file-size -- generated tables, splitting is churn\n";
    let h = run_hook(
        "filescope",
        &go_with_directive_at(12, ignore),
        Some(&[(12, 12)]),
    );
    let lines = unused_lines(&h);
    assert_eq!(lines.len(), 1, "{:#?}", h.results);
    assert!(lines[0].contains("first 10 lines"), "{}", lines[0]);
}
