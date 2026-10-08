//! Hook-side logic that runs once after the per-check loop in `run_checks_for_trigger`.
//! Kept out of `check.rs` so advisories about directives (blocking suppressions, and later
//! wrong-row unused ignores) do not grow that file.

use std::path::Path;

use crate::accepted_findings::AcceptedFindings;
use crate::check::CheckResult;
use crate::config::Check;
use crate::plugin::Registry;

/// Everything the post-pass may read. `checks`, `repo_root`, `accepted` and `registry` are
/// carried for the unused-ignore rerun (Story 2.2.3), which needs the same context as the loop.
#[allow(dead_code)]
pub(crate) struct PostPassInput<'a> {
    pub checks: &'a [Check],
    pub repo_root: &'a Path,
    pub file_path: &'a Path,
    pub changed_lines: Option<&'a [(usize, usize)]>,
    pub results: &'a [CheckResult],
    pub accepted: &'a AcceptedFindings,
    pub registry: &'a Registry,
}

/// Synthesized `inline-ignore` results to append after the first pass. Empty unless the run
/// is diff-scoped: an unscoped run reports through `kibitzer run`'s footer instead.
pub(crate) fn run(input: PostPassInput) -> Vec<CheckResult> {
    if input.changed_lines.is_none() {
        return Vec::new();
    }
    Vec::new()
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
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
            repo_root: Path::new("/repo"),
            file_path: Path::new("/repo/doc.md"),
            changed_lines,
            results,
            accepted: &AcceptedFindings::default(),
            registry: &Registry::default(),
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
}
