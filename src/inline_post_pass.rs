//! Hook-side logic that runs once after the per-check loop in `run_checks_for_trigger`.
//! Kept out of `check.rs` so advisories about directives (blocking suppressions, and later
//! wrong-row unused ignores) do not grow that file.

use std::path::Path;

use crate::accepted_findings::AcceptedFindings;
use crate::check_result::CheckResult;
use crate::checker::MAX_NATIVE_CHECK_BYTES;
use crate::config::{Check, Severity};
use crate::inline_ignores::{
    Directive, FirstPass, InlineOutcome, Line, LineSpan, RawFinding, Reason, RuleId, UnusedKind,
    owned_by, rows_intersect, unowned_verdicts, unused_ignores, valid_directives,
};

/// Reruns one native check against `source` with inline ignores disabled; injected so this
/// module does not depend on the check pipeline.
pub(crate) type RawRerun<'a> = &'a dyn Fn(&Check, &Path, &str) -> anyhow::Result<Vec<RawFinding>>;

/// Everything the post-pass may read; `checks`, `accepted` and `raw_rerun` feed the unused-ignore rerun.
pub(crate) struct PostPassInput<'a> {
    pub checks: &'a [Check],
    pub file_path: &'a Path,
    pub changed_lines: Option<&'a [(usize, usize)]>,
    pub results: &'a [CheckResult],
    pub accepted: &'a AcceptedFindings,
    pub raw_rerun: RawRerun<'a>,
}

/// Synthesized `inline-ignore` results to append after the first pass. Empty unless the run
/// is diff-scoped: an unscoped run reports through `kibitzer run`'s footer instead.
pub(crate) fn run(input: PostPassInput) -> Vec<CheckResult> {
    let Some(changed_lines) = input.changed_lines else {
        return Vec::new();
    };
    let blocking = blocking_suppressions(input.results, changed_lines)
        .into_iter()
        .map(|s| s.render(input.file_path));
    let unused = unused_advisories(&input, changed_lines);
    blocking
        .chain(unused)
        .map(|line| advisory_result(&line))
        .collect()
}

/// `[unused-ignore]` lines for directives the edit just touched that silence nothing. Reruns
/// only the owning checks, and only when a touched directive names a rule they own.
fn unused_advisories(input: &PostPassInput, changed_lines: &[(usize, usize)]) -> Vec<String> {
    let Some(source) = read_markered_source(input.file_path) else {
        return Vec::new();
    };
    let scanned = input
        .accepted
        .inline
        .scan_memo
        .scan(input.file_path, &source);
    let touched: Vec<&Directive> = valid_directives(&scanned)
        .into_iter()
        .filter(|d| rows_intersect(d, changed_lines))
        .collect();
    if touched.is_empty() {
        return Vec::new();
    }
    let first = first_pass(input.results);
    let raw = rerun_owning_checks(input, &first.ran, &touched, &source);
    unused_ignores(touched.iter().copied(), &raw, &first.ran, None)
        .into_iter()
        .chain(unowned_verdicts(
            touched.iter().copied(),
            &first,
            Some(changed_lines),
        ))
        .map(|v| located(input.file_path, v.row, &v.message))
        .collect()
}

fn first_pass(results: &[CheckResult]) -> FirstPass<'_> {
    FirstPass {
        ran: results.iter().map(|r| r.check_name.as_str()).collect(),
        dropped: results.iter().flat_map(|r| &r.inline.dropped).collect(),
        kept: results.iter().flat_map(|r| &r.inline.kept).collect(),
    }
}

/// `{file}:{row}: {message}`, the line shape every advisory shares.
fn located(file_path: &Path, row: Line, message: &str) -> String {
    format!("{}:{}: {message}", file_path.display(), row.get())
}

/// `kibitzer run` audit: a typo'd rule must not be silent in the CLI even
/// though the full unused-ignore audit is deferred. First-pass data only, no rerun.
pub(crate) fn unknown_rule_advisories(
    file_path: &Path,
    results: &[CheckResult],
    accepted: &AcceptedFindings,
) -> Vec<CheckResult> {
    let Some(source) = read_markered_source(file_path) else {
        return Vec::new();
    };
    let scanned = accepted.inline.scan_memo.scan(file_path, &source);
    let verdicts = unowned_verdicts(valid_directives(&scanned), &first_pass(results), None);
    verdicts
        .iter()
        .filter(|v| v.kind == UnusedKind::UnknownRule)
        .map(|v| advisory_result(&located(file_path, v.row, &v.message)))
        .collect()
}

/// The file text, only when it is small enough for the first pass to have judged it and
/// carries the marker; one extra page-cache-hot read per hook call.
fn read_markered_source(file_path: &Path) -> Option<String> {
    let too_big = std::fs::metadata(file_path)
        .map(|m| m.len() > MAX_NATIVE_CHECK_BYTES)
        .unwrap_or(false);
    if too_big {
        return None;
    }
    std::fs::read_to_string(file_path)
        .ok()
        .filter(|source| source.contains("kibitzer"))
}

/// Raw findings from the checks that ran and own a rule some touched directive names.
/// Never diff-scoped, so a finding outside `changed_lines` still counts.
fn rerun_owning_checks(
    input: &PostPassInput,
    ran: &[&str],
    touched: &[&Directive],
    source: &str,
) -> Vec<RawFinding> {
    let owners = input.checks.iter().filter(|c| {
        ran.contains(&c.name.as_str())
            && touched
                .iter()
                .flat_map(|d| d.rules())
                .any(|r| owned_by(r.as_str(), &c.name))
    });
    let mut raw = Vec::new();
    for check in owners {
        #[cfg(test)]
        input
            .accepted
            .inline
            .scan_memo
            .raw_reruns
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // A failing checker was already reported by the first pass.
        if let Ok(found) = (input.raw_rerun)(check, input.file_path, source) {
            raw.extend(found);
        }
    }
    raw
}

/// One directive that silenced at least one blocking finding, with every rule it silenced.
struct BlockingSuppression {
    row: Line,
    rules: Vec<RuleId>,
    reason: Reason,
}

impl BlockingSuppression {
    fn render(&self, file_path: &Path) -> String {
        let rules: Vec<&str> = self.rules.iter().map(RuleId::as_str).collect();
        let message = format!(
            "[blocking-suppressed] {} finding silenced inline (reason: {}); tell the user you silenced a blocking check and why, so they can confirm it",
            rules.join(", "),
            self.reason.as_str()
        );
        located(file_path, self.row, &message)
    }
}

/// Directives touching `changed_lines` that dropped a blocking finding, grouped per directive
/// so one comment naming two rules or silencing two findings yields one advisory.
fn blocking_suppressions(
    results: &[CheckResult],
    changed_lines: &[(usize, usize)],
) -> Vec<BlockingSuppression> {
    let mut found: Vec<(LineSpan, BlockingSuppression)> = Vec::new();
    let blocking_drops = results
        .iter()
        .flat_map(|r| &r.inline.dropped)
        .filter(|d| d.severity == Severity::Blocking);
    for drop in blocking_drops {
        let rows = drop.directive_span();
        if !rows.intersects(changed_lines) {
            continue;
        }
        match found.iter_mut().find(|(key, _)| *key == rows) {
            Some((_, existing)) if !existing.rules.contains(&drop.rule) => {
                existing.rules.push(drop.rule.clone());
            }
            Some(_) => {}
            None => found.push((
                rows,
                BlockingSuppression {
                    row: drop.directive_start,
                    rules: vec![drop.rule.clone()],
                    reason: drop.reason.clone(),
                },
            )),
        }
    }
    found.into_iter().map(|(_, s)| s).collect()
}

fn advisory_result(output: &str) -> CheckResult {
    CheckResult {
        check_name: crate::checkers::inline_ignore::NAME.to_string(),
        severity: Severity::Advisory,
        passed: false,
        output: output.to_string(),
        message: None,
        command: String::new(),
        findings: Vec::new(),
        plugin_missing: false,
        inline: InlineOutcome::default(),
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use crate::config::Severity;
    use crate::inline_ignores::{Anchor, DroppedFinding, InlineOutcome, Line, Reason, RuleId};

    pub(super) fn result_with_dropped(
        check_name: &str,
        dropped: Vec<DroppedFinding>,
    ) -> CheckResult {
        CheckResult {
            check_name: check_name.to_string(),
            severity: Severity::Blocking,
            passed: true,
            output: String::new(),
            message: None,
            command: String::new(),
            findings: Vec::new(),
            plugin_missing: false,
            inline: InlineOutcome {
                dropped,
                ..Default::default()
            },
        }
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
            accepted: &AcceptedFindings::default(),
            raw_rerun: &no_rerun,
        })
    }

    #[test]
    fn run_should_ReturnNothing_When_ChangedLinesIsNone() {
        let results = [result_with_dropped(
            "markdown-link-integrity",
            vec![dropped(
                "markdown-link-integrity",
                "placeholder for later",
                (4, 4),
                Severity::Blocking,
            )],
        )];
        assert!(run_with(None, &results).is_empty());
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
            "/repo/doc.md:4: [blocking-suppressed] markdown-link-integrity finding silenced inline (reason: placeholder for later); tell the user you silenced a blocking check and why, so they can confirm it"
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
            accepted: &AcceptedFindings::default(),
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
            accepted: &AcceptedFindings::default(),
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
}
