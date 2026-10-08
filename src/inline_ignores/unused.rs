//! Judging which directive rules suppress nothing, and the repair messages for them.

use super::rules::{did_you_mean, has_owner, known_rule, owner_matches, rule_matches};
use super::sanitize::{ECHO_RULE_TEXT_CHARS, echo};
use super::types::*;

/// Row of the raw finding `rule` answers to that lies nearest `from` (earlier row on a tie).
fn nearest_finding_line(rule: &RuleId, raw: &[RawFinding], from: Line) -> Option<Line> {
    raw.iter()
        .filter(|f| rule_matches(rule, &f.checker, &f.message))
        .map(|f| f.line)
        .min_by_key(|l| (l.get().abs_diff(from.get()), l.get()))
}

/// Why a directive rule was reported as unused; `kibitzer run` keeps only `UnknownRule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnusedKind {
    WrongRow,
    FileHead,
    RemoveIt,
    UnknownRule,
}

/// One `[unused-ignore]` finding for a directive rule, from either judgement path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnusedVerdict {
    pub row: Line,
    pub kind: UnusedKind,
    pub message: String,
}

impl UnusedVerdict {
    fn at(d: &Directive, kind: UnusedKind, message: String) -> Self {
        UnusedVerdict {
            row: d.start_line,
            kind,
            message,
        }
    }
}

/// What the first pass already saw, for judging rules no checker-ownership entry covers.
pub(crate) struct FirstPass<'a> {
    pub ran: Vec<&'a str>,
    pub dropped: Vec<&'a DroppedFinding>,
    pub kept: Vec<&'a Anchor>,
}

/// Verdicts for the rules of `directives` (limited to those touching `rows`, when given) whose
/// ownership is `owned`, each rule judged by `judge`.
fn judge_rules<'a>(
    directives: impl IntoIterator<Item = &'a Directive>,
    rows: Option<&[(usize, usize)]>,
    owned: bool,
    judge: impl Fn(&Directive, &RuleId) -> Option<UnusedVerdict>,
) -> Vec<UnusedVerdict> {
    directives
        .into_iter()
        .filter(|d| !rows.is_some_and(|r| !rows_intersect(d, r)))
        .flat_map(|d| {
            let judge = &judge;
            d.rules()
                .iter()
                .filter(move |r| has_owner(r.as_str()) == owned)
                .filter_map(move |r| judge(d, r))
        })
        .collect()
}

/// Owned rules that suppress nothing, judged against `raw` (findings before any directive or
/// `accepted/` entry, so a shadowed ignore still counts as used). A rule is judged only when
/// its owning checker is in `ran_checkers`. `rows` limits judgement to directives touching
/// those ranges (the hook's changed lines).
pub(crate) fn unused_ignores<'a>(
    directives: impl IntoIterator<Item = &'a Directive>,
    raw: &[RawFinding],
    ran_checkers: &[&str],
    rows: Option<&[(usize, usize)]>,
) -> Vec<UnusedVerdict> {
    judge_rules(directives, rows, true, |d, rule| {
        owned_verdict(d, rule, raw, ran_checkers)
    })
}

fn owned_verdict(
    d: &Directive,
    rule: &RuleId,
    raw: &[RawFinding],
    ran_checkers: &[&str],
) -> Option<UnusedVerdict> {
    let used = raw
        .iter()
        .any(|f| d.covers(rule, &f.checker, &f.message, f.line));
    let judged = ran_checkers
        .iter()
        .any(|c| owner_matches(rule.as_str(), c) == Some(true));
    if used || !judged {
        return None;
    }
    let (kind, message) = if FILE_SCOPE_RULES.contains(&rule.as_str()) {
        (UnusedKind::FileHead, file_head_message(rule))
    } else if let Some(n) = nearest_finding_line(rule, raw, d.start_line) {
        (UnusedKind::WrongRow, wrong_row_message(d, rule, n))
    } else {
        (UnusedKind::RemoveIt, remove_it_message(rule.as_str()))
    };
    Some(UnusedVerdict::at(d, kind, message))
}

/// Judges rules the ownership table does not cover from first-pass data only (no rerun): a
/// rule that dropped a finding is used; otherwise the nearest surviving finding of that rule
/// names the right row, a ran check's own name means "remove it", and a name nothing
/// recognizes is reported as a probable typo.
pub(crate) fn unowned_verdicts<'a>(
    directives: impl IntoIterator<Item = &'a Directive>,
    first: &FirstPass,
    rows: Option<&[(usize, usize)]>,
) -> Vec<UnusedVerdict> {
    judge_rules(directives, rows, false, |d, rule| {
        unowned_verdict(d, rule, first)
    })
}

fn unowned_verdict(d: &Directive, rule: &RuleId, first: &FirstPass) -> Option<UnusedVerdict> {
    let used = first
        .dropped
        .iter()
        .any(|f| f.directive_start == d.start_line && f.rule == *rule);
    if used {
        return None;
    }
    let name = rule.as_str();
    let nearest = first
        .kept
        .iter()
        .filter(|a| a.rule == *rule)
        .map(|a| a.line)
        .min_by_key(|l| (l.get().abs_diff(d.start_line.get()), l.get()));
    let (kind, message) = if let Some(n) = nearest {
        (UnusedKind::WrongRow, wrong_row_message(d, rule, n))
    } else if first.ran.contains(&name) {
        (UnusedKind::RemoveIt, remove_it_message(name))
    } else if !known_rule(name) && did_you_mean(name).is_none() {
        (UnusedKind::UnknownRule, unknown_rule_message(name))
    } else {
        return None;
    };
    Some(UnusedVerdict::at(d, kind, message))
}

fn remove_it_message(rule: &str) -> String {
    format!("[unused-ignore] kibitzer:ignore {rule} suppresses nothing - remove it")
}

pub(super) fn unknown_rule_message(rule: &str) -> String {
    let rule = echo(rule, ECHO_RULE_TEXT_CHARS);
    format!(
        "[unused-ignore] '{rule}' is not a known rule or checker; run 'kibitzer check list' to see valid names"
    )
}

fn file_head_message(rule: &RuleId) -> String {
    format!(
        "[unused-ignore] kibitzer:ignore {} must sit in the first {FILE_HEAD_LINES} lines of the file to apply; move the comment there",
        rule.as_str()
    )
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
