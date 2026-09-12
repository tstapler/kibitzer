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
    /// The flagged line's exact text when accepted. If the current line no longer
    /// matches, suppression silently lapses and the finding reappears.
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
    let mut dir = crate::config::start_dir(start);
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

/// The `[rule-id]` a checker self-prefixes its message with, or `fallback` (the
/// checker's own name) when it doesn't.
fn extract_rule<'a>(message: &'a str, fallback: &'a str) -> &'a str {
    message
        .strip_prefix('[')
        .and_then(|rest| rest.find(']').map(|end| &rest[..end]))
        .unwrap_or(fallback)
}

/// Output of [`filter_accepted`]: the filtered text plus whether anything was
/// actually dropped from it, so a caller recomputing `passed` can tell "genuinely
/// clean" from "clean only because every finding was accepted away."
pub(crate) struct FilterOutcome {
    pub output: String,
    // `drop_accepted_findings` derives `passed` from `output` alone (empty vs.
    // non-empty) rather than from this flag — read by this module's own tests, not
    // yet consumed by any production caller, hence `#[allow(dead_code)]`.
    #[allow(dead_code)]
    pub dropped_any: bool,
}

/// Drops any line of `output` (the `{file}:{line}: {message}` convention every native
/// and shell-out check's output follows) matching an [`AcceptedFinding`] whose
/// recorded `content` still agrees with `file_path`'s current line — same
/// line-oriented parsing as `check::scope_output_to_changed_lines`, since
/// `CheckResult` carries no structured per-finding data for these check kinds (only
/// architecture checks do, via `ArchFinding`, out of scope here — see
/// `docs/accepting-findings.md`).
pub fn filter_accepted(
    output: &str,
    file_path: &Path,
    rel_file: &str,
    checker_name: &str,
    accepted: &AcceptedFindings,
) -> FilterOutcome {
    // Only worth reading the file at all if some entry actually names it — most
    // repos' accepted-findings entries, if any exist, are scattered across many
    // files, so this skips a disk read for every other file being checked.
    if !accepted.accepted.iter().any(|e| e.file == rel_file) {
        return FilterOutcome {
            output: output.to_string(),
            dropped_any: false,
        };
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

    FilterOutcome {
        output: kept.join("\n"),
        dropped_any,
    }
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
        assert!(found.accepted.is_empty());
        std::fs::remove_dir_all(&dir).ok();
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
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty reason"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Precedent: `config.rs`'s `find_repo_root_walks_up_to_nearest_dot_git` tests the
    /// same kind of upward walk for `.git`; this is the equivalent for
    /// `.claude/kibitzer-accepted.json`.
    #[test]
    fn find_accepted_findings_walks_upward_from_a_nested_directory() {
        let dir = temp_dir("upward-walk");
        write_repo(
            &dir,
            "main.go",
            "package main\n",
            r#"{"accepted": [{"rule": "flag-argument", "file": "main.go", "line": 1, "content": "package main", "reason": "walked up to find this"}]}"#,
        );
        let nested = dir.join("a/b/c");
        std::fs::create_dir_all(&nested).unwrap();

        let found = find_accepted_findings(&nested).unwrap();
        assert_eq!(found.accepted.len(), 1);
        assert_eq!(found.accepted[0].file, "main.go");
        std::fs::remove_dir_all(&dir).ok();
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
        let outcome = filter_accepted(&output, &file_path, "main.go", "syntax-rules-go", &accepted);
        assert!(outcome.dropped_any);
        assert!(outcome.output.is_empty());
        std::fs::remove_dir_all(&dir).ok();
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
        let outcome = filter_accepted(&output, &file_path, "main.go", "syntax-rules-go", &accepted);
        assert!(!outcome.dropped_any);
        assert_eq!(outcome.output, output);
        std::fs::remove_dir_all(&dir).ok();
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
        let outcome = filter_accepted(&output, &file_path, "main.go", "syntax-rules-go", &accepted);
        assert!(!outcome.dropped_any);
        assert_eq!(outcome.output, output);
        std::fs::remove_dir_all(&dir).ok();
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
        let outcome = filter_accepted(
            &output,
            &file_path,
            "user.go",
            "primitive-obsession",
            &accepted,
        );
        assert!(outcome.dropped_any);
        assert!(outcome.output.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The early-return fast path (no entry names this file) must leave `output`
    /// completely untouched and never even attempt to read the checked file from
    /// disk — proven concretely here by pointing `file_path` at a file that doesn't
    /// exist at all; a build that tried to read it would panic or return an error
    /// instead of the unchanged output this asserts.
    #[test]
    fn filter_accepted_fast_path_skips_the_disk_read_for_an_unrelated_file() {
        let dir = temp_dir("unrelated-file-fast-path");
        write_repo(
            &dir,
            "main.go",
            "package main\n\nfunc f(verbose bool) {}\n",
            r#"{"accepted": [{"rule": "flag-argument", "file": "main.go", "line": 3, "content": "func f(verbose bool) {}", "reason": "CLI -v toggle, standard exception"}]}"#,
        );
        let accepted = find_accepted_findings(&dir).unwrap();
        let nonexistent = dir.join("does-not-exist-on-disk.go");
        let output = format!(
            "{}:1: [flag-argument] boolean parameter `x`...",
            nonexistent.display()
        );

        let outcome = filter_accepted(
            &output,
            &nonexistent,
            "does-not-exist-on-disk.go",
            "syntax-rules-go",
            &accepted,
        );
        assert!(!outcome.dropped_any);
        assert_eq!(outcome.output, output);
        std::fs::remove_dir_all(&dir).ok();
    }
}
