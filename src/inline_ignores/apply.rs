//! Applying valid directives to a checker's findings, and the per-run context that does so.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::rules::anchor_rule;
use super::scan::ScanMemo;
use super::types::*;
use crate::checker::Finding;
use crate::config::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InlineIgnoreMode {
    #[default]
    Apply,
    /// Raw findings: used by `--no-inline-ignores` and the unused-ignore rerun.
    Disabled,
}

/// Findings dropped inline in one run, of which from a blocking check.
#[derive(Debug, Default)]
pub struct SuppressionCounts {
    pub total: AtomicUsize,
    pub blocking: AtomicUsize,
}

impl SuppressionCounts {
    /// Repo-wide summary line, so 1-4 ignores per file across many files still add up to a
    /// visible total; `None` when nothing was suppressed.
    pub fn footer(&self) -> Option<String> {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return None;
        }
        let blocking = self.blocking.load(Ordering::Relaxed);
        let noun = if total == 1 { "finding" } else { "findings" };
        let blocking_part = if blocking > 0 {
            format!(" ({blocking} from blocking checks)")
        } else {
            String::new()
        };
        Some(format!(
            "[kibitzer] {total} {noun} suppressed inline{blocking_part} (rerun with --no-inline-ignores to see them)"
        ))
    }
}

/// How one run applies inline ignores. Cloning shares the counter and memo.
#[derive(Debug, Clone, Default)]
pub struct InlineIgnoreContext {
    pub mode: InlineIgnoreMode,
    pub counter: Option<Arc<SuppressionCounts>>,
    pub scan_memo: Arc<ScanMemo>,
}

impl InlineIgnoreContext {
    pub fn disabled() -> Self {
        InlineIgnoreContext {
            mode: InlineIgnoreMode::Disabled,
            ..Default::default()
        }
    }

    /// Same mode, no counter and a fresh memo: a HEAD-baseline replay scans different
    /// content for the same path and must neither be counted nor evict the live entry.
    pub fn without_counter(&self) -> Self {
        InlineIgnoreContext {
            mode: self.mode,
            ..Default::default()
        }
    }
}

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
pub struct AppliedIgnores {
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

/// Drops findings covered by a valid directive. The early returns run before any hashing,
/// locking, or parsing, so the common no-marker hook path stays a substring search.
pub(crate) fn apply_inline_ignores(
    findings: Vec<Finding>,
    file: &Path,
    source: &str,
    checker_name: &str,
    severity: Severity,
    ctx: &InlineIgnoreContext,
) -> AppliedIgnores {
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
