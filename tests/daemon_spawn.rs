//! Concurrent hooks against a missing or stale daemon must leave at most one daemon bound to
//! the socket. Runs a private copy of the binary under a scratch dir so `ps` can count exactly
//! the daemons this test started, with a scratch `XDG_RUNTIME_DIR` and a hard timeout.
#![allow(non_snake_case)]
#![cfg(unix)]

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Scratch {
    root: PathBuf,
    exe: PathBuf,
    runtime: PathBuf,
    repo: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = PathBuf::from("/tmp").join(format!("ks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // `current_exe` reports the canonical path (macOS /var -> /private/var), which `ps` shows.
        let root = root.canonicalize().unwrap();
        let runtime = root.join("rt");
        let repo = root.join("repo");
        std::fs::create_dir_all(&runtime).unwrap();
        // The daemon only trusts a runtime directory nobody else can reach.
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::create_dir_all(root.join("tmp")).unwrap();
        std::fs::create_dir_all(repo.join(".kibitzer")).unwrap();
        std::fs::write(repo.join(".kibitzer/inspect.json"), r#"{"checks": []}"#).unwrap();
        std::fs::write(repo.join("a.txt"), "x\n").unwrap();
        let exe = root.join("kibitzer-under-test");
        std::fs::hard_link(env!("CARGO_BIN_EXE_kibitzer"), &exe)
            .or_else(|_| std::fs::copy(env!("CARGO_BIN_EXE_kibitzer"), &exe).map(|_| ()))
            .unwrap();
        Self {
            root,
            exe,
            runtime,
            repo,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("USER", "spawntest")
            .env("TMPDIR", self.root.join("tmp"))
            .current_dir(&self.repo);
        cmd
    }

    fn socket(&self) -> PathBuf {
        self.runtime.join("kibitzer-spawntest.sock")
    }

    fn hook(&self) {
        let payload = serde_json::json!({
            "cwd": self.repo,
            "hook_event_name": "PostToolUse",
            "tool_input": {"file_path": self.repo.join("a.txt"), "content": "x\n"}
        });
        // A file, not a pipe: concurrent spawns leak pipe ends into each other's children
        // on macOS, so a hook would never see EOF on its stdin.
        let input = self
            .root
            .join(format!("hook-{:?}.json", std::thread::current().id()));
        std::fs::write(&input, payload.to_string()).unwrap();
        self.command()
            .arg("hook")
            .stdin(std::fs::File::open(&input).unwrap())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
    }

    /// Pids of live `kibitzer daemon start` processes running this scratch's private binary.
    fn daemon_pids(&self) -> HashSet<u32> {
        daemon_pids_of(&self.exe)
    }

    fn burst(&self, hooks: usize, parallel: usize) -> HashSet<u32> {
        let seen = Arc::new(Mutex::new(HashSet::new()));
        let done = Arc::new(AtomicBool::new(false));
        let sampler = {
            let (seen, done) = (Arc::clone(&seen), Arc::clone(&done));
            let exe = self.exe.clone();
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    seen.lock().unwrap().extend(daemon_pids_of(&exe));
                    std::thread::sleep(Duration::from_millis(15));
                }
            })
        };
        let next = Arc::new(Mutex::new(0usize));
        std::thread::scope(|scope| {
            for _ in 0..parallel {
                scope.spawn(|| {
                    loop {
                        {
                            let mut n = next.lock().unwrap();
                            if *n >= hooks {
                                return;
                            }
                            *n += 1;
                        }
                        self.hook();
                    }
                });
            }
        });
        done.store(true, Ordering::SeqCst);
        sampler.join().unwrap();
        seen.lock().unwrap().clone()
    }

    /// Waits for daemon processes to settle, then returns the survivors.
    fn settled_daemons(&self) -> HashSet<u32> {
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut last = self.daemon_pids();
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(400));
            let now = self.daemon_pids();
            if now == last {
                break;
            }
            last = now;
        }
        last
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = self.command().args(["daemon", "stop"]).output();
        std::thread::sleep(Duration::from_millis(300));
        for pid in self.daemon_pids() {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn daemon_pids_of(exe: &Path) -> HashSet<u32> {
    let out = Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .unwrap();
    let needle = format!("{} daemon start", exe.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.contains(&needle))
        .filter_map(|l| l.split_whitespace().next()?.parse().ok())
        .collect()
}

/// A listener that answers like a pre-version daemon (no `version`) and exits on `shutdown`.
fn fake_old_daemon(socket: &Path, honor_shutdown: bool) -> std::thread::JoinHandle<()> {
    let listener = UnixListener::bind(socket).unwrap();
    let socket = socket.to_path_buf();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let mut line = String::new();
            let _ = BufReader::new(stream.try_clone().unwrap()).read_line(&mut line);
            let mut writer = stream;
            let _ = writeln!(writer, r#"{{"ok":true,"results":[]}}"#);
            if honor_shutdown && line.contains("\"shutdown\"") {
                let _ = std::fs::remove_file(&socket);
                return;
            }
        }
    })
}

#[test]
fn hooks_should_LeaveExactlyOneDaemon_When_BurstStartsWithNoDaemon() {
    let s = Scratch::new("cold");
    let seen = s.burst(40, 20);
    let alive = s.settled_daemons();
    assert_eq!(alive.len(), 1, "alive: {alive:?}, ever seen: {seen:?}");
    assert!(seen.len() <= 4, "unbounded spawning: {seen:?}");
    let status = s.command().args(["daemon", "status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("is running"));
}

#[test]
fn hooks_should_LeaveAtMostOneDaemon_When_BurstMeetsStaleDaemon() {
    let s = Scratch::new("stale");
    let fake = fake_old_daemon(&s.socket(), true);
    let seen = s.burst(40, 20);
    let alive = s.settled_daemons();
    assert!(
        alive.len() <= 1,
        "orphaned daemons: {alive:?}, seen: {seen:?}"
    );
    assert!(seen.len() <= 4, "unbounded spawning: {seen:?}");
    drop(fake);
}

#[test]
fn run_daemon_should_ExitWithoutStealingSocket_When_AnotherDaemonOwnsIt() {
    let s = Scratch::new("second");
    let first = s
        .command()
        .args(["daemon", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !s.socket().exists() {
        assert!(Instant::now() < deadline, "first daemon never bound");
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut second = s
        .command()
        .args(["daemon", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let status = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(10) {
            let _ = second.kill();
            panic!("second daemon kept running next to a live one");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(status.success(), "{status:?}");
    let ping = s.command().args(["daemon", "status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&ping.stdout).contains("is running"));
    let mut first = first;
    let _ = s.command().args(["daemon", "stop"]).output();
    let _ = first.wait();
}

#[test]
fn hooks_should_NotStealSocketOrStorm_When_StaleDaemonIgnoresShutdown() {
    let s = Scratch::new("stubborn");
    let _fake = fake_old_daemon(&s.socket(), false);
    let seen = s.burst(40, 20);
    // A daemon that finds a stubborn one waits out its predecessor window (3s) before giving up.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut alive = s.daemon_pids();
    while !alive.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
        alive = s.daemon_pids();
    }
    assert!(alive.is_empty(), "a daemon displaced a live one: {alive:?}");
    assert!(seen.len() <= 4, "unbounded spawning: {seen:?}");
    let ping = s.command().args(["daemon", "status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&ping.stdout).contains("is running"));
}

fn start_daemon(s: &Scratch) -> (std::process::Child, u32) {
    let child = s
        .command()
        .args(["daemon", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !s.socket().exists() {
        assert!(Instant::now() < deadline, "daemon never bound");
        std::thread::sleep(Duration::from_millis(20));
    }
    let pid = child.id();
    // Binding is not readiness: stop only a daemon that is serving, so the lock holds its pid and a
    // SIGSTOP cannot land before the accept loop exists.
    wait_for("the daemon to answer a ping", READY_LIMIT, || {
        String::from_utf8_lossy(
            &s.command()
                .args(["daemon", "status"])
                .output()
                .unwrap()
                .stdout,
        )
        .contains("is running")
    });
    (child, pid)
}

/// Generous because the suite runs next to cargo builds; a real hang is unbounded, not slow.
const READY_LIMIT: Duration = Duration::from_secs(60);
const BOUND: Duration = Duration::from_secs(20);

fn signal(pid: u32, sig: &str) {
    let _ = Command::new("kill").args([sig, &pid.to_string()]).status();
}

/// SIGSTOP and wait until the kernel reports the process stopped, so the test starts from the
/// wedged state instead of racing the signal's delivery.
fn stop_and_confirm(pid: u32) {
    signal(pid, "-STOP");
    wait_for("the daemon to stop", READY_LIMIT, || {
        let out = Command::new("ps")
            .args(["-o", "state=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim_start()
            .starts_with('T')
    });
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn hook_should_FallBackWithinBound_And_ReplaceDaemon_When_HolderIsStopped() {
    let s = Scratch::new("wedged-hook");
    let (mut child, pid) = start_daemon(&s);
    stop_and_confirm(pid);
    let started = Instant::now();
    s.hook();
    let first = started.elapsed();
    assert!(first < BOUND, "hook blocked {first:?} on a stopped daemon");
    // Hooks keep flowing quickly while a successor displaces the stopped holder.
    let mut slowest = Duration::ZERO;
    wait_for("a replacement daemon", Duration::from_secs(90), || {
        let t = Instant::now();
        s.hook();
        slowest = slowest.max(t.elapsed());
        // `try_wait` reaps the displaced child; a zombie would still answer `kill -0`.
        child.try_wait().unwrap().is_some() && s.daemon_pids().iter().any(|p| *p != pid)
    });
    assert!(slowest < BOUND, "a hook blocked {slowest:?}");
    let status = s.command().args(["daemon", "status"]).output().unwrap();
    assert!(String::from_utf8_lossy(&status.stdout).contains("is running"));
    assert!(
        s.daemon_pids().len() <= 1,
        "unbounded spawns: {:?}",
        s.daemon_pids()
    );
}

#[test]
fn daemon_stop_should_TerminateStoppedDaemon_Within_Bound() {
    let s = Scratch::new("wedged-stop");
    let (mut child, pid) = start_daemon(&s);
    stop_and_confirm(pid);
    let started = Instant::now();
    let out = s.command().args(["daemon", "stop"]).output().unwrap();
    let took = started.elapsed();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(took < BOUND, "daemon stop blocked {took:?}");
    assert!(stdout.contains("terminated"), "{stdout}");
    let _ = child.wait();
    assert!(!process_alive(pid));
}

#[test]
fn daemon_stop_should_NotSignalAnything_When_SocketIsSilentButLockIsFree() {
    let s = Scratch::new("silent-socket");
    let listener = UnixListener::bind(s.socket()).unwrap();
    let mut bystander = Command::new("sleep").arg("30").spawn().unwrap();
    // Names an unrelated process in a lock nobody holds, as a recycled pid would.
    std::fs::write(
        s.runtime.join("kibitzer-spawntest.lock"),
        bystander.id().to_string(),
    )
    .unwrap();
    let started = Instant::now();
    let out = s.command().args(["daemon", "stop"]).output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(8));
    assert!(!String::from_utf8_lossy(&out.stdout).contains("terminated"));
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "bystander was signalled"
    );
    let _ = bystander.kill();
    let _ = bystander.wait();
    drop(listener);
}

/// Runs `cmd` to completion, killing the child it started if it outlives `limit`.
fn output_within(mut cmd: Command, limit: Duration) -> std::process::Output {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + limit;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command outlived {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.wait_with_output().unwrap()
}

fn untrusted_leftovers(s: &Scratch) -> Vec<String> {
    [s.root.join("tmp"), s.runtime.clone(), PathBuf::from("/tmp")]
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("kibitzer-untrusted"))
        .collect()
}

fn make_runtime_loose(s: &Scratch) {
    // Writable by others: the one mode a user-provided runtime directory is refused for.
    std::fs::set_permissions(&s.runtime, std::fs::Permissions::from_mode(0o777)).unwrap();
}

fn fallback_dir(s: &Scratch) -> PathBuf {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    s.root.join("tmp").join(format!("kibitzer-{uid}"))
}

/// Plants a symlink where kibitzer's private fallback directory would go, so no directory at all
/// is trustworthy. Returns the directory the link points at.
fn make_fallback_untrusted(s: &Scratch) -> PathBuf {
    let elsewhere = s.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, fallback_dir(s)).unwrap();
    elsewhere
}

#[test]
fn daemon_start_should_Refuse_When_NoRuntimeDirIsTrustworthy() {
    let s = Scratch::new("untrusted-start");
    make_runtime_loose(&s);
    make_fallback_untrusted(&s);
    let mut cmd = s.command();
    cmd.args(["daemon", "start"]);
    let out = output_within(cmd, Duration::from_secs(15));
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("runtime dir untrusted"), "{stderr}");
    assert!(
        stderr.contains(&s.runtime.display().to_string()),
        "the refused XDG_RUNTIME_DIR is named: {stderr}"
    );
    assert!(stderr.contains("0777"), "{stderr}");
    assert!(s.daemon_pids().is_empty());
    assert_eq!(untrusted_leftovers(&s), Vec::<String>::new());
    // The directory was refused, not repaired.
    let mode = std::fs::metadata(&s.runtime).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o777);
}

#[test]
fn hooks_should_UsePrivateFallbackDir_And_NoteItOnce_When_XdgDirIsWritableByOthers() {
    let s = Scratch::new("fallback-hook");
    make_runtime_loose(&s);
    s.hook();
    s.hook();
    wait_for("a daemon in the fallback dir", READY_LIMIT, || {
        fallback_dir(&s).join("kibitzer-spawntest.sock").exists() && !s.daemon_pids().is_empty()
    });
    assert!(
        !s.socket().exists(),
        "nothing may be bound in the refused dir"
    );
    let log = std::fs::read_to_string(s.root.join("cache/kibitzer/hook-log.jsonl")).unwrap();
    let notes = log
        .lines()
        .filter(|l| l.contains("daemon_fallback"))
        .count();
    assert_eq!(notes, 1, "{log}");
    assert!(!log.contains("daemon_degraded"), "{log}");
    let out = output_within(
        {
            let mut c = s.command();
            c.arg("status");
            c
        },
        Duration::from_secs(15),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Runtime directory fallback"), "{stdout}");
}

#[test]
fn daemon_status_and_stop_should_NameUntrustedDirAndLeftoverDaemon_When_NoDirIsTrustworthy() {
    let s = Scratch::new("untrusted-symlink");
    let link = s.root.join("rt-link");
    std::os::unix::fs::symlink(&s.runtime, &link).unwrap();
    std::fs::set_permissions(&s.runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
    let elsewhere = make_fallback_untrusted(&s);
    // A daemon started before the directory went bad left its pid in the lock.
    std::fs::write(elsewhere.join("kibitzer-spawntest.lock"), "4242").unwrap();
    for action in ["status", "stop"] {
        let mut cmd = s.command();
        cmd.env("XDG_RUNTIME_DIR", &link).args(["daemon", action]);
        let out = output_within(cmd, Duration::from_secs(15));
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("runtime dir untrusted: "),
            "{action}: {stdout}"
        );
        assert!(
            stdout.contains("a daemon started earlier may still be running (pid 4242 in "),
            "{action}: {stdout}"
        );
        assert!(!stdout.contains("no daemon running"), "{action}: {stdout}");
        assert!(
            !stdout.contains("no daemon can be running"),
            "{action}: {stdout}"
        );
        assert_eq!(out.status.code(), Some(1), "{action}: {out:?}");
    }
}

#[test]
fn daemon_start_should_FollowXdgSymlink_When_TargetIsPrivateOwnDir() {
    let s = Scratch::new("xdg-symlink-ok");
    let link = s.root.join("rt-link");
    std::os::unix::fs::symlink(&s.runtime, &link).unwrap();
    let mut cmd = s.command();
    cmd.env("XDG_RUNTIME_DIR", &link).args(["daemon", "start"]);
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the daemon behind the symlinked dir", READY_LIMIT, || {
        s.runtime.join("kibitzer-spawntest.sock").exists()
    });
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn hooks_should_LogDegradeOnce_AndStatusShouldForgetIt_When_DirBecomesUsable() {
    let s = Scratch::new("untrusted-hook");
    make_runtime_loose(&s);
    let planted = make_fallback_untrusted(&s);
    s.hook();
    s.hook();
    assert!(s.daemon_pids().is_empty());
    let log = std::fs::read_to_string(s.root.join("cache/kibitzer/hook-log.jsonl")).unwrap();
    let notes = log
        .lines()
        .filter(|l| l.contains("daemon_degraded"))
        .count();
    assert_eq!(notes, 1, "{log}");
    assert!(log.contains("runtime dir untrusted"), "{log}");
    let status = |s: &Scratch| {
        let mut cmd = s.command();
        cmd.arg("status");
        String::from_utf8_lossy(&output_within(cmd, Duration::from_secs(15)).stdout).into_owned()
    };
    let stdout = status(&s);
    assert!(stdout.contains("Hooks ran without the daemon"), "{stdout}");
    assert!(stdout.contains("2 firings"), "{stdout}");
    // The directory becomes usable: the next hook clears the degrade and `status` stops reporting it.
    std::fs::remove_file(fallback_dir(&s)).unwrap();
    std::fs::remove_dir_all(&planted).unwrap();
    s.hook();
    let stdout = status(&s);
    assert!(!stdout.contains("Hooks ran without the daemon"), "{stdout}");
}

#[test]
fn daemon_status_should_ReportNotResponding_When_DaemonIsStopped() {
    let s = Scratch::new("status-stopped");
    let (mut child, pid) = start_daemon(&s);
    stop_and_confirm(pid);
    let mut cmd = s.command();
    cmd.args(["daemon", "status"]);
    let started = Instant::now();
    let out = output_within(cmd, BOUND);
    assert!(started.elapsed() < BOUND);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!("daemon not responding (pid {pid})")),
        "{stdout}"
    );
    assert!(!stdout.contains("no daemon running"), "{stdout}");
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    signal(pid, "-KILL");
    let _ = child.wait();
}

#[test]
fn daemon_stop_should_NamePid_When_HolderCannotBeVerified() {
    let s = Scratch::new("stop-unverified");
    let _listener = UnixListener::bind(s.socket()).unwrap();
    let mut bystander = Command::new("sleep").arg("30").spawn().unwrap();
    // A live holder that is not a kibitzer daemon: the lock is held and records its pid.
    let lock_path = s.runtime.join("kibitzer-spawntest.lock");
    let mut lock = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&lock_path)
        .unwrap();
    lock.try_lock().unwrap();
    std::io::Write::write_all(&mut lock, bystander.id().to_string().as_bytes()).unwrap();
    let out = output_within(
        {
            let mut c = s.command();
            c.args(["daemon", "stop"]);
            c
        },
        BOUND,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!(
            "daemon (pid {}) is not responding",
            bystander.id()
        )),
        "{stdout}"
    );
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        bystander.try_wait().unwrap().is_none(),
        "bystander was signalled"
    );
    let _ = bystander.kill();
    let _ = bystander.wait();
}
