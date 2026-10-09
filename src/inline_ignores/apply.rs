//! Applying valid directives to a checker's findings.

use std::path::Path;
use std::sync::atomic::Ordering;

use super::rules::anchor_rule;
use super::types::*;
use crate::checker::Finding;
use crate::config::Severity;

/// Most anchors a `CheckResult` keeps; the footer only names the first and the hook is capped.
const MAX_INLINE_ANCHORS: usize = 20;

/// The first `MAX_INLINE_ANCHORS` of `findings` as anchors, in finding order.
pub(crate) fn capped_anchors<'a>(
    checker_name: &str,
    findings: impl Iterator<Item = &'a Finding>,
) -> Vec<Anchor> {
    findings
        .take(MAX_INLINE_ANCHORS)
        .map(|f| Anchor {
            rule: anchor_rule(checker_name, f),
            line: Line::new(f.line),
        })
        .collect()
}

#[derive(Debug, Default)]
pub(crate) struct AppliedIgnores {
    pub kept: Vec<Finding>,
    pub dropped: Vec<DroppedFinding>,
}

/// First valid directive rule that matches `finding` and covers its row (or, for file-scope
/// rules, sits in the file head).
fn covering<'a>(
    directives: &'a [&'a Directive],
    checker_name: &str,
    finding: &Finding,
) -> Option<(&'a Directive, &'a RuleId)> {
    let line = Line::new(finding.line);
    directives.iter().find_map(|d| {
        d.rules().iter().find_map(|r| {
            d.covers(r, checker_name, &finding.message, line)
                .then_some((*d, r))
        })
    })
}

/// What one checker's findings were reported against: the file text and the checker (whose
/// `severity` is recorded on each dropped finding).
#[derive(Clone, Copy)]
pub(crate) struct IgnoreTarget<'a> {
    pub file: &'a Path,
    pub source: &'a str,
    pub checker_name: &'a str,
    pub severity: Severity,
}

/// Drops findings covered by a valid directive. The early returns run before any hashing,
/// locking, or parsing, so the common no-marker hook path stays a substring search.
pub(crate) fn apply_inline_ignores(
    findings: Vec<Finding>,
    target: IgnoreTarget,
    ctx: &InlineIgnoreContext,
) -> AppliedIgnores {
    let IgnoreTarget {
        file,
        source,
        checker_name,
        severity,
    } = target;
    if findings.is_empty() || ctx.mode == InlineIgnoreMode::Disabled || !has_marker(source) {
        return AppliedIgnores {
            kept: findings,
            dropped: Vec::new(),
        };
    }
    let scanned = ctx.scan_memo.scan(file, source);
    let directives = valid_directives(&scanned);
    let mut applied = AppliedIgnores::default();
    for finding in findings {
        match covering(&directives, checker_name, &finding) {
            Some((d, rule)) => applied.dropped.push(DroppedFinding {
                directive_start: d.start_line,
                directive_end: d.end_line,
                rule: rule.clone(),
                reason: d.reason.clone(),
                finding_line: Line::new(finding.line),
                severity,
            }),
            None => applied.kept.push(finding),
        }
    }
    if let Some(counter) = &ctx.counter {
        let n = applied.dropped.len();
        counter.total.fetch_add(n, Ordering::Relaxed);
        if severity == Severity::Blocking {
            counter.blocking.fetch_add(n, Ordering::Relaxed);
        }
    }
    applied
}
