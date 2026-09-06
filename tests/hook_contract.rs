//! Round-trips a sample `PostToolUse` payload through the real `kibitzer hook`
//! binary (not `run_hook()` directly — it reads real stdin, so this exercises the
//! same stdin/exit-code/stdout contract Claude Code relies on). Isolates the cache
//! via a private `XDG_CACHE_HOME` so it can't collide with a real cache on the
//! machine running the tests or with a concurrently-running test.

use serde_json::json;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct TempRepo {
    dir: PathBuf,
    cache_dir: PathBuf,
}

impl TempRepo {
    fn new(name: &str, check_json: serde_json::Value) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = format!(
            "{}-{name}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let dir = std::env::temp_dir().join(format!("kibitzer-hook-contract-{unique}"));
        let cache_dir = std::env::temp_dir().join(format!("kibitzer-hook-cache-{unique}"));
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(
            dir.join(".claude").join("inspect.json"),
            serde_json::to_string(&json!({ "checks": [check_json] })).unwrap(),
        )
        .unwrap();
        Self { dir, cache_dir }
    }

    fn path(&self, rel_path: &str) -> PathBuf {
        self.dir.join(rel_path)
    }

    /// Invokes `kibitzer hook` as a real subprocess with a `PostToolUse` payload for
    /// `rel_path`/`content`, returning (exit_code, stdout, stderr). Writes `content`
    /// to disk first — like the real Edit/Write tool, the hook only checks the file
    /// as it already exists on disk; the JSON payload's `content` field is solely for
    /// diff-scoping, not for producing the file under test.
    fn run_hook(&self, rel_path: &str, content: &str) -> (i32, String, String) {
        let file_path = self.path(rel_path);
        std::fs::write(&file_path, content).unwrap();

        let payload = json!({
            "cwd": self.dir,
            "hook_event_name": "PostToolUse",
            "tool_input": {
                "file_path": file_path,
                "content": content,
            }
        });

        let mut child = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
            .arg("hook")
            .current_dir(&self.dir)
            .env("XDG_CACHE_HOME", &self.cache_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn kibitzer hook");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_dir_all(&self.cache_dir);
    }
}

#[test]
fn advisory_check_exits_zero_and_reports_via_stdout_context() {
    let repo = TempRepo::new(
        "advisory",
        json!({
            "name": "no-bad-marker",
            "command": "! grep -q BAD {file}",
            "severity": "advisory",
            "message": "found a BAD marker",
        }),
    );
    let (code, stdout, stderr) = repo.run_hook("foo.txt", "line1\nBAD\nline3\n");

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stderr.is_empty());
    assert!(stdout.contains("no-bad-marker"));
    assert!(stdout.contains("found a BAD marker"));
    assert!(stdout.contains("hookSpecificOutput"));
}

#[test]
fn passing_check_exits_zero_with_no_output() {
    let repo = TempRepo::new(
        "passing",
        json!({
            "name": "no-bad-marker",
            "command": "! grep -q BAD {file}",
            "severity": "blocking",
            "message": "found a BAD marker",
        }),
    );
    let (code, stdout, stderr) = repo.run_hook("foo.txt", "line1\nline2\n");

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[test]
fn blocking_check_gets_one_edit_of_grace_then_blocks_with_exit_code_2() {
    let repo = TempRepo::new(
        "blocking",
        json!({
            "name": "no-bad-marker",
            "command": "! grep -q BAD {file}",
            "severity": "blocking",
            "message": "found a BAD marker",
        }),
    );
    // First failure on this file+check: downgraded to advisory (cache.rs's grace
    // period), so it must not block yet.
    let (code, stdout, _) = repo.run_hook("foo.txt", "line1\nBAD\nline3\n");
    assert_eq!(code, 0);
    assert!(stdout.contains("hookSpecificOutput"));

    // Still failing on the next touch: grace is spent, this must block.
    let (code, stdout, stderr) = repo.run_hook("foo.txt", "line1\nBAD\nline3\n");
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(stderr.contains("no-bad-marker"));
    assert!(stderr.contains("blocking"));
}
