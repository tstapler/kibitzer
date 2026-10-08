use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Repo-root-relative directory holding one file per accepted finding — deliberately
/// its own top-level dir rather than living under `.claude` (that's Claude Code's own
/// config namespace, not kibitzer's) and deliberately a directory of many small files
/// rather than one shared JSON array: two branches each adding an entry create two new
/// files instead of both editing the same array, so merging never conflicts.
pub const ACCEPTED_FINDINGS_DIR: &str = ".kibitzer/accepted";

/// One deliberately-accepted finding: a specific checker rule firing correctly at a
/// specific line, kept on purpose rather than fixed or globally suppressed. Distinct
/// from both `docs/reporting-false-positives.md` (for a checker misfiring, not a real
/// hit) and `.kibitzer/inspect.json`'s `disabled`/`scope` (whole-checker granularity,
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
    /// Per-run inline-ignore mode, counter, and scan memo. Rides on this already-threaded
    /// struct to avoid a new parameter on every check entry point; never read from JSON.
    #[serde(skip)]
    pub inline: crate::inline_ignores::InlineIgnoreContext,
}

impl AcceptedFindings {
    fn candidate(&self, rule: &str, rel_file: &str, line: usize) -> Option<&AcceptedFinding> {
        self.accepted
            .iter()
            .find(|e| e.rule == rule && e.file == rel_file && e.line == line)
    }
}

/// Walks upward from `start` looking for a `.kibitzer/accepted/` directory, the same
/// convention as `config::find_config`. Returns an empty [`AcceptedFindings`] (not an
/// error) when no such directory exists anywhere above `start` — accepting findings is
/// opt-in, most repos will never have one. Every `*.json` file directly inside it (not
/// recursed into subdirectories) holds one [`AcceptedFinding`] object; entries are
/// collected in filename order for deterministic output.
pub fn find_accepted_findings(start: &Path) -> Result<AcceptedFindings> {
    let mut dir = crate::config::start_dir(start);
    loop {
        let candidate = dir.join(ACCEPTED_FINDINGS_DIR);
        if candidate.is_dir() {
            return read_accepted_dir(&candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Ok(AcceptedFindings::default()),
        }
    }
}

fn read_accepted_dir(dir: &Path) -> Result<AcceptedFindings> {
    let mut entry_paths = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry
            .with_context(|| format!("reading an entry of {}", dir.display()))?
            .path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            entry_paths.push(path);
        }
    }
    entry_paths.sort();

    let mut accepted = Vec::with_capacity(entry_paths.len());
    for path in entry_paths {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let entry: AcceptedFinding =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
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
        accepted.push(entry);
    }
    Ok(AcceptedFindings {
        accepted,
        ..Default::default()
    })
}

/// The `[rule-id]` a checker self-prefixes its message with, or `fallback` (the
/// checker's own name) when it doesn't.
fn extract_rule<'a>(message: &'a str, fallback: &'a str) -> &'a str {
    message
        .strip_prefix('[')
        .and_then(|rest| rest.find(']').map(|end| &rest[..end]))
        .unwrap_or(fallback)
}

/// The lines of one file an [`AcceptedFinding`] may match, ready to test output lines one at
/// a time.
pub(crate) struct AcceptedLines<'a> {
    accepted: &'a AcceptedFindings,
    rel_file: &'a str,
    checker_name: &'a str,
    prefix: String,
    source_lines: Vec<String>,
}

impl<'a> AcceptedLines<'a> {
    /// `None` when no entry names `rel_file`: most repos' entries are scattered across many
    /// files, so this skips a disk read for every other file being checked.
    pub(crate) fn for_file(
        file_path: &Path,
        rel_file: &'a str,
        checker_name: &'a str,
        accepted: &'a AcceptedFindings,
    ) -> Option<Self> {
        if !accepted.accepted.iter().any(|e| e.file == rel_file) {
            return None;
        }
        let source = std::fs::read_to_string(file_path).unwrap_or_default();
        Some(AcceptedLines {
            accepted,
            rel_file,
            checker_name,
            prefix: format!("{}:", file_path.display()),
            source_lines: source.lines().map(str::to_string).collect(),
        })
    }

    /// Whether `line` (`{file}:{line}: {message}`) matches an entry whose recorded `content`
    /// still agrees with the file's current line.
    pub(crate) fn is_accepted(&self, line: &str) -> bool {
        (|| {
            let rest = line.strip_prefix(&self.prefix)?;
            let (line_no_str, message) = rest.split_once(": ")?;
            let line_no: usize = line_no_str.parse().ok()?;
            let entry = self.accepted.candidate(
                extract_rule(message, self.checker_name),
                self.rel_file,
                line_no,
            )?;
            let current = self.source_lines.get(line_no.saturating_sub(1))?.trim();
            (entry.content.trim() == current).then_some(())
        })()
        .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FilterOutcome {
        output: String,
        dropped_any: bool,
    }

    /// Text-level view of [`AcceptedLines`]: `output` minus the accepted lines.
    fn filter_accepted(
        output: &str,
        file_path: &Path,
        rel_file: &str,
        checker_name: &str,
        accepted: &AcceptedFindings,
    ) -> FilterOutcome {
        let Some(lines) = AcceptedLines::for_file(file_path, rel_file, checker_name, accepted)
        else {
            return FilterOutcome {
                output: output.to_string(),
                dropped_any: false,
            };
        };
        let (dropped, kept): (Vec<&str>, Vec<&str>) =
            output.lines().partition(|line| lines.is_accepted(line));
        FilterOutcome {
            output: kept.join("\n"),
            dropped_any: !dropped.is_empty(),
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn accepted_findings_should_DeserializeUnchanged_When_InlineFieldSkipped() {
        let parsed: AcceptedFindings =
            serde_json::from_str(r#"{"accepted": [], "inline": {"mode": "Disabled"}}"#).unwrap();
        assert!(parsed.accepted.is_empty());
        assert_eq!(
            parsed.inline.mode,
            crate::inline_ignores::InlineIgnoreMode::Apply
        );
    }

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

    /// `accepted_json` is the old `{"accepted": [...]}` shape purely so every call site
    /// below can keep listing entries inline — it's split into one file per array entry
    /// under `ACCEPTED_FINDINGS_DIR`, the real on-disk layout `read_accepted_dir` parses.
    fn write_repo(dir: &Path, file_rel: &str, file_content: &str, accepted_json: &str) {
        let accepted_dir = dir.join(ACCEPTED_FINDINGS_DIR);
        std::fs::create_dir_all(&accepted_dir).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(accepted_json).unwrap();
        for (i, entry) in parsed["accepted"].as_array().unwrap().iter().enumerate() {
            std::fs::write(accepted_dir.join(format!("{i}.json")), entry.to_string()).unwrap();
        }
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
    /// `ACCEPTED_FINDINGS_DIR`.
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

    /// The whole point of one-file-per-finding: two entries added independently (as two
    /// separate files, the way two branches each adding one would land after a
    /// conflict-free merge) both load, regardless of which filename sorts first.
    #[test]
    fn find_accepted_findings_aggregates_every_file_in_the_directory() {
        let dir = temp_dir("multi-file");
        let accepted_dir = dir.join(ACCEPTED_FINDINGS_DIR);
        std::fs::create_dir_all(&accepted_dir).unwrap();
        std::fs::write(
            accepted_dir.join("flag-argument-main-go-3.json"),
            r#"{"rule": "flag-argument", "file": "main.go", "line": 3, "content": "func f(verbose bool) {}", "reason": "CLI -v toggle"}"#,
        )
        .unwrap();
        std::fs::write(
            accepted_dir.join("primitive-obsession-user-go-3.json"),
            r#"{"rule": "primitive-obsession", "file": "user.go", "line": 3, "content": "func newUser(name, email string) {}", "reason": "not worth a newtype"}"#,
        )
        .unwrap();
        // A stray non-JSON file in the same directory (e.g. a README explaining the
        // convention) must be ignored rather than failing the whole load.
        std::fs::write(
            accepted_dir.join("README.md"),
            "see docs/accepting-findings.md",
        )
        .unwrap();

        let found = find_accepted_findings(&dir).unwrap();
        assert_eq!(found.accepted.len(), 2);
        // Pins the "filename order" half of the doc comment's determinism claim, not
        // just the count: "flag-argument-..." sorts before "primitive-obsession-...".
        assert_eq!(found.accepted[0].rule, "flag-argument");
        assert_eq!(found.accepted[1].rule, "primitive-obsession");
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
