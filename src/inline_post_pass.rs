//! Hook-side logic that runs once after the per-check loop in `run_checks_for_trigger`.
//! Kept out of `check.rs` so advisories about directives (blocking suppressions, and later
//! wrong-row unused ignores) do not grow that file.

use std::path::Path;

use std::sync::atomic::Ordering;

use crate::accepted_findings::AcceptedFindings;
use crate::check::{CheckResult, MAX_NATIVE_CHECK_BYTES, raw_findings_for_check};
use crate::config::{Check, Severity};
use crate::inline_ignores::{
    Directive, DirectiveParse, DroppedFinding, FILE_HEAD_LINES, FILE_SCOPE_RULES, InlineOutcome,
    Line, RawFinding, Reason, RuleId, did_you_mean, has_owner, known_rule, nearest_finding_line,
    owned_by, rows_intersect, span_intersects, unused_ignores,
};

/// Everything the post-pass may read; `checks` and `accepted` feed the unused-ignore rerun.
pub(crate) struct PostPassInput<'a> {
    pub checks: &'a [Check],
    pub file_path: &'a Path,
    pub changed_lines: Option<&'a [(usize, usize)]>,
    pub results: &'a [CheckResult],
    pub accepted: &'a AcceptedFindings,
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
    let touched: Vec<&Directive> = scanned
        .iter()
        .filter_map(|(_, p)| match p {
            DirectiveParse::Valid(d) if rows_intersect(d, changed_lines) => Some(d),
            _ => None,
        })
        .collect();
    if touched.is_empty() {
        return Vec::new();
    }
    let ran: Vec<&str> = input
        .results
        .iter()
        .map(|r| r.check_name.as_str())
        .collect();
    let raw = rerun_owning_checks(input, &ran, &touched, &source);
    let mut out = Vec::new();
    for d in &touched {
        for rule in d.rules().iter().filter(|r| has_owner(r.as_str())) {
            if let Some(msg) = judge_owned(d, rule, &raw, &ran) {
                out.push(format!(
                    "{}:{}: {msg}",
                    input.file_path.display(),
                    d.start_line.get()
                ));
            }
        }
    }
    let dropped: Vec<&DroppedFinding> = input
        .results
        .iter()
        .flat_map(|r| &r.inline.dropped)
        .collect();
    let kept: Vec<&(RuleId, Line)> = input.results.iter().flat_map(|r| &r.inline.kept).collect();
    for (row, msg) in judge_unowned(&touched, &dropped, &ran, &kept, Some(changed_lines)) {
        out.push(format!(
            "{}:{}: {msg}",
            input.file_path.display(),
            row.get()
        ));
    }
    out
}

/// Judges rules the ownership table does not cover from first-pass data only (no rerun):
/// a rule that dropped a finding is used; otherwise the nearest surviving finding of that
/// rule names the right row, a ran check's own name means "remove it", and a name nothing
/// recognizes is reported as a probable typo. `rows` limits judgement to touched directives
/// (`None` judges every directive, for `kibitzer run`). Returns `(directive row, message)`.
pub(crate) fn judge_unowned(
    directives: &[&Directive],
    dropped: &[&DroppedFinding],
    ran_checkers: &[&str],
    kept: &[&(RuleId, Line)],
    rows: Option<&[(usize, usize)]>,
) -> Vec<(Line, String)> {
    unowned_verdicts(directives, dropped, ran_checkers, kept, rows)
        .into_iter()
        .map(|v| (v.row, v.message))
        .collect()
}

/// Why an unowned rule was reported; `kibitzer run` keeps only the typo kind.
#[derive(PartialEq, Eq)]
enum UnownedKind {
    WrongRow,
    RemoveIt,
    UnknownRule,
}

struct UnownedVerdict {
    row: Line,
    kind: UnownedKind,
    message: String,
}

fn unowned_verdicts(
    directives: &[&Directive],
    dropped: &[&DroppedFinding],
    ran_checkers: &[&str],
    kept: &[&(RuleId, Line)],
    rows: Option<&[(usize, usize)]>,
) -> Vec<UnownedVerdict> {
    let mut out = Vec::new();
    for d in directives {
        if rows.is_some_and(|r| !rows_intersect(d, r)) {
            continue;
        }
        for rule in d.rules().iter().filter(|r| !has_owner(r.as_str())) {
            let used = dropped
                .iter()
                .any(|f| f.directive_start == d.start_line && f.rule == *rule);
            if used {
                continue;
            }
            let nearest = kept
                .iter()
                .filter(|(r, _)| r == rule)
                .map(|(_, l)| *l)
                .min_by_key(|l| (l.get().abs_diff(d.start_line.get()), l.get()));
            let name = rule.as_str();
            let verdict = if let Some(n) = nearest {
                Some((UnownedKind::WrongRow, wrong_row_message(d, rule, n)))
            } else if ran_checkers.contains(&name) {
                Some((
                    UnownedKind::RemoveIt,
                    format!(
                        "[unused-ignore] kibitzer:ignore {name} suppresses nothing - remove it"
                    ),
                ))
            } else if !known_rule(name) && did_you_mean(name).is_none() {
                Some((
                    UnownedKind::UnknownRule,
                    format!(
                        "[unused-ignore] '{name}' is not a known rule or checker; run 'kibitzer check list' to see valid names"
                    ),
                ))
            } else {
                None
            };
            out.extend(verdict.map(|(kind, message)| UnownedVerdict {
                row: d.start_line,
                kind,
                message,
            }));
        }
    }
    out
}

/// `kibitzer run` audit (Task 2.2.2e): a typo'd rule must not be silent in the CLI even
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
    let directives: Vec<&Directive> = scanned
        .iter()
        .filter_map(|(_, p)| match p {
            DirectiveParse::Valid(d) => Some(d),
            _ => None,
        })
        .collect();
    let ran: Vec<&str> = results.iter().map(|r| r.check_name.as_str()).collect();
    let dropped: Vec<&DroppedFinding> = results.iter().flat_map(|r| &r.inline.dropped).collect();
    let kept: Vec<&(RuleId, Line)> = results.iter().flat_map(|r| &r.inline.kept).collect();
    unowned_verdicts(&directives, &dropped, &ran, &kept, None)
        .into_iter()
        .filter(|v| v.kind == UnownedKind::UnknownRule)
        .map(|v| {
            advisory_result(&format!(
                "{}:{}: {}",
                file_path.display(),
                v.row.get(),
                v.message
            ))
        })
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
        input
            .accepted
            .inline
            .scan_memo
            .raw_reruns
            .fetch_add(1, Ordering::Relaxed);
        // A failing checker was already reported by the first pass.
        if let Ok(found) = raw_findings_for_check(check, input.file_path, source) {
            raw.extend(found);
        }
    }
    raw
}

/// Advisory text for one owned rule of `d` that covers no raw finding, with the nearest
/// finding's row when there is one.
fn judge_owned(d: &Directive, rule: &RuleId, raw: &[RawFinding], ran: &[&str]) -> Option<String> {
    let single = Directive::new(
        vec![rule.clone()],
        d.reason.clone(),
        d.start_line,
        d.end_line,
        d.whole_line,
    )?;
    let judged = unused_ignores(&[single], raw, ran, None).pop()?;
    if FILE_SCOPE_RULES.contains(&rule.as_str()) {
        return Some(format!(
            "[unused-ignore] kibitzer:ignore {rule} must sit in the first {FILE_HEAD_LINES} lines of the file to apply; move the comment there",
            rule = rule.as_str()
        ));
    }
    match nearest_finding_line(rule, raw, d.start_line) {
        Some(n) => Some(wrong_row_message(d, rule, n)),
        None => Some(judged.message),
    }
}

fn wrong_row_message(d: &Directive, rule: &RuleId, finding: Line) -> String {
    let (start, end) = (d.start_line.get(), d.end_line.get());
    let covered = match (start == end, d.whole_line) {
        (true, true) => format!("line {start} or {}", end + 1),
        (true, false) => format!("line {start}"),
        (false, true) => format!("lines {start}-{end} or {}", end + 1),
        (false, false) => format!("lines {start}-{end}"),
    };
    let n = finding.get();
    format!(
        "[unused-ignore] kibitzer:ignore {} matches no finding at {covered}; the finding is at line {n}. Move the comment to the line directly above line {n} (or the end of line {n})",
        rule.as_str()
    )
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
        format!(
            "{}:{}: [blocking-suppressed] {} finding silenced inline (reason: {}); tell the user you silenced a blocking check and why, so they can confirm it",
            file_path.display(),
            self.row.get(),
            rules.join(", "),
            self.reason.as_str()
        )
    }
}

/// Directives touching `changed_lines` that dropped a blocking finding, grouped per directive
/// so one comment naming two rules or silencing two findings yields one advisory.
fn blocking_suppressions(
    results: &[CheckResult],
    changed_lines: &[(usize, usize)],
) -> Vec<BlockingSuppression> {
    let mut found: Vec<((Line, Line), BlockingSuppression)> = Vec::new();
    let blocking_drops = results
        .iter()
        .flat_map(|r| &r.inline.dropped)
        .filter(|d| d.severity == Severity::Blocking);
    for drop in blocking_drops {
        let rows = (drop.directive_start, drop.directive_end);
        if !span_intersects(rows, changed_lines) {
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
    use crate::plugin::Registry;
    use crate::config::Severity;
    use crate::inline_ignores::{DroppedFinding, InlineOutcome, Line, Reason, RuleId};

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
            &AcceptedFindings::default(),
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
        accepted: AcceptedFindings,
    }

    fn run_hook(name: &str, source: &str, changed: Option<&[(usize, usize)]>) -> Hook {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-post-pass-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.go");
        std::fs::write(&file, source).unwrap();
        let accepted = AcceptedFindings::default();
        let results = crate::check::run_checks_for_trigger(
            &crate::config::default_checks(),
            "PostToolUse",
            &dir,
            &file,
            changed,
            &Registry::default(),
            &accepted,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        Hook { results, accepted }
    }

    fn unused_lines(h: &Hook) -> Vec<&str> {
        h.results
            .iter()
            .filter(|r| r.output.contains("[unused-ignore]"))
            .map(|r| r.output.as_str())
            .collect()
    }

    fn reruns(h: &Hook) -> usize {
        h.accepted
            .inline
            .scan_memo
            .raw_reruns
            .load(Ordering::Relaxed)
    }

    #[test]
    fn run_checks_for_trigger_should_EmitUnusedIgnoreWithNearestRow_When_AddedDirectiveOnWrongRow()
    {
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
        kept: &[(RuleId, Line)],
    ) -> Vec<(Line, String)> {
        let d = directive_for(rule, 12);
        let dropped: Vec<&DroppedFinding> = dropped.iter().collect();
        let kept: Vec<&(RuleId, Line)> = kept.iter().collect();
        judge_unowned(&[&d], &dropped, ran, &kept, Some(&[(12, 12)]))
    }

    fn kept_at(rule: &str, row: usize) -> (RuleId, Line) {
        (RuleId::new(rule).unwrap(), Line::new(row))
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
        assert!(judge_unowned(&[&d], &[], &[], &[], Some(&[(1, 3)])).is_empty());
        assert_eq!(judge_unowned(&[&d], &[], &[], &[], None).len(), 1);
    }

    #[test]
    fn run_checks_for_trigger_should_EmitUnusedIgnore_When_UnownedRuleOnWrongRowAndSurvivingFindingExists()
     {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-post-pass-unowned-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.go");
        std::fs::write(
            &file,
            go_with_directive_at(
                12,
                "// kibitzer:ignore acme-rule -- legacy API, callers pinned",
            ),
        )
        .unwrap();
        let mut first = result_with_dropped("acme-lint", Vec::new());
        first.inline.kept = vec![kept_at("acme-rule", 20)];
        let out = run(PostPassInput {
            checks: &[],
            file_path: &file,
            changed_lines: Some(&[(12, 12)]),
            results: &[first],
            accepted: &AcceptedFindings::default(),
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
}
