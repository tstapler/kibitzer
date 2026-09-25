use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cache::{Cache, default_cache_path};
use crate::check::{CheckResult, run_checks_for_trigger};
use crate::config::{find_effective_config, resolve_config_path};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    RunChecks {
        cwd: PathBuf,
        file_path: PathBuf,
        trigger: String,
        #[serde(default)]
        changed_lines: Option<Vec<(usize, usize)>>,
    },
    Ping,
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    ok: bool,
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
    if socket_path.exists() {
        // A stale socket from a crashed prior daemon; a live daemon would have failed
        // to start in the first place (see `is_alive` check callers should do first).
        std::fs::remove_file(socket_path).ok();
    }
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
                results: None,
                error: None,
            },
            Ok(Request::Shutdown) => {
                let ack = Response {
                    ok: true,
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
            }) => match handle_run_checks(
                &cwd,
                &file_path,
                &trigger,
                changed_lines.as_deref(),
                cache,
                cache_path,
            ) {
                Ok(results) => Response {
                    ok: true,
                    results: Some(results),
                    error: None,
                },
                Err(e) => Response {
                    ok: false,
                    results: None,
                    error: Some(e.to_string()),
                },
            },
            Err(e) => Response {
                ok: false,
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
    cache: &Arc<Mutex<Cache>>,
    cache_path: &Path,
) -> Result<Vec<CheckResult>> {
    let (config, repo_root) = find_effective_config(cwd)?;
    let config_path = resolve_config_path(&repo_root);

    // Cached entries aren't keyed by changed_lines — only bypass the cache lookup when a
    // diff-aware caller actually passed ranges, so the common no-diff path keeps caching.
    if changed_lines.is_none()
        && let Ok(guard) = cache.lock()
        && let Some(cached) = guard.get(
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
    let accepted = crate::accepted_findings::find_accepted_findings(&repo_root)?;
    let mut results = run_checks_for_trigger(
        &config.checks,
        trigger,
        &repo_root,
        file_path,
        changed_lines,
        &registry,
        &accepted,
    )?;

    if let Ok(mut guard) = cache.lock() {
        guard.apply_grace(&mut results, file_path, trigger);
        // A scoped result only reflects the diffed ranges, not the whole file — writing it
        // to the cache would let a later unscoped (e.g. batch) request read back a partial
        // result as if it were a full-file one. `grace_pending` isn't affected by this
        // concern (it's not part of `entries`, the file-results cache `put` populates), so
        // `save` still needs to run unconditionally below to persist it.
        if changed_lines.is_none() {
            guard.put(
                file_path,
                &config_path,
                &crate::plugin::default_registry_path(),
                trigger,
                results.clone(),
            );
        }
        // Persist regardless of `changed_lines`: without this, a diff-scoped (Edit-tool)
        // call under this daemon would still work correctly in-memory for as long as the
        // daemon stays up, but a daemon restart mid-sequence would silently lose
        // `apply_grace`'s escalation state, exactly like the no-daemon fallback below.
        let _ = guard.save(cache_path);
    }
    Ok(results)
}

fn connect() -> Option<UnixStream> {
    let stream = UnixStream::connect(default_socket_path()).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok()?;
    Some(stream)
}

/// True if a kibitzer daemon is listening and responds to a ping.
pub fn is_alive() -> bool {
    request(&Request::Ping).is_some()
}

pub fn shutdown() -> bool {
    request(&Request::Shutdown).is_some()
}

fn request(req: &Request) -> Option<Response> {
    let mut stream = connect()?;
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
) -> Option<Vec<CheckResult>> {
    let response = request(&Request::RunChecks {
        cwd: cwd.to_path_buf(),
        file_path: file_path.to_path_buf(),
        trigger: trigger.to_string(),
        changed_lines: changed_lines.map(|r| r.to_vec()),
    })?;
    if response.ok { response.results } else { None }
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
    let marker = default_socket_path().with_extension("spawn-attempt");
    let marker_modified = std::fs::metadata(&marker)
        .ok()
        .and_then(|m| m.modified().ok());
    if spawn_is_debounced(marker_modified, SystemTime::now()) {
        return;
    }
    // Claim the marker atomically rather than read-mtime-then-write: two `hook`
    // processes racing through the staleness check above could otherwise both pass
    // it and both spawn a daemon. `create_new` makes the loser's claim fail instead,
    // so at most one spawns per debounce window even under a race.
    let _ = std::fs::remove_file(&marker);
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .is_err()
    {
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
) -> Result<Vec<CheckResult>> {
    if let Some(results) = try_run_checks_via_daemon(cwd, file_path, trigger, changed_lines) {
        return Ok(results);
    }
    maybe_spawn_daemon();
    run_uncached(cwd, file_path, trigger, changed_lines)
}

/// The no-daemon-reachable path of `run_checks_smart`: run checks in-process and persist
/// results/grace-state to the on-disk cache directly, split out so `run_checks_smart`
/// itself stays a short dispatch between the daemon and this fallback.
fn run_uncached(
    cwd: &Path,
    file_path: &Path,
    trigger: &str,
    changed_lines: Option<&[(usize, usize)]>,
) -> Result<Vec<CheckResult>> {
    let (config, repo_root) = find_effective_config(cwd)?;
    let config_path = resolve_config_path(&repo_root);
    let registry = crate::plugin::Registry::load(&crate::plugin::default_registry_path());
    let accepted = crate::accepted_findings::find_accepted_findings(&repo_root)?;
    let mut results = run_checks_for_trigger(
        &config.checks,
        trigger,
        &repo_root,
        file_path,
        changed_lines,
        &registry,
        &accepted,
    )?;

    let cache_path = default_cache_path();
    let mut cache = Cache::load(&cache_path);
    cache.apply_grace(&mut results, file_path, trigger);
    // See handle_run_checks: don't let a diff-scoped partial result overwrite the
    // full-file cache entry. `grace_pending` isn't part of that cache entry, so it still
    // needs `save` to run unconditionally below.
    if changed_lines.is_none() {
        cache.put(
            file_path,
            &config_path,
            &crate::plugin::default_registry_path(),
            trigger,
            results.clone(),
        );
    }
    // Persist regardless of `changed_lines`: this is a fresh `Cache::load` per process
    // (no daemon running), so without an unconditional save here, `apply_grace`'s
    // escalation state for a diff-scoped (Edit-tool) call never reaches disk at all — every
    // subsequent `kibitzer hook` invocation would see an empty `grace_pending` and treat a
    // still-failing check as a fresh "first occurrence" forever, so a Blocking check could
    // never actually escalate back to blocking without a daemon.
    let _ = cache.save(&cache_path);

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
