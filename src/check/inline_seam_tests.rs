//! Inline-ignore seams of the native check path: directives, `accepted/` entries, diff scoping, counts.
#![allow(non_snake_case)]

use super::*;
use crate::accepted_findings::{AcceptedFinding, AcceptedFindings};
use crate::inline_ignores::{RuleId, SuppressionCounts};
use std::sync::Arc;

const IGNORE: &str = "// kibitzer:ignore flag-argument -- legacy api pinned\n";
const FUNC_F: &str = "func f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n";
const FUNC_G: &str = "func g(c bool) {\n\tif c {\n\t\tprintln(\"y\")\n\t}\n}\n";
const FUNC_H: &str = "func h(d bool) {\n\tif d {\n\t\tprintln(\"z\")\n\t}\n}\n";

/// Two flag-argument findings, at the `func` lines of `f` and `g`.
fn go_source(f_prefix: &str, g_prefix: &str) -> String {
    format!("package main\n\n{f_prefix}{FUNC_F}\n{g_prefix}{FUNC_G}")
}

fn run_src(source: &str, ctx: &InlineIgnoreContext) -> SourceCheck {
    run_checker_against_source(
        NativeRun {
            checker_name: "syntax-rules",
            options: None,
            severity: Severity::Advisory,
            inline_ctx: ctx,
        },
        Path::new("x.go"),
        source,
    )
    .unwrap()
}

#[test]
fn run_checker_against_source_should_DropFinding_When_WholeLineIgnoreAbove() {
    let out = run_src(&go_source(IGNORE, ""), &InlineIgnoreContext::default());
    assert!(!out.passed);
    assert!(
        out.combined.contains("x.go:10: [flag-argument]"),
        "{}",
        out.combined
    );
    assert!(!out.combined.contains("x.go:4:"), "{}", out.combined);
    assert_eq!(out.inline.dropped.len(), 1);
}

#[test]
fn run_checker_against_source_should_Pass_When_AllFindingsCovered() {
    let out = run_src(&go_source(IGNORE, IGNORE), &InlineIgnoreContext::default());
    assert!(out.passed);
    assert!(out.combined.is_empty());
    assert!(out.findings.is_empty());
    assert_eq!(out.inline.dropped.len(), 2);
}

#[test]
fn run_checker_against_source_should_KeepFinding_When_DirectiveTwoLinesAbove() {
    let out = run_src(
        &go_source(
            "// kibitzer:ignore flag-argument -- legacy api pinned\n\n",
            IGNORE,
        ),
        &InlineIgnoreContext::default(),
    );
    assert!(out.combined.contains("x.go:5:"), "{}", out.combined);
}

#[test]
fn run_checker_against_source_should_KeepFinding_When_DirectiveMalformed() {
    let out = run_src(
        &go_source("// kibitzer:ignore flag-argument\n", ""),
        &InlineIgnoreContext::default(),
    );
    assert_eq!(out.findings.len(), 2);
    assert!(out.inline.dropped.is_empty());
}

#[test]
fn run_checker_against_source_should_ReturnRawFindings_When_ModeDisabled() {
    let out = run_src(&go_source(IGNORE, IGNORE), &InlineIgnoreContext::disabled());
    assert_eq!(out.findings.len(), 2);
    assert!(!out.passed);
}

#[test]
fn raw_findings_for_check_should_ReturnStructuredFindings_When_DirectiveWouldSuppress() {
    let check = crate::config::default_checks()
        .into_iter()
        .find(|c| c.name == "syntax-rules-go")
        .unwrap();
    let source = go_source(IGNORE, "");
    let raw = raw_findings_for_check(&check, Path::new("x.go"), &source).unwrap();
    let flag: Vec<_> = raw
        .iter()
        .filter(|f| f.rule.as_str() == "flag-argument")
        .collect();
    assert_eq!(flag.len(), 2, "{raw:?}");
    assert_eq!(flag[0].line.get(), 4);
    assert_eq!(flag[0].checker, "syntax-rules");
}

#[test]
fn raw_findings_for_check_should_BeEmpty_When_CheckIsNotNative() {
    let mut check = crate::config::default_checks().remove(0);
    check.checker = None;
    assert!(
        raw_findings_for_check(&check, Path::new("x.go"), "package main\n")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn run_checker_against_source_should_MatchCheckerName_When_NoBracketPrefix() {
    let source = "package main\n\n// kibitzer:ignore primitive-obsession -- ids are plain strings\nfunc f(a, b string) {}\n";
    let ctx = InlineIgnoreContext::default();
    let out = run_checker_against_source(
        NativeRun {
            checker_name: "primitive-obsession",
            options: None,
            severity: Severity::Advisory,
            inline_ctx: &ctx,
        },
        Path::new("x.go"),
        source,
    )
    .unwrap();
    assert!(out.passed, "{}", out.combined);
    let raw = run_checker_against_source(
        NativeRun {
            checker_name: "primitive-obsession",
            options: None,
            severity: Severity::Advisory,
            inline_ctx: &InlineIgnoreContext::disabled(),
        },
        Path::new("x.go"),
        source,
    )
    .unwrap();
    assert!(!raw.passed);
}

#[test]
fn ignore_should_SurviveEdits_When_LinesInsertedAbove() {
    let source = format!(
        "// license\n// header\n// lines\n// to\n// add\n{}",
        go_source(IGNORE, IGNORE)
    );
    assert!(run_src(&source, &InlineIgnoreContext::default()).passed);
}

#[test]
fn ignore_should_StopSuppressing_When_FindingMovesAwayFromComment() {
    let source = go_source(&format!("{IGNORE}\n"), IGNORE);
    let out = run_src(&source, &InlineIgnoreContext::default());
    assert_eq!(out.findings.len(), 1);
    assert_eq!(out.findings[0].line, 5);
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kibitzer-inline-seam-{}-{name}-{}",
        std::process::id(),
        TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn flag_check(severity: Severity) -> Check {
    Check {
        name: "syntax-rules-go".to_string(),
        command: None,
        checker: Some("syntax-rules".to_string()),
        architecture_checker: None,
        severity,
        scope: vec![],
        triggers: vec![],
        message: None,
        output_format: None,
        options: None,
    }
}

fn accept(rule: &str, line: usize, content: &str) -> AcceptedFinding {
    AcceptedFinding {
        rule: rule.to_string(),
        file: "main.go".to_string(),
        line,
        content: content.to_string(),
        reason: "kept on purpose".to_string(),
    }
}

/// Findings at lines 3, 9 and 15.
fn three_findings() -> String {
    format!("package main\n\n{FUNC_F}\n{FUNC_G}\n{FUNC_H}")
}

fn native(
    dir: &Path,
    source: &str,
    changed: Option<&[(usize, usize)]>,
    run_ctx: &RunContext,
) -> CheckResult {
    let file = dir.join("main.go");
    std::fs::write(&file, source).unwrap();
    run_native_check(
        &flag_check(Severity::Advisory),
        "syntax-rules",
        dir,
        &file,
        changed,
        run_ctx,
    )
    .unwrap()
}

#[test]
fn run_native_check_should_ExcludeScopedOutAndAcceptedFindings_When_ComputingShown() {
    let dir = tmp_dir("shown");
    let run_ctx = RunContext::new(AcceptedFindings {
        accepted: vec![accept("flag-argument", 9, "func g(c bool) {")],
    });
    let result = native(&dir, &three_findings(), Some(&[(3, 3), (9, 9)]), &run_ctx);
    let rule = |l: usize| crate::inline_ignores::Anchor {
        rule: RuleId::new("flag-argument").unwrap(),
        line: Line::new(l),
    };
    assert_eq!(result.inline.shown, vec![rule(3)]);
    assert_eq!(result.inline.first_anchor().map(|(_, l)| l.get()), Some(3));
    assert_eq!(result.inline.kept, vec![rule(3), rule(9), rule(15)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_native_check_should_ReportDropped_When_IgnoreCovers() {
    let dir = tmp_dir("dropped");
    let source = format!("package main\n\n{IGNORE}{FUNC_F}\n{FUNC_G}");
    let result = native(&dir, &source, None, &RunContext::default());
    assert_eq!(result.inline.dropped.len(), 1);
    assert_eq!(result.inline.dropped[0].finding_line.get(), 4);
    assert_eq!(result.inline.shown.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn accepted_entry_should_StillSuppress_When_NoInlineDirective() {
    let dir = tmp_dir("accepted-only");
    let run_ctx = RunContext::new(AcceptedFindings {
        accepted: vec![
            accept("flag-argument", 3, "func f(b bool) {"),
            accept("flag-argument", 9, "func g(c bool) {"),
        ],
    });
    let result = native(&dir, &go_source("", ""), None, &run_ctx);
    assert!(result.passed, "{}", result.output);
    assert!(result.inline.dropped.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn accepted_entry_should_NotBeAffected_When_InlineDisabled() {
    let dir = tmp_dir("accepted-disabled");
    let run_ctx = RunContext {
        accepted: AcceptedFindings {
            accepted: vec![accept("flag-argument", 4, "func f(b bool) {")],
        },
        inline: InlineIgnoreContext::disabled(),
        ..RunContext::default()
    };
    let result = native(&dir, &go_source(IGNORE, IGNORE), None, &run_ctx);
    assert!(!result.passed);
    assert!(result.output.contains("main.go:11:"), "{}", result.output);
    assert!(!result.output.contains("main.go:4:"), "{}", result.output);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn inline_should_RunBeforeAccepted_When_BothMatch() {
    let dir = tmp_dir("both");
    let run_ctx = RunContext::new(AcceptedFindings {
        accepted: vec![accept("flag-argument", 4, "func f(b bool) {")],
    });
    let result = native(&dir, &go_source(IGNORE, ""), None, &run_ctx);
    assert_eq!(
        result.inline.dropped.len(),
        1,
        "inline takes the finding first"
    );
    assert!(result.output.contains("main.go:10:"), "{}", result.output);
    assert!(!result.output.contains("main.go:4:"), "{}", result.output);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_result_should_DeserializeWithEmptyInline_When_CacheJsonLacksInlineKey() {
    let old_shape_json = r#"{
        "check_name": "syntax-rules-go",
        "severity": "advisory",
        "passed": false,
        "output": "x.go:3: [flag-argument] b",
        "message": null,
        "command": "kibitzer check native syntax-rules x.go"
    }"#;
    let result: CheckResult = serde_json::from_str(old_shape_json).unwrap();
    assert_eq!(result.inline, InlineOutcome::default());
}

#[test]
fn run_checks_for_trigger_should_DropFinding_When_IgnoreCoversChangedLine() {
    let dir = tmp_dir("trigger");
    let file = dir.join("main.go");
    std::fs::write(&file, go_source(IGNORE, "")).unwrap();
    let results = run_checks_for_trigger(
        &[flag_check(Severity::Advisory)],
        "PostToolUse",
        &dir,
        &file,
        Some(&[(4, 4)]),
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    assert!(results.iter().all(|r| r.passed), "{results:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_checks_for_trigger_should_PerformNoRawRerun_When_EditTouchesNoDirectiveRow() {
    let dir = tmp_dir("norerun");
    let file = dir.join("main.go");
    std::fs::write(&file, go_source(IGNORE, "")).unwrap();
    let run_ctx = RunContext::default();
    run_checks_for_trigger(
        &[flag_check(Severity::Advisory)],
        "PostToolUse",
        &dir,
        &file,
        Some(&[(8, 9)]),
        &Registry::default(),
        &run_ctx,
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        run_ctx.inline.scan_memo.raw_reruns.load(Ordering::Relaxed),
        0
    );
}

fn counting_ctx() -> (Arc<SuppressionCounts>, RunContext) {
    let counter = Arc::new(SuppressionCounts::default());
    let run_ctx = RunContext {
        inline: InlineIgnoreContext {
            counter: Some(Arc::clone(&counter)),
            ..Default::default()
        },
        ..Default::default()
    };
    (counter, run_ctx)
}

#[test]
fn run_native_check_should_CountBlockingSeparately_When_SeverityDiffers() {
    let dir = tmp_dir("counts");
    let file = dir.join("main.go");
    std::fs::write(&file, format!("package main\n\n{IGNORE}{FUNC_F}")).unwrap();
    let (counter, run_ctx) = counting_ctx();
    let run = |severity| {
        run_native_check(
            &flag_check(severity),
            "syntax-rules",
            &dir,
            &file,
            None,
            &run_ctx,
        )
        .unwrap()
    };
    let blocking = run(Severity::Blocking);
    assert_eq!(blocking.inline.dropped[0].severity, Severity::Blocking);
    let advisory = run(Severity::Advisory);
    assert_eq!(advisory.inline.dropped[0].severity, Severity::Advisory);
    assert_eq!(counter.total.load(Ordering::Relaxed), 2);
    assert_eq!(counter.blocking.load(Ordering::Relaxed), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_native_against_git_head_should_NotChangeCounter_When_BaselineReplay() {
    let dir = tmp_dir("replay-counter");
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);
    let file = dir.join("main.go");
    std::fs::write(&file, go_source(IGNORE, "")).unwrap();
    git(&["add", "main.go"]);
    git(&["commit", "-q", "-m", "init"]);
    let (counter, run_ctx) = counting_ctx();
    // g's finding stays, so the blocking check fails and triggers the HEAD replay.
    let result = run_native_check(
        &flag_check(Severity::Blocking),
        "syntax-rules",
        &dir,
        &file,
        None,
        &run_ctx,
    )
    .unwrap();
    assert!(!result.passed);
    assert_eq!(
        counter.total.load(Ordering::Relaxed),
        1,
        "replay must not add to the count"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_native_against_git_head_should_HonorIgnore_When_IgnorePresentAtHead() {
    let dir = tmp_dir("head");
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap()
                .success()
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);
    let file = dir.join("main.go");
    std::fs::write(&file, go_source(IGNORE, IGNORE)).unwrap();
    git(&["add", "main.go"]);
    git(&["commit", "-q", "-m", "init"]);
    let at_head = |ctx: &InlineIgnoreContext| {
        check_native_against_git_head(
            NativeRun {
                checker_name: "syntax-rules",
                options: None,
                severity: Severity::Blocking,
                inline_ctx: ctx,
            },
            &dir,
            &file,
            None,
        )
    };
    assert_eq!(at_head(&InlineIgnoreContext::default()), Some(true));
    assert_eq!(at_head(&InlineIgnoreContext::disabled()), Some(false));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn apply_inline_ignores_should_Suppress_When_RuleAbsentFromKnownRules() {
    let rule = "some-new-rule";
    assert!(!crate::inline_ignores::known_rule(rule));
    let source = format!(
        "package main\n\n// kibitzer:ignore {rule} -- brand new rule, table not updated\nfunc f() {{}}\n"
    );
    let finding = crate::checker::Finding {
        line: 4,
        message: format!("[{rule}] something"),
    };
    let out = apply_inline_ignores(
        vec![finding],
        IgnoreTarget {
            file: Path::new("x.go"),
            source: &source,
            checker_name: "native-checker",
            severity: Severity::Advisory,
        },
        &InlineIgnoreContext::default(),
    );
    assert!(out.kept.is_empty(), "{:?}", out.kept);
    assert_eq!(out.dropped.len(), 1);
}

#[test]
fn run_command_check_should_NotApplyInlineIgnores_When_ShellOutCommandCheck() {
    let dir = tmp_dir("command-no-inline");
    let file = dir.join("main.go");
    std::fs::write(&file, go_source(IGNORE, "")).unwrap();
    let check = Check {
        name: "shell-lint".to_string(),
        command: Some("printf 'main.go:4: [flag-argument] shell finding\\n'; exit 1".to_string()),
        checker: None,
        ..flag_check(Severity::Advisory)
    };
    let result = run_check(
        &check,
        &dir,
        &file,
        None,
        &Registry::default(),
        &RunContext::default(),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!result.passed);
    assert!(
        result.output.contains("main.go:4: [flag-argument]"),
        "{}",
        result.output
    );
    assert!(result.inline.dropped.is_empty());
}

fn inline_ignore_check() -> Check {
    Check {
        name: crate::checkers::inline_ignore::NAME.to_string(),
        checker: Some(crate::checkers::inline_ignore::NAME.to_string()),
        ..flag_check(Severity::Advisory)
    }
}

fn run_on_bytes(file: &str, bytes: &[u8], check: &Check, checker: &str) -> CheckResult {
    let dir = tmp_dir("bytes");
    let path = dir.join(file);
    std::fs::write(&path, bytes).unwrap();
    let result =
        run_native_check(check, checker, &dir, &path, None, &RunContext::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn inline_ignore_should_YieldPassingEmptyResult_When_BinaryFile() {
    let result = run_on_bytes(
        "blob.md",
        b"\x89PNG\r\n\x1a\n\xff\xfe",
        &inline_ignore_check(),
        crate::checkers::inline_ignore::NAME,
    );
    assert!(result.passed);
    assert!(result.output.is_empty(), "{}", result.output);
}

#[test]
fn inline_ignore_should_YieldPassingEmptyResult_When_InvalidUtf8GoFile() {
    let result = run_on_bytes(
        "bad.go",
        b"package main\n// \xff\xfe\n",
        &inline_ignore_check(),
        crate::checkers::inline_ignore::NAME,
    );
    assert!(result.passed);
    assert!(result.output.is_empty(), "{}", result.output);
}

#[test]
fn other_checker_should_KeepFailedResult_When_ReadError() {
    let result = run_on_bytes(
        "bad.go",
        b"package main\n// \xff\xfe\n",
        &flag_check(Severity::Advisory),
        "syntax-rules",
    );
    assert!(!result.passed);
    assert!(result.output.contains("reading"), "{}", result.output);
}

#[test]
fn inline_ignore_volume_should_SurviveDiffScoping_When_ChangedLinesIsFifthDirectiveRow() {
    let dir = tmp_dir("volume");
    let mut source = String::from("package main\n");
    for row in [10, 20, 30, 35, 40] {
        while source.lines().count() + 1 < row {
            source.push('\n');
        }
        source.push_str("// kibitzer:ignore flag-argument -- pinned by caller\n");
    }
    let file = dir.join("main.go");
    std::fs::write(&file, &source).unwrap();
    let result = run_native_check(
        &inline_ignore_check(),
        crate::checkers::inline_ignore::NAME,
        &dir,
        &file,
        Some(&[(40, 40)]),
        &RunContext::default(),
    )
    .unwrap();
    assert!(!result.passed);
    assert!(
        result.output.contains("main.go:40: [ignore-volume]"),
        "{}",
        result.output
    );
    let _ = std::fs::remove_dir_all(&dir);
}
