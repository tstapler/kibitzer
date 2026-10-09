use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cache::{Cache, Stamps, default_cache_path};
use crate::check::{CheckResult, run_checks_for_trigger};
use crate::config::{find_effective_config, locate_config_path};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    RunChecks {
        cwd: PathBuf,
        file_path: PathBuf,
        trigger: String,
        #[serde(default)]
        changed_lines: Option<Vec<(usize, usize)>>,
        /// The edit removed text at an unknown position (see `RunContext::unlocated_deletion`).
        #[serde(default)]
        unlocated_deletion: bool,
    },
    Ping,
    Shutdown,
}

/// The kibitzer version a daemon answers with. A daemon started before an upgrade keeps
/// running the old code, so the client compares this to its own and retires a mismatch.
const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    ok: bool,
    /// Absent from a pre-version daemon's replies, which therefore read as a mismatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    results: Option<Vec<CheckResult>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Per-user socket path so multiple users on a shared machine never collide, and so a
/// leftover socket from a previous login session doesn't get reused across reboots
/// unexpectedly (XDG_RUNTIME_DIR is normally tmpfs, reset on boot).
pub fn default_socket_path() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let user = std::env::var("USER").unwrap_or_else(|_| "kibitzer".to_string());
    base.join(format!("kibitzer-{user}.sock"))
}

/// Run the daemon in the foreground on `socket_path` until it receives a `Shutdown`
/// request or the process is killed. Callers that want it in the background are
/// expected to background it themselves (`kibitzer daemon &`, a systemd/launchd unit,
/// etc.) — the daemon does not self-detach. `run_checks_smart` is one such caller: its
/// `maybe_spawn_daemon` backgrounds this automatically when no daemon is reachable.
pub fn run_daemon(socket_path: &Path) -> Result<()> {
    // Held until the process exits: the kernel releases it on any death, so a crashed daemon
    // never blocks its successor, and two daemons can never both pass this point.
    let Some(_owner_lock) = acquire_owner_lock(socket_path)? else {
        eprintln!(
            "[kibitzer] another daemon already owns {}",
            socket_path.display()
        );
        return Ok(());
    };
    if !clear_unlocked_daemon(socket_path) {
        eprintln!(
            "[kibitzer] a daemon is still answering on {}",
            socket_path.display()
        );
        return Ok(());
    }
    // Only a socket nobody answers on is stale; the owner lock above rules out a live peer.
    std::fs::remove_file(socket_path).ok();
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("binding daemon socket at {}", socket_path.display()))?;
    eprintln!("[kibitzer] daemon listening on {}", socket_path.display());

    let cache_path = default_cache_path();
    let cache = Arc::new(Mutex::new(Cache::load(&cache_path)));

    for stream in listener.incoming() {
        let stream = stream?;
        let cache = Arc::clone(&cache);
        let cache_path = cache_path.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle_conn(stream, &cache, &cache_path) {
                eprintln!("[kibitzer] daemon connection error: {e}");
            }
        });
    }
    Ok(())
}

/// How long a new daemon waits for a retiring predecessor to release its lock and socket.
const PREDECESSOR_WAIT: Duration = Duration::from_secs(3);

fn owner_lock_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("lock")
}

fn open_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// The exclusive daemon lock, or `None` when a live same-version daemon owns it or a previous
/// owner did not exit within `PREDECESSOR_WAIT`.
fn acquire_owner_lock(socket_path: &Path) -> Result<Option<std::fs::File>> {
    let lock_path = owner_lock_path(socket_path);
    if let Some(dir) = lock_path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let file = open_lock_file(&lock_path)
        .with_context(|| format!("opening daemon lock {}", lock_path.display()))?;
    let deadline = std::time::Instant::now() + PREDECESSOR_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
        }
        let owner_is_current = request(socket_path, &Request::Ping)
            .is_some_and(|r| r.version.as_deref() == Some(DAEMON_VERSION));
        if owner_is_current || std::time::Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// With the owner lock held, anything still answering on the socket is a daemon that predates
/// the lock. Asks it to exit and waits; false means it stayed up and must not be displaced.
fn clear_unlocked_daemon(socket_path: &Path) -> bool {
    if request(socket_path, &Request::Ping).is_none() {
        return true;
    }
    shutdown_at(socket_path);
    let deadline = std::time::Instant::now() + PREDECESSOR_WAIT;
    while std::time::Instant::now() < deadline {
        if request(socket_path, &Request::Ping).is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn handle_conn(stream: UnixStream, cache: &Arc<Mutex<Cache>>, cache_path: &Path) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(Request::Ping) => Response {
                ok: true,
                version: Some(DAEMON_VERSION.to_string()),
                results: None,
                error: None,
            },
            Ok(Request::Shutdown) => {
                let ack = Response {
                    ok: true,
                    version: Some(DAEMON_VERSION.to_string()),
                    results: None,
                    error: None,
                };
                writeln!(writer, "{}", serde_json::to_string(&ack)?)?;
                writer.flush()?;
                std::process::exit(0);
            }
            Ok(Request::RunChecks {
                cwd,
                file_path,
                trigger,
                changed_lines,
                unlocated_deletion,
            }) => match handle_run_checks(
                &cwd,
                &file_path,
                &trigger,
                changed_lines.as_deref(),
                unlocated_deletion,
                cache,
                cache_path,
            ) {
                Ok(results) => Response {
                    ok: true,
                    version: Some(DAEMON_VERSION.to_string()),
                    results: Some(results),
                    error: None,
                },
                Err(e) => Response {
                    ok: false,
                    version: Some(DAEMON_VERSION.to_string()),
                    results: None,
                    error: Some(e.to_string()),
                },
            },
            Err(e) => Response {
                ok: false,
                version: Some(DAEMON_VERSION.to_string()),
                results: None,
                error: Some(format!("bad request: {e}")),
            },
        };
        writeln!(writer, "{}", serde_json::to_string(&response)?)?;
        writer.flush()?;
    }
    Ok(())
}

fn handle_run_checks(
    cwd: &Path,
    file_path: &Path,
    trigger: &str,
    changed_lines: Option<&[(usize, usize)]>,
    unlocated_deletion: bool,
    cache: &Arc<Mutex<Cache>>,
    cache_path: &Path,
) -> Result<Vec<CheckResult>> {
    // Fingerprinted before the first config or registry read, so an edit racing that read
    // can only make the stamp older than the data (a miss later), never newer (a stale hit).
    let config_path = locate_config_path(cwd);
    let before = Stamps::capture(
        file_path,
        &config_path,
        &crate::plugin::default_registry_path(),
    );
    let (config, repo_root) = find_effective_config(cwd)?;

    // Cached entries aren't keyed by changed_lines — only bypass the cache lookup when a
    // diff-aware caller actually passed ranges, so the common no-diff path keeps caching.
    if changed_lines.is_none()
        && let Some(cached) = lock_cache(cache).get(
            file_path,
            &config_path,
            &crate::plugin::default_registry_path(),
            trigger,
        )
    {
        return Ok(cached);
    }

    // Loaded once for this request's whole check loop, not once per check — see
    // `run_checks_for_trigger`'s doc comment.
    let registry = crate::plugin::Registry::load(&crate::plugin::default_registry_path());
    let mut run_ctx = crate::run_context::RunContext::load(&repo_root)?;
    run_ctx.unlocated_deletion = unlocated_deletion;
    let mut results = run_checks_for_trigger(
        &config.checks,
        trigger,
        &repo_root,
        file_path,
        changed_lines,
        &registry,
        &run_ctx,
    )?;

    {
        let mut guard = lock_cache(cache);
        guard.apply_grace(&mut results, file_path, trigger);
        // A scoped result only reflects the diffed ranges, not the whole file — writing it
        // to the cache would let a later unscoped (e.g. batch) request read back a partial
        // result as if it were a full-file one. `grace_pending` isn't affected by this
        // concern (it's not part of `entries`, the file-results cache `put` populates), so
        // `save` still needs to run unconditionally below to persist it.
        if changed_lines.is_none() {
            guard.put(before, trigger, results.clone());
        }
        // Persist regardless of `changed_lines`: without this, a diff-scoped (Edit-tool)
        // call under this daemon would still work correctly in-memory for as long as the
        // daemon stays up, but a daemon restart mid-sequence would silently lose
        // `apply_grace`'s escalation state, exactly like the no-daemon fallback below.
        let _ = guard.save(cache_path);
    }
    Ok(results)
}

/// A poisoned lock only means another connection thread panicked mid-request; the cache map
/// is still usable, and a daemon that stops caching for the rest of its life is worse.
fn lock_cache(cache: &Mutex<Cache>) -> std::sync::MutexGuard<'_, Cache> {
    cache.lock().unwrap_or_else(|e| e.into_inner())
}

fn connect(socket_path: &Path) -> Option<UnixStream> {
    let stream = UnixStream::connect(socket_path).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok()?;
    Some(stream)
}

/// True if a kibitzer daemon is listening and responds to a ping.
pub fn is_alive() -> bool {
    request(&default_socket_path(), &Request::Ping).is_some()
}

pub fn shutdown() -> bool {
    shutdown_at(&default_socket_path())
}

fn shutdown_at(socket_path: &Path) -> bool {
    request(socket_path, &Request::Shutdown).is_some()
}

fn request(socket_path: &Path, req: &Request) -> Option<Response> {
    let mut stream = connect(socket_path)?;
    let mut payload = serde_json::to_string(req).ok()?;
    payload.push('\n');
    stream.write_all(payload.as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

/// Ask the daemon to run checks for `file_path`/`trigger`, if one is reachable.
/// Returns `None` (rather than an error) when no daemon is running so callers can
/// transparently fall back to running the checks in-process.
pub fn try_run_checks_via_daemon(
    cwd: &Path,
    file_path: &Path,
    trigger: &str,
    changed_lines: Option<&[(usize, usize)]>,
    unlocated_deletion: bool,
) -> Option<Vec<CheckResult>> {
    try_run_checks_at(
        &default_socket_path(),
        cwd,
        file_path,
        trigger,
        changed_lines,
        unlocated_deletion,
    )
}

fn try_run_checks_at(
    socket_path: &Path,
    cwd: &Path,
    file_path: &Path,
    trigger: &str,
    changed_lines: Option<&[(usize, usize)]>,
    unlocated_deletion: bool,
) -> Option<Vec<CheckResult>> {
    let response = request(
        socket_path,
        &Request::RunChecks {
            cwd: cwd.to_path_buf(),
            file_path: file_path.to_path_buf(),
            trigger: trigger.to_string(),
            changed_lines: changed_lines.map(|r| r.to_vec()),
            unlocated_deletion,
        },
    )?;
    if response.version.as_deref() != Some(DAEMON_VERSION) {
        retire_stale_daemon(socket_path);
        return None;
    }
    if response.ok { response.results } else { None }
}

/// A daemon from another kibitzer version (or from before replies carried one) answers with
/// code that predates this binary, so its results are discarded and it is asked to exit. The
/// spawn debounce stays in force: clearing it let two kibitzer versions on one machine retire
/// and respawn each other on every hook.
fn retire_stale_daemon(socket_path: &Path) {
    shutdown_at(socket_path);
}

/// Minimum time between background-spawn attempts, so a daemon that keeps failing to
/// start (e.g. permission denied binding the socket) doesn't get re-spawned on every
/// single hook invocation.
const SPAWN_RETRY_INTERVAL: Duration = Duration::from_secs(10);

/// True when `marker_modified` is recent enough that another spawn attempt should be
/// skipped. Split out from `maybe_spawn_daemon` so the debounce window can be tested
/// without actually spawning a process or touching the real socket path.
fn spawn_is_debounced(marker_modified: Option<SystemTime>, now: SystemTime) -> bool {
    marker_modified
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|elapsed| elapsed < SPAWN_RETRY_INTERVAL)
}

/// Best-effort: spawn `kibitzer daemon start` detached in the background when no daemon
/// is reachable, so caching kicks in for later calls without the user having to remember
/// to start one themselves. Errors are swallowed — the caller already has an in-process
/// fallback, so a failed spawn just means the next call falls back the same way.
///
/// Set `KIBITZER_NO_AUTO_DAEMON` (any value) to disable — `tests/hook_contract.rs` does,
/// since it exercises the deterministic no-daemon fallback path itself and a real daemon
/// racing to life mid-test would otherwise answer later calls instead.
fn maybe_spawn_daemon() {
    if std::env::var_os("KIBITZER_NO_AUTO_DAEMON").is_some() {
        return;
    }
    let socket_path = default_socket_path();
    let marker = socket_path.with_extension("spawn-attempt");
    // Non-blocking exclusive gate: a hook that loses it knows another is spawning right now.
    // The marker mtime is read and written only under the gate, so concurrent hooks cannot
    // both pass the debounce (an earlier remove-then-create_new marker let one hook delete
    // another's fresh claim).
    let Ok(gate) = open_lock_file(&socket_path.with_extension("spawn-lock")) else {
        return;
    };
    if gate.try_lock().is_err() {
        return;
    }
    let marker_modified = std::fs::metadata(&marker)
        .ok()
        .and_then(|m| m.modified().ok());
    if spawn_is_debounced(marker_modified, SystemTime::now()) {
        return;
    }
    if std::fs::write(&marker, DAEMON_VERSION).is_err() {
        return;
    }
    spawn_detached_daemon();
}

/// The actual `kibitzer daemon start` spawn, split out of `maybe_spawn_daemon` so that
/// function's debounce/marker logic stays readable on its own.
fn spawn_detached_daemon() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // `process_group(0)` detaches the child into its own session/process group so it
    // outlives this short-lived hook process even if the hook's whole group gets
    // signaled (e.g. on a hook timeout) — otherwise the "background" daemon would die
    // right along with the hook invocation that spawned it.
    use std::os::unix::process::CommandExt;
    let _ = std::process::Command::new(exe)
        .args(["daemon", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}

/// Run checks via the daemon if one is up, otherwise run them directly in-process
/// (uncached) and kick off a background daemon (see `maybe_spawn_daemon`) so later calls
/// don't pay the uncached cost too. This is the entry point `hook`/`run` should use
/// instead of calling `find_config` + `run_checks_for_trigger` themselves.
pub fn run_checks_smart(
    cwd: &Path,
    file_path: &Path,
    trigger: &str,
    changed_lines: Option<&[(usize, usize)]>,
    unlocated_deletion: bool,
) -> Result<Vec<CheckResult>> {
    if let Some(results) =
        try_run_checks_via_daemon(cwd, file_path, trigger, changed_lines, unlocated_deletion)
    {
        return Ok(results);
    }
    maybe_spawn_daemon();
    run_uncached(cwd, file_path, trigger, changed_lines, unlocated_deletion)
}

/// The no-daemon-reachable path of `run_checks_smart`: run checks in-process and persist
/// results/grace-state to the on-disk cache directly, split out so `run_checks_smart`
/// itself stays a short dispatch between the daemon and this fallback.
fn run_uncached(
    cwd: &Path,
    file_path: &Path,
    trigger: &str,
    changed_lines: Option<&[(usize, usize)]>,
    unlocated_deletion: bool,
) -> Result<Vec<CheckResult>> {
    run_uncached_with_cache(
        cwd,
        file_path,
        trigger,
        changed_lines,
        unlocated_deletion,
        &default_cache_path(),
    )
}

fn run_uncached_with_cache(
    cwd: &Path,
    file_path: &Path,
    trigger: &str,
    changed_lines: Option<&[(usize, usize)]>,
    unlocated_deletion: bool,
    cache_path: &Path,
) -> Result<Vec<CheckResult>> {
    // Stamped before the first config read and before the checks run; see `Stamps`.
    let before = Stamps::capture(
        file_path,
        &locate_config_path(cwd),
        &crate::plugin::default_registry_path(),
    );
    let (config, repo_root) = find_effective_config(cwd)?;
    let registry = crate::plugin::Registry::load(&crate::plugin::default_registry_path());
    let mut run_ctx = crate::run_context::RunContext::load(&repo_root)?;
    run_ctx.unlocated_deletion = unlocated_deletion;
    let mut results = run_checks_for_trigger(
        &config.checks,
        trigger,
        &repo_root,
        file_path,
        changed_lines,
        &registry,
        &run_ctx,
    )?;

    let mut cache = Cache::load(cache_path);
    cache.apply_grace(&mut results, file_path, trigger);
    // See handle_run_checks: don't let a diff-scoped partial result overwrite the
    // full-file cache entry. `grace_pending` isn't part of that cache entry, so it still
    // needs `save` to run unconditionally below.
    if changed_lines.is_none() {
        cache.put(before, trigger, results.clone());
    }
    // Persist regardless of `changed_lines`: this is a fresh `Cache::load` per process
    // (no daemon running), so without an unconditional save here, `apply_grace`'s
    // escalation state for a diff-scoped (Edit-tool) call never reaches disk at all — every
    // subsequent `kibitzer hook` invocation would see an empty `grace_pending` and treat a
    // still-failing check as a fresh "first occurrence" forever, so a Blocking check could
    // never actually escalate back to blocking without a daemon.
    let _ = cache.save(cache_path);

    Ok(results)
}

#[cfg(test)]
mod spawn_debounce_tests {
    use super::*;

    #[test]
    fn debounces_a_recent_attempt() {
        let now = SystemTime::now();
        assert!(spawn_is_debounced(Some(now), now));
    }

    #[test]
    fn allows_a_retry_once_the_interval_elapses() {
        let now = SystemTime::now();
        let stale = now - SPAWN_RETRY_INTERVAL - Duration::from_secs(1);
        assert!(!spawn_is_debounced(Some(stale), now));
    }

    #[test]
    fn allows_the_first_attempt() {
        assert!(!spawn_is_debounced(None, SystemTime::now()));
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod inline_cache_tests {
    use super::*;

    #[test]
    fn handle_run_checks_should_ReturnFirstAnchor_When_ResultServedFromCache() {
        let _guard = crate::plugin::XDG_DATA_HOME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("kibitzer-daemon-inline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".kibitzer")).unwrap();
        std::fs::write(dir.join(".kibitzer/inspect.json"), r#"{"checks": []}"#).unwrap();
        let file = dir.join("main.go");
        std::fs::write(
            &file,
            "package main\n\nfunc f(b bool) {\n\tif b {\n\t\tprintln(\"x\")\n\t}\n}\n",
        )
        .unwrap();
        let cache = Arc::new(Mutex::new(Cache::default()));
        let cache_path = dir.join("cache.json");

        let anchor = |results: &[CheckResult]| {
            results
                .iter()
                .find(|r| r.check_name == "syntax-rules-go")
                .and_then(|r| {
                    r.inline
                        .first_anchor()
                        .map(|(rule, line)| (rule.as_str().to_string(), line.get()))
                })
        };
        let first =
            handle_run_checks(&dir, &file, "batch", None, false, &cache, &cache_path).unwrap();
        assert_eq!(anchor(&first), Some(("flag-argument".to_string(), 3)));
        // Unchanged file and config: the second call must be a cache hit that still carries the anchor.
        assert!(
            cache
                .lock()
                .unwrap()
                .get(
                    &file,
                    &crate::config::resolve_config_path(&dir),
                    &crate::plugin::default_registry_path(),
                    "batch"
                )
                .is_some(),
            "second request would not be a cache hit"
        );
        let second =
            handle_run_checks(&dir, &file, "batch", None, false, &cache, &cache_path).unwrap();
        assert_eq!(anchor(&second), anchor(&first));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod stale_daemon_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-daemon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A fake daemon that answers up to two connections with `reply_for(line)`, and gives up
    /// after `FAKE_DAEMON_DEADLINE` so a client that never connects fails the test instead of
    /// hanging `join`.
    fn fake_daemon(
        socket: &Path,
        reply_for: fn(&str) -> String,
    ) -> (std::thread::JoinHandle<()>, Arc<AtomicBool>) {
        const FAKE_DAEMON_DEADLINE: Duration = Duration::from_secs(5);
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let saw_shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&saw_shutdown);
        let handle = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + FAKE_DAEMON_DEADLINE;
            let mut served = 0;
            while served < 2 && std::time::Instant::now() < deadline {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                // Accepted sockets inherit non-blocking mode on some platforms.
                // Best effort: a peer that already hung up makes these fail with EINVAL.
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if let Ok(clone) = stream.try_clone() {
                    let _ = BufReader::new(clone).read_line(&mut line);
                }
                if line.contains("\"shutdown\"") {
                    flag.store(true, Ordering::SeqCst);
                }
                let mut writer = stream;
                let _ = writeln!(writer, "{}", reply_for(&line));
                served += 1;
            }
        });
        (handle, saw_shutdown)
    }

    fn old_style_reply(_line: &str) -> String {
        r#"{"ok":true,"results":[]}"#.to_string()
    }

    fn current_reply(_line: &str) -> String {
        format!(r#"{{"ok":true,"version":"{DAEMON_VERSION}","results":[]}}"#)
    }

    #[test]
    fn client_should_TreatDaemonAsDeadAndShutItDown_When_ReplyHasNoVersion() {
        let dir = temp_dir("old-style");
        let socket = dir.join("k.sock");
        std::fs::write(socket.with_extension("spawn-attempt"), "").unwrap();
        let (handle, saw_shutdown) = fake_daemon(&socket, old_style_reply);
        let out = try_run_checks_at(&socket, &dir, &dir.join("f.go"), "batch", None, false);
        handle.join().unwrap();
        assert!(out.is_none(), "an old daemon's results must be discarded");
        assert!(
            saw_shutdown.load(Ordering::SeqCst),
            "old daemon must be asked to exit"
        );
        assert!(
            socket.with_extension("spawn-attempt").exists(),
            "debounce must survive a retire so versions cannot respawn each other per hook"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn client_should_TreatDaemonAsDead_When_VersionDiffers() {
        let dir = temp_dir("other-version");
        let socket = dir.join("k.sock");
        let (handle, saw_shutdown) = fake_daemon(&socket, |_| {
            r#"{"ok":true,"version":"0.0.0-old","results":[]}"#.to_string()
        });
        let out = try_run_checks_at(&socket, &dir, &dir.join("f.go"), "batch", None, false);
        handle.join().unwrap();
        assert!(out.is_none());
        assert!(saw_shutdown.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn client_should_UseResults_When_VersionMatches() {
        let dir = temp_dir("same-version");
        let socket = dir.join("k.sock");
        let (handle, saw_shutdown) = fake_daemon(&socket, current_reply);
        let out = try_run_checks_at(&socket, &dir, &dir.join("f.go"), "batch", None, false);
        // The fake accepts two connections; a matching daemon gets only one.
        let _ = UnixStream::connect(&socket);
        handle.join().unwrap();
        assert_eq!(out.map(|r| r.len()), Some(0));
        assert!(!saw_shutdown.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ping_reply_should_CarryVersion_When_DaemonAnswers() {
        let dir = temp_dir("ping");
        let socket = dir.join("k.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let cache = Arc::new(Mutex::new(Cache::default()));
        let cache_path = dir.join("cache.json");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_conn(stream, &cache, &cache_path).unwrap();
        });
        let response = request(&socket, &Request::Ping).unwrap();
        assert_eq!(response.version.as_deref(), Some(DAEMON_VERSION));
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod stale_result_tests {
    use super::*;

    /// Config whose check touches `started` before sleeping, so a test waits on that marker
    /// instead of guessing how long the slow run needs to begin.
    fn write_slow_on_bad_config(dir: &Path) {
        let started = dir.join("started");
        std::fs::write(
            dir.join(".kibitzer/inspect.json"),
            format!(
                r#"{{"checks": [{{"name": "no-bad", "command": "if grep -q BAD {{file}}; then touch {}; sleep 2; exit 1; fi", "severity": "advisory", "message": "bad"}}]}}"#,
                started.display()
            ),
        )
        .unwrap();
    }

    fn wait_for_slow_check_to_start(dir: &Path) {
        let started = dir.join("started");
        for _ in 0..500 {
            if started.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("slow check never started");
    }

    /// A shell check that sleeps only on BAD content: the slow request on old content is the
    /// one that finishes last, after a fast request already cached the new content's result.
    #[test]
    fn handle_run_checks_should_NotCacheStaleResult_When_SlowRunFinishesAfterFileChanged() {
        let _guard = crate::plugin::XDG_DATA_HOME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("kibitzer-daemon-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".kibitzer")).unwrap();
        write_slow_on_bad_config(&dir);
        let file = dir.join("notes.txt");
        std::fs::write(&file, "BAD\n").unwrap();
        let cache = Arc::new(Mutex::new(Cache::default()));
        let cache_path = dir.join("cache.json");

        let slow = {
            let (dir, file, cache, cache_path) = (
                dir.clone(),
                file.clone(),
                Arc::clone(&cache),
                cache_path.clone(),
            );
            std::thread::spawn(move || {
                handle_run_checks(&dir, &file, "batch", None, false, &cache, &cache_path).unwrap()
            })
        };
        wait_for_slow_check_to_start(&dir);
        std::fs::write(&file, "GOOD content\n").unwrap();
        let fast =
            handle_run_checks(&dir, &file, "batch", None, false, &cache, &cache_path).unwrap();
        assert!(fast.iter().all(|r| r.passed), "{fast:?}");
        let stale = slow.join().unwrap();
        assert!(
            stale.iter().any(|r| !r.passed),
            "slow run saw the BAD content"
        );

        let again =
            handle_run_checks(&dir, &file, "batch", None, false, &cache, &cache_path).unwrap();
        assert!(
            again.iter().all(|r| r.passed),
            "stale BAD result was served for the new content: {again:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The no-daemon fallback has the same stale-result hazard as the daemon path.
    #[test]
    fn run_uncached_should_NotCacheStaleResult_When_SlowRunFinishesAfterFileChanged() {
        let _guard = crate::plugin::XDG_DATA_HOME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("kibitzer-uncached-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".kibitzer")).unwrap();
        write_slow_on_bad_config(&dir);
        let file = dir.join("notes.txt");
        std::fs::write(&file, "BAD\n").unwrap();
        let cache_path = dir.join("cache").join("cache.json");

        let slow = {
            let (dir, file, cache_path) = (dir.clone(), file.clone(), cache_path.clone());
            std::thread::spawn(move || {
                run_uncached_with_cache(&dir, &file, "batch", None, false, &cache_path).unwrap()
            })
        };
        wait_for_slow_check_to_start(&dir);
        std::fs::write(&file, "GOOD content\n").unwrap();
        let fast = run_uncached_with_cache(&dir, &file, "batch", None, false, &cache_path).unwrap();
        assert!(fast.iter().all(|r| r.passed), "{fast:?}");
        let stale = slow.join().unwrap();
        assert!(stale.iter().any(|r| !r.passed), "slow run saw BAD content");

        let cached = Cache::load(&cache_path)
            .get(
                &file,
                &crate::config::resolve_config_path(&dir),
                &crate::plugin::default_registry_path(),
                "batch",
            )
            .expect("fast result should be cached");
        assert!(
            cached.iter().all(|r| r.passed),
            "stale BAD cached: {cached:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn handle_run_checks_should_KeepServing_When_CacheMutexIsPoisoned() {
        let _guard = crate::plugin::XDG_DATA_HOME_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("kibitzer-daemon-poison-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".kibitzer")).unwrap();
        std::fs::write(dir.join(".kibitzer/inspect.json"), r#"{"checks": []}"#).unwrap();
        let file = dir.join("main.go");
        std::fs::write(&file, "package main\n").unwrap();
        let cache = Arc::new(Mutex::new(Cache::default()));
        let poisoner = Arc::clone(&cache);
        let _ = std::thread::spawn(move || {
            let _held = poisoner.lock().unwrap();
            panic!("poison the cache lock");
        })
        .join();
        assert!(cache.lock().is_err(), "setup: mutex must be poisoned");
        let out = handle_run_checks(
            &dir,
            &file,
            "batch",
            None,
            false,
            &cache,
            &dir.join("cache.json"),
        );
        assert!(out.is_ok(), "{out:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
