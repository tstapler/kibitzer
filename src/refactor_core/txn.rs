//! All-or-nothing multi-file write with crash recovery: back up originals and make a log
//! durable, write temp files, rename them in; the caller then verifies and calls
//! [`Pending::commit`] (drop the log) or [`Pending::rollback`]. A crash leaves the log for
//! [`recover`]; dropping an undecided [`Pending`] rolls back.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Component, Path, PathBuf};
use std::sync::MutexGuard;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// Serializes transactions in this process so one never rolls back another's live log. Other
/// processes are kept out by [`PlansLock`].
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const PLANS_DIR: &str = ".kibitzer/plans";
const WAL_FILE: &str = ".wal";
const BACKUP_DIR: &str = ".wal-backup";
const LOCK_FILE: &str = ".lock";

/// Cross-process advisory lock on a repo's plans dir (`flock` on [`LOCK_FILE`]), held for the life
/// of a [`Session`] or [`Pending`] so a second process never recovers or clobbers a live log. The
/// file is removed on release; an acquirer re-checks the path still names the file it locked.
struct PlansLock {
    root: PathBuf,
    _file: std::fs::File,
}

impl PlansLock {
    /// `Ok(None)` when another open file description holds the lock.
    fn try_acquire(root: &Path) -> Result<Option<Self>> {
        use std::os::unix::fs::MetadataExt as _;
        ensure_plans_dir_is_real(root)?;
        let path = plans_dir(root).join(LOCK_FILE);
        // A releasing holder unlinks the file and its empty dirs, which can race with us.
        for _ in 0..8 {
            let opened = std::fs::create_dir_all(plans_dir(root))
                .and_then(|()| crate::daemon_lock::open_lock_file(&path));
            let file = match opened {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e).with_context(|| format!("opening {}", path.display())),
            };
            match file.try_lock() {
                Ok(()) => {}
                Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(e).with_context(|| format!("locking {}", path.display()));
                }
            }
            let same_file = std::fs::symlink_metadata(&path).is_ok_and(|m| {
                file.metadata()
                    .is_ok_and(|f| (f.dev(), f.ino()) == (m.dev(), m.ino()))
            });
            if same_file {
                return Ok(Some(Self {
                    root: root.to_path_buf(),
                    _file: file,
                }));
            }
        }
        bail!("could not lock {}", path.display())
    }

    fn acquire(root: &Path) -> Result<Self> {
        Self::try_acquire(root)?.ok_or_else(|| {
            anyhow::anyhow!(
                "another kibitzer process is applying a refactor in this repository \
                 ({} is locked); retry when it finishes",
                plans_dir(root).join(LOCK_FILE).display()
            )
        })
    }
}

impl Drop for PlansLock {
    fn drop(&mut self) {
        // Unlink while still locked (our fd closes after this), then tidy empty dirs.
        let _ = std::fs::remove_file(plans_dir(&self.root).join(LOCK_FILE));
        let _ = std::fs::remove_dir(plans_dir(&self.root));
        let _ = std::fs::remove_dir(self.root.join(".kibitzer"));
    }
}

pub enum Op {
    /// Write `bytes`. The file takes the permissions of `mode_from` (a moved file's old path),
    /// else of the file it replaces, else the umask default.
    Put {
        bytes: Vec<u8>,
        mode_from: Option<PathBuf>,
    },
    Delete,
}

pub struct Change {
    pub path: PathBuf,
    pub op: Op,
    /// When set, the bytes read for the backup must hash to this, closing the window between the
    /// caller's staleness check and the swap.
    pub expect_sha256: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Wal {
    plan_id: String,
    entries: Vec<Entry>,
    created_dirs: Vec<PathBuf>,
}

/// The first release of the MCP engine logged `{original, backup, dest, temp}`; a crash under that
/// build must still be recoverable after an upgrade.
#[derive(Deserialize)]
struct LegacyEntry {
    original: PathBuf,
    backup: PathBuf,
    dest: PathBuf,
    temp: PathBuf,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AnyEntry {
    Current(Entry),
    Legacy(LegacyEntry),
}

#[derive(Deserialize)]
struct WalRepr {
    plan_id: String,
    entries: Vec<AnyEntry>,
    created_dirs: Vec<PathBuf>,
}

impl From<WalRepr> for Wal {
    fn from(r: WalRepr) -> Self {
        let mut entries = Vec::new();
        for e in r.entries {
            match e {
                AnyEntry::Current(e) => entries.push(e),
                AnyEntry::Legacy(l) if l.dest == l.original => entries.push(Entry {
                    path: l.original,
                    backup: Some(l.backup),
                    temp: Some(l.temp),
                    mode: None,
                }),
                AnyEntry::Legacy(l) => {
                    entries.push(Entry {
                        path: l.original,
                        backup: Some(l.backup),
                        temp: None,
                        mode: None,
                    });
                    entries.push(Entry {
                        path: l.dest,
                        backup: None,
                        temp: Some(l.temp),
                        mode: None,
                    });
                }
            }
        }
        Wal {
            plan_id: r.plan_id,
            entries,
            created_dirs: r.created_dirs,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    /// Copy of the original; `None` when the file did not exist, so rollback removes it.
    backup: Option<PathBuf>,
    /// Temp file written beside `path` for a `Put`.
    temp: Option<PathBuf>,
    /// Permission bits of the original, restored after the backup is copied back.
    #[serde(default)]
    mode: Option<u32>,
}

pub type Rename<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

pub fn real_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

fn plans_dir(root: &Path) -> PathBuf {
    root.join(PLANS_DIR)
}

fn wal_path(root: &Path) -> PathBuf {
    plans_dir(root).join(WAL_FILE)
}

fn backup_dir(root: &Path) -> PathBuf {
    plans_dir(root).join(BACKUP_DIR)
}

/// A transaction whose writes landed but which is not yet committed or rolled back.
pub struct Pending {
    root: PathBuf,
    wal: Wal,
    settled: bool,
    // Field order matters: the file lock must release before the in-process mutex.
    _flock: PlansLock,
    _lock: MutexGuard<'static, ()>,
}

impl Pending {
    /// Keeps the writes and removes the log and backups.
    pub fn commit(mut self) -> Result<()> {
        self.settled = true;
        cleanup(&self.root)
    }

    /// Restores every original. On failure the log is kept so [`recover`] can retry.
    pub fn rollback(mut self) -> Result<()> {
        self.settled = true;
        rollback(&self.root, &self.wal)
    }

    fn fail(mut self, cause: anyhow::Error) -> anyhow::Error {
        self.settled = true;
        match rollback(&self.root, &self.wal) {
            Ok(()) => cause.context("apply rolled back; the tree is unchanged"),
            Err(rb) => anyhow::Error::new(RollbackFailed {
                cause: format!("{cause:#}"),
                rollback: format!("{rb:#}"),
            }),
        }
    }
}

// A killed process runs no destructors; tests flip this to leave the log behind like one.
#[cfg(test)]
thread_local! {
    static KILLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub fn simulate_kill(on: bool) {
    KILLED.with(|k| k.set(on));
}

fn killed() -> bool {
    #[cfg(test)]
    return KILLED.with(std::cell::Cell::get);
    #[cfg(not(test))]
    false
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.settled
            && !killed()
            && let Err(e) = rollback(&self.root, &self.wal)
        {
            eprintln!(
                "kibitzer: rollback failed ({e:#}); the log stays in {}; the next kibitzer \
                 refactor start retries it automatically",
                wal_path(&self.root).display()
            );
        }
    }
}

/// Holds the process-wide lock after crash recovery, so a caller can check staleness and compute
/// its new file bodies against a tree no other transaction can change, then [`Session::begin`].
pub struct Session {
    root: PathBuf,
    flock: PlansLock,
    lock: MutexGuard<'static, ()>,
}

/// Takes the lock, then rolls back any transaction a crashed process left behind.
pub fn session(root: &Path) -> Result<Session> {
    let lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let flock = PlansLock::acquire(root)?;
    recover_locked(root)?;
    Ok(Session {
        root: root.to_path_buf(),
        flock,
        lock,
    })
}

/// Returned (inside an `anyhow::Error`) when a failed apply could not be fully rolled back: the
/// tree may be dirty and the log is kept for [`recover`]. Callers `downcast_ref` to tell this from
/// a clean rollback.
#[derive(Debug)]
pub struct RollbackFailed {
    pub cause: String,
    pub rollback: String,
}

impl std::fmt::Display for RollbackFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\nrollback also failed ({}); the WAL is kept so the next start can retry",
            self.cause, self.rollback
        )
    }
}

impl std::error::Error for RollbackFailed {}

impl Session {
    /// Applies `changes` (absolute paths under the root). A failure before this returns rolls
    /// the tree back; afterwards the caller owns the decision.
    pub fn begin(self, plan_id: &str, changes: &[Change], rename: Rename) -> Result<Pending> {
        let root = self.root;
        let mut seen = BTreeSet::new();
        if let Some(c) = changes.iter().find(|c| !seen.insert(&c.path)) {
            bail!("{} is changed twice in one plan", c.path.display());
        }
        ensure_plans_dir_is_real(&root)?;
        let wal = match write_wal(&root, plan_id, changes) {
            Ok(w) => w,
            Err(e) => {
                let _ = cleanup(&root);
                return Err(e);
            }
        };
        let pending = Pending {
            root,
            wal,
            settled: false,
            _flock: self.flock,
            _lock: self.lock,
        };
        match swap(&pending.wal, changes, rename) {
            Ok(()) => Ok(pending),
            Err(e) => Err(pending.fail(e)),
        }
    }
}

/// [`session`] then [`Session::begin`], for callers with nothing to check in between.
#[cfg(test)]
pub fn begin(root: &Path, plan_id: &str, changes: &[Change], rename: Rename) -> Result<Pending> {
    session(root)?.begin(plan_id, changes, rename)
}

/// Rolls back a transaction a crashed process left half-done. Returns the interrupted plan id,
/// or `None` when no log exists or another process holds the repo's transaction lock (its log is
/// live, not orphaned). Safe to call at every startup and before every transaction.
pub fn recover(root: &Path) -> Result<Option<String>> {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ensure_plans_dir_is_real(root)?;
    if !plans_dir(root).exists() {
        return Ok(None);
    }
    let Some(_flock) = PlansLock::try_acquire(root)? else {
        return Ok(None);
    };
    recover_locked(root)
}

/// A log is a few KiB; refusing a huge one keeps a hostile repo from exhausting memory on every
/// automatic recovery.
const MAX_WAL_BYTES: u64 = 8 * 1024 * 1024;

/// `.kibitzer` or `.kibitzer/plans` as a symlink would send the log, backups and cleanup outside
/// the repo.
fn ensure_plans_dir_is_real(root: &Path) -> Result<()> {
    for d in [root.join(".kibitzer"), plans_dir(root)] {
        if std::fs::symlink_metadata(&d).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(
                "{} is a symlink; refusing to use it for the transaction log",
                d.display()
            );
        }
    }
    Ok(())
}

fn recover_locked(root: &Path) -> Result<Option<String>> {
    ensure_plans_dir_is_real(root)?;
    let path = wal_path(root);
    let text = match std::fs::File::open(&path) {
        Ok(f) => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::Read::take(f, MAX_WAL_BYTES + 1), &mut s)
                .with_context(|| format!("reading {}", path.display()))?;
            if s.len() as u64 > MAX_WAL_BYTES {
                bail!(
                    "{} is larger than {MAX_WAL_BYTES} bytes; refusing to read it",
                    path.display()
                );
            }
            s
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let wal: Wal = serde_json::from_str::<WalRepr>(&text)
        .map(Wal::from)
        .with_context(|| {
            format!(
                "{} is corrupt; restore from {BACKUP_DIR} by hand",
                path.display()
            )
        })?;
    check_wal_paths(root, &wal)?;
    rollback(root, &wal)?;
    Ok(Some(wal.plan_id))
}

/// `p` resolved against the real filesystem (symlinks followed on the deepest existing ancestor),
/// relative to the canonical root; `None` when it escapes the root, uses `..`, or is itself a
/// symlink (a symlink at the final component would redirect a restore or delete).
fn resolved_rel(root: &Path, p: &Path) -> Option<PathBuf> {
    if !p.starts_with(root) || p.components().any(|c| c == Component::ParentDir) {
        return None;
    }
    if std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
        return None;
    }
    let canon_root = root.canonicalize().ok()?;
    let existing = p
        .ancestors()
        .find(|a| std::fs::symlink_metadata(a).is_ok())?;
    let tail = p.strip_prefix(existing).ok()?;
    let resolved = existing.canonicalize().ok()?.join(tail);
    resolved
        .strip_prefix(&canon_root)
        .ok()
        .map(Path::to_path_buf)
}

/// Shell commands that undo the log by hand: copy each backup back, remove files the plan created.
/// Printed when automatic recovery refuses, so the user is not left to reverse-engineer the log.
fn manual_restore_commands(wal: &Wal) -> String {
    wal.entries
        .iter()
        .map(|e| match &e.backup {
            Some(b) => format!("  cp -p '{}' '{}'", b.display(), e.path.display()),
            None => format!("  rm -f '{}'", e.path.display()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Crash recovery trusts nothing in the on-disk log: a hostile repo can plant one. Every path it
/// names must stay inside the repo after symlinks are resolved and must not touch `.git` or
/// `.kibitzer` (hooks and check commands are code execution). A crash during a change to a
/// `.kibitzer` file therefore needs a manual rollback from the backups; in-process rollback uses
/// the in-memory log and is not subject to this.
fn check_wal_paths(root: &Path, wal: &Wal) -> Result<()> {
    let backups = backup_dir(root);
    let refuse = |p: &Path| {
        anyhow::anyhow!(
            "WAL names {} outside the repository or in .git/.kibitzer; refusing to roll back \
             automatically; restore by hand, then delete {}:\n{}",
            p.display(),
            wal_path(root).display(),
            manual_restore_commands(wal)
        )
    };
    let allowed = |p: &Path| {
        resolved_rel(root, p).is_some_and(|rel| {
            !rel.components()
                .any(|c| c.as_os_str() == ".git" || c.as_os_str() == ".kibitzer")
        })
    };
    for e in &wal.entries {
        if !allowed(&e.path) {
            return Err(refuse(&e.path));
        }
        if let Some(t) = &e.temp
            && (!allowed(t) || *t != temp_path(&e.path))
        {
            return Err(refuse(t));
        }
        if let Some(b) = &e.backup
            && (b.parent() != Some(backups.as_path())
                || !std::fs::symlink_metadata(b).is_ok_and(|m| m.is_file()))
        {
            bail!(
                "WAL backup {} is not a regular file in {}",
                b.display(),
                backups.display()
            );
        }
    }
    if let Some(d) = wal.created_dirs.iter().find(|d| !allowed(d)) {
        return Err(refuse(d));
    }
    Ok(())
}

/// Attempts every entry even after a failure; the log is kept unless all succeed.
fn rollback(root: &Path, wal: &Wal) -> Result<()> {
    let errors: Vec<String> = wal
        .entries
        .iter()
        .filter_map(|e| restore_entry(e).err().map(|err| format!("{err:#}")))
        .collect();
    for dir in wal.created_dirs.iter().rev() {
        // Non-empty or already gone: either way there is nothing of ours left to remove.
        let _ = std::fs::remove_dir(dir);
    }
    if !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    cleanup(root)
}

fn restore_entry(e: &Entry) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    match &e.backup {
        Some(b) => {
            if let Some(parent) = e.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(b, &e.path)
                .with_context(|| format!("restoring {} from {}", e.path.display(), b.display()))?;
            if let Some(mode) = e.mode {
                std::fs::set_permissions(&e.path, std::fs::Permissions::from_mode(mode))
                    .with_context(|| format!("restoring mode of {}", e.path.display()))?;
            }
            // The backups are deleted next; make the restored file durable first.
            std::fs::File::open(&e.path)
                .and_then(|f| f.sync_all())
                .with_context(|| format!("syncing {}", e.path.display()))?;
        }
        None => ignore_missing(std::fs::remove_file(&e.path))?,
    }
    if let Some(t) = &e.temp {
        ignore_missing(std::fs::remove_file(t))?;
    }
    Ok(())
}

/// The log is the commit point: drop it before the backups so a crash in between never leaves a
/// log whose backups are gone.
fn cleanup(root: &Path) -> Result<()> {
    ignore_missing(std::fs::remove_file(wal_path(root)))?;
    let _ = std::fs::remove_dir_all(backup_dir(root));
    // Only succeed when empty, so a repo's own `.kibitzer/` files are never touched.
    let _ = std::fs::remove_dir(plans_dir(root));
    let _ = std::fs::remove_dir(root.join(".kibitzer"));
    Ok(())
}

fn ignore_missing(r: std::io::Result<()>) -> Result<()> {
    ignore_missing_io(r).map_err(Into::into)
}

fn ignore_missing_io(r: std::io::Result<()>) -> std::io::Result<()> {
    match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // Never follow a planted symlink at a temp name: drop whatever is there and create anew.
    ignore_missing_io(std::fs::remove_file(path))?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Best-effort directory fsync so a rename survives power loss (not supported on every platform).
fn sync_dir(dir: &Path) {
    let _ = std::fs::File::open(dir).and_then(|f| f.sync_all());
}

fn temp_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    dest.with_file_name(format!(".{name}.kibitzer-tmp"))
}

/// Missing ancestors of `dir`, outermost first, so rollback removes only what we created.
fn missing_ancestors(dir: &Path) -> Vec<PathBuf> {
    let mut missing: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|d| !d.exists())
        .map(Path::to_path_buf)
        .collect();
    missing.reverse();
    missing
}

/// Backs up every original and makes the log durable, before any destination is touched.
fn write_wal(root: &Path, plan_id: &str, changes: &[Change]) -> Result<Wal> {
    std::fs::create_dir_all(backup_dir(root))?;
    let mut created_dirs: Vec<PathBuf> = Vec::new();
    let mut entries = Vec::new();
    for (i, c) in changes.iter().enumerate() {
        let mut mode = None;
        let backup = match std::fs::symlink_metadata(&c.path) {
            Ok(m) if m.is_file() => {
                mode = Some(std::os::unix::fs::PermissionsExt::mode(&m.permissions()));
                let b = backup_dir(root).join(i.to_string());
                let original = std::fs::read(&c.path)?;
                if let Some(want) = &c.expect_sha256
                    && &super::sha256_hex(&original) != want
                {
                    bail!("{} changed since the plan was made", c.path.display());
                }
                // Read + synced write (not `copy`): the log must never point at a backup that a
                // power loss could leave empty.
                write_synced(&b, &original)?;
                Some(b)
            }
            Ok(_) => bail!("{} is not a regular file", c.path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("reading {}", c.path.display())),
        };
        let temp = matches!(c.op, Op::Put { .. }).then(|| temp_path(&c.path));
        if temp.is_some() {
            for d in c.path.parent().map(missing_ancestors).unwrap_or_default() {
                if !created_dirs.contains(&d) {
                    created_dirs.push(d);
                }
            }
        }
        entries.push(Entry {
            path: c.path.clone(),
            backup,
            temp,
            mode,
        });
    }
    let wal = Wal {
        plan_id: plan_id.to_string(),
        entries,
        created_dirs,
    };
    let tmp = plans_dir(root).join(".wal.tmp");
    write_synced(&tmp, &serde_json::to_vec(&wal)?)?;
    std::fs::rename(&tmp, wal_path(root))?;
    sync_dir(&plans_dir(root));
    Ok(wal)
}

fn swap(wal: &Wal, changes: &[Change], rename: Rename) -> Result<()> {
    for (c, e) in changes.iter().zip(&wal.entries) {
        let (Op::Put { bytes, mode_from }, Some(temp)) = (&c.op, &e.temp) else {
            continue;
        };
        if let Some(parent) = c.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_synced(temp, bytes).with_context(|| format!("writing {}", temp.display()))?;
        let perms = mode_from
            .as_ref()
            .and_then(|m| std::fs::metadata(m).ok())
            .or_else(|| e.backup.as_ref().and_then(|b| std::fs::metadata(b).ok()));
        if let Some(meta) = perms {
            std::fs::set_permissions(temp, meta.permissions())?;
        }
    }
    for e in &wal.entries {
        if let Some(temp) = &e.temp {
            rename(temp, &e.path).with_context(|| format!("renaming into {}", e.path.display()))?;
            if let Some(dir) = e.path.parent() {
                sync_dir(dir);
            }
        }
    }
    for (c, e) in changes.iter().zip(&wal.entries) {
        if matches!(c.op, Op::Delete) {
            ignore_missing(std::fs::remove_file(&e.path))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kibitzer-txn-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn put(path: PathBuf, text: &str) -> Change {
        Change {
            path,
            expect_sha256: None,
            op: Op::Put {
                bytes: text.as_bytes().to_vec(),
                mode_from: None,
            },
        }
    }

    #[test]
    fn a_file_changed_after_the_staleness_check_is_refused_and_left_alone() {
        let root = scratch("toctou");
        std::fs::write(root.join("a"), "edited-after-plan").unwrap();
        let mut stale = put(root.join("a"), "new");
        stale.expect_sha256 = Some(crate::refactor_core::sha256_hex(b"as-planned"));
        let err = begin(&root, "p", &[stale], &real_rename).err().unwrap();
        assert!(
            format!("{err:#}").contains("changed since the plan"),
            "{err:#}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a")).unwrap(),
            "edited-after-plan"
        );
        assert!(!wal_path(&root).exists(), "no log left behind");

        let mut fresh = put(root.join("a"), "new");
        fresh.expect_sha256 = Some(crate::refactor_core::sha256_hex(b"edited-after-plan"));
        begin(&root, "p2", &[fresh], &real_rename)
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "new");
    }

    #[test]
    fn commit_keeps_writes_and_removes_the_log() {
        let root = scratch("commit");
        std::fs::write(root.join("a"), "old").unwrap();
        let changes = [
            put(root.join("a"), "new"),
            put(root.join("d/b"), "fresh"),
            Change {
                path: root.join("gone"),
                expect_sha256: None,
                op: Op::Delete,
            },
        ];
        std::fs::write(root.join("gone"), "x").unwrap();
        begin(&root, "p", &changes, &real_rename)
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(root.join("d/b")).unwrap(), "fresh");
        assert!(!root.join("gone").exists());
        assert!(!wal_path(&root).exists());
    }

    #[test]
    fn rollback_and_drop_restore_originals_and_remove_new_files() {
        for explicit in [true, false] {
            let root = scratch(if explicit { "rb" } else { "drop" });
            std::fs::write(root.join("a"), "old").unwrap();
            let changes = [put(root.join("a"), "new"), put(root.join("d/b"), "fresh")];
            let pending = begin(&root, "p", &changes, &real_rename).unwrap();
            if explicit {
                pending.rollback().unwrap();
            } else {
                drop(pending);
            }
            assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "old");
            assert!(!root.join("d").exists());
            assert!(!wal_path(&root).exists());
        }
    }

    #[test]
    fn a_failing_rename_rolls_back_before_returning() {
        let root = scratch("fail");
        std::fs::write(root.join("a"), "old").unwrap();
        let calls = std::cell::Cell::new(0);
        let flaky = |from: &Path, to: &Path| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                return Err(std::io::Error::other("injected"));
            }
            std::fs::rename(from, to)
        };
        let changes = [put(root.join("a"), "new"), put(root.join("b"), "fresh")];
        let err = begin(&root, "p", &changes, &flaky).err().unwrap();
        assert!(format!("{err:#}").contains("rolled back"), "{err:#}");
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "old");
        assert!(!root.join("b").exists());
    }

    #[test]
    fn recover_finishes_a_rollback_a_crash_left_behind() {
        let root = scratch("crash");
        std::fs::write(root.join("a"), "old").unwrap();
        let changes = [put(root.join("a"), "new")];
        let pending = begin(&root, "p9", &changes, &real_rename).unwrap();
        simulate_kill(true);
        drop(pending);
        simulate_kill(false);
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "new");
        assert!(wal_path(&root).exists());
        assert_eq!(recover(&root).unwrap().as_deref(), Some("p9"));
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "old");
        assert_eq!(recover(&root).unwrap(), None);
    }

    #[test]
    fn duplicate_paths_are_rejected_before_any_write() {
        let root = scratch("dup");
        let changes = [put(root.join("a"), "1"), put(root.join("a"), "2")];
        assert!(begin(&root, "p", &changes, &real_rename).is_err());
        assert!(!root.join("a").exists());
    }

    // --- hostile or damaged logs -----------------------------------------------------------

    fn write_wal_json(root: &Path, wal: serde_json::Value, backups: &[(&str, &str)]) {
        let plans = root.join(".kibitzer/plans");
        std::fs::create_dir_all(plans.join(".wal-backup")).unwrap();
        for (name, body) in backups {
            std::fs::write(plans.join(".wal-backup").join(name), body).unwrap();
        }
        std::fs::write(plans.join(".wal"), wal.to_string()).unwrap();
    }

    fn entry(path: &Path, backup: Option<&Path>, temp: Option<&Path>) -> serde_json::Value {
        serde_json::json!({ "path": path, "backup": backup, "temp": temp })
    }

    #[test]
    fn a_planted_log_cannot_restore_into_kibitzer_or_git() {
        let root = scratch("hostile-kibitzer");
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join(".kibitzer/inspect.json"), "{}").ok();
        std::fs::create_dir_all(root.join(".kibitzer")).unwrap();
        std::fs::write(root.join(".kibitzer/inspect.json"), "good").unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), "good-hook").unwrap();
        let backup = root.join(".kibitzer/plans/.wal-backup/0");
        for victim in [
            root.join(".kibitzer/inspect.json"),
            root.join(".git/hooks/pre-commit"),
        ] {
            write_wal_json(
                &root,
                serde_json::json!({"plan_id":"x","created_dirs":[],"entries":[entry(&victim, Some(&backup), None)]}),
                &[("0", "PAYLOAD")],
            );
            let err = recover(&root).unwrap_err();
            let msg = format!("{err:#}");
            assert!(msg.contains("refusing to roll back"), "{msg}");
            let restore = format!("cp -p '{}' '{}'", backup.display(), victim.display());
            assert!(msg.contains(&restore), "missing restore command in: {msg}");
            assert!(
                std::fs::read_to_string(&victim)
                    .unwrap()
                    .starts_with("good"),
                "{} was overwritten by a planted log",
                victim.display()
            );
        }
    }

    #[test]
    fn a_symlink_cannot_launder_a_protected_target_past_the_path_check() {
        let root = scratch("hostile-symlink");
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), "good-hook").unwrap();
        std::os::unix::fs::symlink(".git/hooks/pre-commit", root.join("link")).unwrap();
        // A symlinked directory component too: `d -> .git/hooks`, entry `d/pre-commit`.
        std::os::unix::fs::symlink(".git/hooks", root.join("d")).unwrap();
        let backup = root.join(".kibitzer/plans/.wal-backup/0");
        for victim in [root.join("link"), root.join("d/pre-commit")] {
            write_wal_json(
                &root,
                serde_json::json!({"plan_id":"x","created_dirs":[],"entries":[entry(&victim, Some(&backup), None)]}),
                &[("0", "PAYLOAD")],
            );
            assert!(
                recover(&root).is_err(),
                "{} passed the guard",
                victim.display()
            );
            assert_eq!(
                std::fs::read_to_string(root.join(".git/hooks/pre-commit")).unwrap(),
                "good-hook"
            );
        }
    }

    #[test]
    fn a_symlinked_backup_and_a_mismatched_temp_are_refused() {
        let root = scratch("hostile-backup");
        let secret = scratch("hostile-secret").join("key");
        std::fs::write(&secret, "SECRET").unwrap();
        std::fs::write(root.join("a.go"), "package a\n").unwrap();
        std::fs::write(root.join("victim.go"), "keep me\n").unwrap();
        let backups = root.join(".kibitzer/plans/.wal-backup");
        std::fs::create_dir_all(&backups).unwrap();
        std::os::unix::fs::symlink(&secret, backups.join("0")).unwrap();
        write_wal_json(
            &root,
            serde_json::json!({"plan_id":"x","created_dirs":[],"entries":[entry(&root.join("a.go"), Some(&backups.join("0")), None)]}),
            &[],
        );
        assert!(recover(&root).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("a.go")).unwrap(),
            "package a\n"
        );

        // `temp` must be the one name this module would have chosen, or rollback would delete
        // any in-repo file the log names.
        write_wal_json(
            &root,
            serde_json::json!({"plan_id":"x","created_dirs":[],"entries":[entry(&root.join("a.go"), None, Some(&root.join("victim.go")))]}),
            &[],
        );
        assert!(recover(&root).is_err());
        assert!(root.join("victim.go").exists());
    }

    #[test]
    fn created_dirs_in_a_planted_log_are_checked_too() {
        let root = scratch("hostile-dirs");
        std::fs::create_dir_all(root.join(".git/refs")).unwrap();
        write_wal_json(
            &root,
            serde_json::json!({"plan_id":"x","created_dirs":[root.join(".git/refs")],"entries":[]}),
            &[],
        );
        assert!(recover(&root).is_err());
        assert!(root.join(".git/refs").exists());
    }

    #[test]
    fn a_symlinked_kibitzer_dir_is_refused_and_nothing_is_written_through_it() {
        let root = scratch("kibitzer-symlink");
        let outside = scratch("kibitzer-outside");
        std::os::unix::fs::symlink(&outside, root.join(".kibitzer")).unwrap();
        let changes = [put(root.join("a"), "x")];
        assert!(begin(&root, "p", &changes, &real_rename).is_err());
        assert!(recover(&root).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        assert!(!root.join("a").exists());
    }

    #[test]
    fn an_oversized_log_is_refused_without_being_read_into_memory() {
        let root = scratch("huge-wal");
        let plans = root.join(".kibitzer/plans");
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(
            plans.join(".wal"),
            vec![b' '; (MAX_WAL_BYTES + 10) as usize],
        )
        .unwrap();
        let err = recover(&root).unwrap_err();
        assert!(format!("{err:#}").contains("larger than"), "{err:#}");
    }

    #[test]
    fn a_torn_log_is_a_clear_error_that_leaves_the_tree_alone() {
        let root = scratch("torn-wal");
        std::fs::write(root.join("a"), "old").unwrap();
        let plans = root.join(".kibitzer/plans");
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(plans.join(".wal"), "{\"plan_id\":\"x\",\"entries\":[{\"pa").unwrap();
        let err = recover(&root).unwrap_err();
        assert!(format!("{err:#}").contains("corrupt"), "{err:#}");
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "old");
        // And a new transaction refuses to start over it rather than clobbering the evidence.
        assert!(begin(&root, "p", &[put(root.join("b"), "x")], &real_rename).is_err());
        assert!(!root.join("b").exists());
    }

    #[test]
    fn a_log_from_the_first_mcp_release_is_still_recoverable() {
        let root = scratch("legacy-wal");
        // Crash state of a moved-and-rewritten file under the old format: a.go deleted, b.go new.
        std::fs::write(root.join("b.go"), "NEW BODY").unwrap();
        let plans = root.join(".kibitzer/plans");
        std::fs::create_dir_all(plans.join(".wal-backup")).unwrap();
        std::fs::write(plans.join(".wal-backup/0"), "ORIGINAL").unwrap();
        let wal = serde_json::json!({
            "plan_id": "old",
            "created_dirs": [],
            "entries": [{
                "original": root.join("a.go"), "backup": plans.join(".wal-backup/0"),
                "dest": root.join("b.go"), "temp": root.join(".b.go.kibitzer-tmp"),
            }]
        });
        std::fs::write(plans.join(".wal"), wal.to_string()).unwrap();
        assert_eq!(recover(&root).unwrap().as_deref(), Some("old"));
        assert_eq!(
            std::fs::read_to_string(root.join("a.go")).unwrap(),
            "ORIGINAL"
        );
        assert!(!root.join("b.go").exists());
    }

    // --- crash recovery ------------------------------------------------------------------

    #[test]
    fn a_crash_mid_swap_with_leftover_temps_and_deletes_is_fully_rolled_back() {
        let root = scratch("midswap");
        std::fs::write(root.join("a"), "A-old").unwrap();
        std::fs::write(root.join("gone"), "G-old").unwrap();
        let changes = [
            put(root.join("a"), "A-new"),
            put(root.join("sub/b"), "B-new"),
            Change {
                path: root.join("gone"),
                expect_sha256: None,
                op: Op::Delete,
            },
        ];
        let renames = std::cell::Cell::new(0);
        let dies_on_second = |from: &Path, to: &Path| {
            renames.set(renames.get() + 1);
            if renames.get() == 2 {
                panic!("simulated kill");
            }
            std::fs::rename(from, to)
        };
        simulate_kill(true);
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = begin(&root, "p", &changes, &dies_on_second);
        }));
        simulate_kill(false);
        assert!(crashed.is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("a")).unwrap(),
            "A-new",
            "first rename landed"
        );
        assert!(
            root.join("sub/.b.kibitzer-tmp").exists(),
            "a temp is left behind"
        );
        assert!(wal_path(&root).exists());

        assert_eq!(recover(&root).unwrap().as_deref(), Some("p"));
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "A-old");
        assert_eq!(std::fs::read_to_string(root.join("gone")).unwrap(), "G-old");
        assert!(
            !root.join("sub").exists(),
            "created directory and temp removed"
        );
        assert!(!root.join(".kibitzer").exists());
    }

    #[test]
    fn the_next_transaction_recovers_an_orphan_before_doing_anything() {
        let root = scratch("orphan-then-begin");
        std::fs::write(root.join("a"), "old").unwrap();
        let p1 = begin(&root, "p1", &[put(root.join("a"), "from-p1")], &real_rename).unwrap();
        simulate_kill(true);
        drop(p1);
        simulate_kill(false);
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "from-p1");
        let p2 = session(&root).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("a")).unwrap(),
            "old",
            "session() must roll the orphan back before the caller reads the tree"
        );
        drop(p2);
    }

    #[test]
    fn a_crash_while_changing_a_kibitzer_file_needs_a_manual_rollback_with_a_clear_message() {
        let root = scratch("kibitzer-crash");
        std::fs::create_dir_all(root.join(".kibitzer")).unwrap();
        std::fs::write(root.join(".kibitzer/inspect.json"), "old").unwrap();
        let target = root.join(".kibitzer/inspect.json");
        // In-process rollback (a failed verify) restores it normally: it uses the in-memory log.
        let p = begin(&root, "p", &[put(target.clone(), "new")], &real_rename).unwrap();
        p.rollback().unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        // A crash leaves the on-disk log, which recovery refuses to act on for `.kibitzer` paths.
        let p = begin(&root, "p", &[put(target.clone(), "new")], &real_rename).unwrap();
        simulate_kill(true);
        drop(p);
        simulate_kill(false);
        let err = recover(&root).unwrap_err();
        assert!(format!("{err:#}").contains("restore by hand"), "{err:#}");
        assert!(
            root.join(".kibitzer/plans/.wal-backup/0").exists(),
            "backup kept for the manual step"
        );
    }

    #[test]
    fn a_symlink_planted_at_the_temp_name_is_replaced_not_followed() {
        let root = scratch("temp-symlink");
        let outside = scratch("temp-outside").join("target");
        std::fs::write(&outside, "UNTOUCHED").unwrap();
        std::fs::write(root.join("a"), "old").unwrap();
        std::os::unix::fs::symlink(&outside, root.join(".a.kibitzer-tmp")).unwrap();
        begin(&root, "p", &[put(root.join("a"), "new")], &real_rename)
            .unwrap()
            .commit()
            .unwrap();
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "UNTOUCHED");
        assert_eq!(std::fs::read_to_string(root.join("a")).unwrap(), "new");
    }

    #[test]
    fn a_failed_rollback_is_a_typed_error_not_just_text() {
        let root = scratch("rollback-fails");
        std::fs::write(root.join("a"), "old").unwrap();
        let sabotage = |_: &Path, _: &Path| {
            // Destroy the backup the rollback needs, then fail the rename.
            let _ = std::fs::remove_dir_all(root.join(".kibitzer/plans/.wal-backup"));
            Err(std::io::Error::other("injected"))
        };
        let err = begin(&root, "p", &[put(root.join("a"), "new")], &sabotage)
            .err()
            .unwrap();
        assert!(err.downcast_ref::<RollbackFailed>().is_some(), "{err:#}");
        assert!(wal_path(&root).exists(), "log kept so recover can retry");
    }

    #[test]
    fn rollback_restores_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = scratch("mode");
        std::fs::write(root.join("run.sh"), "old").unwrap();
        std::fs::set_permissions(root.join("run.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let p = begin(&root, "p", &[put(root.join("run.sh"), "new")], &real_rename).unwrap();
        // A same-mode put keeps 0755; clobber it to prove the restore sets the mode itself.
        std::fs::set_permissions(root.join("run.sh"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        p.rollback().unwrap();
        let mode = std::fs::metadata(root.join("run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
        assert_eq!(std::fs::read_to_string(root.join("run.sh")).unwrap(), "old");
    }

    #[test]
    fn rollback_attempts_every_entry_and_keeps_the_log_on_failure() {
        let root = scratch("rb-all");
        std::fs::write(root.join("a"), "A").unwrap();
        std::fs::write(root.join("b"), "B").unwrap();
        let changes = [put(root.join("a"), "A2"), put(root.join("b"), "B2")];
        let p = begin(&root, "p", &changes, &real_rename).unwrap();
        // Break the first entry's restore only (its backup vanishes).
        std::fs::remove_file(root.join(".kibitzer/plans/.wal-backup/0")).unwrap();
        let err = p.rollback().unwrap_err();
        assert!(format!("{err:#}").contains("restoring"), "{err:#}");
        assert_eq!(
            std::fs::read_to_string(root.join("b")).unwrap(),
            "B",
            "later entry restored"
        );
        assert!(wal_path(&root).exists(), "log kept for a retry");
    }

    #[test]
    fn a_second_session_on_a_locked_root_errors_and_recover_skips_it() {
        let root = scratch("flock");
        std::fs::write(root.join("a"), "old").unwrap();
        // A live transaction elsewhere: its log exists and its lock is held.
        let held = PlansLock::acquire(&root).unwrap();
        std::fs::write(
            wal_path(&root),
            r#"{"plan_id":"live","entries":[],"created_dirs":[]}"#,
        )
        .unwrap();
        assert!(PlansLock::try_acquire(&root).unwrap().is_none());
        let err = session(&root).err().unwrap();
        assert!(
            format!("{err:#}").contains("another kibitzer process"),
            "{err:#}"
        );
        assert_eq!(
            recover(&root).unwrap(),
            None,
            "recovery must not touch a live log"
        );
        assert!(wal_path(&root).exists());
        drop(held);
        assert_eq!(recover(&root).unwrap().as_deref(), Some("live"));
    }

    #[test]
    fn the_lock_covers_everything_between_session_and_commit() {
        let root = scratch("lock");
        let first = session(&root).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let r2 = root.clone();
        let t = std::thread::spawn(move || {
            let _second = session(&r2).unwrap();
            tx.send(()).unwrap();
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "a second session started while the first still held the lock"
        );
        drop(first);
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("second session after release");
        t.join().unwrap();
    }
}
