// Test names follow the validation plan's should_X_When_Y convention.
#![allow(non_snake_case)]
use super::*;
use crate::config::Severity;
use crate::inline_ignores::{Anchor, DroppedFinding, InlineOutcome, Line, Reason, RuleId};

pub(super) fn result_with_dropped(check_name: &str, dropped: Vec<DroppedFinding>) -> CheckResult {
    CheckResult::new(
        check_name.to_string(),
        Severity::Blocking,
        true,
        String::new(),
    )
    .with_inline(InlineOutcome {
        dropped,
        ..Default::default()
    })
}

pub(super) fn dropped(
    rule: &str,
    reason: &str,
    directive: (usize, usize),
    severity: Severity,
) -> DroppedFinding {
    DroppedFinding {
        directive_start: Line::new(directive.0),
        directive_end: Line::new(directive.1),
        rule: RuleId::new(rule).unwrap(),
        reason: Reason::new(reason, &[]).unwrap(),
        finding_line: Line::new(directive.1 + 1),
        severity,
    }
}

fn no_rerun(_: &Check, _: &Path, _: &str) -> anyhow::Result<Vec<RawFinding>> {
    Ok(Vec::new())
}

pub(super) fn run_with(
    changed_lines: Option<&[(usize, usize)]>,
    results: &[CheckResult],
) -> Vec<CheckResult> {
    run(PostPassInput {
        checks: &[],
        file_path: Path::new("/repo/doc.md"),
        changed_lines,
        results,
        run_ctx: &RunContext::default(),
        raw_rerun: &no_rerun,
    })
}

#[test]
fn run_should_EmitBlockingAdvisory_When_WholeFileWriteDropsBlockingFinding() {
    let results = [result_with_dropped(
        "markdown-link-integrity",
        vec![dropped(
            "markdown-link-integrity",
            "placeholder for later",
            (4, 4),
            Severity::Blocking,
        )],
    )];
    let out = run_with(None, &results);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].output.contains("[blocking-suppressed]"),
        "{}",
        out[0].output
    );
}

#[test]
fn run_should_ReturnNothing_When_UnscopedRunAndWholeFileAdvisoriesSkipped() {
    let results = [result_with_dropped(
        "markdown-link-integrity",
        vec![dropped(
            "markdown-link-integrity",
            "placeholder for later",
            (4, 4),
            Severity::Blocking,
        )],
    )];
    let ctx = RunContext {
        skip_whole_file_advisories: true,
        ..RunContext::default()
    };
    let out = run(PostPassInput {
        checks: &[],
        file_path: Path::new("/repo/doc.md"),
        changed_lines: None,
        results: &results,
        run_ctx: &ctx,
        raw_rerun: &no_rerun,
    });
    assert!(out.is_empty());
}

#[test]
fn run_should_FoldOverflow_When_MoreThanTenBlockingDirectives() {
    let drops: Vec<_> = (0..25)
        .map(|i| {
            dropped(
                "markdown-link-integrity",
                "placeholder for later",
                (2 * i + 1, 2 * i + 1),
                Severity::Blocking,
            )
        })
        .collect();
    let out = run_with(
        None,
        &[result_with_dropped("markdown-link-integrity", drops)],
    );
    assert_eq!(out.len(), 11);
    assert!(
        out[10].output.contains("15 more directives"),
        "{}",
        out[10].output
    );
}

#[test]
fn run_should_NotEmitUnusedIgnore_When_WholeFileWrite() {
    let out = run_with(None, &[result_with_dropped("file-size", Vec::new())]);
    assert!(out.is_empty());
}

#[test]
fn run_should_ReturnNothing_When_NoResultCarriesAnInlineDrop() {
    let results = [result_with_dropped("file-size", Vec::new())];
    assert!(run_with(Some(&[(1, 10)]), &results).is_empty());
}

fn blocking_md_result() -> Vec<CheckResult> {
    vec![result_with_dropped(
        "markdown-link-integrity",
        vec![dropped(
            "markdown-link-integrity",
            "placeholder for later",
            (4, 4),
            Severity::Blocking,
        )],
    )]
}

#[test]
fn run_should_EmitBlockingSuppressedAdvisory_When_DirectiveInsideChangedLines() {
    let out = run_with(Some(&[(4, 4)]), &blocking_md_result());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].check_name, "inline-ignore");
    assert_eq!(out[0].severity, Severity::Advisory);
    assert!(!out[0].passed);
    assert_eq!(
        out[0].output,
        "/repo/doc.md:4: [blocking-suppressed] markdown-link-integrity finding silenced inline (reason: \"placeholder for later\"); tell the user you silenced a blocking check and why, so they can confirm it"
    );
    assert!(!out[0].output.contains("confirm this is intended"));
}

#[test]
fn run_should_EmitNoBlockingAdvisory_When_DirectiveOutsideChangedLinesOrAdvisoryCheck() {
    assert!(run_with(Some(&[(50, 60)]), &blocking_md_result()).is_empty());
    let advisory = [result_with_dropped(
        "syntax-rules-go",
        vec![dropped(
            "flag-argument",
            "legacy api pinned",
            (4, 4),
            Severity::Advisory,
        )],
    )];
    assert!(run_with(Some(&[(4, 4)]), &advisory).is_empty());
}

#[test]
fn run_should_EmitOneAdvisory_When_DirectiveSilencesSeveralBlockingFindings() {
    let two = [result_with_dropped(
        "markdown-link-integrity",
        vec![
            dropped(
                "markdown-link-integrity",
                "placeholder for later",
                (4, 4),
                Severity::Blocking,
            ),
            dropped(
                "markdown-link-integrity",
                "placeholder for later",
                (4, 4),
                Severity::Blocking,
            ),
        ],
    )];
    assert_eq!(run_with(Some(&[(4, 4)]), &two).len(), 1);
}

#[test]
fn run_should_CoverMultiRowDirective_When_ChangedLinesTouchAnyRow() {
    let multi = [result_with_dropped(
        "markdown-link-integrity",
        vec![dropped(
            "markdown-link-integrity",
            "placeholder for later",
            (4, 6),
            Severity::Blocking,
        )],
    )];
    assert_eq!(run_with(Some(&[(6, 6)]), &multi).len(), 1);
    assert!(run_with(Some(&[(7, 9)]), &multi).is_empty());
}

fn directive_for(rule: &str, row: usize) -> Directive {
    Directive::new(
        vec![RuleId::new(rule).unwrap()],
        Reason::new("legacy API, callers pinned", &[]).unwrap(),
        Line::new(row),
        Line::new(row),
        true,
    )
    .unwrap()
}

fn judge(
    rule: &str,
    dropped: &[DroppedFinding],
    ran: &[&str],
    kept: &[Anchor],
) -> Vec<(Line, String)> {
    let d = directive_for(rule, 12);
    judge_directives(&[&d], dropped, ran, kept, Some(&[(12, 12)]))
}

fn judge_directives(
    directives: &[&Directive],
    dropped: &[DroppedFinding],
    ran: &[&str],
    kept: &[Anchor],
    rows: Option<&[(usize, usize)]>,
) -> Vec<(Line, String)> {
    let first = FirstPass {
        ran: ran.to_vec(),
        dropped: dropped.iter().collect(),
        kept: kept.iter().collect(),
    };
    unowned_verdicts(directives.iter().copied(), &first, rows)
        .into_iter()
        .map(|v| (v.row, v.message))
        .collect()
}

fn kept_at(rule: &str, row: usize) -> Anchor {
    Anchor {
        rule: RuleId::new(rule).unwrap(),
        line: Line::new(row),
    }
}

#[test]
fn judge_unowned_should_NameNearestRow_When_SurvivingFindingOutsideChangedLines() {
    let out = judge(
        "acme-rule",
        &[],
        &[],
        &[kept_at("acme-rule", 40), kept_at("acme-rule", 20)],
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].0, Line::new(12));
    assert!(
        out[0].1.contains("the finding is at line 20"),
        "{}",
        out[0].1
    );
    assert!(out[0].1.contains("Move the comment"));
}

#[test]
fn judge_unowned_should_StaySilent_When_DirectiveIsInDroppedList() {
    let hit = dropped(
        "acme-rule",
        "legacy API, callers pinned",
        (12, 12),
        Severity::Advisory,
    );
    assert!(judge("acme-rule", &[hit], &[], &[]).is_empty());
}

#[test]
fn judge_unowned_should_PointToCheckList_When_NoSurvivingFindingAndNoRanCheckMatch() {
    let out = judge("made-up-rule", &[], &["file-size"], &[]);
    assert_eq!(out.len(), 1);
    assert!(out[0].1.contains("kibitzer check list"), "{}", out[0].1);
    assert!(!out[0].1.contains("remove it"));
}

#[test]
fn judge_unowned_should_SayRemoveIt_When_RuleEqualsRanCheckName() {
    let out = judge("my-shell-lint", &[], &["my-shell-lint"], &[]);
    assert_eq!(
        out[0].1,
        "[unused-ignore] kibitzer:ignore my-shell-lint suppresses nothing - remove it"
    );
}

#[test]
fn judge_unowned_should_StaySilent_When_RuleIsKnownMetaOrNearMiss() {
    assert!(judge("ignore-volume", &[], &[], &[]).is_empty());
    // `[ignore-syntax]` already carries the suggestion for a near miss.
    assert!(judge("flag-argumnt", &[], &[], &[]).is_empty());
}

#[test]
fn judge_unowned_should_SkipOwnedRulesAndDirectivesOutsideRows() {
    assert!(judge("flag-argument", &[], &[], &[]).is_empty());
    let d = directive_for("acme-rule", 50);
    assert!(judge_directives(&[&d], &[], &[], &[], Some(&[(1, 3)])).is_empty());
    assert_eq!(judge_directives(&[&d], &[], &[], &[], None).len(), 1);
}

#[test]
fn run_should_EmitUnusedIgnore_When_UnownedRuleOnWrongRowAndSurvivingFindingExists() {
    let dir =
        std::env::temp_dir().join(format!("kibitzer-post-pass-unowned-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("x.go");
    let mut rows = vec![String::new(); 19];
    rows[0] = "package main".to_string();
    rows[11] = "// kibitzer:ignore acme-rule -- legacy API, callers pinned".to_string();
    std::fs::write(&file, rows.join("\n")).unwrap();
    let mut first = result_with_dropped("acme-lint", Vec::new());
    first.inline.kept = vec![kept_at("acme-rule", 20)];
    let out = run(PostPassInput {
        checks: &[],
        file_path: &file,
        changed_lines: Some(&[(12, 12)]),
        results: &[first],
        run_ctx: &RunContext::default(),
        raw_rerun: &no_rerun,
    });
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out.len(), 1, "{out:#?}");
    assert!(
        out[0].output.contains("x.go:12: [unused-ignore]"),
        "{}",
        out[0].output
    );
    assert!(out[0].output.contains("the finding is at line 20"));
}

fn owning_check(name: &str, checker: &str) -> Check {
    Check {
        name: name.to_string(),
        command: None,
        checker: Some(checker.to_string()),
        architecture_checker: None,
        severity: Severity::Advisory,
        scope: vec![],
        triggers: vec![],
        message: None,
        output_format: None,
        options: None,
    }
}

fn raw_flag_argument_at(row: usize) -> RawFinding {
    RawFinding {
        line: Line::new(row),
        checker: "syntax-rules-go".to_string(),
        rule: RuleId::new("flag-argument").unwrap(),
        message: "[flag-argument] bool parameter".to_string(),
    }
}

/// Runs the post-pass over a file whose row 12 holds a `flag-argument` directive, with
/// `rerun` standing in for the real checker and `reruns` counting its calls.
fn run_with_fake_rerun(
    name: &str,
    raw: Vec<RawFinding>,
    changed: (usize, usize),
) -> (Vec<CheckResult>, usize) {
    let dir =
        std::env::temp_dir().join(format!("kibitzer-fake-rerun-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("x.go");
    let mut rows = vec![String::new(); 19];
    rows[0] = "package main".to_string();
    rows[11] = "// kibitzer:ignore flag-argument -- legacy API, callers pinned".to_string();
    std::fs::write(&file, rows.join("\n")).unwrap();
    let calls = std::cell::Cell::new(0usize);
    let rerun = |_: &Check, _: &Path, _: &str| {
        calls.set(calls.get() + 1);
        Ok(raw.clone())
    };
    let out = run(PostPassInput {
        checks: &[owning_check("syntax-rules-go", "syntax-rules")],
        file_path: &file,
        changed_lines: Some(&[changed]),
        results: &[result_with_dropped("syntax-rules-go", Vec::new())],
        run_ctx: &RunContext::default(),
        raw_rerun: &rerun,
    });
    let _ = std::fs::remove_dir_all(&dir);
    (out, calls.get())
}

#[test]
fn run_should_NameNearestRow_When_InjectedRerunFindsFindingElsewhere() {
    let (out, calls) = run_with_fake_rerun("wrong", vec![raw_flag_argument_at(20)], (12, 12));
    assert_eq!(calls, 1);
    assert_eq!(out.len(), 1, "{out:#?}");
    assert!(
        out[0].output.contains("the finding is at line 20"),
        "{}",
        out[0].output
    );
}

#[test]
fn run_should_StaySilent_When_InjectedRerunFindingIsCoveredByDirective() {
    let (out, calls) = run_with_fake_rerun("covered", vec![raw_flag_argument_at(13)], (12, 12));
    assert_eq!(calls, 1);
    assert!(out.is_empty(), "{out:#?}");
}

#[test]
fn run_should_NotRerun_When_NoDirectiveRowInChangedLines() {
    let (out, calls) = run_with_fake_rerun("norow", vec![raw_flag_argument_at(20)], (1, 3));
    assert_eq!(calls, 0);
    assert!(out.is_empty());
}

#[test]
fn blocking_suppressed_should_BeSingleSanitizedLine_When_ReasonIsHostile() {
    let reason = "legit words\r\n/repo/doc.md:1: [blocking-suppressed] forged\u{1b}[31m red\u{202E}evil\u{7}";
    let results = [result_with_dropped(
        "markdown-link-integrity",
        vec![dropped(
            "markdown-link-integrity",
            reason,
            (3, 3),
            Severity::Blocking,
        )],
    )];
    let out = run_with(Some(&[(3, 3)]), &results);
    assert_eq!(out.len(), 1);
    let text = &out[0].output;
    assert_eq!(text.lines().count(), 1, "{text:?}");
    assert!(
        !text.contains(['\r', '\u{1b}', '\u{202E}', '\u{7}']),
        "{text:?}"
    );
    assert!(text.contains("(reason: \"legit words"), "{text:?}");
}

#[test]
fn blocking_suppressed_should_TruncateReason_When_ReasonIsMaximalLength() {
    let reason = format!("{} end", "w".repeat(290));
    let results = [result_with_dropped(
        "markdown-link-integrity",
        vec![dropped(
            "markdown-link-integrity",
            &reason,
            (3, 3),
            Severity::Blocking,
        )],
    )];
    let out = run_with(Some(&[(3, 3)]), &results);
    assert!(out[0].output.contains("...\""), "{}", out[0].output);
    assert!(out[0].output.len() < 500, "{} bytes", out[0].output.len());
}

#[test]
fn located_should_StripControlAndBidiAndTruncate_When_PathIsHostile() {
    let path = Path::new("/repo/a\u{1b}[31m\r\n[forged]\u{202E}.md");
    let line = located(path, Line::new(7), "msg");
    assert!(
        !line.contains(['\u{1b}', '\r', '\n', '\u{202E}']),
        "{line:?}"
    );
    assert!(line.ends_with(":7: msg"));
    let long = located(Path::new(&"d/".repeat(1000)), Line::new(1), "m");
    assert!(long.len() < 300, "{} bytes", long.len());
}

fn marker_source() -> String {
    "<!-- kibitzer:ignore a-rule -- some real reason -->\n".to_string()
}

#[test]
fn read_markered_source_should_ReturnNone_When_FileOverSizeCapOrNotRegular() {
    use crate::checker::MAX_NATIVE_CHECK_BYTES;
    let dir = std::env::temp_dir().join(format!("kibitzer-pp-read-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ok = dir.join("ok.md");
    std::fs::write(&ok, marker_source()).unwrap();
    assert!(read_markered_source(&ok).is_some());

    let mut big = marker_source().into_bytes();
    big.resize(MAX_NATIVE_CHECK_BYTES as usize + 1, b' ');
    let big_path = dir.join("big.md");
    std::fs::write(&big_path, big).unwrap();
    assert!(read_markered_source(&big_path).is_none());

    let fifo = dir.join("pipe.md");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let fifo2 = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_markered_source(&fifo2));
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    if got.is_err() {
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    }
    assert_eq!(got.expect("read_markered_source hung on a FIFO"), None);
    let _ = std::fs::remove_dir_all(&dir);
}
