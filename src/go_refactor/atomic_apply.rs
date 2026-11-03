//! Atomic, idempotent plan application. Classifies each step (pending / already done / stale),
//! hands the writes to the shared write-ahead-logged transaction (`refactor_core::txn`), verifies
//! with the Go toolchain, and commits or restores the tree byte-for-byte.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use super::types::{ApplyResult, ApplyStatus, MovePlan, OperationKind, PlanStep, PlanStepKind};
use super::verify::{DEFAULT_TIMEOUT, run_verify};
use crate::refactor_core::sha256_hex;
use crate::refactor_core::txn::{self, Change, Op, Rename};

#[derive(Default)]
pub struct ApplyOptions {
    /// Run `go build/vet/test` after writing (always on from the CLI).
    pub verify: bool,
    pub affected_only: bool,
    pub post_apply: Option<String>,
    /// Per-command budget for verification; `None` means 30 minutes.
    pub verify_timeout: Option<Duration>,
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
    // `read` maps every error to "absent"; an unreadable file must not look like a missing one.
    for p in [&t.old_path, &t.new_path].into_iter().flatten() {
        if let Err(e) = std::fs::read(root.join(p))
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return StepState::Conflict(format!("{p} is not readable: {e}"));
        }
    }
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
    if plan.plan_id != plan.compute_id() {
        bail!(
            "plan_id does not match the plan's contents (edited or corrupted); regenerate the plan"
        );
    }
    let policy = plan.operation == OperationKind::SyncPolicy;
    for s in &plan.steps {
        if s.transform.old_path.is_some() && s.transform.old_sha256.is_none() {
            bail!("plan step is missing old_sha256 for an existing file; refusing to apply");
        }
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
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                    Err(e) => bail!("plan path `{p}` cannot be inspected: {e}; refusing to apply"),
                }
            }
        }
    }
    for pkg in &plan.affected_packages {
        let plain = pkg.starts_with("./")
            && !pkg.ends_with(".go")
            && !pkg.split('/').any(|c| c == "..")
            && pkg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "./_-".contains(c));
        if pkg != "." && !plain {
            bail!("plan package `{pkg}` is not a relative package path; refusing to apply");
        }
    }
    Ok(())
}

fn changes_for(root: &Path, plan: &MovePlan) -> Vec<Change> {
    let mut changes = Vec::new();
    for s in &plan.steps {
        let t = &s.transform;
        if let (Some(new), Some(content)) = (&t.new_path, &t.new_content) {
            changes.push(Change {
                path: root.join(new),
                op: Op::Put {
                    bytes: content.clone().into_bytes(),
                    mode_from: Some(root.join(t.old_path.as_deref().unwrap_or(new))),
                },
            });
        }
        if let Some(old) = &t.old_path
            && t.new_path.as_ref() != Some(old)
        {
            changes.push(Change {
                path: root.join(old),
                op: Op::Delete,
            });
        }
    }
    changes
}

/// A sync-policy plan must be exactly what the tool derives from the tree and its own path map,
/// so a foreign or hand-edited plan cannot plant content (a check `command`, a loosened guard)
/// in a policy file under a valid-looking id.
fn check_policy_derivation(plan: &MovePlan, root: &Path) -> Result<()> {
    let expected = super::policy::sync_policy(root, plan)?;
    let shape = |p: &MovePlan| {
        p.steps
            .iter()
            .map(|s| {
                (
                    s.transform.new_path.clone(),
                    s.transform.new_content.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    if shape(&expected) != shape(plan) {
        bail!(
            "sync-policy plan differs from what the tool derives from its path map; refusing to apply"
        );
    }
    Ok(())
}

pub fn apply(plan: &MovePlan, root: &Path, opts: &ApplyOptions) -> Result<ApplyResult> {
    apply_with(plan, root, opts, &txn::real_rename)
}

/// [`apply`] with an injectable `rename`, so tests can fail a chosen write.
pub(super) fn apply_with(
    plan: &MovePlan,
    root: &Path,
    opts: &ApplyOptions,
    rename: Rename,
) -> Result<ApplyResult> {
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

    if plan.operation == OperationKind::SyncPolicy {
        check_policy_derivation(plan, root)?;
    }
    let pending = txn::begin(root, &plan.plan_id, &changes_for(root, plan), rename)?;
    let mut verify = None;
    if opts.verify {
        let v = run_verify(
            root,
            &plan.affected_packages,
            opts.affected_only,
            opts.post_apply.as_deref(),
            opts.verify_timeout.unwrap_or(DEFAULT_TIMEOUT),
        );
        let ok = v.ok();
        verify = Some(v);
        if !ok {
            let restore_errors = pending
                .rollback()
                .err()
                .map(|e| vec![format!("{e:#}")])
                .unwrap_or_default();
            return Ok(ApplyResult {
                status: ApplyStatus::RolledBack,
                applied_steps: 0,
                verify,
                restore_errors,
                plan: plan.clone(),
            });
        }
    }
    pending.commit()?;
    prune_empty_dirs(root, plan);
    Ok(ApplyResult {
        status: ApplyStatus::Applied,
        applied_steps: plan.steps.len(),
        verify,
        restore_errors: vec![],
        plan: plan.clone(),
    })
}
