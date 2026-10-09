//! Hook-side logic that runs once after the per-check loop in `run_checks_for_trigger`.
//! Kept out of `check.rs` so advisories about directives (blocking suppressions, and later
//! wrong-row unused ignores) do not grow that file.

use std::path::Path;

use crate::check_result::CheckResult;
use crate::checker::{NativeSource, read_native_source};
use crate::config::{Check, Severity};
use crate::inline_ignores::sanitize::{ECHO_PATH_CHARS, echo_path, quote_reason};
use crate::inline_ignores::{
    Directive, DroppedFinding, FirstPass, Line, LineSpan, RawFinding, Reason, RuleId, UnusedKind,
    owned_by, rows_intersect, unowned_verdicts, unused_ignores, valid_directives,
};
use crate::run_context::RunContext;

mod baseline;
pub(crate) use baseline::{HeadDrops, HeadReader, HeadSnapshot};
use baseline::{head_counts, mark_new, suppression_key};

/// Reruns one native check against `source` with inline ignores disabled; injected so this
/// module does not depend on the check pipeline.
pub(crate) type RawRerun<'a> = &'a dyn Fn(&Check, &Path, &str) -> anyhow::Result<Vec<RawFinding>>;

/// Everything the post-pass may read; `checks`, `run_ctx` and `raw_rerun` feed the unused-ignore
/// rerun, `head` and `head_drops` the blocking-suppression baseline.
pub(crate) struct PostPassInput<'a> {
    pub checks: &'a [Check],
    pub file_path: &'a Path,
    pub changed_lines: Option<&'a [(usize, usize)]>,
    pub results: &'a [CheckResult],
    pub run_ctx: &'a RunContext,
    pub raw_rerun: RawRerun<'a>,
    pub head: HeadReader<'a>,
    pub head_drops: HeadDrops<'a>,
}

/// Most `[blocking-suppressed]` lines one run emits; the rest fold into a count line.
const MAX_BLOCKING_ADVISORIES: usize = 10;

const WHOLE_FILE: &[(usize, usize)] = &[(1, usize::MAX)];

/// Synthesized `inline-ignore` results to append after the first pass. A diff-scoped run
/// reports blocking suppressions and unused ignores inside the changed lines. A whole-file
/// write (`changed_lines` is `None`, e.g. the `Write` tool) treats every directive as just
/// added, but reports only blocking suppressions. `kibitzer run` and the LSP skip both.
pub(crate) fn run(input: PostPassInput) -> Vec<CheckResult> {
    let (changed_lines, scoped) = match input.changed_lines {
        Some(lines) => (lines, true),
        None if input.run_ctx.skip_whole_file_advisories => return Vec::new(),
        None => (WHOLE_FILE, false),
    };
    let source = read_markered_source(input.file_path);
    let blocking = blocking_advisories(&input, changed_lines, scoped, source.as_deref());
    let unused = if scoped {
        unused_advisories(&input, changed_lines)
    } else {
        Vec::new()
    };
    blocking
        .into_iter()
        .chain(unused)
        .map(|line| advisory_result(&line))
        .collect()
}

/// One advisory line per blocking suppression, the overflow past `MAX_BLOCKING_ADVISORIES`
/// folded into a single count line so a rewrite with many ignores stays bounded.
fn blocking_advisories(
    input: &PostPassInput,
    changed_lines: &[(usize, usize)],
    scoped: bool,
    source: Option<&str>,
) -> Vec<String> {
    let file_path = input.file_path;
    let all = blocking_suppressions(input, changed_lines, scoped, source);
    let overflow = all.len().saturating_sub(MAX_BLOCKING_ADVISORIES);
    let mut lines: Vec<String> = all
        .iter()
        .take(MAX_BLOCKING_ADVISORIES)
        .map(|s| s.render(file_path))
        .collect();
    if overflow > 0 {
        lines.push(located(
            file_path,
            all[MAX_BLOCKING_ADVISORIES].row,
            &format!(
                "[blocking-suppressed] {overflow} more directives silenced blocking findings in this file; run kibitzer run --no-inline-ignores to list them"
            ),
        ));
    }
    lines
}

/// `[unused-ignore]` lines for directives the edit just touched that silence nothing. Reruns
/// only the owning checks, and only when a touched directive names a rule they own.
fn unused_advisories(input: &PostPassInput, changed_lines: &[(usize, usize)]) -> Vec<String> {
    let Some(source) = read_markered_source(input.file_path) else {
        return Vec::new();
    };
    let scanned = input
        .run_ctx
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
    let path = echo_path(&file_path.display().to_string(), ECHO_PATH_CHARS);
    format!("{path}:{}: {message}", row.get())
}

/// `kibitzer run` audit: reports only unknown-rule verdicts, from first-pass data with no
/// rerun, so a typo'd rule is not silent in the CLI. The rerun-based unused-ignore audit
/// is hook-only (`run`).
pub(crate) fn unknown_rule_advisories(
    file_path: &Path,
    results: &[CheckResult],
    run_ctx: &RunContext,
) -> Vec<CheckResult> {
    let Some(source) = read_markered_source(file_path) else {
        return Vec::new();
    };
    let scanned = run_ctx.inline.scan_memo.scan(file_path, &source);
    let verdicts = unowned_verdicts(valid_directives(&scanned), &first_pass(results), None);
    verdicts
        .iter()
        .filter(|v| v.kind == UnusedKind::UnknownRule)
        .map(|v| advisory_result(&located(file_path, v.row, &v.message)))
        .collect()
}

/// The file text, only when it is small enough for the first pass to have judged it and
/// carries the marker; one extra page-cache-hot read per hook call. A write landing between
/// that pass's read and this one can skew an advisory's row; the next edit's hook run corrects it.
fn read_markered_source(file_path: &Path) -> Option<String> {
    match read_native_source(file_path) {
        Ok(NativeSource::Text(source)) if crate::inline_ignores::has_marker(&source) => {
            Some(source)
        }
        _ => None,
    }
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
            .run_ctx
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
            quote_reason(self.reason.as_str())
        );
        located(file_path, self.row, &message)
    }
}

/// Whether an edit touched the directive or the silenced row; those drops always report.
fn touched_by_edit(drop: &DroppedFinding, changed_lines: &[(usize, usize)]) -> bool {
    let finding_row = LineSpan {
        start: drop.finding_line,
        end: drop.finding_line,
    };
    drop.directive_span().intersects(changed_lines) || finding_row.intersects(changed_lines)
}

/// Which `drops` the agent should hear about. A drop whose directive or silenced row an
/// `Edit`/`MultiEdit` touched always reports. The rest report only when the file now drops more
/// of that kind than at git HEAD (see `baseline`): a finding that appeared, slid under a
/// directive, or was duplicated. With no usable baseline, a whole-file `Write` reports
/// everything (a new file had nothing before) and an `Edit` stays quiet about untouched rows.
fn rows_to_report(
    input: &PostPassInput,
    drops: &[&DroppedFinding],
    changed_lines: &[(usize, usize)],
    scoped: bool,
    source: Option<&str>,
) -> Vec<bool> {
    let mut report: Vec<bool> = drops
        .iter()
        .map(|d| scoped && touched_by_edit(d, changed_lines))
        .collect();
    if report.iter().all(|&r| r) {
        return report;
    }
    let owners: Vec<&Check> = input
        .checks
        .iter()
        .filter(|c| {
            input
                .results
                .iter()
                .any(|r| r.check_name == c.name && has_blocking_drop(r))
        })
        .collect();
    let head = (input.head)(input.file_path);
    match head_counts(head, &owners, input.file_path, input.head_drops) {
        Some(baseline) => {
            let keys: Vec<String> = drops.iter().map(|d| suppression_key(d, source)).collect();
            mark_new(&keys, &baseline, &mut report);
        }
        None if !scoped => report.fill(true),
        None => {}
    }
    report
}

fn has_blocking_drop(result: &CheckResult) -> bool {
    result
        .inline
        .dropped
        .iter()
        .any(|d| d.severity == Severity::Blocking)
}

/// Directives that dropped a blocking finding and are worth reporting (see `rows_to_report`),
/// grouped per directive so one comment naming two rules or silencing two findings yields one advisory.
fn blocking_suppressions(
    input: &PostPassInput,
    changed_lines: &[(usize, usize)],
    scoped: bool,
    source: Option<&str>,
) -> Vec<BlockingSuppression> {
    let mut drops: Vec<&DroppedFinding> = input
        .results
        .iter()
        .flat_map(|r| &r.inline.dropped)
        .filter(|d| d.severity == Severity::Blocking)
        .collect();
    if drops.is_empty() {
        return Vec::new();
    }
    // File order, so the drops promoted past the baseline are the ones nearest the file's end.
    drops.sort_by_key(|d| (d.finding_line, d.directive_start));
    let report = rows_to_report(input, &drops, changed_lines, scoped, source);
    let mut found: Vec<(LineSpan, BlockingSuppression)> = Vec::new();
    for drop in drops
        .iter()
        .zip(&report)
        .filter_map(|(d, &r)| r.then_some(d))
    {
        let rows = drop.directive_span();
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
    CheckResult::new(
        crate::checkers::inline_ignore::NAME.to_string(),
        Severity::Advisory,
        false,
        output.to_string(),
    )
}

#[cfg(test)]
mod tests;
