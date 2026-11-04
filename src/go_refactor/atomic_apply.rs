//! Atomic, idempotent plan application. Classifies each step (pending / already done / stale),
//! hands the writes to the shared write-ahead-logged transaction (`refactor_core::txn`), verifies
//! with the Go toolchain, and commits or restores the tree byte-for-byte.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use super::types::{ApplyResult, ApplyStatus, MovePlan, OperationKind, PlanStep, PlanStepKind};
use super::verify::{DEFAULT_TIMEOUT, run_verify};
use crate::refactor_core::txn::{self, Change, Op, Rename};
use crate::refactor_core::{guard, sha256_hex};

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

/// One read per file: `Ok(None)` is a missing file, `Err` an unreadable one (which must not
/// look like a missing one).
fn read_once(root: &Path, rel: &str) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(root.join(rel)) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn classify(root: &Path, s: &PlanStep) -> StepState {
    let t = &s.transform;
    let load = |p: &Option<String>| match p {
        None => Ok(None),
        Some(p) => read_once(root, p).map_err(|e| format!("{p} is not readable: {e}")),
    };
    let (old, new) = match (load(&t.old_path), load(&t.new_path)) {
        (Ok(o), Ok(n)) => (o, n),
        (Err(m), _) | (_, Err(m)) => return StepState::Conflict(m),
    };
    let want = t.new_content.as_deref().map(str::as_bytes);
    let old_matches = match (&old, &t.old_sha256) {
        (Some(b), Some(h)) => &sha256_hex(b) == h,
        (Some(_), None) => true,
        (None, _) => false,
    };
    match (&t.old_path, &t.new_path) {
        (Some(o), Some(n)) if o == n => {
            if old.as_deref() == want {
                StepState::Done
            } else if old_matches {
                StepState::Pending
            } else {
                StepState::Conflict(format!("{o} changed since the plan was made"))
            }
        }
        (Some(o), Some(n)) => match (old.is_some(), new) {
            (false, Some(b)) if Some(b.as_slice()) == want => StepState::Done,
            (true, None) if old_matches => StepState::Pending,
            (true, None) => StepState::Conflict(format!("{o} changed since the plan was made")),
            _ => StepState::Conflict(format!(
                "{o} -> {n}: neither the plan's before nor after state"
            )),
        },
        (None, Some(n)) => match new {
            None => StepState::Pending,
            Some(b) if Some(b.as_slice()) == want => StepState::Done,
            Some(_) => StepState::Conflict(format!("{n} already exists with different content")),
        },
        (Some(o), None) => {
            if old.is_none() {
                StepState::Done
            } else if old_matches {
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
            guard::check_plan_rel_path(p)?;
            let allowed = if policy {
                POLICY_FILES.contains(&p.as_str())
            } else {
                p.ends_with(".go") && !p.split('/').any(|c| c.starts_with('.'))
            };
            if !allowed {
                bail!("plan path `{p}` is not a file this operation may touch; refusing to apply");
            }
            guard::check_no_symlink_crossing(root, p)?;
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
    // Lock and crash-recover before reading anything: staleness and idempotence below must be
    // judged against a tree no other apply can change before our writes land.
    let session = txn::session(root)?;
    if let Some(already) = preflight(plan, root)? {
        return Ok(already);
    }
    let pending = match session.begin(&plan.plan_id, &changes_for(root, plan), rename) {
        Ok(p) => p,
        // The write failed and so did the rollback: report a dirty tree (exit 4), not a plain error.
        Err(e) if e.downcast_ref::<txn::RollbackFailed>().is_some() => {
            return Ok(result(
                plan,
                ApplyStatus::RolledBack,
                0,
                None,
                vec![format!("{e:#}")],
            ));
        }
        Err(e) => return Err(e),
    };
    verify_and_settle(plan, root, opts, pending)
}

fn result(
    plan: &MovePlan,
    status: ApplyStatus,
    applied_steps: usize,
    verify: Option<super::types::VerifyResult>,
    restore_errors: Vec<String>,
) -> ApplyResult {
    ApplyResult {
        status,
        applied_steps,
        verify,
        restore_errors,
        plan: plan.clone(),
    }
}

/// Validates the plan and judges every step against the tree. `Some` means there is nothing to
/// do (already applied); an `Err` means the plan is invalid, stale or partially applied.
fn preflight(plan: &MovePlan, root: &Path) -> Result<Option<ApplyResult>> {
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
        return Ok(Some(result(
            plan,
            ApplyStatus::AlreadyApplied,
            0,
            None,
            vec![],
        )));
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
    Ok(None)
}

/// Verifies the written tree (when asked) and either commits it or restores it byte-for-byte.
fn verify_and_settle(
    plan: &MovePlan,
    root: &Path,
    opts: &ApplyOptions,
    pending: txn::Pending,
) -> Result<ApplyResult> {
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
            return Ok(result(
                plan,
                ApplyStatus::RolledBack,
                0,
                verify,
                restore_errors,
            ));
        }
    }
    pending.commit()?;
    prune_empty_dirs(root, plan);
    Ok(result(
        plan,
        ApplyStatus::Applied,
        plan.steps.len(),
        verify,
        vec![],
    ))
}
