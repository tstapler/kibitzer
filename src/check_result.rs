//! The result of one check run. A leaf module so both `check` and `inline_post_pass` can name
//! it without depending on each other.

use serde::{Deserialize, Serialize};

use crate::config::Severity;
use crate::inline_ignores::InlineOutcome;

/// Beyond this many lines, `describe()` truncates the command's raw output and points
/// the agent at `command` to see the rest, instead of dumping everything inline —
/// checks like whole-repo doc-structure reports can emit hundreds of lines, which
/// buries the actionable part of the message and burns the agent's context on a single
/// failed check.
const MAX_SUMMARY_LINES: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_name: String,
    pub severity: Severity,
    pub passed: bool,
    pub output: String,
    pub message: Option<String>,
    /// The shell command that produced `output`, substitutions already applied — shown
    /// in the truncation note so the agent can re-run it directly to see everything.
    /// `serde(default)` so a `cache.json` written before this field existed deserializes
    /// (with an empty command) instead of `Cache::load` silently discarding the whole
    /// cache on the first run after upgrade.
    #[serde(default)]
    pub command: String,
    /// Structured findings behind `output`'s flattened text, populated by
    /// [`run_architecture_check`] from the checker's own `Vec<ArchFinding>` return value —
    /// empty for every other `CheckResult` source (shell-out `run_check`, native
    /// `run_native_check`, `SYNTAX_RULES_CHECKERS` in `mcp.rs`), none of which produce
    /// `ArchFinding`s. Exists so a renderer can read each finding's own
    /// `severity_override` instead of only the one `severity` flattened uniformly across
    /// all of `output` — see `mcp.rs::architecture_assessment`. `#[serde(default)]` for
    /// the same reason as `command` above: a `cache.json` written before this field
    /// existed must still deserialize (as an empty `Vec`) instead of `Cache::load`
    /// silently discarding the whole cache on the first run after upgrade.
    #[serde(default)]
    pub findings: Vec<crate::architecture_checks::ArchFinding>,
    /// `true` iff this result is `run_check`'s early-return for a plugin-backed check
    /// whose binary is missing from disk (Task 4.3.1b) — distinct from a check that ran
    /// and failed, so `mcp.rs`/`hook.rs` can render `[skipped]` instead of
    /// `[Advisory]`/`[Blocking]` and an agent doesn't misdiagnose a missing install as a
    /// code defect. `#[serde(default)]` for the same cache-compatibility reason as
    /// `command`/`findings` above.
    #[serde(default)]
    pub plugin_missing: bool,
    /// What inline `kibitzer:ignore` directives did to this check's native findings.
    /// Serialized so a cache hit keeps the footer anchor and dropped list; `serde(default)`
    /// so an older `cache.json` still loads.
    #[serde(default)]
    pub inline: InlineOutcome,
}

impl CheckResult {
    /// A result with no message, command, structured findings or inline outcome; chain the
    /// `with_*` builders to set those.
    pub fn new(
        check_name: impl Into<String>,
        severity: Severity,
        passed: bool,
        output: impl Into<String>,
    ) -> Self {
        CheckResult {
            check_name: check_name.into(),
            severity,
            passed,
            output: output.into(),
            message: None,
            command: String::new(),
            findings: Vec::new(),
            plugin_missing: false,
            inline: InlineOutcome::default(),
        }
    }

    /// A passed result with no output.
    pub fn passing(check_name: impl Into<String>, severity: Severity) -> Self {
        CheckResult::new(check_name, severity, true, String::new())
    }

    pub fn with_message(mut self, message: Option<String>) -> Self {
        self.message = message;
        self
    }

    pub fn with_command(mut self, command: impl Into<String>) -> Self {
        self.command = command.into();
        self
    }

    pub fn with_findings(mut self, findings: Vec<crate::architecture_checks::ArchFinding>) -> Self {
        self.findings = findings;
        self
    }

    pub fn with_plugin_missing(mut self) -> Self {
        self.plugin_missing = true;
        self
    }

    pub fn with_inline(mut self, inline: InlineOutcome) -> Self {
        self.inline = inline;
        self
    }

    /// Text to show the agent for a failed check: a top-level summary by default, with
    /// an explicit path to the full detail on demand. The config `message` explains
    /// *why* the rule exists / is blocking; the command's own `output` says *where* the
    /// violation is. Neither alone is enough to act on, so show both whenever both are
    /// present — but cap `output` at [`MAX_SUMMARY_LINES`] rather than concatenating an
    /// unbounded wall of text, and tell the agent how to drill into the rest.
    pub fn describe(&self) -> String {
        let summary = self.summarize_output();
        match (&self.message, summary.is_empty()) {
            (Some(message), false) => format!("{message}\n{summary}"),
            (Some(message), true) => message.clone(),
            (None, _) => summary,
        }
    }

    fn summarize_output(&self) -> String {
        let lines: Vec<&str> = self.output.trim().lines().collect();
        if lines.len() <= MAX_SUMMARY_LINES {
            return lines.join("\n");
        }
        let shown = lines[..MAX_SUMMARY_LINES].join("\n");
        let hidden = lines.len() - MAX_SUMMARY_LINES;
        format!(
            "{shown}\n… {hidden} more line(s) truncated — see everything, run: {}",
            self.command
        )
    }
}

#[cfg(test)]
mod describe_tests {
    use super::*;

    fn result(message: Option<&str>, output: &str) -> CheckResult {
        CheckResult::new(
            "test-check".to_string(),
            Severity::Blocking,
            false,
            output.to_string(),
        )
        .with_message(message.map(String::from))
        .with_command("some-check-command".to_string())
    }

    #[test]
    fn combines_message_and_output_when_both_present() {
        let r = result(Some("why this is blocking"), "file.md:12: bad anchor");
        assert_eq!(r.describe(), "why this is blocking\nfile.md:12: bad anchor");
    }

    #[test]
    fn falls_back_to_message_when_output_is_empty() {
        let r = result(Some("why this is blocking"), "");
        assert_eq!(r.describe(), "why this is blocking");
    }

    #[test]
    fn falls_back_to_output_when_no_message_configured() {
        let r = result(None, "file.md:12: bad anchor");
        assert_eq!(r.describe(), "file.md:12: bad anchor");
    }

    #[test]
    fn truncates_long_output_and_points_to_full_command() {
        let lines: Vec<String> = (1..=30)
            .map(|n| format!("file.md:{n}: violation"))
            .collect();
        let r = result(None, &lines.join("\n"));
        let described = r.describe();
        let described_lines: Vec<&str> = described.lines().collect();
        assert_eq!(described_lines.len(), MAX_SUMMARY_LINES + 1);
        assert!(
            described_lines[..MAX_SUMMARY_LINES]
                .iter()
                .zip(&lines)
                .all(|(a, b)| a == b)
        );
        assert!(described.contains("10 more line(s) truncated"));
        assert!(described.contains("some-check-command"));
    }

    #[test]
    fn short_output_is_not_truncated() {
        let r = result(None, "one\ntwo\nthree");
        assert_eq!(r.describe(), "one\ntwo\nthree");
    }
}
