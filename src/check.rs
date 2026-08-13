use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::config::{Check, Severity};
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
            "{shown}\n… {hidden} more line(s) truncated — see everything: `{}`",
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
    let cmd_str = substitute_command(&check.command, file_path, changed_lines);
    let output = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .current_dir(repo_root)
        .output()?;

    let passed_raw = output.status.success();
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

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

    if !passed
        && severity == Severity::Blocking
        && check.command.contains("{file}")
        && let Some(false) = check_against_git_head(check, repo_root, file_path, changed_lines)
    {
        severity = Severity::Advisory;
        message = Some(format!(
            "{} (downgraded: this violation predates your edits — already present in \
             the git HEAD version of this file)",
            message.unwrap_or_default()
        ));
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

    let cmd_str = substitute_command(&check.command, &tmp_path, head_ranges.as_deref());
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
            command: "! grep -n BAD {file}".to_string(),
            severity: Severity::Blocking,
            scope: vec![],
            triggers: vec![],
            message: Some("found BAD marker".to_string()),
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
}
