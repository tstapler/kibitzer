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

/// True when `dir` is a real directory (not a symlink) owned by this user that no one else can
/// write into; a looser mode on a directory we own is tightened to 0700 first.
fn is_private_own_dir(dir: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(dir) else {
        return false;
    };
    if !meta.is_dir() || meta.uid() != current_uid() {
        return false;
    }
    if meta.mode() & 0o077 != 0 {
        return std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).is_ok();
    }
    true
}

/// Where the socket, lock and markers live: `XDG_RUNTIME_DIR` when set, else a `kibitzer-<uid>`
/// directory (mode 0700) under the temp dir, so another user on a shared `/tmp` cannot pre-create
/// our lock or socket. `None` when the directory exists but is not ours; the caller then runs
/// without a daemon rather than trusting it.
pub(crate) fn trusted_runtime_dir() -> Option<PathBuf> {
    trusted_runtime_dir_in(
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        &std::env::temp_dir(),
    )
}

fn trusted_runtime_dir_in(xdg: Option<PathBuf>, temp: &Path) -> Option<PathBuf> {
    if let Some(dir) = xdg {
        return is_private_own_dir(&dir).then_some(dir);
    }
    let dir = temp.join(format!("kibitzer-{}", current_uid()));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => Some(dir),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            is_private_own_dir(&dir).then_some(dir)
        }
        Err(_) => None,
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
    command: String,
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

fn parse_ps_line(line: &str) -> Option<ProcInfo> {
    let mut fields = line.split_whitespace();
    let uid = fields.next()?.parse().ok()?;
    let elapsed = parse_etime(fields.next()?)?;
    let command = fields.collect::<Vec<_>>().join(" ");
    (!command.is_empty()).then_some(ProcInfo {
        uid,
        elapsed,
        command,
    })
}

fn proc_info(pid: u32) -> Option<ProcInfo> {
    let out = std::process::Command::new("ps")
        .args(["-o", "uid=,etime=,command=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| parse_ps_line(String::from_utf8_lossy(&out.stdout).lines().next()?))?
}

/// Whether `info` is a kibitzer daemon of this user that started no later than the lock file was
/// last written (a recycled pid belongs to a process younger than the lock).
fn is_our_daemon(info: &ProcInfo, lock_mtime: SystemTime, now: SystemTime) -> bool {
    let mut words = info.command.split_whitespace();
    let exe_is_kibitzer = words
        .next()
        .and_then(|exe| Path::new(exe).file_name())
        .is_some_and(|name| name.to_string_lossy().starts_with("kibitzer"));
    let args: Vec<&str> = words.collect();
    let is_daemon_start = args.windows(2).any(|w| w == ["daemon", "start"]);
    let started = now.checked_sub(info.elapsed);
    let started_before_lock = started.is_some_and(|s| s <= lock_mtime + Duration::from_secs(2));
    info.uid == current_uid() && exe_is_kibitzer && is_daemon_start && started_before_lock
}

/// Whether some process currently holds the lock (an unreadable lock counts as not held).
pub(crate) fn lock_is_held(lock_path: &Path) -> bool {
    open_lock_file(lock_path).is_ok_and(|f| f.try_lock().is_err())
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
    // SAFETY: plain kill(2) on a pid we just verified is our own kibitzer daemon.
    unsafe {
        libc::kill(pid as libc::pid_t, sig);
    }
}

/// Frees `lock_path` from a holder that keeps the flock but no longer answers (a stopped or
/// deadlocked daemon). Signals only a process whose pid the lock records, that is this user's
/// `kibitzer daemon start`, and that is older than the lock file. SIGTERM first, SIGKILL after a
/// short grace (a stopped process only dies to SIGKILL). True when the lock is free afterwards.
pub(crate) fn displace_wedged_holder(lock_path: &Path) -> bool {
    if lock_is_free(lock_path) {
        return true;
    }
    let Some(pid) = std::fs::read_to_string(lock_path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|&pid| pid > 1 && pid != std::process::id())
    else {
        return false;
    };
    let Some(lock_mtime) = std::fs::metadata(lock_path)
        .ok()
        .and_then(|m| m.modified().ok())
    else {
        return false;
    };
    let verified =
        proc_info(pid).is_some_and(|info| is_our_daemon(&info, lock_mtime, SystemTime::now()));
    if !verified {
        return false;
    }
    signal(pid, libc::SIGTERM);
    if wait_until_free(lock_path, TERM_GRACE) {
        return true;
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

    #[test]
    fn is_our_daemon_should_RejectOtherProcessesAndRecycledPids() {
        let now = SystemTime::now();
        let lock_written = now - Duration::from_secs(100);
        let info = |uid, elapsed, command: &str| ProcInfo {
            uid,
            elapsed: Duration::from_secs(elapsed),
            command: command.to_string(),
        };
        let mine = current_uid();
        assert!(is_our_daemon(
            &info(mine, 500, "/opt/bin/kibitzer daemon start"),
            lock_written,
            now
        ));
        assert!(!is_our_daemon(
            &info(mine + 1, 500, "/opt/bin/kibitzer daemon start"),
            lock_written,
            now
        ));
        assert!(!is_our_daemon(
            &info(mine, 500, "/usr/bin/vim kibitzer daemon start"),
            lock_written,
            now
        ));
        assert!(!is_our_daemon(
            &info(mine, 500, "/opt/bin/kibitzer hook"),
            lock_written,
            now
        ));
        // Younger than the lock file: the pid was recycled after the daemon died.
        assert!(!is_our_daemon(
            &info(mine, 10, "/opt/bin/kibitzer daemon start"),
            lock_written,
            now
        ));
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
        assert!(!displace_wedged_holder(&lock));
        assert!(
            sleeper.try_wait().unwrap().is_none(),
            "unrelated process must survive"
        );
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
        assert_eq!(trusted_runtime_dir_in(None, &temp), Some(dir.clone()));
        // A looser mode on our own directory is tightened rather than trusted as-is.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(trusted_runtime_dir_in(None, &temp).is_some());
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        // A symlink planted at the name is never followed.
        std::fs::remove_dir_all(&dir).unwrap();
        let elsewhere = scratch("trusted-elsewhere");
        std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
        assert_eq!(trusted_runtime_dir_in(None, &temp), None);
        assert_eq!(trusted_runtime_dir_in(Some(dir.clone()), &temp), None);
        let _ = std::fs::remove_dir_all(&temp);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }
}
