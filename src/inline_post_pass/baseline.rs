//! Stateless baseline for `[blocking-suppressed]`: the blocking findings directives drop in the
//! file's git-HEAD content, as a multiset. An edit is told about a drop only when the file now
//! drops more of that kind than HEAD did, so nothing is remembered between hook calls.

use std::collections::HashMap;
use std::path::Path;

use crate::config::{Check, Severity};
use crate::inline_ignores::{DroppedFinding, is_file_scope};

/// What git holds for the edited file.
pub(crate) enum HeadSnapshot {
    /// Not a git checkout, or HEAD unreadable: nothing to compare against.
    Unavailable,
    /// The file is new to git, so nothing was suppressed before the edit.
    Absent,
    Source(String),
}

/// Reads the file's content at git HEAD.
pub(crate) type HeadReader<'a> = &'a dyn Fn(&Path) -> HeadSnapshot;

/// Reruns one native check against HEAD content with inline ignores applied, returning what
/// directives dropped. Injected so this module does not depend on the check pipeline.
pub(crate) type HeadDrops<'a> =
    &'a dyn Fn(&Check, &Path, &str) -> anyhow::Result<Vec<DroppedFinding>>;

/// Longest slice of the silenced line folded into a key.
const KEY_TEXT_CHARS: usize = 120;

/// Identity of one suppression that survives row shifts: the rule plus the trimmed text of the
/// silenced line. File-scope rules carry no line of their own, so growth (403 -> 603 lines)
/// keeps the same key.
pub(super) fn suppression_key(drop: &DroppedFinding, source: Option<&str>) -> String {
    let rule = drop.rule.as_str();
    if is_file_scope(rule) {
        return rule.to_string();
    }
    let text: String = source
        .and_then(|s| s.lines().nth(drop.finding_line.get().saturating_sub(1)))
        .map(|t| t.trim().chars().take(KEY_TEXT_CHARS).collect())
        .unwrap_or_default();
    format!("{rule}\u{1f}{text}")
}

pub(super) type Counts = HashMap<String, usize>;

/// Blocking drops of `head` per key, or `None` when git has no usable baseline. `owners` are
/// the checks that dropped something now; only those can have a baseline to compare.
pub(super) fn head_counts(
    head: HeadSnapshot,
    owners: &[&Check],
    file_path: &Path,
    head_drops: HeadDrops,
) -> Option<Counts> {
    let source = match head {
        HeadSnapshot::Unavailable => return None,
        HeadSnapshot::Absent => return Some(Counts::new()),
        HeadSnapshot::Source(source) => source,
    };
    let mut counts = Counts::new();
    for check in owners {
        // A checker that fails on HEAD contributes nothing, which errs toward reporting.
        let Ok(drops) = head_drops(check, file_path, &source) else {
            continue;
        };
        for drop in drops.iter().filter(|d| d.severity == Severity::Blocking) {
            *counts
                .entry(suppression_key(drop, Some(&source)))
                .or_default() += 1;
        }
    }
    Some(counts)
}

/// Marks which of `keys` (one per drop, in order) exceed the baseline. Drops already marked in
/// `report` (edited rows) are not touched but count toward the excess, so they never also
/// promote an old duplicate.
pub(super) fn mark_new(keys: &[String], baseline: &Counts, report: &mut [bool]) {
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, key) in keys.iter().enumerate() {
        groups.entry(key).or_default().push(i);
    }
    for (key, rows) in groups {
        let already = rows.iter().filter(|&&i| report[i]).count();
        let was = baseline.get(key).copied().unwrap_or(0);
        let remaining = rows.len().saturating_sub(was).saturating_sub(already);
        let quiet: Vec<usize> = rows.iter().rev().copied().filter(|&i| !report[i]).collect();
        for i in quiet.into_iter().take(remaining) {
            report[i] = true;
        }
    }
}
