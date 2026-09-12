use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::config::CONFIG_DIR;

pub const ACCEPTED_FINDINGS_FILENAME: &str = "kibitzer-accepted.json";

/// One deliberately-accepted finding: a specific checker rule firing correctly at a
/// specific line, kept on purpose rather than fixed or globally suppressed. Distinct
/// from both `docs/reporting-false-positives.md` (for a checker misfiring, not a real
/// hit) and `.claude/inspect.json`'s `disabled`/`scope` (whole-checker granularity,
/// no reason required) — see `docs/accepting-findings.md`.
#[derive(Debug, Clone, Deserialize)]
pub struct AcceptedFinding {
    /// The rule id a finding's message self-prefixes with `[rule-id]` (most native
    /// checkers), or the check's own registered name for one that doesn't — see
    /// `extract_rule`.
    pub rule: String,
    /// Repo-root-relative path, forward-slash-separated — the same convention
    /// `Check::scope` glob patterns use (`check::relativize`).
    pub file: String,
    pub line: usize,
    /// The flagged line's exact source text (leading/trailing whitespace ignored) at
    /// the time this entry was written. A finding only matches when the line's
    /// *current* content still agrees — once it drifts, the entry silently stops
    /// suppressing (the finding reappears) rather than papering over a since-changed
    /// line, so an accepted tradeoff can't quietly outlive the code it was actually
    /// about.
    pub content: String,
    /// Required, non-empty (enforced by [`find_accepted_findings`]) — the whole point
    /// of this file over a global disable is that the tradeoff gets written down.
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AcceptedFindings {
    #[serde(default)]
    pub accepted: Vec<AcceptedFinding>,
}

impl AcceptedFindings {
    fn candidate(&self, rule: &str, rel_file: &str, line: usize) -> Option<&AcceptedFinding> {
        self.accepted
            .iter()
            .find(|e| e.rule == rule && e.file == rel_file && e.line == line)
    }
}

/// Walks upward from `start` looking for `.claude/kibitzer-accepted.json`, the same
/// convention as `config::find_config`. Returns an empty [`AcceptedFindings`] (not an
/// error) when no such file exists anywhere above `start` — accepting findings is
/// opt-in, most repos will never have one.
pub fn find_accepted_findings(start: &Path) -> Result<AcceptedFindings> {
    let mut dir = if start.is_file() {
        start.parent().unwrap_or(start).to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        let candidate = dir.join(CONFIG_DIR).join(ACCEPTED_FINDINGS_FILENAME);
        if candidate.is_file() {
            let raw = std::fs::read_to_string(&candidate)
                .with_context(|| format!("reading {}", candidate.display()))?;
            let parsed: AcceptedFindings = serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", candidate.display()))?;
            validate(&parsed, &candidate)?;
            return Ok(parsed);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Ok(AcceptedFindings::default()),
        }
    }
}

fn validate(parsed: &AcceptedFindings, path: &Path) -> Result<()> {
    for entry in &parsed.accepted {
        if entry.reason.trim().is_empty() {
            anyhow::bail!(
                "{}: entry for [{}] {}:{} has an empty reason — a reason is required, \
                 that's the whole point of this file over a blanket disable",
                path.display(),
                entry.rule,
                entry.file,
                entry.line
            );
        }
    }
    Ok(())
}

/// The rule id embedded in a native checker's message via its own `[rule-id]`
/// self-prefix convention (most native checkers do this — `syntax-rules-*`,
/// `comment-quality-*`, the architecture checks), or `fallback` (the check's own
/// registered name) for a message that doesn't self-prefix one (e.g.
/// `primitive-obsession`, `duplicate-code`, `file-complexity`) — the same fallback
/// `scripts/backtest-triage.py` uses for the same reason.
fn extract_rule<'a>(message: &'a str, fallback: &'a str) -> &'a str {
    message
        .strip_prefix('[')
        .and_then(|rest| rest.find(']').map(|end| &rest[..end]))
        .unwrap_or(fallback)
}

/// Drops any line of `output` (the `{file}:{line}: {message}` convention every native
/// and shell-out check's output follows) matching an [`AcceptedFinding`] whose
/// recorded `content` still agrees with `file_path`'s current line — same
/// line-oriented parsing as `check::scope_output_to_changed_lines`, since
/// `CheckResult` carries no structured per-finding data for these check kinds (only
/// architecture checks do, via `ArchFinding`, out of scope here — see
/// `docs/accepting-findings.md`). Returns the filtered output and whether anything
/// was actually dropped, so a caller recomputing `passed` can tell "genuinely clean"
/// from "clean only because every finding was accepted away."
pub fn filter_accepted(
    output: &str,
    file_path: &Path,
    rel_file: &str,
    checker_name: &str,
    accepted: &AcceptedFindings,
) -> (String, bool) {
    // Only worth reading the file at all if some entry actually names it — most
    // repos' accepted-findings entries, if any exist, are scattered across many
    // files, so this skips a disk read for every other file being checked.
    if !accepted.accepted.iter().any(|e| e.file == rel_file) {
        return (output.to_string(), false);
    }
    let source = std::fs::read_to_string(file_path).unwrap_or_default();
    let source_lines: Vec<&str> = source.lines().collect();

    let prefix = format!("{}:", file_path.display());
    let mut kept = Vec::new();
    let mut dropped_any = false;

    for line in output.lines() {
        let matched = (|| {
            let rest = line.strip_prefix(&prefix)?;
            let (line_no_str, message) = rest.split_once(": ")?;
            let line_no: usize = line_no_str.parse().ok()?;
            let entry =
                accepted.candidate(extract_rule(message, checker_name), rel_file, line_no)?;
            let current = source_lines.get(line_no.saturating_sub(1))?.trim();
            (entry.content.trim() == current).then_some(())
        })()
        .is_some();

        if matched {
            dropped_any = true;
        } else {
            kept.push(line);
        }
    }

    (kept.join("\n"), dropped_any)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rule_reads_the_bracket_prefix() {
        assert_eq!(
            extract_rule("[flag-argument] boolean parameter `x`...", "fallback"),
            "flag-argument"
        );
    }

    #[test]
    fn extract_rule_falls_back_when_no_bracket_prefix() {
        assert_eq!(
            extract_rule(
                "parameter declaration names 2 identifiers...",
                "primitive-obsession"
            ),
            "primitive-obsession"
        );
    }

    fn write_repo(dir: &Path, file_rel: &str, file_content: &str, accepted_json: &str) {
        std::fs::create_dir_all(dir.join(CONFIG_DIR)).unwrap();
        std::fs::write(
            dir.join(CONFIG_DIR).join(ACCEPTED_FINDINGS_FILENAME),
            accepted_json,
        )
        .unwrap();
        let file_path = dir.join(file_rel);
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(&file_path, file_content).unwrap();
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        crate::test_support::unique_temp_dir(&format!("accepted-findings-test-{name}"))
    }

    #[test]
    fn find_accepted_findings_returns_empty_when_no_file_present() {
        let dir = temp_dir("missing");
        let found = find_accepted_findings(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(found.accepted.is_empty());
    }

    #[test]
    fn find_accepted_findings_rejects_an_empty_reason() {
        let dir = temp_dir("empty-reason");
        write_repo(
            &dir,
            "main.go",
            "package main\n",
            r#"{"accepted": [{"rule": "flag-argument", "file": "main.go", "line": 1, "content": "package main", "reason": ""}]}"#,
        );
        let result = find_accepted_findings(&dir);
        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty reason"));
    }

    #[test]
    fn filter_accepted_drops_a_matching_finding_with_unchanged_content() {
        let dir = temp_dir("match");
        write_repo(
            &dir,
            "main.go",
            "package main\n\nfunc f(verbose bool) {}\n",
            r#"{"accepted": [{"rule": "flag-argument", "file": "main.go", "line": 3, "content": "func f(verbose bool) {}", "reason": "CLI -v toggle, standard exception"}]}"#,
        );
        let accepted = find_accepted_findings(&dir).unwrap();
        let file_path = dir.join("main.go");
        let output = format!(
            "{}:3: [flag-argument] boolean parameter `verbose` is branched on directly...",
            file_path.display()
        );
        let (filtered, dropped) =
            filter_accepted(&output, &file_path, "main.go", "syntax-rules-go", &accepted);
        std::fs::remove_dir_all(&dir).ok();
        assert!(dropped);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_accepted_keeps_a_finding_once_the_line_content_has_drifted() {
        let dir = temp_dir("stale");
        write_repo(
            &dir,
            "main.go",
            "package main\n\nfunc f(verbose, debug bool) {}\n",
            r#"{"accepted": [{"rule": "flag-argument", "file": "main.go", "line": 3, "content": "func f(verbose bool) {}", "reason": "CLI -v toggle, standard exception"}]}"#,
        );
        let accepted = find_accepted_findings(&dir).unwrap();
        let file_path = dir.join("main.go");
        let output = format!(
            "{}:3: [flag-argument] boolean parameter `verbose` is branched on directly...",
            file_path.display()
        );
        let (filtered, dropped) =
            filter_accepted(&output, &file_path, "main.go", "syntax-rules-go", &accepted);
        std::fs::remove_dir_all(&dir).ok();
        assert!(!dropped);
        assert_eq!(filtered, output);
    }

    #[test]
    fn filter_accepted_keeps_a_finding_for_a_different_rule_at_the_same_line() {
        let dir = temp_dir("different-rule");
        write_repo(
            &dir,
            "main.go",
            "package main\n\nfunc f(verbose bool) {}\n",
            r#"{"accepted": [{"rule": "flag-argument", "file": "main.go", "line": 3, "content": "func f(verbose bool) {}", "reason": "CLI -v toggle, standard exception"}]}"#,
        );
        let accepted = find_accepted_findings(&dir).unwrap();
        let file_path = dir.join("main.go");
        let output = format!(
            "{}:3: [long-function] body spans 41 lines...",
            file_path.display()
        );
        let (filtered, dropped) =
            filter_accepted(&output, &file_path, "main.go", "syntax-rules-go", &accepted);
        std::fs::remove_dir_all(&dir).ok();
        assert!(!dropped);
        assert_eq!(filtered, output);
    }

    #[test]
    fn filter_accepted_falls_back_to_checker_name_for_a_bracket_free_message() {
        let dir = temp_dir("no-bracket");
        write_repo(
            &dir,
            "user.go",
            "package main\n\nfunc newUser(name, email string) {}\n",
            r#"{"accepted": [{"rule": "primitive-obsession", "file": "user.go", "line": 3, "content": "func newUser(name, email string) {}", "reason": "deliberately not worth a newtype here"}]}"#,
        );
        let accepted = find_accepted_findings(&dir).unwrap();
        let file_path = dir.join("user.go");
        let output = format!(
            "{}:3: parameter declaration names 2 identifiers of primitive type `string`...",
            file_path.display()
        );
        let (filtered, dropped) = filter_accepted(
            &output,
            &file_path,
            "user.go",
            "primitive-obsession",
            &accepted,
        );
        std::fs::remove_dir_all(&dir).ok();
        assert!(dropped);
        assert!(filtered.is_empty());
    }
}
