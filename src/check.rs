use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::checker::{CheckContext, GrammarCache};
use crate::config::{Check, OutputFormat, Severity};
use crate::glob::matches_scope;

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
/// 1-indexed inclusive line ranges — see [`scope_output_to_changed_lines`].
pub fn run_check(
    check: &Check,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> anyhow::Result<CheckResult> {
    if let Some(checker_name) = &check.checker {
        return run_native_check(check, checker_name, repo_root, file_path, changed_lines);
    }

    let command = check
        .command
        .as_deref()
        .expect("config-load validation guarantees command is set when checker is not");

    let cmd_str = substitute_command(command, file_path, changed_lines);
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .current_dir(repo_root)
        .output()?;

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
    })
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
    let checker = crate::checker::lookup(checker_name)
        .ok_or_else(|| anyhow::anyhow!("no checker named '{checker_name}' registered"))?;
    let cache = GrammarCache::new();
    let tree = match checker.language() {
        Some(language) => Some(cache.parse(language, source)?),
        None => None,
    };
    let ctx = CheckContext {
        source,
        tree: tree.as_ref(),
    };
    let findings = checker.check(file_path, &ctx)?;
    let passed = findings.is_empty();
    let combined = findings
        .iter()
        .map(|f| format!("{}:{}: {}", file_path.display(), f.line, f.message))
        .collect::<Vec<_>>()
        .join("\n");
    Ok((combined, passed))
}

fn run_checker_against_file(
    checker_name: &str,
    file_path: &Path,
) -> anyhow::Result<(String, bool)> {
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
    if passed_raw || ranges.is_empty() {
        return (output.to_string(), passed_raw);
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

/// Runs a whole-repo, in-process [`crate::architecture_checks::ArchitectureChecker`]
/// against the import graph built from `files` — the native counterpart to
/// `run_check`'s shell-out path for `WholeRepoNative` checks. `files` is expected to
/// already be walked/collected by the caller (batch mode builds it once and reuses it
/// across every whole-repo check, native or not).
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

    let checker = match crate::architecture_checks::lookup(arch_name) {
        Some(checker) => checker,
        None => {
            return Ok(CheckResult {
                check_name: check.name.clone(),
                severity: check.severity,
                passed: false,
                output: format!("no architecture checker named '{arch_name}' registered"),
                message: check.message.clone(),
                command: cmd_str,
            });
        }
    };

    let graph = match crate::import_graph::build(repo_root, files) {
        Ok(graph) => graph,
        Err(err) => {
            return Ok(CheckResult {
                check_name: check.name.clone(),
                severity: check.severity,
                passed: false,
                output: format!("{err:#}"),
                message: check.message.clone(),
                command: cmd_str,
            });
        }
    };

    let findings = checker.check(&graph, arch_config);
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
    })
}

/// Native-checker counterpart to [`check_against_git_head_repo`]: snapshots HEAD the same
/// way, but builds the import graph and runs the architecture checker in-process against
/// the snapshot instead of shelling out.
fn check_native_against_git_head_repo(
    arch_name: &str,
    repo_root: &Path,
    arch_config: &crate::config::ArchitectureConfig,
) -> Option<bool> {
    let checker = crate::architecture_checks::lookup(arch_name)?;

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
    let result = files.and_then(|files| {
        crate::import_graph::build(&snapshot_dir, &files)
            .ok()
            .map(|graph| checker.check(&graph, arch_config).is_empty())
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
pub fn run_checks_for_trigger(
    checks: &[Check],
    trigger: &str,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
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
        results.push(run_check(check, repo_root, file_path, changed_lines)?);
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
        let result = run_check(&check, Path::new("."), Path::new("src/lib.rs"), None).unwrap();
        assert_eq!(result.output.trim(), "not sarif at all");
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
    fn run_check_downgrades_severity_when_violation_predates_edit() {
        let repo = TempRepo::new("run-check-downgrade");
        repo.write_and_commit("foo.txt", "line1\nBAD\nline3\n", "init");
        repo.write_uncommitted("foo.txt", "line1\nBAD\nline3-changed\n");

        let result = run_check(
            &bad_marker_check(),
            &repo.dir,
            &repo.path("foo.txt"),
            Some(&[(2, 2)]),
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

        let result = run_check(&repo_wide_bad_marker_check(), &repo.dir, &repo.dir, None).unwrap();
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

        let result = run_check(&primitive_obsession_check(), &dir, &file, None)
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

        let result = run_check(&primitive_obsession_check(), &dir, &file, None).unwrap();
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
        let result = run_check(&blank_imports_check(), &dir, &file, Some(&[(5, 5)])).unwrap();
        assert!(!result.passed);
        assert!(result.output.contains("unjustified/inside"));
        assert!(!result.output.contains("unjustified/outside"));

        // Scoping to a range with no findings at all reports a pass, proving the
        // filtering — not just the checker itself — determines the outcome.
        let clean = run_check(&blank_imports_check(), &dir, &file, Some(&[(1, 1)])).unwrap();
        assert!(clean.passed);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
