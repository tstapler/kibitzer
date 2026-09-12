//! Round-trips a sample `PostToolUse` payload through the real `kibitzer hook`
//! binary (not `run_hook()` directly — it reads real stdin, so this exercises the
//! same stdin/exit-code/stdout contract Claude Code relies on). Isolates the cache
//! via a private `XDG_CACHE_HOME` so it can't collide with a real cache on the
//! machine running the tests or with a concurrently-running test. Also isolates
//! `XDG_RUNTIME_DIR` (`default_socket_path` in `src/daemon.rs` derives the daemon
//! socket path from it) so these subprocesses never silently talk to a real `kibitzer
//! daemon` left running on the dev machine — without this, a stray daemon answers
//! `try_run_checks_via_daemon` instead of exercising the no-daemon fallback path
//! these tests mean to cover.

use serde_json::json;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

struct TempRepo {
    dir: PathBuf,
    cache_dir: PathBuf,
    runtime_dir: PathBuf,
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
        let runtime_dir = std::env::temp_dir().join(format!("kibitzer-hook-runtime-{unique}"));
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::create_dir_all(&runtime_dir).unwrap();
        std::fs::write(
            dir.join(".claude").join("inspect.json"),
            serde_json::to_string(&json!({ "checks": [check_json] })).unwrap(),
        )
        .unwrap();
        Self {
            dir,
            cache_dir,
            runtime_dir,
        }
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
        self.spawn_hook(&payload)
    }

    /// Same as `run_hook`, but sends an `Edit`-shaped payload (`old_string`/
    /// `new_string`) instead of `content` — so `compute_changed_lines` (`src/hook.rs`)
    /// scopes the call to a diff range (`Some`) instead of treating it as an unscoped
    /// whole-file rewrite (`None`). Real Claude Code `Edit` tool calls are always
    /// diff-scoped this way; only `Write` sends `content`. `new_string` must appear
    /// verbatim in `final_content` or `compute_changed_lines` can't locate the range.
    fn run_hook_edit(
        &self,
        rel_path: &str,
        final_content: &str,
        old_string: &str,
        new_string: &str,
    ) -> (i32, String, String) {
        let file_path = self.path(rel_path);
        std::fs::write(&file_path, final_content).unwrap();

        let payload = json!({
            "cwd": self.dir,
            "hook_event_name": "PostToolUse",
            "tool_input": {
                "file_path": file_path,
                "old_string": old_string,
                "new_string": new_string,
            }
        });
        self.spawn_hook(&payload)
    }

    fn spawn_hook(&self, payload: &serde_json::Value) -> (i32, String, String) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
            .arg("hook")
            .current_dir(&self.dir)
            .env("XDG_CACHE_HOME", &self.cache_dir)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
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
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
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

/// Regression test for docs/markdown-link-integrity-false-positives.md's `dcb5a7eb`
/// entry: a reference-style link introduced in one `Edit` with no matching
/// `[ref]: target` definition yet, resolved by the very next `Edit` adding it. Under
/// `Cache::apply_grace` (`src/cache.rs`) this must downgrade to advisory on the first
/// (diff-scoped) touch and never escalate, since the second touch passes outright —
/// covering the "fail once, then pass" shape of that entry's blocks 1 and 2-3 for the
/// real native `markdown-link-integrity` checker (`src/markdown_link_integrity.rs`),
/// not just a generic shell-command check.
#[test]
fn markdown_link_integrity_dangling_reference_then_its_definition_never_hard_blocks() {
    let repo = TempRepo::new(
        "md-link-integrity-use-then-def",
        json!({
            "name": "markdown-link-integrity",
            "checker": "markdown-link-integrity",
            "severity": "blocking",
        }),
    );

    // Edit 1: introduces a reference-style use with no definition yet.
    let (code, stdout, stderr) = repo.run_hook_edit(
        "doc.md",
        "# Doc\n\nSee [ref link][myref] for more.\n",
        "# Doc\n",
        "\nSee [ref link][myref] for more.\n",
    );
    assert_eq!(code, 0, "first failure must get grace, not block: {stderr}");
    assert!(stdout.contains("myref"), "stdout: {stdout}");

    // Edit 2: adds the matching definition, fully resolving it — the check now
    // passes outright, so grace is cleared rather than escalating.
    let (code, stdout, stderr) = repo.run_hook_edit(
        "doc.md",
        "# Doc\n\nSee [ref link][myref] for more.\n\n[myref]: https://example.com/ref\n",
        "for more.\n",
        "for more.\n\n[myref]: https://example.com/ref\n",
    );
    assert_eq!(code, 0, "resolving edit must not block: {stderr}");
    assert!(stdout.is_empty(), "no findings once resolved: {stdout}");
}

/// Mirror-image direction of the test above (the `dcb5a7eb` entry's blocks 2-3): a
/// `[ref]: target` definition added with no use anywhere yet, resolved by the very
/// next edit adding a use. Confirms `unused_definition_findings`
/// (`src/markdown_link_integrity.rs`) feeds into the same `check_name` and so gets
/// the same grace treatment as the dangling-use direction — the check is genuinely
/// bidirectional, not just "use before definition".
#[test]
fn markdown_link_integrity_dangling_definition_then_its_use_never_hard_blocks() {
    let repo = TempRepo::new(
        "md-link-integrity-def-then-use",
        json!({
            "name": "markdown-link-integrity",
            "checker": "markdown-link-integrity",
            "severity": "blocking",
        }),
    );

    // Edit 1: adds a definition with no use anywhere in the doc yet.
    let (code, stdout, stderr) = repo.run_hook_edit(
        "doc.md",
        "# Doc\n\nSome prose.\n\n[myref]: https://example.com/ref\n",
        "Some prose.\n",
        "Some prose.\n\n[myref]: https://example.com/ref\n",
    );
    assert_eq!(code, 0, "first failure must get grace, not block: {stderr}");
    assert!(stdout.contains("myref"), "stdout: {stdout}");

    // Edit 2: adds a matching use elsewhere in the doc, fully resolving it.
    let (code, stdout, stderr) = repo.run_hook_edit(
        "doc.md",
        "# Doc\n\nSome prose. See [it][myref].\n\n[myref]: https://example.com/ref\n",
        "Some prose.\n",
        "Some prose. See [it][myref].\n",
    );
    assert_eq!(code, 0, "resolving edit must not block: {stderr}");
    assert!(stdout.is_empty(), "no findings once resolved: {stdout}");
}

/// Regression test for the disk-persistence gap in the no-daemon fallback
/// (`run_checks_smart` in `src/daemon.rs`): before the fix, `cache.save()` only ran
/// when `changed_lines.is_none()` (an unscoped `Write`), so a diff-scoped `Edit` call
/// — the common case, and the shape every entry in
/// docs/markdown-link-integrity-false-positives.md actually is — never persisted
/// `Cache::grace_pending` to disk. Each subsequent `kibitzer hook` process (no daemon
/// running) then reloaded an empty `grace_pending` map and treated every failure as a
/// fresh "first occurrence," so a still-failing check could never escalate back to
/// Blocking at all without a daemon. This spawns two separate `kibitzer hook`
/// processes (never a daemon) with Edit-shaped (diff-scoped) payloads and asserts the
/// second, still-failing touch does escalate.
#[test]
fn blocking_check_grace_persists_across_diff_scoped_edits_without_a_daemon() {
    let repo = TempRepo::new(
        "blocking-edit-scoped-grace",
        json!({
            "name": "no-bad-marker",
            "command": "! grep -q BAD {file}",
            "severity": "blocking",
            "message": "found a BAD marker",
        }),
    );

    // First diff-scoped (Edit-shaped) failure: must still get grace.
    let (code, stdout, stderr) =
        repo.run_hook_edit("foo.txt", "line1\nBAD\nline3\n", "line2\n", "BAD\nline3\n");
    assert_eq!(
        code, 0,
        "first diff-scoped failure must get grace: {stderr}"
    );
    assert!(stdout.contains("hookSpecificOutput"));

    // Second diff-scoped touch, from a brand-new `kibitzer hook` process (no daemon
    // ever ran) — an unrelated edit to the same file, with the marker still present.
    // Grace was already spent on the first touch, so this must escalate to blocking.
    let (code, stdout, stderr) = repo.run_hook_edit(
        "foo.txt",
        "line1\nBAD\nline3\nmore\n",
        "line3\n",
        "line3\nmore\n",
    );
    assert_eq!(
        code, 2,
        "grace must have persisted to disk from the first diff-scoped call: stdout={stdout} stderr={stderr}"
    );
    assert!(stdout.is_empty());
    assert!(stderr.contains("no-bad-marker"));
}
