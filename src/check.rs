use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::config::{Check, OutputFormat, Severity};
use crate::glob::matches_scope;
use crate::plugin::Registry;

/// Beyond this many lines, `describe()` truncates the command's raw output and points
/// the agent at `command` to see the rest, instead of dumping everything inline —
/// checks like whole-repo doc-structure reports can emit hundreds of lines, which
/// buries the actionable part of the message and burns the agent's context on a single
/// failed check.
const MAX_SUMMARY_LINES: usize = 20;

/// Wall-clock ceiling on a single `run_check` command dispatch (see
/// [`run_command_with_timeout`]). Plugin binaries are the class of `command` check most
/// likely to genuinely hang (a first-run model load, a downloaded runtime waiting on
/// something that never arrives), so this applies to every shell-out check, not just
/// plugin-backed ones.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

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
}

impl CheckResult {
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
        CheckResult {
            check_name: "test-check".to_string(),
            severity: Severity::Blocking,
            passed: false,
            output: output.to_string(),
            message: message.map(String::from),
            command: "some-check-command".to_string(),
            findings: Vec::new(),
            plugin_missing: false,
        }
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

/// Run a single check against `file_path` (already confirmed in-scope by the caller).
/// `changed_lines`, when present, scopes the result to findings that fall within those
/// 1-indexed inclusive line ranges — see [`scope_output_to_changed_lines`]. `registry`
/// should be loaded once per batch/request by the caller (see [`run_checks_for_trigger`])
/// rather than reloaded here — `registry.json` is read on every plugin-missing check
/// otherwise, even for a repo with zero plugins installed.
pub fn run_check(
    check: &Check,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    registry: &Registry,
) -> anyhow::Result<CheckResult> {
    run_check_with_timeout(
        check,
        repo_root,
        file_path,
        changed_lines,
        COMMAND_TIMEOUT,
        registry,
    )
}

/// Parameterized-timeout counterpart to [`run_check`] — the real entry point calls this
/// with [`COMMAND_TIMEOUT`]; tests call it directly with a short duration so the timeout
/// path can be exercised without an actual 30-second wait.
fn run_check_with_timeout(
    check: &Check,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    timeout: Duration,
    registry: &Registry,
) -> anyhow::Result<CheckResult> {
    if let Some(checker_name) = &check.checker {
        return run_native_check(check, checker_name, repo_root, file_path, changed_lines);
    }

    if check.architecture_checker.is_some() {
        // A whole-repo architecture check (`CheckKind::WholeRepoNative`) only ever runs
        // against a real import graph via `run_architecture_check` — `run.rs::run_batch`
        // partitions `Check`s by `is_per_file()` before dispatching either checker for
        // exactly this reason. A per-file caller of `run_checks_for_trigger` has no
        // import graph to run one against, so treat it as trivially passing here rather
        // than falling through to the `command`-only branch below, which would panic on
        // `check.command` being unset (an architecture check has neither `command` nor
        // `checker`).
        return Ok(CheckResult {
            check_name: check.name.clone(),
            severity: check.severity,
            passed: true,
            output: String::new(),
            message: None,
            command: String::new(),
            findings: Vec::new(),
            plugin_missing: false,
        });
    }

    // Task 4.3.1b: a plugin-backed check (always `command`-based, never `checker`) whose
    // binary has gone missing (removed out-of-band, or the whole plugin uninstalled)
    // short-circuits here, before a `sh -c` is ever spawned against a path that doesn't
    // exist — that would otherwise surface as generic shell "command not found" noise
    // indistinguishable from a real check failure. Severity is forced to `Advisory`
    // regardless of `check.severity` so a missing install never blocks an edit.
    if let Some(binary_path) = registry.missing_binary_for(&check.name) {
        return Ok(CheckResult {
            check_name: check.name.clone(),
            severity: Severity::Advisory,
            passed: false,
            output: String::new(),
            message: Some(format!(
                "plugin '{}' is not installed (expected binary at {}) — run `kibitzer plugin install {}`",
                check.name,
                binary_path.display(),
                check.name
            )),
            command: String::new(),
            findings: Vec::new(),
            plugin_missing: true,
        });
    }

    let command = check
        .command
        .as_deref()
        .expect("config-load validation guarantees command is set when checker is not");

    let cmd_str = substitute_command(command, file_path, changed_lines);
    let output = match run_command_with_timeout(&cmd_str, repo_root, timeout)? {
        CommandOutcome::Completed(output) => output,
        CommandOutcome::TimedOut => {
            return Ok(CheckResult {
                check_name: check.name.clone(),
                severity: check.severity,
                passed: false,
                output: format!("command timed out after {timeout:?} and was killed: {cmd_str}"),
                message: Some(format!(
                    "{}check timed out after {timeout:?} and was killed",
                    check
                        .message
                        .as_ref()
                        .map(|m| format!("{m} — "))
                        .unwrap_or_default()
                )),
                command: cmd_str,
                findings: Vec::new(),
                plugin_missing: false,
            });
        }
    };

    let passed_raw = output.status.success();

    let (mut combined, is_sarif) = match check.output_format {
        Some(OutputFormat::Sarif) => match render_sarif_output(&output.stdout) {
            Some(rendered) => (rendered, true),
            None => {
                let mut fallback = String::from_utf8_lossy(&output.stdout).into_owned();
                fallback.push_str(&String::from_utf8_lossy(&output.stderr));
                (fallback, false)
            }
        },
        None => {
            let mut c = String::from_utf8_lossy(&output.stdout).into_owned();
            c.push_str(&String::from_utf8_lossy(&output.stderr));
            (c, false)
        }
    };

    // SARIF output is already a structured summary, not `{file}:{line}: message` text —
    // diff-aware scoping only understands the latter, so it's skipped here. Documented
    // as a known limitation in docs/output-formats.md.
    let passed = if is_sarif {
        passed_raw
    } else if let Some(ranges) = changed_lines {
        let (scoped, scoped_passed) =
            scope_output_to_changed_lines(&combined, file_path, ranges, passed_raw);
        combined = scoped;
        scoped_passed
    } else {
        passed_raw
    };

    let mut severity = check.severity;
    let mut message = check.message.clone();

    if !passed && severity == Severity::Blocking {
        let baseline = if command.contains("{file}") {
            check_against_git_head(check, repo_root, file_path, changed_lines)
        } else {
            check_against_git_head_repo(check, repo_root)
        };
        if let Some(false) = baseline {
            severity = Severity::Advisory;
            message = Some(format!(
                "{} (downgraded: this violation predates your edits — already present \
                 at the git HEAD commit)",
                message.unwrap_or_default()
            ));
        }
    }

    Ok(CheckResult {
        check_name: check.name.clone(),
        severity,
        passed,
        output: combined,
        message,
        command: cmd_str,
        findings: Vec::new(),
        plugin_missing: false,
    })
}

/// Outcome of [`run_command_with_timeout`]: either the child exited (successfully or not —
/// that's `Output.status`'s job to say) within the deadline, or it didn't and was killed.
enum CommandOutcome {
    Completed(std::process::Output),
    TimedOut,
}

/// Runs `sh -c <cmd_str>` in `repo_root`, killing it and reporting [`CommandOutcome::TimedOut`]
/// if it hasn't exited within `timeout`, instead of blocking `run_check` forever.
///
/// The child is spawned with piped stdout/stderr and its `wait_with_output()` — which does
/// its own correct concurrent draining of both pipes — runs on a background thread that
/// reports back over a channel; the caller does a bounded `recv_timeout` on that channel
/// rather than polling or hand-rolling pipe reads (hand-rolled reads risk a deadlock if a
/// pipe buffer fills while the reader is blocked on the other one). On timeout, `kill -KILL`
/// is shelled out to the recorded pid to unblock the background thread's `wait_with_output()`
/// so it doesn't leak, then `TimedOut` is returned immediately without waiting on it further.
///
/// Four known v1 limitations, accepted for this pass (Tech Debt Disposition, `src/check.rs`
/// row):
/// - If the child writes more than the OS pipe buffer (~64KB on Linux) before exiting, it
///   can block on `write()` before the timeout is ever noticed. Acceptable today because
///   every check in `default_checks()` produces small, single-file lint output.
/// - `kill -KILL <pid>` only signals the direct child (`sh`), not its descendants, so a
///   compound/piped command (e.g. `foo | bar`) can leave orphaned grandchild processes
///   running after the timeout fires. Fully closing this would need process-group/session
///   based killing (`setsid` + `killpg`), out of scope here.
/// - A distinct issue from the one above: if a descendant the killed `sh` leaves behind
///   still holds the piped stdout/stderr fds open, the background thread's
///   `wait_with_output()` (which reads both pipes to EOF) never returns — this function
///   itself still returns promptly (it doesn't join that thread), but the thread leaks
///   for the rest of the process's lifetime, one per such hang. Acceptable for now in a
///   short-lived CLI invocation; a long-running daemon/MCP-server process should watch
///   for this if check hangs become common.
/// - `kill -KILL <pid>` targets a bare pid with no liveness check first, so on a system
///   under enough process churn the pid could theoretically have already been reused by
///   an unrelated process between the child exiting on its own and the kill call landing
///   — signaling the wrong process. Low likelihood in practice and no cheap fix exists on
///   stable `std::process` (no atomic "kill iff still my child" primitive); accepted as a
///   documented risk rather than worked around.
fn run_command_with_timeout(
    cmd_str: &str,
    repo_root: &Path,
    timeout: Duration,
) -> anyhow::Result<CommandOutcome> {
    let child = Command::new("sh")
        .arg("-c")
        .arg(cmd_str)
        .current_dir(repo_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id();

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(timeout) {
        Ok(result) => Ok(CommandOutcome::Completed(result?)),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Unix-only, matching daemon.rs's existing Unix-only assumption
            // (std::os::unix::net::UnixListener) — this codebase doesn't target Windows.
            let _ = Command::new("kill")
                .arg("-KILL")
                .arg(pid.to_string())
                .status();
            Ok(CommandOutcome::TimedOut)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("command dispatch thread disconnected without reporting a result")
        }
    }
}

/// In-process counterpart to the shell-out path above for `config::Check::checker`-based
/// checks: runs the named native [`crate::checker::Checker`] against `file_path` instead
/// of spawning a command, but otherwise applies the exact same diff-scoping and git-HEAD
/// baseline-suppression logic so a check's behavior doesn't change based on whether it's
/// implemented natively or as a shell-out.
fn run_native_check(
    check: &Check,
    checker_name: &str,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> anyhow::Result<CheckResult> {
    let cmd_str = format!(
        "kibitzer check native {checker_name} {}",
        file_path.display()
    );

    if let Some(checker) = crate::checker::lookup(checker_name) {
        let globs: Vec<String> = checker.file_globs().iter().map(|g| g.to_string()).collect();
        let rel_path = relativize(repo_root, file_path);
        if !matches_scope(&rel_path, &globs) {
            return Ok(CheckResult {
                check_name: check.name.clone(),
                severity: check.severity,
                passed: true,
                output: String::new(),
                message: None,
                command: cmd_str,
                findings: Vec::new(),
                plugin_missing: false,
            });
        }
    }

    // Degrade to a failed CheckResult on error (e.g. an unreadable file) instead of
    // propagating, matching the shell-out path above where a command's own failure is
    // captured as `passed_raw = false` rather than aborting the whole batch — a single
    // bad file shouldn't kill every other check/file in the run.
    let (mut combined, passed_raw) = match run_checker_against_file(checker_name, file_path) {
        Ok(result) => result,
        Err(err) => {
            return Ok(CheckResult {
                check_name: check.name.clone(),
                severity: check.severity,
                passed: false,
                output: format!("{err:#}"),
                message: check.message.clone(),
                command: cmd_str,
                findings: Vec::new(),
                plugin_missing: false,
            });
        }
    };

    let passed = if let Some(ranges) = changed_lines {
        let (scoped, scoped_passed) =
            scope_output_to_changed_lines(&combined, file_path, ranges, passed_raw);
        combined = scoped;
        scoped_passed
    } else {
        passed_raw
    };

    let mut severity = check.severity;
    let mut message = check.message.clone();

    if !passed && severity == Severity::Blocking {
        let baseline =
            check_native_against_git_head(checker_name, repo_root, file_path, changed_lines);
        if let Some(false) = baseline {
            severity = Severity::Advisory;
            message = Some(format!(
                "{} (downgraded: this violation predates your edits — already present \
                 at the git HEAD commit)",
                message.unwrap_or_default()
            ));
        }
    }

    Ok(CheckResult {
        check_name: check.name.clone(),
        severity,
        passed,
        output: combined,
        message,
        command: cmd_str,
        findings: Vec::new(),
        plugin_missing: false,
    })
}

/// Runs `checker_name` against `source` (as if it were the content of `file_path`),
/// producing output in the same `{file}:{line}: {message}` convention a shell-out check's
/// command output would follow, so downstream diff-scoping and baseline logic can treat
/// native and shell-out checks identically.
fn run_checker_against_source(
    checker_name: &str,
    file_path: &Path,
    source: &str,
) -> anyhow::Result<(String, bool)> {
    let findings = crate::checker::run_checker(checker_name, file_path, source)?;
    let passed = findings.is_empty();
    let combined = findings
        .iter()
        .map(|f| format!("{}:{}: {}", file_path.display(), f.line, f.message))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((combined, passed))
}

/// Native per-file checks skip any file at or above this size rather than parsing it.
/// Now that [`crate::config::default_checks`] turns every native checker on for every
/// repo with no `.claude/inspect.json` of its own, this is what keeps that on-by-default
/// behavior cheap: vendored bundles, generated code, and minified assets can be
/// megabytes, and every native checker pays a full read plus (for most of them) a
/// tree-sitter parse. `PostToolUse` runs this path on every single edit, so a file this
/// large — which a "long file"/"long function" checker has nothing useful to say about
/// anyway — is worth skipping outright rather than paying that cost every time.
const MAX_NATIVE_CHECK_BYTES: u64 = 2 * 1024 * 1024;

fn run_checker_against_file(
    checker_name: &str,
    file_path: &Path,
) -> anyhow::Result<(String, bool)> {
    if let Ok(metadata) = std::fs::metadata(file_path)
        && metadata.len() > MAX_NATIVE_CHECK_BYTES
    {
        return Ok((String::new(), true));
    }
    let source = std::fs::read_to_string(file_path)
        .with_context(|| format!("reading {}", file_path.display()))?;
    run_checker_against_source(checker_name, file_path, &source)
}

/// Native-checker counterpart to [`check_against_git_head`]: same git-HEAD comparison, but
/// runs the checker in-process against the HEAD content instead of shelling out to a
/// substituted command.
fn check_native_against_git_head(
    checker_name: &str,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> Option<bool> {
    let rel_path = relativize(repo_root, file_path);
    let show = Command::new("git")
        .args(["show", &format!("HEAD:{rel_path}")])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !show.status.success() {
        return None;
    }

    let head_ranges = match changed_lines {
        Some(ranges) => Some(map_ranges_to_head(repo_root, &rel_path, ranges)?),
        None => None,
    };
    if let Some(ranges) = &head_ranges
        && ranges.is_empty()
    {
        return Some(true);
    }

    let source = String::from_utf8(show.stdout).ok()?;
    let (combined, passed_raw) =
        run_checker_against_source(checker_name, file_path, &source).ok()?;

    let passed = if let Some(ranges) = &head_ranges {
        let (_, scoped_passed) =
            scope_output_to_changed_lines(&combined, file_path, ranges, passed_raw);
        scoped_passed
    } else {
        passed_raw
    };
    Some(passed)
}

/// Substitute `{file}` and, if present, `{changed_lines}` into `command`. `{changed_lines}`
/// is a comma-separated list of `start-end` (1-indexed, inclusive) ranges, e.g. `12-15,40-40`,
/// or empty when no ranges are known — the documented convention for shell-command checks
/// that want to scope their own scan instead of relying on output-line filtering.
fn substitute_command(
    command: &str,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> String {
    let mut cmd = command.replace("{file}", &file_path.display().to_string());
    if cmd.contains("{changed_lines}") {
        let ranges_str = changed_lines
            .map(|ranges| {
                ranges
                    .iter()
                    .map(|(start, end)| format!("{start}-{end}"))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        cmd = cmd.replace("{changed_lines}", &ranges_str);
    }
    cmd
}

#[derive(Debug, Deserialize)]
struct SarifLog {
    #[serde(default)]
    runs: Vec<SarifRun>,
}

#[derive(Debug, Deserialize)]
struct SarifRun {
    #[serde(default)]
    results: Vec<SarifResult>,
}

#[derive(Debug, Deserialize)]
struct SarifResult {
    #[serde(default)]
    level: Option<String>,
    #[serde(rename = "ruleId", default)]
    rule_id: Option<String>,
    message: SarifMessage,
    #[serde(default)]
    locations: Vec<SarifLocation>,
}

#[derive(Debug, Deserialize)]
struct SarifMessage {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
struct SarifLocation {
    #[serde(rename = "physicalLocation", default)]
    physical_location: Option<SarifPhysicalLocation>,
}

#[derive(Debug, Deserialize)]
struct SarifPhysicalLocation {
    #[serde(rename = "artifactLocation", default)]
    artifact_location: Option<SarifArtifactLocation>,
    #[serde(default)]
    region: Option<SarifRegion>,
}

#[derive(Debug, Deserialize)]
struct SarifArtifactLocation {
    #[serde(default)]
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SarifRegion {
    #[serde(rename = "startLine", default)]
    start_line: Option<u64>,
}

/// Parse a linter's SARIF 2.1.0 log (`output_format: "sarif"`) into a plain-text summary:
/// a leading count-by-level line (so "1 warning" and "50 errors" no longer read the same),
/// followed by one `{uri}:{line}: [{level}] {message} ({ruleId})` line per result. Returns
/// `None` on anything that doesn't parse as SARIF, so the caller can fall back to raw
/// stdout+stderr instead of hiding a misconfigured `output_format` behind an empty result.
fn render_sarif_output(stdout: &[u8]) -> Option<String> {
    let log: SarifLog = serde_json::from_slice(stdout).ok()?;

    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut lines = Vec::new();

    for result in log.runs.iter().flat_map(|run| &run.results) {
        let level = result
            .level
            .clone()
            .unwrap_or_else(|| "warning".to_string());
        *counts.entry(level.clone()).or_insert(0) += 1;

        let physical = result
            .locations
            .first()
            .and_then(|loc| loc.physical_location.as_ref());
        let uri = physical
            .and_then(|p| p.artifact_location.as_ref())
            .and_then(|a| a.uri.as_deref());
        let start_line = physical
            .and_then(|p| p.region.as_ref())
            .and_then(|r| r.start_line);

        let location = match (uri, start_line) {
            (Some(uri), Some(line)) => format!("{uri}:{line}: "),
            (Some(uri), None) => format!("{uri}: "),
            (None, _) => String::new(),
        };
        let rule = result
            .rule_id
            .as_deref()
            .map(|id| format!(" ({id})"))
            .unwrap_or_default();
        lines.push(format!("{location}[{level}] {}{rule}", result.message.text));
    }

    let header = if counts.is_empty() {
        "0 findings".to_string()
    } else {
        counts
            .iter()
            .map(|(level, n)| format!("{n} {level}(s)"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut rendered = vec![header];
    rendered.extend(lines);
    Some(rendered.join("\n"))
}

#[cfg(test)]
mod sarif_tests {
    use super::*;

    fn sarif_log(results_json: &str) -> String {
        format!(r#"{{"version": "2.1.0", "runs": [{{"results": [{results_json}]}}]}}"#)
    }

    #[test]
    fn renders_counts_and_findings() {
        let log = sarif_log(
            r#"{"level": "error", "ruleId": "no-foo", "message": {"text": "found a foo"},
                "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/lib.rs"},
                "region": {"startLine": 12}}}]}"#,
        );
        let rendered = render_sarif_output(log.as_bytes()).unwrap();
        assert_eq!(
            rendered,
            "1 error(s)\nsrc/lib.rs:12: [error] found a foo (no-foo)"
        );
    }

    #[test]
    fn defaults_missing_level_to_warning() {
        let log = sarif_log(r#"{"message": {"text": "no level given"}, "locations": []}"#);
        let rendered = render_sarif_output(log.as_bytes()).unwrap();
        assert_eq!(rendered, "1 warning(s)\n[warning] no level given");
    }

    #[test]
    fn empty_results_render_as_zero_findings() {
        let log = r#"{"version": "2.1.0", "runs": [{"results": []}]}"#;
        let rendered = render_sarif_output(log.as_bytes()).unwrap();
        assert_eq!(rendered, "0 findings");
    }

    #[test]
    fn counts_multiple_levels_separately() {
        let log = sarif_log(
            r#"{"level": "error", "message": {"text": "e1"}, "locations": []},
               {"level": "error", "message": {"text": "e2"}, "locations": []},
               {"level": "note", "message": {"text": "n1"}, "locations": []}"#,
        );
        let rendered = render_sarif_output(log.as_bytes()).unwrap();
        assert!(rendered.starts_with("2 error(s), 1 note(s)\n"));
    }

    #[test]
    fn returns_none_for_invalid_json() {
        assert!(render_sarif_output(b"not json").is_none());
    }
}

/// Filter a check's `{file}:{line}: message`-style output down to lines whose line number
/// falls within `ranges`, and recompute pass/fail from what survives. Lines that don't
/// follow the `{file}:{line}:` convention are kept as-is (their line can't be attributed to
/// a range) and count toward a failure if any survive — this is deliberately conservative:
/// output kibitzer doesn't understand should not be silently swallowed.
fn scope_output_to_changed_lines(
    output: &str,
    file_path: &Path,
    ranges: &[(usize, usize)],
    passed_raw: bool,
) -> (String, bool) {
    if passed_raw {
        return (output.to_string(), passed_raw);
    }
    if ranges.is_empty() {
        // Diff-scoping is active (the caller only reaches this function when
        // `changed_lines` was `Some(_)`) but there are zero changed-line ranges to
        // scope to — e.g. a pure-deletion edit (see `hook::compute_changed_lines`).
        // Nothing can be attributed to this edit, so every finding is out of scope,
        // not "unscoped" — unlike the `changed_lines: None` case, this must not fall
        // back to the raw whole-file output.
        return (String::new(), true);
    }

    let prefix = format!("{}:", file_path.display());
    let mut kept = Vec::new();
    let mut any_attributed_line = false;
    let mut any_kept_finding = false;

    for line in output.lines() {
        let has_prefix = line.strip_prefix(&prefix).is_some();
        let line_no = line
            .strip_prefix(&prefix)
            .and_then(|rest| rest.split(':').next())
            .and_then(|n| n.parse::<usize>().ok());

        match line_no {
            Some(n) => {
                any_attributed_line = true;
                if ranges.iter().any(|(start, end)| n >= *start && n <= *end) {
                    any_kept_finding = true;
                    kept.push(line);
                }
            }
            None if has_prefix && !line.is_empty() => {
                // Has the file prefix but the line number after it doesn't parse —
                // can't attribute it to a range, so (per the conservative-keep policy
                // above) keep it displayed AND count it toward failure, same as a
                // line with no prefix at all.
                any_kept_finding = true;
                kept.push(line);
            }
            None => kept.push(line),
        }
    }

    if !any_attributed_line {
        // Output doesn't follow the convention at all — can't scope it, leave untouched.
        return (output.to_string(), passed_raw);
    }

    let filtered = kept.join("\n");
    let unattributed_kept = kept
        .iter()
        .any(|l| l.strip_prefix(&prefix).is_none() && !l.is_empty());
    let passed = !(any_kept_finding || unattributed_kept);
    (filtered, passed)
}

/// Process-wide nonce so concurrent baseline checks (the daemon spawns one thread per
/// connection) never share a temp file path, even when they're checking files with the
/// same extension at the same instant — a shared path let one thread's baseline read/write
/// race with another's, corrupting both checks' results.
static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Re-run `check` against the file's `git show HEAD:<relpath>` content to determine
/// whether a current failure predates this session's edits. `changed_lines`, when present,
/// scopes the baseline's own pass/fail — but `changed_lines` is in *current-file* line
/// coordinates, which don't line up with HEAD's coordinates once the edit (or any earlier
/// edit in the same file) has added or removed lines. We translate the ranges through the
/// current-vs-HEAD diff hunks (`map_ranges_to_head`) before scoping, so a range that maps to
/// unrelated old content doesn't leak into the comparison.
///
/// Returns `Some(true)` if the baseline passes (no violation in HEAD — the edit genuinely
/// introduced this failure), `Some(false)` if the baseline also fails (pre-existing
/// violation, not introduced by the current edit), or `None` if the baseline can't be
/// determined (untracked file, no HEAD, not a git repo, diff can't be parsed, etc.) —
/// callers should treat `None` as "can't tell, don't suppress."
fn check_against_git_head(
    check: &Check,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> Option<bool> {
    let rel_path = relativize(repo_root, file_path);
    let show = Command::new("git")
        .args(["show", &format!("HEAD:{rel_path}")])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !show.status.success() {
        return None;
    }

    let head_ranges = match changed_lines {
        Some(ranges) => Some(map_ranges_to_head(repo_root, &rel_path, ranges)?),
        None => None,
    };
    // A range that maps to "nothing in HEAD" (pure insertion — the lines simply didn't
    // exist before) means there's no baseline content to compare against for it. If every
    // range is like that, the whole edit is new, so there's nothing pre-existing to find.
    if let Some(ranges) = &head_ranges
        && ranges.is_empty()
    {
        return Some(true);
    }

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut tmp_path = file_path.to_path_buf();
    let nonce = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(
        ".kibitzer-head-{}-{}{}",
        std::process::id(),
        nonce,
        if ext.is_empty() {
            String::new()
        } else {
            format!(".{ext}")
        }
    );
    tmp_path.set_file_name(tmp_name);
    std::fs::write(&tmp_path, &show.stdout).ok()?;

    let command = check
        .command
        .as_deref()
        .expect("config-load validation guarantees command is set when checker is not");
    let cmd_str = substitute_command(command, &tmp_path, head_ranges.as_deref());
    let result = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .current_dir(repo_root)
        .output();

    let _ = std::fs::remove_file(&tmp_path);

    let output = result.ok()?;
    let passed_raw = output.status.success();
    let passed = if let Some(ranges) = &head_ranges {
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        let (_, scoped_passed) =
            scope_output_to_changed_lines(&combined, &tmp_path, ranges, passed_raw);
        scoped_passed
    } else {
        passed_raw
    };
    Some(passed)
}

/// Repo-wide counterpart to `check_against_git_head`: re-run `check` against a snapshot of
/// the whole tree at HEAD (via `git archive`, not `git worktree` — see issue #2's plan for
/// why: worktrees mutate repo-global `.git/worktrees/` state, which is unsafe to race under
/// the daemon's one-thread-per-connection model and leaks metadata on a crash between `add`
/// and `remove`). `git archive` is a pure read into a private scratch directory, so
/// concurrent callers never interfere and cleanup is a plain `rm -rf`.
///
/// Returns `Some(true)` if the baseline passes (no violation at HEAD), `Some(false)` if the
/// baseline also fails (pre-existing, not introduced by the current edit), or `None` if the
/// baseline can't be determined (no HEAD, not a git repo, archive/tar failure, etc.).
fn check_against_git_head_repo(check: &Check, repo_root: &Path) -> Option<bool> {
    let nonce = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let snapshot_dir = std::env::temp_dir().join(format!(
        "kibitzer-head-snapshot-{}-{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&snapshot_dir).ok()?;

    let archive = match Command::new("git")
        .args(["archive", "HEAD"])
        .current_dir(repo_root)
        .output()
    {
        Ok(archive) => archive,
        Err(_) => {
            let _ = std::fs::remove_dir_all(&snapshot_dir);
            return None;
        }
    };
    if !archive.status.success() {
        let _ = std::fs::remove_dir_all(&snapshot_dir);
        return None;
    }

    let mut tar = match Command::new("tar")
        .args(["-x", "-C"])
        .arg(&snapshot_dir)
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(tar) => tar,
        Err(_) => {
            let _ = std::fs::remove_dir_all(&snapshot_dir);
            return None;
        }
    };
    let write_ok = tar
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(&archive.stdout).is_ok())
        .unwrap_or(false);
    let wait_ok = tar.wait().map(|s| s.success()).unwrap_or(false);
    if !write_ok || !wait_ok {
        let _ = std::fs::remove_dir_all(&snapshot_dir);
        return None;
    }

    let command = check
        .command
        .as_deref()
        .expect("config-load validation guarantees command is set when checker is not");
    let cmd_str = substitute_command(command, &snapshot_dir, None);
    let result = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .current_dir(&snapshot_dir)
        .output();

    let _ = std::fs::remove_dir_all(&snapshot_dir);

    Some(result.ok()?.status.success())
}

/// One check name resolved against three registries — import-graph
/// ([`ArchitectureChecker`](crate::architecture_checks::ArchitectureChecker)), `ArchModel`
/// ([`ArchModelChecker`](crate::architecture_checks::ArchModelChecker), needs symbol
/// composition), or declaration-based — so callers don't need to know which one a name
/// lives in.
pub enum AnyArchitectureChecker {
    Import(Box<dyn crate::architecture_checks::ArchitectureChecker>),
    Model(Box<dyn crate::architecture_checks::ArchModelChecker>),
    Declaration(Box<dyn crate::declaration_checks::DeclarationChecker>),
}

/// Resolves `name` against [`crate::architecture_checks::registry()`] first, then
/// [`crate::architecture_checks::model_registry()`], falling back to
/// [`crate::declaration_checks::registry()`]'s (Phase-2-only, currently stubbed to always
/// miss) lookup. Returns `None` when `name` isn't found in any of the three — the "unknown
/// architecture checker" case both `validate()` and `run_architecture_check` report.
pub fn lookup_any_architecture_checker(name: &str) -> Option<AnyArchitectureChecker> {
    if let Some(checker) = crate::architecture_checks::lookup(name) {
        return Some(AnyArchitectureChecker::Import(checker));
    }
    if let Some(checker) = crate::architecture_checks::lookup_model(name) {
        return Some(AnyArchitectureChecker::Model(checker));
    }
    crate::declaration_checks::lookup(name).map(AnyArchitectureChecker::Declaration)
}

/// Builds the `ArchModel` an [`AnyArchitectureChecker::Model`] checker needs, reusing
/// `files` (already walked by the caller) instead of walking the repo a second time.
/// Thin wrapper over [`crate::arch_model::build_model_from_files`] — see that function for
/// the actual graph+read-files logic, shared with `arch_model::collect_repo_files`.
pub(crate) fn build_arch_model_for_check(
    repo_root: &Path,
    files: &[PathBuf],
) -> anyhow::Result<crate::arch_model::ArchModel> {
    crate::arch_model::build_model_from_files(
        repo_root,
        files,
        &crate::arch_model::PruneConfig::default(),
    )
}

/// Runs a whole-repo, in-process architecture/declaration checker resolved through
/// [`lookup_any_architecture_checker`] — the native counterpart to `run_check`'s
/// shell-out path for `WholeRepoNative` checks. `files` is expected to already be
/// walked/collected by the caller (batch mode builds it once and reuses it across every
/// whole-repo check, native or not). Signature is unchanged from before the dual-registry
/// dispatch was added (Epic 1.2) — every existing caller in `check.rs`/`mcp.rs` keeps
/// working without modification.
pub fn run_architecture_check(
    check: &Check,
    repo_root: &Path,
    files: &[PathBuf],
    arch_config: &crate::config::ArchitectureConfig,
) -> anyhow::Result<CheckResult> {
    let arch_name = check
        .architecture_checker
        .as_deref()
        .expect("config-load validation guarantees architecture_checker is set");

    let cmd_str = format!("kibitzer check architecture {arch_name}");
    let error_result = |output: String| CheckResult {
        check_name: check.name.clone(),
        severity: check.severity,
        passed: false,
        output,
        message: check.message.clone(),
        command: cmd_str.clone(),
        findings: Vec::new(),
        plugin_missing: false,
    };

    let Some(any_checker) = lookup_any_architecture_checker(arch_name) else {
        return Ok(error_result(format!(
            "no architecture checker named '{arch_name}' registered"
        )));
    };

    let findings = match any_checker {
        AnyArchitectureChecker::Import(checker) => {
            let graph = match crate::import_graph::build(repo_root, files) {
                Ok(graph) => graph,
                Err(err) => return Ok(error_result(format!("{err:#}"))),
            };
            checker.check(&graph, arch_config)
        }
        AnyArchitectureChecker::Model(checker) => {
            let model = match build_arch_model_for_check(repo_root, files) {
                Ok(model) => model,
                Err(err) => return Ok(error_result(format!("{err:#}"))),
            };
            checker.check(&model, arch_config)
        }
        AnyArchitectureChecker::Declaration(checker) => {
            let components = arch_config.effective_components();
            let graph = match crate::declarations::build(repo_root, files, &components) {
                Ok(graph) => graph,
                Err(err) => return Ok(error_result(format!("{err:#}"))),
            };
            checker.check(&graph, arch_config)
        }
    };

    let passed = findings.is_empty();
    let combined = findings
        .iter()
        .map(|f| match (&f.file, f.line) {
            (Some(file), Some(line)) => format!("{}:{}: {}", file.display(), line, f.message),
            (Some(file), None) => format!("{}: {}", file.display(), f.message),
            (None, _) => f.message.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut severity = check.severity;
    let mut message = check.message.clone();

    if !passed && severity == Severity::Blocking {
        let baseline = check_native_against_git_head_repo(arch_name, repo_root, arch_config);
        if let Some(false) = baseline {
            severity = Severity::Advisory;
            message = Some(format!(
                "{} (downgraded: this violation predates your edits — already present \
                 at the git HEAD commit)",
                message.unwrap_or_default()
            ));
        }
    }

    Ok(CheckResult {
        check_name: check.name.clone(),
        severity,
        passed,
        output: combined,
        message,
        command: cmd_str,
        findings,
        plugin_missing: false,
    })
}

/// Native-checker counterpart to [`check_against_git_head_repo`]: snapshots HEAD the same
/// way, but builds the import/declaration graph and runs the architecture/declaration
/// checker in-process against the snapshot instead of shelling out.
///
/// Story 2.2.3 (BLOCKER fix): dispatches through [`lookup_any_architecture_checker`] —
/// the same dual-registry resolution [`run_architecture_check`] uses — instead of only
/// ever searching [`crate::architecture_checks::lookup`]. Before this fix, a
/// Declaration-kind checker (`content-rules`/`naming-rules`) could never be found here,
/// so this function always returned `None` for them and the "predates your edits"
/// downgrade in [`run_architecture_check`] never fired for a blocking-severity
/// `content-rules`/`naming-rules` check.
fn check_native_against_git_head_repo(
    arch_name: &str,
    repo_root: &Path,
    arch_config: &crate::config::ArchitectureConfig,
) -> Option<bool> {
    let any_checker = lookup_any_architecture_checker(arch_name)?;

    let nonce = TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let snapshot_dir = std::env::temp_dir().join(format!(
        "kibitzer-arch-head-snapshot-{}-{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&snapshot_dir).ok()?;

    let archive = match Command::new("git")
        .args(["archive", "HEAD"])
        .current_dir(repo_root)
        .output()
    {
        Ok(archive) => archive,
        Err(_) => {
            let _ = std::fs::remove_dir_all(&snapshot_dir);
            return None;
        }
    };
    if !archive.status.success() {
        let _ = std::fs::remove_dir_all(&snapshot_dir);
        return None;
    }

    let mut tar = match Command::new("tar")
        .args(["-x", "-C"])
        .arg(&snapshot_dir)
        .stdin(Stdio::piped())
        .spawn()
    {
        Ok(tar) => tar,
        Err(_) => {
            let _ = std::fs::remove_dir_all(&snapshot_dir);
            return None;
        }
    };
    let write_ok = tar
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(&archive.stdout).is_ok())
        .unwrap_or(false);
    let wait_ok = tar.wait().map(|s| s.success()).unwrap_or(false);
    if !write_ok || !wait_ok {
        let _ = std::fs::remove_dir_all(&snapshot_dir);
        return None;
    }

    let files = walk_and_collect_files(&snapshot_dir).ok();
    let result = files.and_then(|files| match any_checker {
        AnyArchitectureChecker::Import(checker) => {
            crate::import_graph::build(&snapshot_dir, &files)
                .ok()
                .map(|graph| checker.check(&graph, arch_config).is_empty())
        }
        AnyArchitectureChecker::Model(checker) => build_arch_model_for_check(&snapshot_dir, &files)
            .ok()
            .map(|model| checker.check(&model, arch_config).is_empty()),
        AnyArchitectureChecker::Declaration(checker) => {
            let components = arch_config.effective_components();
            crate::declarations::build(&snapshot_dir, &files, &components)
                .ok()
                .map(|graph| checker.check(&graph, arch_config).is_empty())
        }
    });

    let _ = std::fs::remove_dir_all(&snapshot_dir);

    result
}

struct DiffHunk {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
}

fn parse_hunk_range(s: &str) -> Option<(usize, usize)> {
    match s.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// Parse `@@ -old_start,old_count +new_start,new_count @@` hunk headers out of unified diff
/// text (as produced by `git diff -U0`). Returns `None` if a header can't be parsed —
/// callers should treat that as "the diff isn't in the shape we expect, bail."
fn parse_diff_hunks(diff_text: &str) -> Option<Vec<DiffHunk>> {
    let mut hunks = Vec::new();
    for line in diff_text.lines() {
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let mut parts = rest.splitn(3, ' ');
        let old = parts.next()?.strip_prefix('-')?;
        let new = parts.next()?.strip_prefix('+')?;
        let (old_start, old_count) = parse_hunk_range(old)?;
        let (new_start, new_count) = parse_hunk_range(new)?;
        hunks.push(DiffHunk {
            old_start,
            old_count,
            new_start,
            new_count,
        });
    }
    Some(hunks)
}

/// Translate `ranges` (1-indexed inclusive line ranges in the *current* file) into the
/// corresponding ranges in the file's HEAD content, using the diff hunks between HEAD and
/// the current working-tree file. A range that falls inside an inserted/changed hunk maps
/// to that hunk's old-side span (the content it actually replaced) — or is dropped entirely
/// if the hunk is a pure insertion (`old_count == 0`), since HEAD has no corresponding lines
/// at all. A range in an untouched region is shifted by the cumulative line-count delta of
/// every hunk before it. Returns `None` if the diff can't be obtained or parsed.
fn map_ranges_to_head(
    repo_root: &Path,
    rel_path: &str,
    ranges: &[(usize, usize)],
) -> Option<Vec<(usize, usize)>> {
    let diff = Command::new("git")
        .args(["diff", "--no-color", "-U0", "HEAD", "--", rel_path])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !diff.status.success() {
        return None;
    }
    let hunks = parse_diff_hunks(&String::from_utf8_lossy(&diff.stdout))?;
    Some(map_ranges_through_hunks(ranges, &hunks))
}

fn map_ranges_through_hunks(ranges: &[(usize, usize)], hunks: &[DiffHunk]) -> Vec<(usize, usize)> {
    let mut mapped = Vec::with_capacity(ranges.len());
    for &(start, end) in ranges {
        let mut offset: isize = 0;
        let mut overlap = None;
        for h in hunks {
            let hunk_new_start = h.new_start;
            let hunk_new_end = if h.new_count == 0 {
                h.new_start
            } else {
                h.new_start + h.new_count - 1
            };
            if h.new_count > 0 && end >= hunk_new_start && start <= hunk_new_end {
                overlap = Some(if h.old_count == 0 {
                    None
                } else {
                    Some((h.old_start, h.old_start + h.old_count - 1))
                });
                break;
            }
            if start > hunk_new_end {
                offset += h.new_count as isize - h.old_count as isize;
                continue;
            }
            break;
        }
        match overlap {
            Some(Some(old_range)) => mapped.push(old_range),
            Some(None) => {} // pure insertion — nothing in HEAD to compare, drop this range
            None => {
                let s = (start as isize - offset).max(1) as usize;
                let e = (end as isize - offset).max(1) as usize;
                mapped.push((s, e));
            }
        }
    }
    mapped
}

/// Run every check in `checks` that applies to `trigger` and whose scope matches
/// `file_path` (given relative to `repo_root`).
///
/// `registry` is threaded straight through to [`run_check`] rather than reloaded here —
/// callers that process multiple files in one batch/request (`run_batch_collect` in
/// `run.rs`) should load it exactly once for the whole batch and pass the same
/// `&Registry` into every call, instead of reloading and reparsing `registry.json` from
/// disk once per call (or, if reloaded again inside the per-`check` loop, once per
/// (file, check) pair).
pub fn run_checks_for_trigger(
    checks: &[Check],
    trigger: &str,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    registry: &Registry,
) -> anyhow::Result<Vec<CheckResult>> {
    let rel_path = relativize(repo_root, file_path);
    let mut results = Vec::new();
    for check in checks {
        if !check.triggers.is_empty() && !check.triggers.iter().any(|t| t == trigger) {
            continue;
        }
        if !matches_scope(&rel_path, &check.scope) {
            continue;
        }
        results.push(run_check(
            check,
            repo_root,
            file_path,
            changed_lines,
            registry,
        )?);
    }
    Ok(results)
}

fn relativize(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Convenience for batch mode: walk `dir` and run checks against every file within it.
pub fn walk_and_collect_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in walk(dir)? {
        if entry.is_file() {
            files.push(entry);
        }
    }
    Ok(files)
}

/// Directories never worth descending into for batch-mode scans: VCS internals and
/// dependency/build trees that can contain vendored source (e.g. flatted's bundled
/// Go port under node_modules) which isn't code this repo owns.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "vendor",
    "target",
    "dist",
    "build",
    ".next",
    // Python virtualenv/bytecode-cache dirs (Epic 5.1).
    "__pycache__",
    ".venv",
    "venv",
    ".tox",
    // Java/Kotlin Gradle/Maven build dirs (Epic 5.2/5.3) — Kotlin/Gradle projects share
    // these same dirs with Java, so no Kotlin-specific additions are needed.
    ".gradle",
    ".mvn",
    "out",
];

fn walk(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| SKIP_DIRS.contains(&name))
        {
            continue;
        }
        if path.is_dir() {
            out.extend(walk(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod diff_scoping_tests {
    use super::*;
    use std::path::PathBuf;

    fn file() -> PathBuf {
        PathBuf::from("src/foo.go")
    }

    #[test]
    fn substitute_command_fills_changed_lines() {
        let cmd = substitute_command(
            "kibitzer check primitive-obsession {file} --lines={changed_lines}",
            &file(),
            Some(&[(12, 15), (40, 40)]),
        );
        assert_eq!(
            cmd,
            "kibitzer check primitive-obsession src/foo.go --lines=12-15,40-40"
        );
    }

    #[test]
    fn substitute_command_empty_changed_lines_when_none() {
        let cmd = substitute_command("cmd {file} {changed_lines}", &file(), None);
        assert_eq!(cmd, "cmd src/foo.go ");
    }

    #[test]
    fn scope_output_keeps_findings_inside_changed_ranges() {
        let output = "src/foo.go:5: unrelated finding\nsrc/foo.go:13: newtype me\n";
        let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
        assert!(!passed);
        assert_eq!(filtered, "src/foo.go:13: newtype me");
    }

    #[test]
    fn scope_output_passes_when_all_findings_outside_changed_ranges() {
        let output = "src/foo.go:5: pre-existing finding\n";
        let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
        assert!(passed);
        assert_eq!(filtered, "");
    }

    #[test]
    fn scope_output_leaves_unconventional_output_untouched() {
        let output = "some linter crashed with no file:line prefix\n";
        let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
        assert!(!passed);
        assert_eq!(filtered, output);
    }

    #[test]
    fn scope_output_noop_when_already_passing() {
        let (filtered, passed) = scope_output_to_changed_lines("", &file(), &[(12, 15)], true);
        assert!(passed);
        assert_eq!(filtered, "");
    }

    #[test]
    fn scope_output_empty_ranges_suppresses_all_findings() {
        // Regression for docs/go-primitive-obsession-false-positives.md's
        // "deletion-only edit flagged" entry: `changed_lines` present but empty
        // (a pure-deletion edit — see `hook::compute_changed_lines`) must suppress
        // every finding, not fall back to raw whole-file output.
        let output = "src/foo.go:5: pre-existing finding\n";
        let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[], false);
        assert!(passed);
        assert_eq!(filtered, "");
    }

    #[test]
    fn scope_output_counts_malformed_prefixed_line_as_failure() {
        // Has the file prefix but the text after it isn't a line number — can't be
        // attributed to a range, so it must count toward failure, not just be displayed.
        let output = "src/foo.go:note: continued from previous finding\n";
        let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
        assert!(!passed);
        assert_eq!(filtered, output);
    }
}

#[cfg(test)]
mod sarif_run_check_tests {
    use super::*;
    use crate::config::OutputFormat;

    fn sarif_check(sarif_json: &str) -> Check {
        Check {
            name: "sarif-linter".to_string(),
            command: Some(format!("cat <<'EOF'\n{sarif_json}\nEOF")),
            checker: None,
            architecture_checker: None,
            severity: Severity::Advisory,
            scope: vec![],
            triggers: vec![],
            message: Some("linter found issues".to_string()),
            output_format: Some(OutputFormat::Sarif),
        }
    }

    #[test]
    fn renders_sarif_output_and_ignores_diff_scoping() {
        let sarif_json = r#"{"version": "2.1.0", "runs": [{"results": [
            {"level": "error", "ruleId": "no-foo", "message": {"text": "found a foo"},
             "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/lib.rs"},
             "region": {"startLine": 12}}}]}
        ]}]}"#;
        let result = run_check(
            &sarif_check(sarif_json),
            Path::new("."),
            Path::new("src/lib.rs"),
            Some(&[(1, 5)]),
            &Registry::default(),
        )
        .unwrap();
        assert_eq!(
            result.output,
            "1 error(s)\nsrc/lib.rs:12: [error] found a foo (no-foo)"
        );
    }

    #[test]
    fn falls_back_to_raw_output_when_stdout_is_not_sarif() {
        let check = Check {
            command: Some("echo 'not sarif at all'".to_string()),
            ..sarif_check("{}")
        };
        let result = run_check(
            &check,
            Path::new("."),
            Path::new("src/lib.rs"),
            None,
            &Registry::default(),
        )
        .unwrap();
        assert_eq!(result.output.trim(), "not sarif at all");
    }
}

/// Epic 4.2 (Tech Debt item a): a hung `command` check must be killed and reported as a
/// timeout instead of blocking `run_check` forever, while a normal fast-exiting command is
/// completely unaffected. Exercises [`run_check_with_timeout`] directly with a short
/// duration so the hang test doesn't actually wait out a real-world timeout.
#[cfg(test)]
mod timeout_tests {
    use super::*;
    use crate::config::Severity;
    use std::time::Instant;

    fn command_check(command: &str) -> Check {
        Check {
            name: "hangy".to_string(),
            command: Some(command.to_string()),
            checker: None,
            architecture_checker: None,
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("command check".to_string()),
            output_format: None,
        }
    }

    #[test]
    fn run_check_reports_timeout_and_kills_process_when_command_hangs() {
        let started = Instant::now();
        let result = run_check_with_timeout(
            &command_check("sleep 60"),
            Path::new("."),
            Path::new("irrelevant.txt"),
            None,
            Duration::from_millis(200),
            &Registry::default(),
        )
        .unwrap();

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "run_check_with_timeout should return in well under 1s, took {:?}",
            started.elapsed()
        );
        assert!(!result.passed);
        assert!(
            result
                .message
                .as_deref()
                .unwrap_or("")
                .contains("timed out")
                || result.output.contains("timed out"),
            "expected 'timed out' in message or output, got message={:?} output={:?}",
            result.message,
            result.output
        );
    }

    #[test]
    fn run_check_behaves_unchanged_when_command_exits_quickly() {
        let result = run_check_with_timeout(
            &command_check("true"),
            Path::new("."),
            Path::new("irrelevant.txt"),
            None,
            Duration::from_millis(200),
            &Registry::default(),
        )
        .unwrap();

        assert!(result.passed);
        assert!(!result.output.to_lowercase().contains("timed out"));
    }
}

/// Task 4.3.1b: a plugin-backed check whose registered binary is missing from disk must
/// short-circuit with `plugin_missing: true` and a forced `Advisory` severity — regardless
/// of the check's own configured severity — without ever spawning the command.
#[cfg(test)]
mod plugin_missing_tests {
    use super::*;
    use crate::config::{OutputFormat, Severity};
    use crate::plugin::test_support::with_xdg_data_home;
    use crate::plugin::{InstalledPlugin, PluginName, Registry};
    use std::time::Instant;

    #[test]
    fn run_check_returns_plugin_missing_result_with_advisory_severity_when_binary_absent() {
        with_xdg_data_home("check-plugin-missing", |dir| {
            let missing_binary = dir.join("kibitzer-check-plugin-missing-test-nonexistent-binary");
            let plugin = InstalledPlugin {
                name: PluginName::parse("kibitzer-stub-plugin").unwrap(),
                version: "0.1.0".to_string(),
                min_kibitzer_version: "0.1.0".to_string(),
                sha256: "deadbeef".to_string(),
                binary_path: missing_binary,
                severity: Severity::Advisory,
                scope: vec!["**/*".to_string()],
                triggers: vec!["batch".to_string()],
                output_format: OutputFormat::Sarif,
            };
            Registry::save(
                &crate::plugin::default_registry_path(),
                &Registry {
                    plugins: vec![plugin],
                },
            )
            .unwrap();

            // Configured `Blocking`, even though a real plugin manifest would normally
            // set its own severity — proves the forced-Advisory downgrade happens
            // regardless of what the `Check` itself is configured with.
            let check = Check {
                name: "kibitzer-stub-plugin".to_string(),
                command: Some("echo should-never-run {file}".to_string()),
                checker: None,
                architecture_checker: None,
                severity: Severity::Blocking,
                scope: vec![],
                triggers: vec![],
                message: None,
                output_format: None,
            };

            let registry = Registry::load(&crate::plugin::default_registry_path());
            let started = Instant::now();
            let result = run_check(
                &check,
                Path::new("."),
                Path::new("irrelevant.txt"),
                None,
                &registry,
            )
            .unwrap();

            assert!(
                started.elapsed() < Duration::from_secs(1),
                "should return near-instantly without spawning a process"
            );
            assert!(result.plugin_missing);
            assert!(!result.passed);
            assert_eq!(result.severity, Severity::Advisory);
            assert!(result.output.is_empty(), "no subprocess should have run");
            assert!(
                result.command.is_empty(),
                "short-circuit result carries no substituted command"
            );
            assert!(
                result
                    .message
                    .as_deref()
                    .unwrap_or("")
                    .contains("is not installed"),
                "message: {:?}",
                result.message
            );
        });
    }
}

#[cfg(test)]
mod head_mapping_tests {
    use super::*;

    fn hunk(old_start: usize, old_count: usize, new_start: usize, new_count: usize) -> DiffHunk {
        DiffHunk {
            old_start,
            old_count,
            new_start,
            new_count,
        }
    }

    #[test]
    fn parses_unified_diff_hunk_headers() {
        let diff = "diff --git a/foo.go b/foo.go\n\
                     --- a/foo.go\n\
                     +++ b/foo.go\n\
                     @@ -10,2 +10,5 @@ func Foo() {\n\
                     -old line\n\
                     +new line 1\n";
        let hunks = parse_diff_hunks(diff).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 10);
        assert_eq!(hunks[0].old_count, 2);
        assert_eq!(hunks[0].new_start, 10);
        assert_eq!(hunks[0].new_count, 5);
    }

    #[test]
    fn range_before_any_hunk_is_unshifted() {
        let hunks = vec![hunk(20, 1, 20, 6)];
        let mapped = map_ranges_through_hunks(&[(1, 3)], &hunks);
        assert_eq!(mapped, vec![(1, 3)]);
    }

    #[test]
    fn range_after_a_growing_hunk_is_shifted_back() {
        // A 1-line -> 6-line edit at old line 20 pushes everything after it down by 5 in
        // the current file. A changed_lines range of (30, 30) in current-file coordinates
        // must map back to (25, 25) in HEAD.
        let hunks = vec![hunk(20, 1, 20, 6)];
        let mapped = map_ranges_through_hunks(&[(30, 30)], &hunks);
        assert_eq!(mapped, vec![(25, 25)]);
    }

    #[test]
    fn range_inside_the_edited_hunk_maps_to_its_old_span() {
        let hunks = vec![hunk(20, 1, 20, 6)];
        let mapped = map_ranges_through_hunks(&[(21, 23)], &hunks);
        assert_eq!(mapped, vec![(20, 20)]);
    }

    #[test]
    fn range_inside_a_pure_insertion_has_no_head_counterpart() {
        let hunks = vec![hunk(20, 0, 21, 4)];
        let mapped = map_ranges_through_hunks(&[(21, 24)], &hunks);
        assert!(mapped.is_empty());
    }
}

#[cfg(test)]
mod git_head_integration_tests {
    use super::*;
    use crate::config::Severity;

    struct TempRepo {
        dir: PathBuf,
    }

    impl TempRepo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "kibitzer-check-test-{}-{name}-{}",
                std::process::id(),
                TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            run(&dir, &["init", "-q"]);
            run(&dir, &["config", "user.email", "test@example.com"]);
            run(&dir, &["config", "user.name", "test"]);
            Self { dir }
        }

        fn write_and_commit(&self, rel_path: &str, content: &str, msg: &str) {
            let path = self.dir.join(rel_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
            run(&self.dir, &["add", rel_path]);
            run(&self.dir, &["commit", "-q", "-m", msg]);
        }

        fn write_uncommitted(&self, rel_path: &str, content: &str) {
            std::fs::write(self.dir.join(rel_path), content).unwrap();
        }

        fn path(&self, rel_path: &str) -> PathBuf {
            self.dir.join(rel_path)
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git available");
        assert!(status.success(), "git {args:?} failed");
    }

    /// A "check" that fails whenever the file contains the string "BAD".
    fn bad_marker_check() -> Check {
        Check {
            name: "no-bad-marker".to_string(),
            command: Some("! grep -n BAD {file}".to_string()),
            checker: None,
            architecture_checker: None,
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("found BAD marker".to_string()),
            output_format: None,
        }
    }

    /// A whole-repo counterpart to `bad_marker_check`: no `{file}` in the command, so it
    /// scans the whole tree it's run from rather than a single file.
    fn repo_wide_bad_marker_check() -> Check {
        Check {
            name: "no-bad-marker-repo".to_string(),
            command: Some("! grep -rn BAD .".to_string()),
            checker: None,
            architecture_checker: None,
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("found BAD marker in repo".to_string()),
            output_format: None,
        }
    }

    #[test]
    fn baseline_passes_when_violation_is_genuinely_new() {
        let repo = TempRepo::new("genuinely-new");
        repo.write_and_commit("foo.txt", "line1\nline2\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

        let result = check_against_git_head(
            &bad_marker_check(),
            &repo.dir,
            &repo.path("foo.txt"),
            Some(&[(2, 2)]),
        );
        assert_eq!(result, Some(true));
    }

    #[test]
    fn baseline_fails_when_violation_predates_the_edit() {
        let repo = TempRepo::new("pre-existing");
        repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

        let result = check_against_git_head(
            &bad_marker_check(),
            &repo.dir,
            &repo.path("foo.txt"),
            Some(&[(2, 2)]),
        );
        assert_eq!(result, Some(false));
    }

    #[test]
    fn baseline_ignores_unrelated_violation_shifted_by_earlier_insertion() {
        // HEAD has a BAD marker at line 2. The current file inserts 3 new lines before
        // it (pushing it to line 5) and introduces a brand-new BAD marker at line 6 via
        // the edit under test. Without line-shift mapping, scoping the baseline to
        // current-file lines (6,6) would land on HEAD's line 6 (out of range / not the
        // marker), or — depending on direction of the bug — could accidentally line up
        // with the pre-existing marker. This asserts the new marker is correctly reported
        // as genuinely new despite the unrelated shifted violation elsewhere in the file.
        let repo = TempRepo::new("shifted");
        repo.write_and_commit("foo.txt", "line1\nBAD\nline3\nline4\n", "init");
        repo.write_uncommitted(
            "foo.txt",
            "line1\ninserted1\ninserted2\ninserted3\nBAD\nline3\nBAD-new\nline4\n",
        );

        let result = check_against_git_head(
            &bad_marker_check(),
            &repo.dir,
            &repo.path("foo.txt"),
            Some(&[(7, 7)]),
        );
        assert_eq!(result, Some(true));
    }

    #[test]
    fn baseline_is_none_when_there_is_no_head_commit() {
        let repo = TempRepo::new("no-head");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

        let result = check_against_git_head(
            &bad_marker_check(),
            &repo.dir,
            &repo.path("foo.txt"),
            Some(&[(2, 2)]),
        );
        assert_eq!(result, None);
    }

    #[test]
    fn baseline_is_none_when_the_file_is_untracked_at_head() {
        let repo = TempRepo::new("untracked-file");
        repo.write_and_commit("committed.txt", "line1\n", "init");
        repo.write_uncommitted("new.txt", "line1\nBAD\n");

        let result = check_against_git_head(
            &bad_marker_check(),
            &repo.dir,
            &repo.path("new.txt"),
            Some(&[(2, 2)]),
        );
        assert_eq!(result, None);
    }

    #[test]
    fn run_check_downgrades_severity_when_violation_predates_edit() {
        let repo = TempRepo::new("run-check-downgrade");
        repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

        let result = run_check(
            &bad_marker_check(),
            &repo.dir,
            &repo.path("foo.txt"),
            Some(&[(2, 2)]),
            &Registry::default(),
        )
        .unwrap();
        assert!(!result.passed);
        assert_eq!(result.severity, Severity::Advisory);
        assert!(result.message.unwrap().contains("predates your edits"));
    }

    #[test]
    fn repo_wide_baseline_fails_when_violation_predates_the_edit() {
        let repo = TempRepo::new("repo-wide-pre-existing");
        repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

        let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &repo.dir);
        assert_eq!(result, Some(false));
    }

    #[test]
    fn repo_wide_baseline_passes_when_violation_is_genuinely_new() {
        let repo = TempRepo::new("repo-wide-genuinely-new");
        repo.write_and_commit("foo.txt", "line1\nline2\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

        let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &repo.dir);
        assert_eq!(result, Some(true));
    }

    #[test]
    fn repo_wide_baseline_is_none_when_there_is_no_head_commit() {
        let repo = TempRepo::new("repo-wide-no-head");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3\n");

        let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &repo.dir);
        assert_eq!(result, None);
    }

    #[test]
    fn repo_wide_baseline_concurrent_calls_do_not_interfere() {
        let repo = TempRepo::new("repo-wide-concurrent");
        repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let dir = repo.dir.clone();
                std::thread::spawn(move || {
                    check_against_git_head_repo(&repo_wide_bad_marker_check(), &dir)
                })
            })
            .collect();

        for handle in handles {
            assert_eq!(handle.join().unwrap(), Some(false));
        }
    }

    #[test]
    fn repo_wide_baseline_cleans_up_snapshot_dir_when_archive_fails_to_spawn() {
        let pid = std::process::id();
        let prefix = format!("kibitzer-head-snapshot-{pid}-");
        let list_matching = || -> std::collections::HashSet<PathBuf> {
            std::fs::read_dir(std::env::temp_dir())
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with(&prefix))
                        .unwrap_or(false)
                })
                .collect()
        };

        let before = list_matching();
        // A nonexistent repo_root makes `Command::current_dir` fail at the OS level before
        // `git` even execs, exercising the same "archive command fails to spawn" path a
        // missing `git` binary would hit.
        let missing_repo_root = std::env::temp_dir().join(format!("kibitzer-does-not-exist-{pid}"));
        let result = check_against_git_head_repo(&repo_wide_bad_marker_check(), &missing_repo_root);
        assert_eq!(result, None);

        let after = list_matching();
        assert!(
            after.is_subset(&before),
            "check_against_git_head_repo leaked a snapshot dir on spawn failure: {:?}",
            after.difference(&before).collect::<Vec<_>>()
        );
    }

    #[test]
    fn run_check_downgrades_severity_for_repo_wide_check_when_violation_predates_edit() {
        let repo = TempRepo::new("run-check-repo-wide-downgrade");
        repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

        let result = run_check(
            &repo_wide_bad_marker_check(),
            &repo.dir,
            &repo.dir,
            None,
            &Registry::default(),
        )
        .unwrap();
        assert!(!result.passed);
        assert_eq!(result.severity, Severity::Advisory);
        assert!(result.message.unwrap().contains("predates your edits"));
    }

    // --- Story 2.2.3: git-HEAD-baseline downgrade for Declaration-kind checkers
    // (BLOCKER fix) ---

    fn content_rules_check() -> Check {
        Check {
            name: "content-rules".to_string(),
            command: None,
            checker: None,
            architecture_checker: Some("content-rules".to_string()),
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("content rule violation".to_string()),
            output_format: None,
        }
    }

    fn domain_content_rules_arch_config() -> crate::config::ArchitectureConfig {
        crate::config::ArchitectureConfig {
            components: vec![crate::config::Component {
                name: "domain".to_string(),
                paths: vec!["**/domain".to_string(), "**/domain/**".to_string()],
            }],
            content_rules: vec![crate::config::ContentRule {
                component: "domain".to_string(),
                allowed_kinds: vec!["struct".to_string()],
            }],
            ..Default::default()
        }
    }

    #[test]
    fn content_rules_blocking_violation_downgrades_when_it_predates_head() {
        let repo = TempRepo::new("content-rules-predates-head");
        repo.write_and_commit(
            "domain/domain.go",
            "package domain\n\ntype Order struct {\n\tID string\n}\n\n\
             func Validate(o Order) error { return nil }\n",
            "init",
        );
        // Unrelated uncommitted edit — the content-rules violation itself is
        // untouched by it, i.e. it predates this "current edit."
        repo.write_uncommitted("README.md", "unrelated edit\n");

        let arch_config = domain_content_rules_arch_config();
        let files = walk_and_collect_files(&repo.dir).unwrap();

        let result =
            run_architecture_check(&content_rules_check(), &repo.dir, &files, &arch_config)
                .unwrap();

        assert!(!result.passed);
        assert_eq!(result.severity, Severity::Advisory);
        assert!(result.message.unwrap().contains("predates your edits"));
    }

    #[test]
    fn content_rules_blocking_violation_stays_blocking_when_new_since_head() {
        let repo = TempRepo::new("content-rules-new-since-head");
        repo.write_and_commit(
            "domain/domain.go",
            "package domain\n\ntype Order struct {\n\tID string\n}\n",
            "init",
        );
        // Uncommitted edit introduces the violating function — absent at HEAD.
        repo.write_uncommitted(
            "domain/domain.go",
            "package domain\n\ntype Order struct {\n\tID string\n}\n\n\
             func Validate(o Order) error { return nil }\n",
        );

        let arch_config = domain_content_rules_arch_config();
        let files = walk_and_collect_files(&repo.dir).unwrap();

        let result =
            run_architecture_check(&content_rules_check(), &repo.dir, &files, &arch_config)
                .unwrap();

        assert!(!result.passed);
        assert_eq!(result.severity, Severity::Blocking);
        assert!(!result.message.unwrap().contains("predates your edits"));
    }

    fn naming_rules_check() -> Check {
        Check {
            name: "naming-rules".to_string(),
            command: None,
            checker: None,
            architecture_checker: Some("naming-rules".to_string()),
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("naming rule violation".to_string()),
            output_format: None,
        }
    }

    #[test]
    fn naming_rules_blocking_violation_downgrades_when_it_predates_head() {
        let repo = TempRepo::new("naming-rules-predates-head");
        repo.write_and_commit(
            "infra/infra.go",
            "package infra\n\ntype OrderStore struct {\n\tID string\n}\n",
            "init",
        );
        repo.write_uncommitted("README.md", "unrelated edit\n");

        let arch_config = crate::config::ArchitectureConfig {
            components: vec![crate::config::Component {
                name: "infra".to_string(),
                paths: vec!["**/infra".to_string(), "**/infra/**".to_string()],
            }],
            naming_rules: vec![crate::config::NamingRule {
                component: "infra".to_string(),
                kind: "struct".to_string(),
                pattern: ".*Repository$|.*Client$".to_string(),
            }],
            ..Default::default()
        };
        let files = walk_and_collect_files(&repo.dir).unwrap();

        let result =
            run_architecture_check(&naming_rules_check(), &repo.dir, &files, &arch_config).unwrap();

        assert!(!result.passed);
        assert_eq!(result.severity, Severity::Advisory);
        assert!(result.message.unwrap().contains("predates your edits"));
    }

    fn instability_check() -> Check {
        Check {
            name: "instability".to_string(),
            command: None,
            checker: None,
            architecture_checker: Some("instability".to_string()),
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("instability violation".to_string()),
            output_format: None,
        }
    }

    /// Regression guard for the `AnyArchitectureChecker::Model` dispatch arm added
    /// alongside `InstabilityChecker`/`DipConcreteCouplingChecker`: proves
    /// `run_architecture_check` actually builds an `ArchModel` and reaches the checker
    /// (not just that the checker's own unit tests pass against a hand-built model), and
    /// — via the blocking-severity downgrade path — that `check_native_against_git_head_repo`
    /// resolves the `Model` arm too, matching the coverage the `Import`/`Declaration` arms
    /// already have (`naming_rules_blocking_violation_downgrades_when_it_predates_head`
    /// above).
    #[test]
    fn instability_model_checker_blocking_violation_downgrades_when_it_predates_head() {
        let repo = TempRepo::new("instability-predates-head");
        repo.write_and_commit(
            "go.mod",
            "module kibitzer.example/instabilitytest\n\ngo 1.21\n",
            "init",
        );
        repo.write_and_commit(
            "stable/stable.go",
            "package stable\n\ntype Widget struct{}\n",
            "init",
        );
        repo.write_and_commit(
            "consumer/consumer.go",
            "package consumer\n\nimport \"kibitzer.example/instabilitytest/stable\"\n\n\
             var _ = stable.Widget{}\n",
            "init",
        );
        repo.write_uncommitted("README.md", "unrelated edit\n");

        let files = walk_and_collect_files(&repo.dir).unwrap();
        let result = run_architecture_check(
            &instability_check(),
            &repo.dir,
            &files,
            &crate::config::ArchitectureConfig::default(),
        )
        .unwrap();

        assert!(!result.passed);
        assert!(result.output.contains("[instability]"));
        assert_eq!(result.severity, Severity::Advisory);
        assert!(result.message.unwrap().contains("predates your edits"));
    }

    fn layering_check() -> Check {
        Check {
            name: "layering".to_string(),
            command: None,
            checker: None,
            architecture_checker: Some("layering".to_string()),
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("layering violation".to_string()),
            output_format: None,
        }
    }

    // Regression guard: `check_native_against_git_head_repo` had zero prior test
    // coverage (verified — no existing test in this module names `layering`,
    // `import-cycles`, or `coupling`), so Task 2.2.3a/b's rewrite (dispatching through
    // `lookup_any_architecture_checker` and branching on the enum) needs its own new
    // test proving the Import-kind path still behaves exactly as before.
    #[test]
    fn import_kind_checker_head_baseline_downgrade_still_works_through_dual_registry_dispatch() {
        let repo = TempRepo::new("import-kind-regression");
        repo.write_and_commit("go.mod", "module fixture\ngo 1.21\n", "init");
        repo.write_and_commit(
            "handlers/handlers.go",
            "package handlers\n\nfunc Do() {}\n",
            "add handlers",
        );
        repo.write_and_commit(
            "domain/domain.go",
            "package domain\n\nimport \"fixture/handlers\"\n\nfunc Do() { handlers.Do() }\n",
            "add domain violating layering",
        );
        repo.write_uncommitted("README.md", "unrelated edit\n");

        let arch_config = crate::config::ArchitectureConfig {
            layers: vec!["handlers".to_string(), "domain".to_string()],
            ..Default::default()
        };
        let files = walk_and_collect_files(&repo.dir).unwrap();

        let result =
            run_architecture_check(&layering_check(), &repo.dir, &files, &arch_config).unwrap();

        assert!(!result.passed);
        assert_eq!(result.severity, Severity::Advisory);
        assert!(result.message.unwrap().contains("predates your edits"));
    }
}

#[cfg(test)]
mod native_check_tests {
    use super::*;
    use crate::config::Severity;

    fn primitive_obsession_check() -> Check {
        Check {
            name: "native".to_string(),
            command: None,
            checker: Some("primitive-obsession".to_string()),
            architecture_checker: None,
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("primitive obsession".to_string()),
            output_format: None,
        }
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-native-check-test-{}-{name}-{}",
            std::process::id(),
            TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_degrades_to_failed_result_instead_of_erroring() {
        let dir = tmp_dir("missing-file");
        let file = dir.join("does-not-exist.go");

        let result = run_check(
            &primitive_obsession_check(),
            &dir,
            &file,
            None,
            &Registry::default(),
        )
        .expect("a missing file must not abort the whole check run");
        assert!(!result.passed);
        assert!(result.output.contains("does-not-exist.go"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_not_matching_checkers_globs_is_skipped_rather_than_misparsed() {
        let dir = tmp_dir("wrong-glob");
        let file = dir.join("notes.md");
        std::fs::write(&file, "func f(a, b string) {}\n").unwrap();

        let result = run_check(
            &primitive_obsession_check(),
            &dir,
            &file,
            None,
            &Registry::default(),
        )
        .unwrap();
        assert!(result.passed);
        assert_eq!(result.output, "");

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn blank_imports_check() -> Check {
        Check {
            name: "native".to_string(),
            command: None,
            checker: Some("go-blank-imports".to_string()),
            architecture_checker: None,
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("blank import".to_string()),
            output_format: None,
        }
    }

    #[test]
    fn oversized_file_is_skipped_rather_than_parsed() {
        let dir = tmp_dir("oversized");
        let file = dir.join("huge.go");
        let mut content = String::from("package main\n\nimport (\n\t_ \"unjustified/pkg\"\n)\n");
        content.push_str(&"// padding\n".repeat(300_000)); // ~3.3MB, over MAX_NATIVE_CHECK_BYTES
        std::fs::write(&file, &content).unwrap();

        let result = run_check(
            &blank_imports_check(),
            &dir,
            &file,
            None,
            &Registry::default(),
        )
        .unwrap();
        assert!(
            result.passed,
            "oversized file should be skipped rather than flagged"
        );
        assert!(result.output.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // A `WholeRepoNative` (`architecture_checker`-set) `Check` has neither `checker`
    // nor `command`, so it must be short-circuited before the `command`-only branch's
    // `.expect()` — reached whenever a per-file trigger dispatch doesn't pre-filter by
    // `is_per_file()` the way `run.rs::run_batch` does.
    #[test]
    fn whole_repo_native_check_dispatched_per_file_passes_trivially_instead_of_panicking() {
        let dir = tmp_dir("whole-repo-native-per-file");
        let file = dir.join("some-file.go");
        std::fs::write(&file, "package main\n").unwrap();

        let check = Check {
            name: "package-size".to_string(),
            command: None,
            checker: None,
            architecture_checker: Some("package-size".to_string()),
            severity: Severity::Advisory,
            scope: vec![],
            triggers: vec![],
            message: None,
            output_format: None,
        };

        let result = run_check(&check, &dir, &file, None, &Registry::default()).unwrap();
        assert!(result.passed);
        assert!(result.output.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Proves criterion 6 concretely for a new native checker (not just
    // primitive-obsession): two unjustified blank imports, one inside
    // `changed_lines` and one outside it, and only the in-range one survives
    // output-filtering and drives the pass/fail result.
    #[test]
    fn native_go_blank_imports_check_scopes_output_to_changed_lines() {
        let dir = tmp_dir("blank-imports-scoping");
        let file = dir.join("main.go");
        std::fs::write(
            &file,
            "package main\n\nimport (\n\t_ \"unjustified/outside\"\n\t_ \"unjustified/inside\"\n)\n",
        )
        .unwrap();

        // Line 4 (outside/pre-existing) is excluded; line 5 (inside) is the
        // only changed line, matching this file's `{file}:{line}:` findings.
        let registry = Registry::default();
        let result = run_check(
            &blank_imports_check(),
            &dir,
            &file,
            Some(&[(5, 5)]),
            &registry,
        )
        .unwrap();
        assert!(!result.passed);
        assert!(result.output.contains("unjustified/inside"));
        assert!(!result.output.contains("unjustified/outside"));

        // Scoping to a range with no findings at all reports a pass, proving the
        // filtering — not just the checker itself — determines the outcome.
        let clean = run_check(
            &blank_imports_check(),
            &dir,
            &file,
            Some(&[(1, 1)]),
            &registry,
        )
        .unwrap();
        assert!(clean.passed);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Regression for docs/go-primitive-obsession-false-positives.md's "pre-existing
    // unchanged signatures flagged" entry: `changed_lines` scoping (already generic
    // across every native checker via `run_native_check`) must exclude a
    // primitive-obsession finding whose signature sits outside the edited range,
    // even though `check_file` itself still parses and flags the whole file.
    #[test]
    fn native_primitive_obsession_check_scopes_output_to_changed_lines() {
        let dir = tmp_dir("primitive-obsession-scoping");
        let file = dir.join("tls.go");
        std::fs::write(
            &file,
            "package main\n\nfunc certCurrent(certFile, hashFile, want string) bool {\n\treturn true\n}\n\nfunc LoadTLSConfig(certFile, keyFile string) (int, error) {\n\treturn 0, nil\n}\n",
        )
        .unwrap();

        // Only line 7 (`LoadTLSConfig`) falls inside the edited range; line 3
        // (`certCurrent`) is pre-existing and untouched.
        let registry = Registry::default();
        let result = run_check(
            &primitive_obsession_check(),
            &dir,
            &file,
            Some(&[(7, 7)]),
            &registry,
        )
        .unwrap();
        assert!(!result.passed);
        assert!(result.output.contains("LoadTLSConfig") || result.output.contains(":7:"));
        assert!(!result.output.contains("certCurrent"));

        // Scoping to a range that touches neither flagged signature reports a pass.
        let clean = run_check(
            &primitive_obsession_check(),
            &dir,
            &file,
            Some(&[(4, 4)]),
            &registry,
        )
        .unwrap();
        assert!(clean.passed);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // Regression for docs/go-primitive-obsession-false-positives.md's "deletion-only
    // edit flagged" entry: a pure-deletion edit (`hook::compute_changed_lines` now
    // returns `Some(vec![])` for one, instead of `None`/unscoped) must not re-surface
    // an unrelated, pre-existing flaggable signature still left in the file.
    #[test]
    fn native_primitive_obsession_check_suppresses_findings_for_deletion_only_edit() {
        let dir = tmp_dir("primitive-obsession-deletion-only");
        let file = dir.join("tls.go");
        // What's left in the file after a hypothetical deletion of dead code —
        // `certCurrent` was already here, untouched by the edit.
        std::fs::write(
            &file,
            "package main\n\nfunc certCurrent(certFile, hashFile, want string) bool {\n\treturn true\n}\n",
        )
        .unwrap();

        let registry = Registry::default();
        let result = run_check(
            &primitive_obsession_check(),
            &dir,
            &file,
            Some(&[]),
            &registry,
        )
        .unwrap();
        assert!(result.passed, "output: {}", result.output);
        assert_eq!(result.output, "");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Closes the gap this fix is for: `ArchFinding.severity_override` was set correctly by
/// every checker (verified by architecture_checks.rs/declaration_checks.rs's own unit
/// tests) but was never threaded past `run_architecture_check` into `CheckResult` — so
/// `mcp.rs`/`main.rs` had no way to render a finding's own effective severity, only the
/// one `CheckResult.severity` flattened uniformly across every finding a `Check` produced.
/// These tests prove `CheckResult.findings` now carries that per-finding data end to end.
#[cfg(test)]
mod findings_wiring_tests {
    use super::*;
    use crate::config::{ArchitectureConfig, Component, Severity};

    fn component_deps_check(severity: Severity) -> Check {
        Check {
            name: "component-deps".to_string(),
            command: None,
            checker: None,
            architecture_checker: Some("component-deps".to_string()),
            severity,
            scope: vec![],
            triggers: vec![],
            message: None,
            output_format: None,
        }
    }

    /// A single Go package with no imports at all: `ComponentDependencyChecker` has no
    /// edges to flag as a real violation, but the declared "ghost" component's glob
    /// matches zero import-graph nodes, so the checker's own
    /// `zero_match_advisory`-produced `ArchFinding` (`severity_override:
    /// Some(Severity::Advisory)`) is the *only* finding produced.
    fn write_zero_match_only_fixture(dir: &Path) {
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(dir.join("pkg/pkg.go"), "package pkg\n\nfunc F() {}\n").unwrap();
    }

    #[test]
    fn component_deps_flags_zero_match_component_as_advisory_even_when_check_is_blocking() {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-findings-wiring-test-{}-{}",
            std::process::id(),
            TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        write_zero_match_only_fixture(&dir);

        let arch_config = ArchitectureConfig {
            components: vec![Component {
                name: "ghost".to_string(),
                paths: vec!["**/ghost".to_string(), "**/ghost/**".to_string()],
            }],
            ..Default::default()
        };
        let files = walk_and_collect_files(&dir).unwrap();

        // Check.severity is Blocking — the exact configuration that, before this fix,
        // made a harmless zero-match advisory masquerade as a blocking finding once it
        // reached rendered output (the bug this fix closes).
        let result = run_architecture_check(
            &component_deps_check(Severity::Blocking),
            &dir,
            &files,
            &arch_config,
        )
        .unwrap();

        let _ = std::fs::remove_dir_all(&dir);

        assert!(!result.passed);
        // The single finding is the zero-match advisory, carrying its own narrowed
        // severity, independent of `result.severity` (which stays whatever the
        // uniform/flattened `CheckResult.severity` resolves to).
        assert_eq!(result.findings.len(), 1, "findings: {:?}", result.findings);
        assert_eq!(
            result.findings[0].severity_override,
            Some(Severity::Advisory)
        );
        assert!(result.findings[0].message.contains("[component]"));
        assert!(result.findings[0].message.contains("matched 0"));
    }

    /// Same additive-field precedent as `command` (`src/check.rs:26-32` at the time this
    /// was written): a `cache.json` blob written before `findings` existed must still
    /// deserialize — as an empty `Vec` — instead of `Cache::load` discarding the whole
    /// cache on the first run after upgrade.
    #[test]
    fn cache_json_without_findings_field_deserializes_with_empty_findings() {
        let old_shape_json = r#"{
            "check_name": "component-deps",
            "severity": "blocking",
            "passed": false,
            "output": "some finding",
            "message": null,
            "command": "kibitzer check architecture component-deps"
        }"#;
        let result: CheckResult = serde_json::from_str(old_shape_json)
            .expect("a pre-`findings`-field CheckResult must still deserialize");
        assert!(result.findings.is_empty());
        assert_eq!(result.check_name, "component-deps");
    }
}
