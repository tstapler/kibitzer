//! All-or-nothing multi-file write with crash recovery.
//!
//! Order: back up every original and make a write-ahead log durable → write temp files → rename
//! them into place and delete removed files → the caller verifies → [`Pending::commit`] drops the
//! log, or [`Pending::rollback`] restores from the backups. A crash leaves the log behind so
//! [`recover`] finishes the rollback on the next start; dropping an undecided [`Pending`]
//! (a panic mid-verify) rolls back too.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::guard::is_inside;

/// Serializes transactions in this process so one never rolls back another's live log. Two
/// separate processes on one repo are not covered.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const PLANS_DIR: &str = ".kibitzer/plans";
const WAL_FILE: &str = ".wal";
const BACKUP_DIR: &str = ".wal-backup";

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
}

#[derive(Serialize, Deserialize)]
struct Wal {
    plan_id: String,
    entries: Vec<Entry>,
    created_dirs: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    /// Copy of the original; `None` when the file did not exist, so rollback removes it.
    backup: Option<PathBuf>,
    /// Temp file written beside `path` for a `Put`.
    temp: Option<PathBuf>,
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
            Err(rb) => anyhow::anyhow!(
                "{cause:#}\nrollback also failed ({rb:#}); the WAL at {} is kept so the next start can retry",
                wal_path(&self.root).display()
            ),
        }
    }
}

/// A killed process runs no destructors; tests flip this to leave the log behind like one.
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
        if !self.settled && !killed() {
            let _ = rollback(&self.root, &self.wal);
        }
    }
}

/// Applies `changes` (absolute paths under `root`). Any failure before this returns rolls the
/// tree back; afterwards the caller owns the decision.
pub fn begin(root: &Path, plan_id: &str, changes: &[Change], rename: Rename) -> Result<Pending> {
    let lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    recover_locked(root)?;
    let mut seen = BTreeSet::new();
    if let Some(c) = changes.iter().find(|c| !seen.insert(&c.path)) {
        bail!("{} is changed twice in one plan", c.path.display());
    }
    let wal = match write_wal(root, plan_id, changes) {
        Ok(w) => w,
        Err(e) => {
            let _ = std::fs::remove_dir_all(backup_dir(root));
            return Err(e);
        }
    };
    let pending = Pending {
        root: root.to_path_buf(),
        wal,
        settled: false,
        _lock: lock,
    };
    match swap(&pending.wal, changes, rename) {
        Ok(()) => Ok(pending),
        Err(e) => Err(pending.fail(e)),
    }
}

/// Rolls back a transaction a crashed process left half-done. Returns the interrupted plan id,
/// or `None` when no log exists. Safe to call at every startup and before every transaction.
pub fn recover(root: &Path) -> Result<Option<String>> {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    recover_locked(root)
}

fn recover_locked(root: &Path) -> Result<Option<String>> {
    let path = wal_path(root);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let wal: Wal = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} is corrupt; restore from {BACKUP_DIR} by hand",
            path.display()
        )
    })?;
    check_wal_paths(root, &wal)?;
    rollback(root, &wal)?;
    Ok(Some(wal.plan_id))
}

/// The log lives in the repo, so a hostile one must not steer rollback at files outside it, at
/// `.git`, or at the log's own directory.
fn check_wal_paths(root: &Path, wal: &Wal) -> Result<()> {
    let backups = backup_dir(root);
    let guarded = |p: &Path| {
        p.strip_prefix(root).is_ok_and(|rel| {
            rel.components().any(|c| c.as_os_str() == ".git") || rel.starts_with(PLANS_DIR)
        })
    };
    for e in &wal.entries {
        for p in std::iter::once(&e.path).chain(e.temp.iter()) {
            if !is_inside(root, p) || guarded(p) {
                bail!(
                    "WAL names {} outside the repository; refusing to roll back",
                    p.display()
                );
            }
        }
        if let Some(b) = &e.backup
            && b.parent() != Some(backups.as_path())
        {
            bail!("WAL backup {} is not in {}", b.display(), backups.display());
        }
    }
    if let Some(d) = wal.created_dirs.iter().find(|d| !is_inside(root, d)) {
        bail!(
            "WAL names {} outside the repository; refusing to roll back",
            d.display()
        );
    }
    Ok(())
}

fn rollback(root: &Path, wal: &Wal) -> Result<()> {
    for e in &wal.entries {
        match &e.backup {
            Some(b) => {
                if let Some(parent) = e.path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(b, &e.path).with_context(|| {
                    format!("restoring {} from {}", e.path.display(), b.display())
                })?;
            }
            None => ignore_missing(std::fs::remove_file(&e.path))?,
        }
        if let Some(t) = &e.temp {
            ignore_missing(std::fs::remove_file(t))?;
        }
    }
    for dir in wal.created_dirs.iter().rev() {
        // Non-empty or already gone: either way there is nothing of ours left to remove.
        let _ = std::fs::remove_dir(dir);
    }
    cleanup(root)
}

/// The log is the commit point: drop it before the backups so a crash in between never leaves a
/// log whose backups are gone.
fn cleanup(root: &Path) -> Result<()> {
    ignore_missing(std::fs::remove_file(wal_path(root)))?;
    let _ = std::fs::remove_dir_all(backup_dir(root));
    Ok(())
}

fn ignore_missing(r: std::io::Result<()>) -> Result<()> {
    match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
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
        let backup = match std::fs::symlink_metadata(&c.path) {
            Ok(m) if m.is_file() => {
                let b = backup_dir(root).join(i.to_string());
                std::fs::copy(&c.path, &b)?;
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
            op: Op::Put {
                bytes: text.as_bytes().to_vec(),
                mode_from: None,
            },
        }
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
}
