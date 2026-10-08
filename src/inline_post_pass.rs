//! Hook-side logic that runs once after the per-check loop in `run_checks_for_trigger`.
//! Kept out of `check.rs` so advisories about directives (blocking suppressions, and later
//! wrong-row unused ignores) do not grow that file.

use std::path::Path;

use crate::check_result::CheckResult;
use crate::checker::MAX_NATIVE_CHECK_BYTES;
use crate::config::{Check, Severity};
use crate::inline_ignores::{
    Directive, FirstPass, Line, LineSpan, RawFinding, Reason, RuleId, UnusedKind, owned_by,
    rows_intersect, unowned_verdicts, unused_ignores, valid_directives,
};
use crate::run_context::RunContext;

/// Reruns one native check against `source` with inline ignores disabled; injected so this
/// module does not depend on the check pipeline.
pub(crate) type RawRerun<'a> = &'a dyn Fn(&Check, &Path, &str) -> anyhow::Result<Vec<RawFinding>>;

/// Everything the post-pass may read; `checks`, `run_ctx` and `raw_rerun` feed the unused-ignore rerun.
pub(crate) struct PostPassInput<'a> {
    pub checks: &'a [Check],
    pub file_path: &'a Path,
    pub changed_lines: Option<&'a [(usize, usize)]>,
    pub results: &'a [CheckResult],
    pub run_ctx: &'a RunContext,
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
    format!("{}:{}: {message}", file_path.display(), row.get())
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
        .filter(|source| crate::inline_ignores::has_marker(source))
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
    CheckResult::new(
        crate::checkers::inline_ignore::NAME.to_string(),
        Severity::Advisory,
        false,
        output.to_string(),
    )
}

#[cfg(test)]
mod tests;
