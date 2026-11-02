//! Atomic apply of a [`RefactorPlan`]: all writes land or the tree is restored.
//!
//! Order: validate and compute every new file body in memory (nothing written) → back up the
//! originals → write the WAL → write temp files → rename loop → verify with the Go toolchain.
//! Any failure after the WAL exists rolls back from the backups the WAL points at, and a crash
//! leaves the WAL behind so [`recover`] can finish the rollback on the next start.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Edit, RefactorPlan, go_syntax};

/// Serializes apply and recovery in this process so one apply never rolls back another's live
/// WAL. Two separate server processes on one repo are not covered.
static APPLY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const PLANS_DIR: &str = ".kibitzer/plans";
const WAL_FILE: &str = ".wal";
const BACKUP_DIR: &str = ".wal-backup";
/// Only these run after an apply; a plan arrives over MCP, so its command list is not trusted.
const ALLOWED_VERIFY: [&str; 2] = ["go build ./...", "go vet ./..."];

/// Side effects the apply path performs that tests need to fail on demand.
pub trait ApplyIo {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    /// Runs `cmd` in `dir`; returns `(success, combined output)`.
    fn run(&self, cmd: &str, dir: &Path) -> (bool, String);
}

pub struct RealIo;

impl ApplyIo for RealIo {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    fn run(&self, cmd: &str, dir: &Path) -> (bool, String) {
        let mut parts = cmd.split_whitespace();
        let Some(program) = parts.next() else {
            return (false, "empty command".into());
        };
        match std::process::Command::new(program)
            .args(parts)
            .current_dir(dir)
            .output()
        {
            Ok(out) => {
                let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&out.stderr));
                (out.status.success(), text)
            }
            Err(e) => (false, format!("failed to run `{cmd}`: {e}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationStep {
    pub cmd: String,
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyResult {
    pub plan_id: String,
    pub files_written: Vec<PathBuf>,
    pub verification: Vec<VerificationStep>,
}

#[derive(Serialize, Deserialize)]
struct Wal {
    plan_id: String,
    entries: Vec<WalEntry>,
    created_dirs: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct WalEntry {
    original: PathBuf,
    backup: PathBuf,
    dest: PathBuf,
    temp: PathBuf,
}

fn plans_dir(root: &Path) -> PathBuf {
    root.join(PLANS_DIR)
}

fn wal_path(root: &Path) -> PathBuf {
    plans_dir(root).join(WAL_FILE)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Rolls back an apply a crashed process left half-done. Returns the interrupted plan id, or
/// `None` when no WAL exists. Safe to call at every startup and before every apply.
pub fn recover(root: &Path) -> Result<Option<String>> {
    let _guard = APPLY_LOCK
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
    rollback(root, &wal)?;
    Ok(Some(wal.plan_id))
}

/// Restores every original from its backup, then removes whatever the apply created. Leaves the
/// WAL in place if a restore fails so a later [`recover`] can retry.
fn rollback(root: &Path, wal: &Wal) -> Result<()> {
    for e in &wal.entries {
        if let Some(parent) = e.original.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&e.backup, &e.original).with_context(|| {
            format!(
                "restoring {} from {}",
                e.original.display(),
                e.backup.display()
            )
        })?;
    }
    for e in &wal.entries {
        if e.dest != e.original {
            ignore_missing(std::fs::remove_file(&e.dest))?;
        }
        ignore_missing(std::fs::remove_file(&e.temp))?;
    }
    for dir in wal.created_dirs.iter().rev() {
        // Non-empty or already gone: either way there is nothing of ours left to remove.
        let _ = std::fs::remove_dir(dir);
    }
    // The WAL is the commit point: drop it before the backups so a crash in between never leaves
    // a WAL whose backups are gone.
    ignore_missing(std::fs::remove_file(wal_path(root)))?;
    let _ = std::fs::remove_dir_all(plans_dir(root).join(BACKUP_DIR));
    Ok(())
}

fn ignore_missing(r: std::io::Result<()>) -> Result<()> {
    match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// One output file: its original path, destination, and new body.
struct Output {
    original: PathBuf,
    dest: PathBuf,
    body: String,
}

fn check_plan(root: &Path, plan: &RefactorPlan) -> Result<()> {
    let mut unsigned = plan.clone();
    unsigned.id = String::new();
    if sha256_hex(serde_json::to_string(&unsigned)?.as_bytes()) != plan.id {
        bail!("plan id does not match its contents; re-run plan_refactor");
    }
    for cmd in &plan.verification_cmds {
        if !ALLOWED_VERIFY.contains(&cmd.as_str()) {
            bail!("verification command `{cmd}` is not allowed");
        }
    }
    let touched: BTreeSet<&Path> = plan.touched.iter().map(|t| t.path.as_path()).collect();
    let mut paths: Vec<&Path> = Vec::new();
    for edit in &plan.edits {
        match edit {
            Edit::MoveFile { from, to } => paths.extend([from.as_path(), to.as_path()]),
            Edit::ReplacePackageDecl { file, .. } | Edit::ReplaceImportPath { file, .. } => {
                paths.push(file.as_path());
            }
        }
        for source in edit_sources(edit) {
            if !touched.contains(source) {
                bail!(
                    "edit targets {} which the plan did not record",
                    source.display()
                );
            }
        }
    }
    for edit in &plan.edits {
        match edit {
            Edit::ReplacePackageDecl { new_name, .. } if !is_go_ident(new_name) => {
                bail!("`{new_name}` is not a valid Go package name");
            }
            Edit::ReplaceImportPath { new_path, .. } if !is_safe_import_path(new_path) => {
                bail!("`{new_path}` is not a valid import path");
            }
            _ => {}
        }
    }
    paths.extend(plan.touched.iter().map(|t| t.path.as_path()));
    if let Some(p) = paths.iter().find(|p| !is_inside(root, p)) {
        bail!("{} is outside the repository", p.display());
    }
    Ok(())
}

fn is_go_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

fn is_safe_import_path(s: &str) -> bool {
    !s.is_empty()
        && !s
            .chars()
            .any(|c| c == '"' || c == '`' || c == '\\' || c.is_whitespace() || c.is_control())
}

/// True when `p` is under `root` without `..` hops or a symlink that leaves the tree. Checks
/// the deepest existing ancestor, since the destination may not exist yet.
pub(super) fn is_inside(root: &Path, p: &Path) -> bool {
    use std::path::Component;
    if !p.starts_with(root) || p.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    let (Ok(canon_root), Some(existing)) =
        (root.canonicalize(), p.ancestors().find(|a| a.exists()))
    else {
        return false;
    };
    existing
        .canonicalize()
        .is_ok_and(|c| c.starts_with(canon_root))
}

fn edit_sources(edit: &Edit) -> Vec<&Path> {
    match edit {
        Edit::MoveFile { from, .. } => vec![from.as_path()],
        Edit::ReplacePackageDecl { file, .. } | Edit::ReplaceImportPath { file, .. } => {
            vec![file.as_path()]
        }
    }
}

fn check_not_stale(plan: &RefactorPlan) -> Result<()> {
    for t in &plan.touched {
        let bytes = std::fs::read(&t.path)
            .with_context(|| format!("stale plan: {} cannot be read", t.path.display()))?;
        if sha256_hex(&bytes) != t.sha256 {
            bail!(
                "stale plan: {} changed since planning; re-run plan_refactor",
                t.path.display()
            );
        }
    }
    Ok(())
}

/// Applies every edit to in-memory bodies. Nothing touches the disk, so an edit that no
/// longer matches (an import that is gone) aborts before the first write.
fn compute_outputs(plan: &RefactorPlan) -> Result<Vec<Output>> {
    let mut bodies: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut dests: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    for t in &plan.touched {
        bodies.insert(t.path.clone(), std::fs::read_to_string(&t.path)?);
        dests.insert(t.path.clone(), t.path.clone());
    }
    for edit in &plan.edits {
        match edit {
            Edit::ReplacePackageDecl { file, new_name, .. } => {
                let body = bodies.get_mut(file).context("unrecorded file")?;
                *body = go_syntax::replace_package_name(body, new_name)
                    .with_context(|| format!("no package clause in {}", file.display()))?;
            }
            Edit::ReplaceImportPath {
                file,
                old_path,
                new_path,
                alias,
            } => {
                let body = bodies.get_mut(file).context("unrecorded file")?;
                *body = go_syntax::replace_import_path(body, old_path, new_path, alias.as_deref())
                    .with_context(|| {
                        format!("import {old_path} not found in {}", file.display())
                    })?;
            }
            Edit::MoveFile { from, to } => {
                if to.exists() {
                    bail!("destination {} already exists", to.display());
                }
                dests.insert(from.clone(), to.clone());
            }
        }
    }
    let unique: BTreeSet<&PathBuf> = dests.values().collect();
    if unique.len() != dests.len() {
        bail!("two files would be written to the same destination");
    }
    let mut outputs = Vec::new();
    for (original, dest) in dests {
        let body = bodies
            .remove(&original)
            .with_context(|| format!("no body for {}", original.display()))?;
        outputs.push(Output {
            original,
            dest,
            body,
        });
    }
    Ok(outputs)
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

fn temp_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    dest.with_file_name(format!(".{name}.kibitzer-tmp"))
}

/// Missing ancestors of `dir`, outermost first, so rollback can remove only what we created.
fn missing_ancestors(dir: &Path) -> Vec<PathBuf> {
    let mut missing: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|d| !d.exists())
        .map(Path::to_path_buf)
        .collect();
    missing.reverse();
    missing
}

fn go_module_dir(root: &Path, start: &Path) -> PathBuf {
    let mut dir = start.parent();
    while let Some(d) = dir {
        if d.join("go.mod").is_file() {
            return d.to_path_buf();
        }
        if d == root {
            break;
        }
        dir = d.parent();
    }
    root.to_path_buf()
}

/// Applies `plan` under `root`. Errors leave the tree as it was: a stale or invalid plan is
/// rejected before any write, and a failed write or failed build/vet is rolled back.
pub fn apply_plan(root: &Path, plan: &RefactorPlan, io: &dyn ApplyIo) -> Result<ApplyResult> {
    let _guard = APPLY_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    recover_locked(root)?;
    check_plan(root, plan)?;
    check_not_stale(plan)?;
    let outputs = compute_outputs(plan)?;

    let wal = begin(root, plan, &outputs)?;
    if let Err(e) = write_and_swap(&wal, &outputs, io) {
        return Err(failed(root, &wal, e));
    }
    let verification = match verify(root, plan, io) {
        Ok(v) => v,
        Err(e) => return Err(failed(root, &wal, e)),
    };
    commit_cleanup(root)?;
    Ok(ApplyResult {
        plan_id: plan.id.clone(),
        files_written: outputs.into_iter().map(|o| o.dest).collect(),
        verification,
    })
}

/// Backs up every original and makes the WAL durable, before any destination is touched.
fn begin(root: &Path, plan: &RefactorPlan, outputs: &[Output]) -> Result<Wal> {
    let backup_root = plans_dir(root).join(BACKUP_DIR);
    std::fs::create_dir_all(&backup_root)?;
    let mut created_dirs = Vec::new();
    let mut entries = Vec::new();
    for (i, o) in outputs.iter().enumerate() {
        let backup = backup_root.join(i.to_string());
        std::fs::copy(&o.original, &backup)?;
        for d in o.dest.parent().map(missing_ancestors).unwrap_or_default() {
            if !created_dirs.contains(&d) {
                created_dirs.push(d);
            }
        }
        entries.push(WalEntry {
            original: o.original.clone(),
            backup,
            dest: o.dest.clone(),
            temp: temp_path(&o.dest),
        });
    }
    let wal = Wal {
        plan_id: plan.id.clone(),
        entries,
        created_dirs,
    };
    let wal_tmp = plans_dir(root).join(".wal.tmp");
    write_synced(&wal_tmp, &serde_json::to_vec(&wal)?)?;
    std::fs::rename(&wal_tmp, wal_path(root))?;
    Ok(wal)
}

fn verify(root: &Path, plan: &RefactorPlan, io: &dyn ApplyIo) -> Result<Vec<VerificationStep>> {
    let module_dir = plan
        .edits
        .iter()
        .find_map(|e| match e {
            Edit::MoveFile { from, .. } => Some(go_module_dir(root, from)),
            _ => None,
        })
        .unwrap_or_else(|| root.to_path_buf());
    let mut steps = Vec::new();
    for cmd in &plan.verification_cmds {
        let (ok, output) = io.run(cmd, &module_dir);
        if !ok {
            bail!("`{cmd}` failed:\n{output}");
        }
        steps.push(VerificationStep {
            cmd: cmd.clone(),
            output,
        });
    }
    Ok(steps)
}

fn write_and_swap(wal: &Wal, outputs: &[Output], io: &dyn ApplyIo) -> Result<()> {
    for (o, e) in outputs.iter().zip(&wal.entries) {
        if let Some(parent) = e.dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_synced(&e.temp, o.body.as_bytes())
            .with_context(|| format!("writing {}", e.temp.display()))?;
        if let Ok(meta) = std::fs::metadata(&e.original) {
            std::fs::set_permissions(&e.temp, meta.permissions())?;
        }
    }
    for e in &wal.entries {
        io.rename(&e.temp, &e.dest)
            .with_context(|| format!("renaming into {}", e.dest.display()))?;
    }
    for e in &wal.entries {
        if e.dest != e.original {
            std::fs::remove_file(&e.original)?;
        }
    }
    Ok(())
}

/// Rolls back after `cause`, folding a failed rollback into the error so it is never silent.
fn failed(root: &Path, wal: &Wal, cause: anyhow::Error) -> anyhow::Error {
    match rollback(root, wal) {
        Ok(()) => cause.context("apply rolled back; the tree is unchanged"),
        Err(rb) => anyhow::anyhow!(
            "{cause:#}\nrollback also failed ({rb:#}); the WAL at {} is kept so the next start can retry",
            wal_path(root).display()
        ),
    }
}

fn commit_cleanup(root: &Path) -> Result<()> {
    ignore_missing(std::fs::remove_file(wal_path(root)))?;
    let _ = std::fs::remove_dir_all(plans_dir(root).join(BACKUP_DIR));
    Ok(())
}
