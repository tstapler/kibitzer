//! How kibitzer runs `git` for its baselines: pinned to the repo it was asked about, and bounded
//! in time, because a hook or daemon inherits its parent's environment and must never hang.

use std::cell::Cell;
use std::io::Read as _;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Longest a baseline `git` call may run before the caller falls back to "no baseline".
pub(crate) const BASELINE_TIMEOUT: Duration = Duration::from_secs(2);

/// Variables that redirect which repository, index or object store `git` reads. A hook or daemon
/// launched from inside another `git` command inherits them, and they would point the baseline at
/// the wrong repo.
const REPO_REDIRECTING_ENV: [&str; 9] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_PREFIX",
];

/// `git` run in `repo_root` with the repo-redirecting environment removed and the repo's
/// filesystem monitor disabled (a hostile checkout's config must not run code for a read).
pub(crate) fn git_command(repo_root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.fsmonitor=false"])
        .current_dir(repo_root);
    for var in REPO_REDIRECTING_ENV {
        cmd.env_remove(var);
    }
    cmd
}

/// Overall cap on the time one hook, daemon request or MCP/LSP call may spend in baseline `git`
/// calls (see [`with_git_budget`]): with a hung git it bounds the whole cost, not each call.
pub(crate) const HOOK_GIT_BUDGET: Duration = Duration::from_secs(5);

/// `git archive` snapshots the whole tree, so it gets a larger per-call bound than `show`/`diff`
/// (still clipped by whatever remains of the budget).
pub(crate) const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(10);

thread_local! {
    /// Git time still available to the active [`with_git_budget`] scope on this thread.
    static GIT_BUDGET: Cell<Option<Duration>> = const { Cell::new(None) };
}

/// Runs `f` with at most `total` of git time shared by every `bounded_output` call on this
/// thread; once spent, further calls return `None` ("unavailable") without spawning. A scope
/// already active (an outer entry point) is kept, so nesting never resets the clock.
pub(crate) fn with_git_budget<T>(total: Duration, f: impl FnOnce() -> T) -> T {
    if GIT_BUDGET.with(Cell::get).is_some() {
        return f();
    }
    GIT_BUDGET.with(|b| b.set(Some(total)));
    let out = f();
    GIT_BUDGET.with(|b| b.set(None));
    out
}

/// Kills the whole process group of `child` (spawned with `process_group(0)`, so its pid is the
/// group id), reaping grandchildren a git shim or hook may have forked.
fn kill_group(child: &Child) {
    let Ok(pgid) = i32::try_from(child.id()) else {
        return;
    };
    // SAFETY: `kill` takes plain integers and touches no memory; `-pgid` addresses the group
    // we created for this child, and a vanished group only yields ESRCH.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}

/// Runs `cmd` to completion or kills it (and any processes it forked) after `timeout`, further
/// clipped by the active [`with_git_budget`]; `None` for a spawn failure, a timeout or an
/// exhausted budget. The child leads its own process group so a timeout leaves no grandchildren
/// behind, and its pipes cannot stay open past this call.
pub(crate) fn bounded_output(mut cmd: Command, timeout: Duration) -> Option<Output> {
    let remaining = GIT_BUDGET.with(Cell::get);
    let timeout = match remaining {
        Some(left) if left.is_zero() => return None,
        Some(left) => timeout.min(left),
        None => timeout,
    };
    let started = Instant::now();
    let output = run_in_group(&mut cmd, timeout);
    if let Some(left) = remaining {
        let left = left.saturating_sub(started.elapsed());
        GIT_BUDGET.with(|b| b.set(Some(left)));
    }
    output
}

fn run_in_group(cmd: &mut Command, timeout: Duration) -> Option<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take()?;
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                kill_group(&child);
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    // A background grandchild would otherwise keep the pipes open and hang the joins below.
    kill_group(&child);
    Some(Output {
        status,
        stdout: out_reader.join().unwrap_or_default(),
        stderr: err_reader.join().unwrap_or_default(),
    })
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn bounded_output_should_KillAndReturnNone_When_ChildOutlivesTimeout() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30 & wait"]);
        let started = Instant::now();
        let out = bounded_output(cmd, Duration::from_millis(200));
        assert!(out.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn bounded_output_should_ReturnOutput_When_ChildFinishesInTime() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf hi; printf err >&2; exit 3"]);
        let out = bounded_output(cmd, Duration::from_secs(5)).unwrap();
        assert_eq!(out.stdout, b"hi");
        assert_eq!(out.stderr, b"err");
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn bounded_output_should_NotDeadlock_When_OutputExceedsPipeBuffer() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "head -c 1000000 /dev/zero"]);
        let out = bounded_output(cmd, Duration::from_secs(10)).unwrap();
        assert_eq!(out.stdout.len(), 1_000_000);
    }

    #[test]
    fn git_command_should_IgnoreGitDirEnv_When_ParentSetsIt() {
        // Environment of the *spawned* command, not this process: no mutation of globals.
        let cmd = git_command(Path::new("."));
        for var in REPO_REDIRECTING_ENV {
            let removed = cmd
                .get_envs()
                .any(|(k, v)| k == std::ffi::OsStr::new(var) && v.is_none());
            assert!(removed, "{var} is not removed");
        }
    }

    /// Number of live processes whose command line mentions `marker` (a unique sleep argument).
    fn processes_with(marker: &str) -> usize {
        let ps = Command::new("ps")
            .args(["-A", "-o", "command"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&ps.stdout)
            .lines()
            .filter(|l| l.contains(marker) && !l.starts_with("ps "))
            .count()
    }

    fn unique_sleep_arg() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        format!("3{}.{nanos}", std::process::id() % 1000)
    }

    #[test]
    fn bounded_output_should_KillGrandchildren_When_ShimForksAndTimesOut() {
        let marker = unique_sleep_arg();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &format!("sleep {marker} & sleep {marker} & wait")]);
        assert!(bounded_output(cmd, Duration::from_millis(300)).is_none());
        let deadline = Instant::now() + Duration::from_secs(3);
        while processes_with(&marker) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(processes_with(&marker), 0, "leaked grandchildren");
    }

    #[test]
    fn bounded_output_should_NotHang_When_ChildExitsButGrandchildHoldsPipes() {
        let marker = unique_sleep_arg();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", &format!("sleep {marker} & echo hi")]);
        let started = Instant::now();
        let out = bounded_output(cmd, Duration::from_secs(5)).unwrap();
        assert_eq!(out.stdout, b"hi\n");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn with_git_budget_should_ReturnNoneWithoutSpawning_When_BudgetExhausted() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let started = Instant::now();
        with_git_budget(Duration::from_millis(300), || {
            assert!(bounded_output(cmd, Duration::from_secs(20)).is_none());
            // The budget is spent: a trivial command is refused too, and instantly.
            let mut quick = Command::new("sh");
            quick.args(["-c", "true"]);
            let t = Instant::now();
            assert!(bounded_output(quick, Duration::from_secs(20)).is_none());
            assert!(t.elapsed() < Duration::from_millis(100));
        });
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn with_git_budget_should_ShareOneBudgetAcrossCalls_When_Nested() {
        with_git_budget(Duration::from_millis(600), || {
            let started = Instant::now();
            for _ in 0..4 {
                let mut cmd = Command::new("sh");
                cmd.args(["-c", "sleep 30"]);
                with_git_budget(Duration::from_secs(60), || {
                    assert!(bounded_output(cmd, Duration::from_millis(250)).is_none());
                });
            }
            assert!(
                started.elapsed() < Duration::from_millis(1500),
                "{:?}",
                started.elapsed()
            );
        });
    }

    #[test]
    fn bounded_output_should_HaveNoOverallLimit_When_NoBudgetScope() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 0.2; printf ok"]);
        assert_eq!(
            bounded_output(cmd, Duration::from_secs(5)).unwrap().stdout,
            b"ok"
        );
    }

    /// Source files that may still run `git` unbounded: explicit CLI subcommands (`kibitzer
    /// hotspots|coupling|root-causes|affected`), never reached from a hook, daemon, MCP or LSP
    /// request. Anything else must go through `git_command` + `bounded_output`.
    const CLI_ONLY_GIT_FILES: [&str; 4] = [
        "affected.rs",
        "hotspots.rs",
        "change_coupling.rs",
        "root_cause_clusters.rs",
    ];

    /// `source` without `#[cfg(test)]` items (a `mod x;` line or a braced module/function).
    fn without_test_items(source: &str) -> String {
        let mut out = String::new();
        let mut lines = source.lines();
        while let Some(line) = lines.next() {
            if line.trim() != "#[cfg(test)]" {
                out.push_str(line);
                out.push('\n');
                continue;
            }
            let mut depth = 0i32;
            let mut opened = false;
            for item_line in lines.by_ref() {
                depth += item_line.matches('{').count() as i32;
                depth -= item_line.matches('}').count() as i32;
                opened |= item_line.contains('{');
                let ends_item = if opened {
                    depth <= 0
                } else {
                    item_line.trim_end().ends_with(';')
                };
                if ends_item {
                    break;
                }
            }
        }
        out
    }

    fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn git_should_NeverRunUnbounded_When_OutsideCliOnlyModules() {
        let mut files = Vec::new();
        rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut violations = Vec::new();
        for path in files {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name == "git_cmd.rs"
                || name.ends_with("tests.rs")
                || name == "test_support.rs"
                || CLI_ONLY_GIT_FILES.contains(&name.as_str())
            {
                continue;
            }
            let source = without_test_items(&std::fs::read_to_string(&path).unwrap());
            for statement in source.split(';') {
                let runs_git = statement.contains("Command::new(\"git\")")
                    || statement.contains("git_command(");
                let unbounded = [".output()", ".status()", ".spawn()"]
                    .iter()
                    .any(|m| statement.contains(m));
                if runs_git && unbounded {
                    violations.push(format!("{}: {}", path.display(), statement.trim()));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "unbounded git calls:\n{}",
            violations.join("\n")
        );
    }
}
