//! Hook-side logic that runs once after the per-check loop in `run_checks_for_trigger`.
//! Kept out of `check.rs` so advisories about directives (blocking suppressions, and later
//! wrong-row unused ignores) do not grow that file.

use std::path::Path;

use crate::accepted_findings::AcceptedFindings;
use crate::check::CheckResult;
use crate::config::{Check, Severity};
use crate::inline_ignores::{InlineOutcome, Line, Reason, RuleId};
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
    let Some(changed_lines) = input.changed_lines else {
        return Vec::new();
    };
    blocking_suppressions(input.results, changed_lines)
        .into_iter()
        .map(|s| advisory_result(&s.render(input.file_path)))
        .collect()
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
        if !touches(rows, changed_lines) {
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

fn touches((start, end): (Line, Line), ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|&(s, e)| s <= end.get() && start.get() <= e)
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
}
