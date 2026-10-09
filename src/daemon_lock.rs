//! What keeps the daemon's lock, socket directory and process safe to trust: a per-user runtime
//! directory nobody else can squat in, an owner-only lock file that records its holder's pid, and
//! displacement of a holder that is alive (it keeps the flock) but no longer answers.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Effective uid of this process.
fn current_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// A candidate runtime directory kibitzer refuses to put its socket and lock in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DirRejected {
    pub dir: PathBuf,
    pub reason: String,
}

impl std::fmt::Display for DirRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "runtime dir untrusted: {} ({})",
            crate::inline_ignores::sanitize::display_path(&self.dir),
            self.reason
        )
    }
}

/// Whether `dir` is a real directory (not a symlink) owned by this user that no one else can
/// reach. `tighten` lets a directory kibitzer made itself be chmodded back to 0700; a directory
/// someone else provided (`XDG_RUNTIME_DIR`) is never changed, only refused.
fn check_private_own_dir(dir: &Path, tighten: bool) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("cannot stat it: {e}"))?;
    if meta.file_type().is_symlink() {
        return Err("it is a symlink".to_string());
    }
    if !meta.is_dir() {
        return Err("it is not a directory".to_string());
    }
    if meta.uid() != current_uid() {
        return Err(format!(
            "it is owned by uid {}, not uid {}",
            meta.uid(),
            current_uid()
        ));
    }
    if meta.mode() & 0o077 != 0 {
        if tighten && std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).is_ok()
        {
            return Ok(());
        }
        return Err(format!(
            "its mode {:04o} is open to group or others; run chmod 0700 on it",
            meta.mode() & 0o7777
        ));
    }
    Ok(())
}

/// Where the socket, lock and markers live: `XDG_RUNTIME_DIR` when set, else a `kibitzer-<uid>`
/// directory (mode 0700) under the temp dir, so another user on a shared `/tmp` cannot pre-create
/// our lock or socket. The error names the directory and why it is not ours; callers then run
/// without a daemon rather than trusting it.
pub(crate) fn trusted_runtime_dir() -> Result<PathBuf, DirRejected> {
    trusted_runtime_dir_in(
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        &std::env::temp_dir(),
    )
}

fn trusted_runtime_dir_in(xdg: Option<PathBuf>, temp: &Path) -> Result<PathBuf, DirRejected> {
    let reject = |dir: &Path, reason: String| DirRejected {
        dir: dir.to_path_buf(),
        reason,
    };
    if let Some(dir) = xdg {
        return match check_private_own_dir(&dir, false) {
            Ok(()) => Ok(dir),
            Err(reason) => Err(reject(&dir, reason)),
        };
    }
    let dir = temp.join(format!("kibitzer-{}", current_uid()));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => Ok(dir),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            match check_private_own_dir(&dir, true) {
                Ok(()) => Ok(dir),
                Err(reason) => Err(reject(&dir, reason)),
            }
        }
        Err(e) => Err(reject(&dir, format!("cannot create it: {e}"))),
    }
}

/// Opens (creating, mode 0600) a lock or marker file, refusing one that is not a regular file
/// owned by this user.
pub(crate) fn open_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != current_uid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("{} is not an owner-only regular file", path.display()),
        ));
    }
    Ok(file)
}

/// Records this process as the lock's holder, so a later process can find a wedged one.
pub(crate) fn record_owner_pid(file: &std::fs::File) {
    let mut file = file;
    if file.set_len(0).is_ok() {
        let _ = write!(file, "{}", std::process::id());
    }
}

/// How long a holder gets between SIGTERM and SIGKILL, and to let go of the lock after each.
const TERM_GRACE: Duration = Duration::from_millis(400);
const KILL_GRACE: Duration = Duration::from_millis(800);

/// What `ps` reports about the process we are about to signal.
#[derive(Debug, PartialEq, Eq)]
struct ProcInfo {
    uid: u32,
    elapsed: Duration,
    /// The executable as the kernel knows it: a full path on macOS, a (15-char) name on Linux.
    comm: String,
    /// The whole command line as one string; may hold spaces inside the executable path.
    args: String,
}

/// `[[dd-]hh:]mm:ss`, as `ps -o etime=` prints it on both macOS and Linux.
fn parse_etime(text: &str) -> Option<Duration> {
    let (days, rest) = match text.split_once('-') {
        Some((d, rest)) => (d.parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let mut secs = 0u64;
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() > 3 || parts.is_empty() {
        return None;
    }
    for part in parts {
        secs = secs * 60 + part.parse::<u64>().ok()?;
    }
    Some(Duration::from_secs(days * 86_400 + secs))
}

/// `uid etime comm...`: comm is last because an install path may contain spaces.
fn parse_ps_head(line: &str) -> Option<(u32, Duration, String)> {
    let mut rest = line.trim_start();
    let mut take = || {
        let end = rest.find(char::is_whitespace)?;
        let (word, tail) = rest.split_at(end);
        rest = tail.trim_start();
        Some(word)
    };
    let uid = take()?.parse().ok()?;
    let elapsed = parse_etime(take()?)?;
    let comm = rest.trim_end().to_string();
    (!comm.is_empty()).then_some((uid, elapsed, comm))
}

fn ps_line(columns: &str, pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-o", columns, "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(str::to_string)
}

fn proc_info(pid: u32) -> Option<ProcInfo> {
    let (uid, elapsed, comm) = parse_ps_head(&ps_line("uid=,etime=,comm=", pid)?)?;
    let args = ps_line("args=", pid)?.trim().to_string();
    Some(ProcInfo {
        uid,
        elapsed,
        comm,
        args,
    })
}

/// Whether `info` is a kibitzer daemon of this user that started no later than the lock file was
/// last written (a recycled pid belongs to a process younger than the lock). The command line
/// must be exactly `<exe> daemon start` with a kibitzer-named `<exe>` (spaces allowed in its
/// path) that agrees with the kernel's name for the process.
fn is_our_daemon(info: &ProcInfo, lock_mtime: SystemTime, now: SystemTime) -> bool {
    let Some(exe) = info.args.strip_suffix(" daemon start").map(str::trim_end) else {
        return false;
    };
    let exe_name = Path::new(exe)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let comm_name = Path::new(&info.comm)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let exe_is_kibitzer = exe_name.starts_with("kibitzer")
        && !comm_name.is_empty()
        && exe_name.starts_with(&comm_name);
    let started = now.checked_sub(info.elapsed);
    let started_before_lock = started.is_some_and(|s| s <= lock_mtime + Duration::from_secs(2));
    info.uid == current_uid() && exe_is_kibitzer && started_before_lock
}

/// Whether some process currently holds the lock (an unreadable lock counts as not held).
pub(crate) fn lock_is_held(lock_path: &Path) -> bool {
    // Never creates the file: asking whether a daemon is there must not leave a lock behind.
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(lock_path)
        .is_ok_and(|f| {
            f.metadata()
                .is_ok_and(|m| m.is_file() && m.uid() == current_uid())
                && f.try_lock().is_err()
        })
}

fn lock_is_free(lock_path: &Path) -> bool {
    open_lock_file(lock_path)
        .map(|f| f.try_lock().is_ok())
        .unwrap_or(false)
}

fn wait_until_free(lock_path: &Path, grace: Duration) -> bool {
    let deadline = std::time::Instant::now() + grace;
    loop {
        if lock_is_free(lock_path) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn signal(pid: u32, sig: libc::c_int) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    // SAFETY: plain kill(2) on a pid we just verified is our own kibitzer daemon.
    unsafe {
        libc::kill(pid, sig);
    }
}

fn recorded_pid(lock_path: &Path) -> Option<u32> {
    std::fs::read_to_string(lock_path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|&pid| pid > 1 && pid != std::process::id())
}

fn lock_mtime(lock_path: &Path) -> Option<SystemTime> {
    std::fs::metadata(lock_path).ok()?.modified().ok()
}

fn verified_daemon(pid: u32, lock_path: &Path) -> bool {
    lock_mtime(lock_path).is_some_and(|mtime| {
        proc_info(pid).is_some_and(|info| is_our_daemon(&info, mtime, SystemTime::now()))
    })
}

/// Frees `lock_path` from a holder that keeps the flock but no longer answers (a stopped or
/// deadlocked daemon). Signals only a process whose pid the lock records, that is this user's
/// `kibitzer daemon start`, and that is older than the lock file. `still_wedged` re-probes the
/// holder right before signalling, so a healthy daemon that took the lock meanwhile survives.
/// SIGTERM first, SIGKILL after a short grace (a stopped process only dies to SIGKILL). True
/// when the lock is free afterwards.
pub(crate) fn displace_wedged_holder(lock_path: &Path, still_wedged: &dyn Fn() -> bool) -> bool {
    displace_with(lock_path, still_wedged, &verified_daemon)
}

fn displace_with(
    lock_path: &Path,
    still_wedged: &dyn Fn() -> bool,
    verify: &dyn Fn(u32, &Path) -> bool,
) -> bool {
    if lock_is_free(lock_path) {
        return true;
    }
    let Some(pid) = recorded_pid(lock_path) else {
        return false;
    };
    if !verify(pid, lock_path) {
        return false;
    }
    // The straddle race: another process may have replaced the holder since the caller probed.
    if !still_wedged() || recorded_pid(lock_path) != Some(pid) {
        return lock_is_free(lock_path);
    }
    signal(pid, libc::SIGTERM);
    if wait_until_free(lock_path, TERM_GRACE) {
        return true;
    }
    if !verify(pid, lock_path) {
        return lock_is_free(lock_path);
    }
    signal(pid, libc::SIGKILL);
    wait_until_free(lock_path, KILL_GRACE)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kibitzer-dl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_etime_should_ReadAllPsShapes() {
        assert_eq!(parse_etime("05:23"), Some(Duration::from_secs(323)));
        assert_eq!(parse_etime("01:02:03"), Some(Duration::from_secs(3723)));
        assert_eq!(
            parse_etime("2-00:00:10"),
            Some(Duration::from_secs(172_810))
        );
        assert_eq!(parse_etime("soon"), None);
    }

    fn info(uid: u32, elapsed: u64, comm: &str, args: &str) -> ProcInfo {
        ProcInfo {
            uid,
            elapsed: Duration::from_secs(elapsed),
            comm: comm.to_string(),
            args: args.to_string(),
        }
    }

    #[test]
    fn is_our_daemon_should_RejectOtherProcessesAndRecycledPids() {
        let now = SystemTime::now();
        let lock_written = now - Duration::from_secs(100);
        let mine = current_uid();
        let ok = |i: ProcInfo| is_our_daemon(&i, lock_written, now);
        let exe = "/opt/bin/kibitzer";
        assert!(ok(info(mine, 500, exe, "/opt/bin/kibitzer daemon start")));
        assert!(!ok(info(
            mine + 1,
            500,
            exe,
            "/opt/bin/kibitzer daemon start"
        )));
        assert!(!ok(info(
            mine,
            500,
            "/usr/bin/vim",
            "/usr/bin/vim kibitzer daemon start"
        )));
        assert!(!ok(info(mine, 500, exe, "/opt/bin/kibitzer hook")));
        assert!(!ok(info(
            mine,
            500,
            exe,
            "/opt/bin/kibitzer daemon start --x"
        )));
        // The kernel's name for the process disagrees with the rewritten command line.
        assert!(!ok(info(
            mine,
            500,
            "/bin/sleep",
            "/opt/bin/kibitzer daemon start"
        )));
        // Younger than the lock file: the pid was recycled after the daemon died.
        assert!(!ok(info(mine, 10, exe, "/opt/bin/kibitzer daemon start")));
    }

    #[test]
    fn is_our_daemon_should_Verify_When_InstallPathContainsSpaces() {
        let now = SystemTime::now();
        let lock_written = now - Duration::from_secs(100);
        let mine = current_uid();
        let spaced = "/Users/me/My Tools/bin/kibitzer";
        let args = "/Users/me/My Tools/bin/kibitzer daemon start";
        // macOS: comm is the full path; Linux: comm is the bare name.
        assert!(is_our_daemon(
            &info(mine, 500, spaced, args),
            lock_written,
            now
        ));
        assert!(is_our_daemon(
            &info(mine, 500, "kibitzer", args),
            lock_written,
            now
        ));
        // A directory merely named kibitzer does not make its contents kibitzer.
        let tricky = "/tmp/kibitzer dir/evil daemon start";
        assert!(!is_our_daemon(
            &info(mine, 500, "evil", tricky),
            lock_written,
            now
        ));
    }

    #[test]
    fn parse_ps_head_should_KeepSpacesInComm() {
        let (uid, elapsed, comm) =
            parse_ps_head("  501 01:02 /Users/me/My Tools/bin/kibitzer").unwrap();
        assert_eq!((uid, elapsed.as_secs()), (501, 62));
        assert_eq!(comm, "/Users/me/My Tools/bin/kibitzer");
        assert_eq!(parse_ps_head("501 01:02"), None);
    }

    #[test]
    fn displace_wedged_holder_should_RefuseUnrelatedProcess_When_LockNamesIt() {
        let dir = scratch("refuse");
        let lock = dir.join("k.lock");
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let file = open_lock_file(&lock).unwrap();
        file.try_lock().unwrap();
        std::fs::write(&lock, sleeper.id().to_string()).unwrap();
        assert!(!displace_wedged_holder(&lock, &|| true));
        assert!(
            sleeper.try_wait().unwrap().is_none(),
            "unrelated process must survive"
        );
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn lock_held_by_test_naming(dir: &Path, pid: u32) -> (PathBuf, std::fs::File) {
        let lock = dir.join("k.lock");
        let file = open_lock_file(&lock).unwrap();
        file.try_lock().unwrap();
        std::fs::write(&lock, pid.to_string()).unwrap();
        (lock, file)
    }

    #[test]
    fn displace_with_should_NotSignal_When_ReprobeFindsHolderHealthy() {
        let dir = scratch("reprobe");
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let (lock, _held) = lock_held_by_test_naming(&dir, sleeper.id());
        let freed = displace_with(&lock, &|| false, &|_, _| true);
        assert!(!freed);
        assert!(
            sleeper.try_wait().unwrap().is_none(),
            "a holder that answers the re-probe must not be signalled"
        );
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn displace_with_should_Signal_When_HolderStillWedgedAfterReprobe() {
        let dir = scratch("reprobe-wedged");
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let (lock, _held) = lock_held_by_test_naming(&dir, sleeper.id());
        // The lock stays held by this test, so false is expected; the holder must still die.
        let _ = displace_with(&lock, &|| true, &|_, _| true);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut dead = false;
        while std::time::Instant::now() < deadline {
            if sleeper.try_wait().unwrap().is_some() {
                dead = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        assert!(dead, "a verified, still-wedged holder must be signalled");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn displace_with_should_NotSignal_When_RecordedPidChangedDuringReprobe() {
        let dir = scratch("pid-changed");
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let (lock, _held) = lock_held_by_test_naming(&dir, sleeper.id());
        let lock2 = lock.clone();
        let freed = displace_with(
            &lock,
            &move || {
                std::fs::write(&lock2, "999999").unwrap();
                true
            },
            &|_, _| true,
        );
        assert!(!freed);
        assert!(sleeper.try_wait().unwrap().is_none());
        let _ = sleeper.kill();
        let _ = sleeper.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_lock_file_should_CreateOwnerOnlyFile_And_RefuseSymlink() {
        let dir = scratch("lockfile");
        let lock = dir.join("k.lock");
        let file = open_lock_file(&lock).unwrap();
        assert_eq!(file.metadata().unwrap().mode() & 0o777, 0o600);
        let link = dir.join("l.lock");
        std::os::unix::fs::symlink(&lock, &link).unwrap();
        assert!(
            open_lock_file(&link).is_err(),
            "symlinked lock must be refused"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trusted_runtime_dir_should_CreatePrivateDir_And_RefuseSymlinkOrLooseForeignPath() {
        let temp = scratch("trusted");
        let dir = trusted_runtime_dir_in(None, &temp).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        assert_eq!(trusted_runtime_dir_in(None, &temp), Ok(dir.clone()));
        // A looser mode on the kibitzer-<uid> directory kibitzer made is tightened.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(trusted_runtime_dir_in(None, &temp).is_ok());
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        // A symlink planted at the name is never followed.
        std::fs::remove_dir_all(&dir).unwrap();
        let elsewhere = scratch("trusted-elsewhere");
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        let err = trusted_runtime_dir_in(None, &temp).unwrap_err();
        assert_eq!(err.dir, dir);
        assert!(err.reason.contains("symlink"), "{err}");
        let err = trusted_runtime_dir_in(Some(dir.clone()), &temp).unwrap_err();
        assert!(
            err.to_string().starts_with("runtime dir untrusted: "),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&temp);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }

    #[test]
    fn trusted_runtime_dir_should_RefuseXdgDirWithLooseMode_WithoutChangingIt() {
        let temp = scratch("xdg-loose");
        let xdg = temp.join("xdg");
        std::fs::create_dir(&xdg).unwrap();
        std::fs::set_permissions(&xdg, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = trusted_runtime_dir_in(Some(xdg.clone()), &temp).unwrap_err();
        assert_eq!(err.dir, xdg);
        assert!(err.reason.contains("0755"), "{err}");
        assert_eq!(std::fs::metadata(&xdg).unwrap().mode() & 0o777, 0o755);
        std::fs::set_permissions(&xdg, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(trusted_runtime_dir_in(Some(xdg.clone()), &temp), Ok(xdg));
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn trusted_runtime_dir_should_Refuse_When_DirIsNotOurs() {
        // `/` is owned by root, so it stands in for another user's directory.
        let err = trusted_runtime_dir_in(Some(PathBuf::from("/")), Path::new("/tmp")).unwrap_err();
        assert!(
            err.reason.contains("owned by uid") || err.reason.contains("mode"),
            "{err}"
        );
    }

    /// A shell that ignores SIGTERM, so only SIGKILL stops it.
    fn spawn_term_immune() -> std::process::Child {
        let child = std::process::Command::new("sh")
            .args(["-c", "trap '' TERM; while :; do sleep 1; done"])
            .spawn()
            .unwrap();
        // Let the trap install before the first signal arrives.
        std::thread::sleep(Duration::from_millis(300));
        child
    }

    #[test]
    fn displace_with_should_NotSigkill_When_PidFailsReverifyAfterTermGrace() {
        let dir = scratch("reverify");
        let mut holder = spawn_term_immune();
        let (lock, _held) = lock_held_by_test_naming(&dir, holder.id());
        let calls = std::sync::atomic::AtomicUsize::new(0);
        // Verified before SIGTERM; by the time the grace runs out the pid belongs to someone else.
        let verify =
            |_: u32, _: &Path| calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        let freed = displace_with(&lock, &|| true, &verify);
        assert!(!freed, "the lock is still held by the test");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(
            holder.try_wait().unwrap().is_none(),
            "a recycled pid must not receive SIGKILL"
        );
        let _ = holder.kill();
        let _ = holder.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn displace_with_should_Sigkill_When_PidStillVerifiedAfterTermGrace() {
        let dir = scratch("sigkill");
        let mut holder = spawn_term_immune();
        let (lock, _held) = lock_held_by_test_naming(&dir, holder.id());
        let _ = displace_with(&lock, &|| true, &|_, _| true);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut dead = false;
        while std::time::Instant::now() < deadline {
            if holder.try_wait().unwrap().is_some() {
                dead = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            dead,
            "a still-verified holder that ignores SIGTERM must be SIGKILLed"
        );
        let _ = holder.kill();
        let _ = holder.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_is_held_should_NotCreateFile_When_PathIsMissing() {
        let dir = scratch("no-create");
        let lock = dir.join("k.lock");
        assert!(!lock_is_held(&lock));
        assert!(!lock.exists(), "lock_is_held created {}", lock.display());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_is_held_should_ReportHeldAndFree_When_FileExists() {
        // A concurrent test's fork can briefly duplicate a probe's descriptor (and so its flock)
        // until the child execs, so each state is awaited rather than sampled once.
        let eventually = |what: &str, cond: &dyn Fn() -> bool| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !cond() {
                assert!(std::time::Instant::now() < deadline, "never became {what}");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let dir = scratch("held-free");
        let lock = dir.join("k.lock");
        let file = open_lock_file(&lock).unwrap();
        eventually("free", &|| !lock_is_held(&lock));
        eventually("locked", &|| file.try_lock().is_ok());
        assert!(lock_is_held(&lock));
        drop(file);
        eventually("free again", &|| !lock_is_held(&lock));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
