use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::config::{Check, Severity};
use crate::glob::matches_scope;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub check_name: String,
    pub severity: Severity,
    pub passed: bool,
    pub output: String,
    pub message: Option<String>,
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
        && let Some(false) = check_against_git_head(check, repo_root, file_path)
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

/// Re-run `check` against the file's `git show HEAD:<relpath>` content to determine
/// whether a current failure predates this session's edits. Returns `Some(true)` if the
/// baseline also fails (pre-existing violation, not introduced by the current edit),
/// `Some(false)` if the baseline passes (the edit genuinely introduced this failure), or
/// `None` if the baseline can't be determined (untracked file, no HEAD, not a git repo,
/// etc.) — callers should treat `None` as "can't tell, don't suppress."
fn check_against_git_head(check: &Check, repo_root: &Path, file_path: &Path) -> Option<bool> {
    let rel_path = relativize(repo_root, file_path);
    let show = Command::new("git")
        .args(["show", &format!("HEAD:{rel_path}")])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !show.status.success() {
        return None;
    }

    let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut tmp_path = file_path.to_path_buf();
    let tmp_name = format!(
        ".kibitzer-head-{}{}",
        std::process::id(),
        if ext.is_empty() {
            String::new()
        } else {
            format!(".{ext}")
        }
    );
    tmp_path.set_file_name(tmp_name);
    std::fs::write(&tmp_path, &show.stdout).ok()?;

    let cmd_str = substitute_command(&check.command, &tmp_path, None);
    let result = Command::new("sh")
        .arg("-c")
        .arg(&cmd_str)
        .current_dir(repo_root)
        .output();

    let _ = std::fs::remove_file(&tmp_path);

    result.ok().map(|out| out.status.success())
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
}
