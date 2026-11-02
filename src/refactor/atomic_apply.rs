//! Atomic, idempotent plan application: snapshot every touched path, write via
//! temp-then-rename, and restore the snapshot byte-for-byte on any failure.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::plan::sha256_hex;
use super::types::{ApplyResult, ApplyStatus, MovePlan, OperationKind, PlanStep, PlanStepKind};
use super::verify::run_verify;

#[derive(Default)]
pub struct ApplyOptions {
    /// Run `go build/vet/test` after writing (always on from the CLI).
    pub verify: bool,
    pub affected_only: bool,
    pub post_apply: Option<String>,
    /// Test hook: fail right after step `n` has been written.
    pub inject_failure_after_step: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
enum StepState {
    Pending,
    Done,
    Conflict(String),
}

fn read(root: &Path, rel: &str) -> Option<Vec<u8>> {
    std::fs::read(root.join(rel)).ok()
}

fn classify(root: &Path, s: &PlanStep) -> StepState {
    let t = &s.transform;
    let want = t.new_content.as_deref().map(str::as_bytes);
    let old_matches = |p: &str| match (read(root, p), &t.old_sha256) {
        (Some(b), Some(h)) => &sha256_hex(&b) == h,
        (Some(_), None) => true,
        (None, _) => false,
    };
    match (&t.old_path, &t.new_path) {
        (Some(o), Some(n)) if o == n => {
            let cur = read(root, o);
            if cur.as_deref() == want {
                StepState::Done
            } else if old_matches(o) {
                StepState::Pending
            } else {
                StepState::Conflict(format!("{o} changed since the plan was made"))
            }
        }
        (Some(o), Some(n)) => {
            let (old, new) = (read(root, o), read(root, n));
            match (old.is_some(), new) {
                (false, Some(b)) if Some(b.as_slice()) == want => StepState::Done,
                (true, None) if old_matches(o) => StepState::Pending,
                (true, None) => StepState::Conflict(format!("{o} changed since the plan was made")),
                _ => StepState::Conflict(format!(
                    "{o} -> {n}: neither the plan's before nor after state"
                )),
            }
        }
        (None, Some(n)) => match read(root, n) {
            None => StepState::Pending,
            Some(b) if Some(b.as_slice()) == want => StepState::Done,
            Some(_) => StepState::Conflict(format!("{n} already exists with different content")),
        },
        (Some(o), None) => {
            if read(root, o).is_none() {
                StepState::Done
            } else if old_matches(o) {
                StepState::Pending
            } else {
                StepState::Conflict(format!("{o} changed since the plan was made"))
            }
        }
        (None, None) => StepState::Done,
    }
}

struct Journal {
    root: PathBuf,
    snapshot: BTreeMap<String, Option<Vec<u8>>>,
    perms: BTreeMap<String, std::fs::Permissions>,
    created_dirs: Vec<PathBuf>,
    tmp_files: Vec<PathBuf>,
}

impl Journal {
    fn new(root: &Path, plan: &MovePlan) -> Result<Self> {
        let mut snapshot = BTreeMap::new();
        let mut perms = BTreeMap::new();
        for s in &plan.steps {
            for p in [&s.transform.old_path, &s.transform.new_path]
                .into_iter()
                .flatten()
            {
                if !snapshot.contains_key(p) {
                    let full = root.join(p);
                    let bytes = match std::fs::read(&full) {
                        Ok(b) => Some(b),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                        Err(e) => bail!("cannot snapshot {p}: {e}"),
                    };
                    if let Ok(m) = std::fs::metadata(&full) {
                        perms.insert(p.clone(), m.permissions());
                    }
                    snapshot.insert(p.clone(), bytes);
                }
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            snapshot,
            perms,
            created_dirs: vec![],
            tmp_files: vec![],
        })
    }

    fn ensure_parent(&mut self, full: &Path) -> std::io::Result<()> {
        let Some(parent) = full.parent() else {
            return Ok(());
        };
        let mut missing = vec![];
        let mut cur = parent;
        while !cur.exists() && cur != self.root {
            missing.push(cur.to_path_buf());
            match cur.parent() {
                Some(p) => cur = p,
                None => break,
            }
        }
        std::fs::create_dir_all(parent)?;
        missing.reverse();
        self.created_dirs.extend(missing);
        Ok(())
    }

    /// Write via an exclusively created temp file (never follows a pre-planted symlink),
    /// carrying over the permissions of `mode_from` so a move keeps a file's mode.
    fn write_atomic(
        &mut self,
        rel: &str,
        content: &[u8],
        mode_from: Option<&str>,
    ) -> std::io::Result<()> {
        use std::io::Write;
        let full = self.root.join(rel);
        self.ensure_parent(&full)?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let tmp = full.with_file_name(format!(
            ".{}.kibitzer-tmp.{}.{nanos}",
            super::workspace::base_of(rel),
            std::process::id()
        ));
        self.tmp_files.push(tmp.clone());
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(content)?;
        drop(file);
        if let Some(perms) = mode_from.and_then(|m| self.perms.get(m)) {
            std::fs::set_permissions(&tmp, perms.clone())?;
        }
        std::fs::rename(&tmp, &full)
    }

    /// Put every snapshotted path back. Errors are collected, never short-circuited.
    fn restore(&mut self) -> Vec<String> {
        let mut errors = vec![];
        for tmp in self.tmp_files.drain(..) {
            let _ = std::fs::remove_file(tmp);
        }
        for (rel, original) in &self.snapshot {
            let full = self.root.join(rel);
            match original {
                Some(bytes) => {
                    let r = full
                        .parent()
                        .map_or(Ok(()), std::fs::create_dir_all)
                        .and_then(|()| std::fs::write(&full, bytes));
                    if let Err(e) = r {
                        errors.push(format!("restore {rel}: {e}"));
                    } else if let Some(perms) = self.perms.get(rel) {
                        let _ = std::fs::set_permissions(&full, perms.clone());
                    }
                }
                None => {
                    if let Err(e) = std::fs::remove_file(&full)
                        && e.kind() != std::io::ErrorKind::NotFound
                    {
                        errors.push(format!("remove {rel}: {e}"));
                    }
                }
            }
        }
        for d in self.created_dirs.iter().rev() {
            let _ = std::fs::remove_dir(d);
        }
        for e in &errors {
            eprintln!("kibitzer refactor: {e}");
        }
        errors
    }
}

fn prune_empty_dirs(root: &Path, plan: &MovePlan) {
    for s in &plan.steps {
        if let Some(old) = &s.transform.old_path
            && s.transform.new_path.as_ref() != Some(old)
        {
            let mut dir = root.join(old);
            while dir.pop() && dir != root && std::fs::remove_dir(&dir).is_ok() {}
        }
    }
}

const POLICY_FILES: &[&str] = &[
    ".kibitzer/inspect.json",
    ".golangci.yml",
    ".golangci.yaml",
    ".go-arch-lint.yml",
    ".depguard.yml",
];

/// Plans are data a reviewer may have edited, so a plan may only touch what the tool itself
/// would produce: `.go` files for move operations, known policy files for `sync_policy`;
/// never a path outside the repo or one that crosses a symlink; and package arguments must
/// be plain relative package paths (never a `go` flag).
fn validate_plan(plan: &MovePlan, root: &Path) -> Result<()> {
    let policy = plan.operation == OperationKind::SyncPolicy;
    for s in &plan.steps {
        if policy != (s.kind == PlanStepKind::PolicyUpdate) {
            bail!(
                "plan step kind {:?} does not belong to a {:?} plan; refusing to apply",
                s.kind,
                plan.operation
            );
        }
        for p in [&s.transform.old_path, &s.transform.new_path]
            .into_iter()
            .flatten()
        {
            let path = Path::new(p);
            let escapes = path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir));
            if escapes || p.is_empty() {
                bail!("plan path `{p}` is absolute or leaves the repository; refusing to apply");
            }
            let allowed = if policy {
                POLICY_FILES.contains(&p.as_str())
            } else {
                p.ends_with(".go") && !p.split('/').any(|c| c.starts_with('.'))
            };
            if !allowed {
                bail!("plan path `{p}` is not a file this operation may touch; refusing to apply");
            }
            let mut cur = root.to_path_buf();
            for comp in path.components() {
                cur.push(comp);
                match std::fs::symlink_metadata(&cur) {
                    Ok(m) if m.file_type().is_symlink() => {
                        bail!("plan path `{p}` crosses a symlink; refusing to apply")
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
    }
    for pkg in &plan.affected_packages {
        if pkg != "." && !(pkg.starts_with("./") && !pkg.contains("..")) {
            bail!("plan package `{pkg}` is not a relative package path; refusing to apply");
        }
    }
    Ok(())
}

pub fn apply(plan: &MovePlan, root: &Path, opts: &ApplyOptions) -> Result<ApplyResult> {
    validate_plan(plan, root)?;
    if !plan.blockers.is_empty() {
        let codes: Vec<&str> = plan.blockers.iter().map(|b| b.code.as_str()).collect();
        bail!(
            "plan has {} blocker(s) ({}); refusing to apply",
            plan.blockers.len(),
            codes.join(", ")
        );
    }
    let states: Vec<StepState> = plan.steps.iter().map(|s| classify(root, s)).collect();
    if let Some(StepState::Conflict(msg)) =
        states.iter().find(|s| matches!(s, StepState::Conflict(_)))
    {
        bail!(
            "stale plan: {msg}; nothing was modified. Re-run the operation to regenerate the plan"
        );
    }
    let done = states.iter().filter(|s| **s == StepState::Done).count();
    if done == plan.steps.len() {
        return Ok(ApplyResult {
            status: ApplyStatus::AlreadyApplied,
            applied_steps: 0,
            verify: None,
            restore_errors: vec![],
            plan: plan.clone(),
        });
    }
    if done > 0 {
        bail!(
            "plan is partially applied ({done} of {} steps already match); nothing was modified. Restore the tree or regenerate the plan",
            plan.steps.len()
        );
    }

    let mut journal = Journal::new(root, plan)?;
    let outcome = write_steps(&mut journal, plan, opts);
    if let Err(e) = outcome {
        let restore_errors = journal.restore();
        if restore_errors.is_empty() {
            return Err(e.context("apply failed; working tree restored"));
        }
        return Err(e.context(format!(
            "apply failed AND restore had errors: {}",
            restore_errors.join("; ")
        )));
    }
    // Temp files are gone after a successful rename; forget them so restore won't look.
    journal.tmp_files.clear();

    let mut verify = None;
    if opts.verify {
        let v = run_verify(
            root,
            &plan.affected_packages,
            opts.affected_only,
            opts.post_apply.as_deref(),
        );
        let ok = v.ok();
        verify = Some(v);
        if !ok {
            let restore_errors = journal.restore();
            return Ok(ApplyResult {
                status: ApplyStatus::RolledBack,
                applied_steps: 0,
                verify,
                restore_errors,
                plan: plan.clone(),
            });
        }
    }
    prune_empty_dirs(root, plan);
    Ok(ApplyResult {
        status: ApplyStatus::Applied,
        applied_steps: plan.steps.len(),
        verify,
        restore_errors: vec![],
        plan: plan.clone(),
    })
}

fn write_steps(journal: &mut Journal, plan: &MovePlan, opts: &ApplyOptions) -> Result<()> {
    for (i, s) in plan.steps.iter().enumerate() {
        let t = &s.transform;
        if let (Some(new), Some(content)) = (&t.new_path, &t.new_content) {
            journal
                .write_atomic(
                    new,
                    content.as_bytes(),
                    t.old_path.as_deref().or(Some(new.as_str())),
                )
                .map_err(|e| anyhow::anyhow!("writing {new}: {e}"))?;
        }
        if let Some(old) = &t.old_path
            && t.new_path.as_ref() != Some(old)
        {
            std::fs::remove_file(journal.root.join(old))
                .map_err(|e| anyhow::anyhow!("removing {old}: {e}"))?;
        }
        if opts.inject_failure_after_step == Some(i) {
            bail!("injected failure after step {i} ({:?})", s.kind);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refactor::types::{FileTransform, OperationKind, PlanStepKind};

    #[test]
    fn restore_recreates_missing_parent_directories() {
        let root = std::env::temp_dir().join(format!("kibitzer-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("x/y")).unwrap();
        std::fs::write(root.join("x/y/z.go"), "package z\n").unwrap();
        let plan = MovePlan {
            operation: OperationKind::MoveFiles,
            repo_root: ".".into(),
            module: "m".into(),
            from: vec![],
            to: vec![],
            steps: vec![PlanStep {
                kind: PlanStepKind::MoveFile,
                transform: FileTransform {
                    old_path: Some("x/y/z.go".into()),
                    old_sha256: None,
                    new_path: Some("n/z.go".into()),
                    new_content: Some("package z\n".into()),
                },
                reason: String::new(),
            }],
            blockers: vec![],
            warnings: vec![],
            surface_changes: vec![],
            path_map: vec![],
            affected_packages: vec![],
            simulation_result: None,
        };
        let mut journal = Journal::new(&root, &plan).unwrap();
        // The whole source directory vanishes mid-apply, as after a pruned move.
        std::fs::remove_dir_all(root.join("x")).unwrap();
        assert!(journal.restore().is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join("x/y/z.go")).unwrap(),
            "package z\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
