//! How kibitzer runs `git` for its baselines: pinned to the repo it was asked about, and bounded
//! in time, because a hook or daemon inherits its parent's environment and must never hang.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
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

/// Runs `cmd` to completion or kills it after `timeout`; `None` for a spawn failure or a timeout.
/// A killed child's pipes may be held open by a grandchild, so the reader threads are left to
/// finish on their own instead of being joined.
pub(crate) fn bounded_output(mut cmd: Command, timeout: Duration) -> Option<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
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
}
