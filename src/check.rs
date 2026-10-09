use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;

use crate::accepted_findings::AcceptedLines;
pub use crate::check_result::CheckResult;
use crate::config::{Check, OutputFormat, Severity};
use crate::git_cmd::{
    ARCHIVE_TIMEOUT, BASELINE_TIMEOUT, HOOK_GIT_BUDGET, bounded_output, git_command,
    with_git_budget,
};
use crate::glob::matches_scope;
use crate::inline_ignores::sanitize::display_path;
use crate::inline_ignores::{
    DroppedFinding, IgnoreTarget, InlineIgnoreContext, InlineOutcome, Line, RawFinding,
    anchor_rule, apply_inline_ignores, capped_anchors,
};
use crate::inline_post_pass::HeadSnapshot;
use crate::plugin::Registry;
use crate::run_context::RunContext;

mod finding_lines;
use finding_lines::FindingLines;

/// Wall-clock ceiling on a single `run_check` command dispatch (see
/// [`run_command_with_timeout`]). Plugin binaries are the class of `command` check most
/// likely to genuinely hang (a first-run model load, a downloaded runtime waiting on
/// something that never arrives), so this applies to every shell-out check, not just
/// plugin-backed ones.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Run a single check against `file_path` (already confirmed in-scope by the caller).
/// `changed_lines`, when present, scopes the result to findings that fall within those
/// 1-indexed inclusive line ranges — see [`scope_output_to_changed_lines`]. `registry`
/// should be loaded once per batch/request by the caller (see [`run_checks_for_trigger`])
/// rather than reloaded here — `registry.json` is read on every plugin-missing check
/// otherwise, even for a repo with zero plugins installed. `run_ctx` is likewise loaded
/// once per batch/request by the caller, for the same reason (see
/// [`run_checks_for_trigger`]'s doc comment).
pub fn run_check(
    check: &Check,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    registry: &Registry,
    run_ctx: &RunContext,
) -> anyhow::Result<CheckResult> {
    with_git_budget(HOOK_GIT_BUDGET, || {
        run_check_with_timeout(
            check,
            repo_root,
            file_path,
            changed_lines,
            COMMAND_TIMEOUT,
            registry,
            run_ctx,
        )
    })
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
    run_ctx: &RunContext,
) -> anyhow::Result<CheckResult> {
    if let Some(checker_name) = &check.checker {
        return run_native_check(
            check,
            checker_name,
            repo_root,
            file_path,
            changed_lines,
            run_ctx,
        );
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
        return Ok(CheckResult::passing(check.name.clone(), check.severity));
    }

    // Task 4.3.1b: a plugin-backed check (always `command`-based, never `checker`) whose
    // binary has gone missing (removed out-of-band, or the whole plugin uninstalled)
    // short-circuits here, before a `sh -c` is ever spawned against a path that doesn't
    // exist — that would otherwise surface as generic shell "command not found" noise
    // indistinguishable from a real check failure. Severity is forced to `Advisory`
    // regardless of `check.severity` so a missing install never blocks an edit.
    if let Some(binary_path) = registry.missing_binary_for(&check.name) {
        return Ok(CheckResult::new(check.name.clone(), Severity::Advisory, false, String::new())
.with_message(Some(format!(
                "plugin '{}' is not installed (expected binary at {}) — run `kibitzer plugin install {}`",
                check.name,
                binary_path.display(),
                check.name
            )))
.with_plugin_missing());
    }

    run_shell_command_check(check, repo_root, file_path, changed_lines, timeout)
}

/// The `command`-based path: runs the shell command (killed after `timeout`), renders its
/// output, diff-scopes it, and downgrades a blocking failure that predates the edit.
fn run_shell_command_check(
    check: &Check,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    timeout: Duration,
) -> anyhow::Result<CheckResult> {
    let command = check
        .command
        .as_deref()
        .expect("config-load validation guarantees command is set when checker is not");

    let cmd_str = substitute_command(command, file_path, changed_lines);
    let output = match run_command_with_timeout(&cmd_str, repo_root, timeout)? {
        CommandOutcome::Completed(output) => output,
        CommandOutcome::TimedOut => {
            return Ok(CheckResult::new(
                check.name.clone(),
                check.severity,
                false,
                format!("command timed out after {timeout:?} and was killed: {cmd_str}"),
            )
            .with_message(Some(format!(
                "{}check timed out after {timeout:?} and was killed",
                check
                    .message
                    .as_ref()
                    .map(|m| format!("{m} — "))
                    .unwrap_or_default()
            )))
            .with_command(cmd_str));
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

    let (severity, message) =
        downgrade_if_predates(passed, check.severity, check.message.clone(), || {
            command_baseline_against_git_head(check, command, repo_root, file_path, changed_lines)
        });

    Ok(
        CheckResult::new(check.name.clone(), severity, passed, combined)
            .with_message(message)
            .with_command(cmd_str),
    )
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
    run_ctx: &RunContext,
) -> anyhow::Result<CheckResult> {
    let cmd_str = format!(
        "kibitzer check native {checker_name} {}",
        file_path.display()
    );
    let rel_path = relativize(repo_root, file_path);

    if let Some(checker) = crate::checker::lookup(checker_name) {
        let globs: Vec<String> = checker.file_globs().iter().map(|g| g.to_string()).collect();
        if !matches_scope(&rel_path, &globs) {
            return Ok(
                CheckResult::passing(check.name.clone(), check.severity).with_command(cmd_str)
            );
        }
    }

    let run = NativeRun::for_check(check, checker_name, &run_ctx.inline);

    // Degrade to a failed CheckResult on error (e.g. an unreadable file) instead of
    // propagating, matching the shell-out path above where a command's own failure is
    // captured as `passed_raw = false` rather than aborting the whole batch — a single
    // bad file shouldn't kill every other check/file in the run.
    let SourceCheck {
        combined,
        passed: passed_raw,
        findings: kept_findings,
        inline: mut inline_outcome,
    } = match run_checker_against_file(run, file_path) {
        Ok(result) => result,
        Err(err) => {
            return Ok(CheckResult::new(
                check.name.clone(),
                check.severity,
                false,
                format!("{err:#}"),
            )
            .with_message(check.message.clone())
            .with_command(cmd_str));
        }
    };

    let mut lines = FindingLines::new(&kept_findings, file_path);
    let passed = apply_diff_scope(&mut lines, file_path, changed_lines, passed_raw);
    let passed = passed
        || apply_accepted(
            &mut lines,
            file_path,
            &rel_path,
            checker_name,
            &run_ctx.accepted,
        );
    let combined = if lines.is_filtered() {
        lines.text()
    } else {
        combined
    };

    let (severity, message) =
        downgrade_if_predates(passed, check.severity, check.message.clone(), || {
            check_native_against_git_head(run, repo_root, file_path, changed_lines)
        });

    capture_inline_anchors(&mut inline_outcome, checker_name, &kept_findings, &lines);

    Ok(
        CheckResult::new(check.name.clone(), severity, passed, combined)
            .with_message(message)
            .with_command(cmd_str)
            .with_inline(inline_outcome),
    )
}

/// Drops the lines outside `changed_lines` (when the edit is diff-scoped) and returns whether
/// the check still passes; output that cannot be scoped is left untouched.
fn apply_diff_scope(
    lines: &mut FindingLines,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    passed_raw: bool,
) -> bool {
    let Some(ranges) = changed_lines else {
        return passed_raw;
    };
    match scope_line_verdicts(&lines.texts(), file_path, ranges, passed_raw) {
        Some(verdicts) => {
            lines.retain(&verdicts.keep);
            verdicts.passed
        }
        None => passed_raw,
    }
}

/// Drops the lines an `accepted/` entry covers and returns whether nothing is left.
fn apply_accepted(
    lines: &mut FindingLines,
    file_path: &Path,
    rel_path: &str,
    checker_name: &str,
    accepted: &crate::accepted_findings::AcceptedFindings,
) -> bool {
    if let Some(accepted_lines) =
        AcceptedLines::for_file(file_path, rel_path, checker_name, accepted)
    {
        let keep: Vec<bool> = lines
            .texts()
            .iter()
            .map(|line| !accepted_lines.is_accepted(line))
            .collect();
        lines.retain(&keep);
    }
    lines.text().trim().is_empty()
}

/// Records which findings were shown (survived scoping and `accepted/`) and kept (survived
/// inline ignores) as capped anchors, so the footer can name the first one.
fn capture_inline_anchors(
    outcome: &mut InlineOutcome,
    checker_name: &str,
    kept_findings: &[crate::checker::Finding],
    lines: &FindingLines,
) {
    let visible = lines.visible();
    outcome.shown = capped_anchors(
        checker_name,
        kept_findings
            .iter()
            .zip(&visible)
            .filter_map(|(finding, shown)| shown.then_some(finding)),
    );
    outcome.kept = capped_anchors(checker_name, kept_findings.iter());
}

/// A blocking failure that was already present at the git HEAD commit predates the edit, so it
/// is reported as advisory. `baseline` runs only for a failing blocking check and reports
/// whether the check passed at HEAD: `Some(false)` means it already failed there, `None`
/// means that could not be determined.
fn downgrade_if_predates(
    passed: bool,
    severity: Severity,
    message: Option<String>,
    baseline: impl FnOnce() -> Option<bool>,
) -> (Severity, Option<String>) {
    if passed || severity != Severity::Blocking {
        return (severity, message);
    }
    match baseline() {
        Some(false) => (
            Severity::Advisory,
            Some(format!(
                "{} (downgraded: this violation predates your edits — already present \
                 at the git HEAD commit)",
                message.unwrap_or_default()
            )),
        ),
        _ => (severity, message),
    }
}

/// What one in-process checker run needs besides the file: which checker, its options, the
/// check's severity (recorded on dropped findings) and the inline-ignore context.
#[derive(Clone, Copy)]
struct NativeRun<'a> {
    checker_name: &'a str,
    options: Option<&'a serde_json::Value>,
    severity: Severity,
    inline_ctx: &'a InlineIgnoreContext,
}

impl<'a> NativeRun<'a> {
    fn for_check(
        check: &'a Check,
        checker_name: &'a str,
        inline_ctx: &'a InlineIgnoreContext,
    ) -> Self {
        NativeRun {
            checker_name,
            options: check.options.as_ref(),
            severity: check.severity,
            inline_ctx,
        }
    }
}

/// Runs `checker_name` against `source` (as if it were the content of `file_path`),
/// producing output in the same `{file}:{line}: {message}` convention a shell-out check's
/// command output would follow, so downstream diff-scoping and baseline logic can treat
/// native and shell-out checks identically.
fn run_checker_against_source(
    run: NativeRun,
    file_path: &Path,
    source: &str,
) -> anyhow::Result<SourceCheck> {
    let NativeRun {
        checker_name,
        options,
        severity,
        inline_ctx,
    } = run;
    let findings = crate::checker::run_checker_configured_with_scans(
        checker_name,
        file_path,
        source,
        options,
        &inline_ctx.scan_memo,
    )?;
    let applied = apply_inline_ignores(
        findings,
        IgnoreTarget {
            file: file_path,
            source,
            checker_name,
            severity,
        },
        inline_ctx,
    );
    let combined = applied
        .kept
        .iter()
        .map(|f| render_finding(file_path, f))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(SourceCheck {
        combined,
        passed: applied.kept.is_empty(),
        findings: applied.kept,
        inline: InlineOutcome {
            dropped: applied.dropped,
            ..InlineOutcome::default()
        },
    })
}

fn render_finding(file_path: &Path, finding: &crate::checker::Finding) -> String {
    format!(
        "{}:{}: {}",
        file_path.display(),
        finding.line,
        finding.message
    )
}

/// Result of one native checker run after inline filtering. `findings` is the structured
/// kept list, so nothing downstream has to parse rule or line back out of `combined`.
struct SourceCheck {
    combined: String,
    passed: bool,
    findings: Vec<crate::checker::Finding>,
    /// Only `dropped` is filled here; `shown` and `kept` are computed in `run_native_check`.
    inline: InlineOutcome,
}

impl SourceCheck {
    fn passing() -> Self {
        SourceCheck {
            combined: String::new(),
            passed: true,
            findings: Vec::new(),
            inline: InlineOutcome::default(),
        }
    }
}

/// Findings of `check`'s native checker as it reported them: inline ignores disabled, and
/// none of diff-scoping, `accepted/` or the HEAD baseline applied (those live in
/// `run_native_check`, which this deliberately bypasses). Empty for a non-native check.
fn raw_findings_for_check(
    check: &Check,
    file_path: &Path,
    source: &str,
) -> anyhow::Result<Vec<RawFinding>> {
    let Some(checker_name) = &check.checker else {
        return Ok(Vec::new());
    };
    let raw = run_checker_against_source(
        NativeRun::for_check(check, checker_name, &InlineIgnoreContext::disabled()),
        file_path,
        source,
    )?;
    Ok(raw
        .findings
        .into_iter()
        .map(|f| RawFinding {
            line: Line::new(f.line),
            checker: checker_name.clone(),
            rule: anchor_rule(checker_name, &f),
            message: f.message,
        })
        .collect())
}

/// The file's content at git HEAD, for the stateless baseline of the blocking-suppression
/// advisory. A staged rename reads the renamed-from blob; a non-UTF-8 blob is decoded lossily.
/// Anything else (no repo, no commits, an untracked, ignored, newly added or submodule path, a
/// file outside the root, a git that errors or exceeds `BASELINE_TIMEOUT`) is `Unavailable`:
/// "not known to be new", never "everything is new".
fn git_head_snapshot(repo_root: &Path, file_path: &Path) -> HeadSnapshot {
    let canonical = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let rel = match file_path.strip_prefix(repo_root) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => match canonical(file_path).strip_prefix(canonical(repo_root)) {
            Ok(rel) => rel.to_path_buf(),
            Err(_) => return HeadSnapshot::Unavailable,
        },
    };
    let rel = rel.to_string_lossy().replace('\\', "/");
    let run = |args: &[&str]| {
        let mut cmd = git_command(repo_root);
        cmd.args(args);
        bounded_output(cmd, BASELINE_TIMEOUT).filter(|out| out.status.success())
    };
    let head_blob = |path: &str| run(&["show", &format!("HEAD:./{path}")]);
    let blob = head_blob(&rel).or_else(|| {
        let from = staged_rename_source(&run, &rel)?;
        head_blob(&from)
    });
    blob.map_or(HeadSnapshot::Unavailable, |out| {
        HeadSnapshot::Source(String::from_utf8_lossy(&out.stdout).into_owned())
    })
}

/// Most staged files rename detection pairs up; bounds the cost on a huge `git mv`.
const RENAME_DETECTION_LIMIT: &str = "-l200";

/// The path `rel` was renamed or copied from in the index (`git mv`), relative to the repo root
/// the hook runs in; `None` when `rel` is not the target of a staged rename.
fn staged_rename_source(
    run: &dyn Fn(&[&str]) -> Option<std::process::Output>,
    rel: &str,
) -> Option<String> {
    let diff = run(&[
        "diff",
        "--cached",
        "--no-ext-diff",
        "-M",
        RENAME_DETECTION_LIMIT,
        "--name-status",
        "--relative",
        "-z",
        "HEAD",
    ])?;
    let text = String::from_utf8_lossy(&diff.stdout);
    let mut fields = text.split('\0');
    while let Some(status) = fields.next() {
        if status.starts_with(['R', 'C']) {
            let (from, to) = (fields.next()?, fields.next()?);
            if to == rel {
                return Some(from.to_string());
            }
        } else {
            fields.next();
        }
    }
    None
}

/// What directives drop when `check`'s native checker runs over `head_source`, under the run's
/// inline mode. Never feeds the run's counter or evicts its scan memo.
fn head_dropped_findings(
    check: &Check,
    file_path: &Path,
    head_source: &str,
    inline: &InlineIgnoreContext,
) -> anyhow::Result<Vec<DroppedFinding>> {
    let Some(checker_name) = &check.checker else {
        return Ok(Vec::new());
    };
    let replay = inline.without_counter();
    let checked = run_checker_against_source(
        NativeRun::for_check(check, checker_name, &replay),
        file_path,
        head_source,
    )?;
    Ok(checked.inline.dropped)
}

fn tolerates_unreadable_files(checker_name: &str) -> bool {
    crate::checker::lookup(checker_name).is_some_and(|c| c.tolerates_unreadable_files())
}

fn run_checker_against_file(run: NativeRun, file_path: &Path) -> anyhow::Result<SourceCheck> {
    let checker_name = run.checker_name;
    let source = match crate::checker::read_native_source(file_path) {
        Ok(crate::checker::NativeSource::Text(source)) => source,
        Ok(crate::checker::NativeSource::TooLarge) => return Ok(SourceCheck::passing()),
        Err(_) if tolerates_unreadable_files(checker_name) => {
            return Ok(SourceCheck::passing());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", file_path.display()));
        }
    };
    run_checker_against_source(run, file_path, &source)
}

/// Native-checker counterpart to [`check_against_git_head`]: same git-HEAD comparison, but
/// runs the checker in-process against the HEAD content instead of shelling out to a
/// substituted command.
fn check_native_against_git_head(
    run: NativeRun,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> Option<bool> {
    let rel_path = relativize(repo_root, file_path);
    let mut show_cmd = git_command(repo_root);
    show_cmd.args(["show", &format!("HEAD:{rel_path}")]);
    let show = bounded_output(show_cmd, BASELINE_TIMEOUT)?;
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
    // Replays honor the run's mode but never feed its counter.
    let replay_ctx = run.inline_ctx.without_counter();
    let SourceCheck {
        combined,
        passed: passed_raw,
        ..
    } = run_checker_against_source(
        NativeRun {
            inline_ctx: &replay_ctx,
            ..run
        },
        file_path,
        &source,
    )
    .ok()?;

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
mod sarif_tests;

/// Per-line outcome of diff-scoping: which output lines survive, and whether any failure does.
struct ScopeVerdicts {
    keep: Vec<bool>,
    passed: bool,
}

/// Decides, per output line, whether it falls within `ranges`; `None` means the output cannot
/// be scoped (already passing, or no line follows the `{file}:{line}:` convention) and must
/// be left untouched. Lines that don't follow the convention are kept as-is (their line
/// can't be attributed to a range) and count toward a failure if any survive — this is
/// deliberately conservative: output kibitzer doesn't understand should not be silently
/// swallowed.
fn scope_line_verdicts(
    lines: &[&str],
    file_path: &Path,
    ranges: &[(usize, usize)],
    passed_raw: bool,
) -> Option<ScopeVerdicts> {
    if passed_raw {
        return None;
    }
    if ranges.is_empty() {
        // Diff-scoping is active (the caller only reaches this function when
        // `changed_lines` was `Some(_)`) but there are zero changed-line ranges to
        // scope to — e.g. a pure-deletion edit (see `hook::compute_changed_lines`).
        // Nothing can be attributed to this edit, so every finding is out of scope,
        // not "unscoped" — unlike the `changed_lines: None` case, this must not fall
        // back to the raw whole-file output.
        return Some(ScopeVerdicts {
            keep: vec![false; lines.len()],
            passed: true,
        });
    }

    let prefix = format!("{}:", file_path.display());
    let attributed: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            line.strip_prefix(&prefix)
                .and_then(|rest| rest.split(':').next())
                .and_then(|n| n.parse::<usize>().ok())
        })
        .collect();
    if attributed.iter().all(Option::is_none) {
        // Output doesn't follow the convention at all — can't scope it, leave untouched.
        return None;
    }
    let keep: Vec<bool> = attributed
        .iter()
        .map(|line_no| match line_no {
            Some(n) => ranges.iter().any(|(start, end)| n >= start && n <= end),
            None => true,
        })
        .collect();
    // Any surviving non-empty line, attributed or not, is a failure.
    let passed = lines
        .iter()
        .zip(&keep)
        .all(|(line, kept)| !kept || line.is_empty());
    Some(ScopeVerdicts { keep, passed })
}

/// Text-level wrapper over [`scope_line_verdicts`] for output that is not tied to structured
/// findings (shell-out checks).
fn scope_output_to_changed_lines(
    output: &str,
    file_path: &Path,
    ranges: &[(usize, usize)],
    passed_raw: bool,
) -> (String, bool) {
    let lines: Vec<&str> = output.lines().collect();
    let Some(verdicts) = scope_line_verdicts(&lines, file_path, ranges, passed_raw) else {
        return (output.to_string(), passed_raw);
    };
    let kept: Vec<&str> = lines
        .iter()
        .zip(&verdicts.keep)
        .filter_map(|(line, kept)| kept.then_some(*line))
        .collect();
    (kept.join("\n"), verdicts.passed)
}

/// Process-wide nonce so concurrent baseline checks (the daemon spawns one thread per
/// connection) never share a temp file path, even when they're checking files with the
/// same extension at the same instant — a shared path let one thread's baseline read/write
/// race with another's, corrupting both checks' results.
static TMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Command-check counterpart to the per-file-vs-repo-wide branch [`run_check_with_timeout`]
/// used to take inline: a whole-repo command (no `{file}` placeholder) baselines against a
/// snapshot of the whole tree; anything else baselines against just this file.
fn command_baseline_against_git_head(
    check: &Check,
    command: &str,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> Option<bool> {
    if command.contains("{file}") {
        check_against_git_head(check, repo_root, file_path, changed_lines)
    } else {
        check_against_git_head_repo(check, repo_root)
    }
}

/// Whether `check` already failed against `file_path`'s content at git HEAD, regardless of
/// `check.severity` — dispatches to the native or command-based baseline path depending on
/// how `check` is implemented. Unlike the `Severity::Blocking`-gated call sites in
/// [`run_check_with_timeout`]/[`run_native_check`], this is exposed for callers (namely
/// `task_stop`'s unscoped Stop-hook recheck) that want to know whether *any* finding —
/// including an Advisory one — predates the current edits, so a whole-file recheck doesn't
/// resurface a pre-existing violation as if it were newly introduced.
///
/// `None` means "can't tell" (untracked file, no HEAD commit, not a git repo, or — for an
/// `architecture_checker`-based check, which has no per-file HEAD content to compare
/// against — not applicable at all); callers should treat that as "don't suppress."
pub(crate) fn check_predates_git_head(
    check: &Check,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
) -> Option<bool> {
    with_git_budget(HOOK_GIT_BUDGET, || {
        if let Some(checker_name) = &check.checker {
            let ctx = InlineIgnoreContext::default();
            let run = NativeRun::for_check(check, checker_name, &ctx);
            return check_native_against_git_head(run, repo_root, file_path, changed_lines);
        }
        let command = check.command.as_deref()?;
        command_baseline_against_git_head(check, command, repo_root, file_path, changed_lines)
    })
}

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
    let mut show_cmd = git_command(repo_root);
    show_cmd.args(["show", &format!("HEAD:{rel_path}")]);
    let show = bounded_output(show_cmd, BASELINE_TIMEOUT)?;
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

    let mut archive_cmd = git_command(repo_root);
    archive_cmd.args(["archive", "HEAD"]);
    let Some(archive) = bounded_output(archive_cmd, ARCHIVE_TIMEOUT) else {
        let _ = std::fs::remove_dir_all(&snapshot_dir);
        return None;
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
    include_private: bool,
) -> anyhow::Result<crate::arch_model::ArchModel> {
    crate::arch_model::build_model_from_files(
        repo_root,
        files,
        &crate::arch_model::PruneConfig { include_private },
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
    let error_result = |output: String| {
        CheckResult::new(check.name.clone(), check.severity, false, output)
            .with_message(check.message.clone())
            .with_command(cmd_str.clone())
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
            let model =
                match build_arch_model_for_check(repo_root, files, checker.needs_private_symbols())
                {
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
            (Some(file), Some(line)) => format!("{}:{}: {}", display_path(file), line, f.message),
            (Some(file), None) => format!("{}: {}", display_path(file), f.message),
            (None, _) => f.message.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n");

    let (severity, message) =
        downgrade_if_predates(passed, check.severity, check.message.clone(), || {
            check_native_against_git_head_repo(arch_name, repo_root, arch_config)
        });

    Ok(
        CheckResult::new(check.name.clone(), severity, passed, combined)
            .with_message(message)
            .with_command(cmd_str)
            .with_findings(findings),
    )
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

    let mut archive_cmd = git_command(repo_root);
    archive_cmd.args(["archive", "HEAD"]);
    let Some(archive) = bounded_output(archive_cmd, ARCHIVE_TIMEOUT) else {
        let _ = std::fs::remove_dir_all(&snapshot_dir);
        return None;
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
        AnyArchitectureChecker::Model(checker) => {
            build_arch_model_for_check(&snapshot_dir, &files, checker.needs_private_symbols())
                .ok()
                .map(|model| checker.check(&model, arch_config).is_empty())
        }
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
    let mut diff_cmd = git_command(repo_root);
    diff_cmd.args([
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-U0",
        "HEAD",
        "--",
        rel_path,
    ]);
    let diff = bounded_output(diff_cmd, BASELINE_TIMEOUT)?;
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
/// (file, check) pair). `run_ctx` (holding `accepted_findings::ACCEPTED_FINDINGS_DIR` entries) must be
/// loaded the same way, by the same caller, for the same reason — and, more importantly,
/// so that a malformed accepted-findings entry surfaces as one clean error before any
/// file work starts, rather than a `?` from inside this per-(file, check) loop
/// nondeterministically discarding every result already accumulated for the batch.
pub fn run_checks_for_trigger(
    checks: &[Check],
    trigger: &str,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    registry: &Registry,
    run_ctx: &RunContext,
) -> anyhow::Result<Vec<CheckResult>> {
    with_git_budget(HOOK_GIT_BUDGET, || {
        run_checks_for_trigger_unbudgeted(
            checks,
            trigger,
            repo_root,
            file_path,
            changed_lines,
            registry,
            run_ctx,
        )
    })
}

fn run_checks_for_trigger_unbudgeted(
    checks: &[Check],
    trigger: &str,
    repo_root: &Path,
    file_path: &Path,
    changed_lines: Option<&[(usize, usize)]>,
    registry: &Registry,
    run_ctx: &RunContext,
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
            run_ctx,
        )?);
    }
    let extra = crate::inline_post_pass::run(crate::inline_post_pass::PostPassInput {
        checks,
        file_path,
        changed_lines,
        results: &results,
        run_ctx,
        raw_rerun: &raw_findings_for_check,
        head: &|path| git_head_snapshot(repo_root, path),
        head_drops: &|check, path, head_source| {
            head_dropped_findings(check, path, head_source, &run_ctx.inline)
        },
    });
    results.extend(extra);
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
mod diff_scoping_tests;

#[cfg(test)]
mod sarif_run_check_tests;

#[cfg(test)]
mod timeout_tests;

#[cfg(test)]
mod plugin_missing_tests;

#[cfg(test)]
mod head_mapping_tests;

#[cfg(test)]
mod git_head_integration_tests;

#[cfg(test)]
mod native_check_tests;

#[cfg(test)]
mod findings_wiring_tests;

#[cfg(test)]
mod inline_seam_tests;

#[cfg(test)]
mod post_pass_pipeline_tests;
