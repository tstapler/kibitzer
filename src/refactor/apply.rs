//! Atomic apply of a `RefactorPlan`: compute every new body in memory, run the shared
//! transaction (`txn`), verify with the Go toolchain, then commit or roll back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::{Edit, RefactorPlan};
use crate::go_syntax;
use crate::refactor_core::exec::{self, EnvPolicy};
use crate::refactor_core::guard::{is_go_ident, is_inside, is_protected, is_safe_import_path};
use crate::refactor_core::sha256_hex;
use crate::refactor_core::txn::{self, Change, Op};

/// Only these run after an apply; a plan arrives over MCP, so its command list is not trusted.
const ALLOWED_VERIFY: [&str; 2] = ["go build ./...", "go vet ./..."];
const VERIFY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Side effects the apply path performs that tests need to fail on demand.
pub trait ApplyIo {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    /// Runs `cmd` in `dir`; returns `(success, combined output)`.
    fn run(&self, cmd: &str, dir: &Path) -> (bool, String);
}

pub struct RealIo;

impl ApplyIo for RealIo {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        txn::real_rename(from, to)
    }

    fn run(&self, cmd: &str, dir: &Path) -> (bool, String) {
        let mut parts = cmd.split_whitespace();
        let Some(program) = parts.next() else {
            return (false, "empty command".into());
        };
        let args: Vec<String> = parts.map(str::to_string).collect();
        let out = exec::run(&exec::Spec {
            program,
            args: &args,
            dir,
            timeout: VERIFY_TIMEOUT,
            env: EnvPolicy::Locked,
        });
        (out.ok(), out.output)
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

/// Rolls back an apply a crashed process left half-done. Returns the interrupted plan id, or
/// `None` when no log exists. Safe to call at every startup and before every apply.
pub fn recover(root: &Path) -> Result<Option<String>> {
    txn::recover(root)
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
            Edit::ReplaceImportPath { old_path, .. } if !is_safe_import_path(old_path) => {
                bail!("`{old_path}` is not a valid import path");
            }
            Edit::ReplaceImportPath { alias: Some(a), .. }
                if !is_go_ident(a) || a == "_" || a == "init" =>
            {
                bail!("`{a}` is not a valid import alias");
            }
            _ => {}
        }
    }
    paths.extend(plan.touched.iter().map(|t| t.path.as_path()));
    if let Some(p) = paths
        .iter()
        .find(|p| p.extension().is_none_or(|e| e != "go") || is_protected(root, p))
    {
        bail!("{} is not an editable .go file", p.display());
    }
    if let Some(p) = paths.iter().find(|p| !is_inside(root, p)) {
        bail!("{} is outside the repository", p.display());
    }
    Ok(())
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
                // The plan carries the final name; the shared rewriter re-appends an existing `_test`.
                let base = new_name.strip_suffix("_test").unwrap_or(new_name);
                *body = go_syntax::rewrite_package_clause(body, base)
                    .with_context(|| format!("no package clause in {}", file.display()))?;
                // Written value must equal the declared one (a hand-edited plan can disagree
                // with the `_test` suffix the file already has).
                if go_syntax::package_name(body).as_deref() != Some(new_name.as_str()) {
                    bail!(
                        "package clause of {} would not become `{new_name}`; refusing to apply",
                        file.display()
                    );
                }
            }
            Edit::ReplaceImportPath {
                file,
                old_path,
                new_path,
                alias,
            } => {
                let body = bodies.get_mut(file).context("unrecorded file")?;
                *body = go_syntax::rewrite_import_path_aliased(
                    body,
                    old_path,
                    new_path,
                    alias.as_deref(),
                )?
                .with_context(|| format!("import {old_path} not found in {}", file.display()))?;
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
    // Lock and crash-recover first, so the stale check and the new bodies below are computed
    // against a tree no other apply can change before our writes land.
    let session = txn::session(root)?;
    check_plan(root, plan)?;
    check_not_stale(plan)?;
    let outputs = compute_outputs(plan)?;
    let mut changes = Vec::new();
    for o in &outputs {
        changes.push(Change {
            path: o.dest.clone(),
            op: Op::Put {
                bytes: o.body.clone().into_bytes(),
                mode_from: Some(o.original.clone()),
            },
        });
        if o.dest != o.original {
            changes.push(Change {
                path: o.original.clone(),
                op: Op::Delete,
            });
        }
    }
    let rename = |from: &Path, to: &Path| io.rename(from, to);
    let pending = session.begin(&plan.id, &changes, &rename)?;
    match verify(root, plan, io) {
        Ok(verification) => {
            pending.commit()?;
            Ok(ApplyResult {
                plan_id: plan.id.clone(),
                files_written: outputs.into_iter().map(|o| o.dest).collect(),
                verification,
            })
        }
        Err(cause) => Err(match pending.rollback() {
            Ok(()) => cause.context("apply rolled back; the tree is unchanged"),
            Err(rb) => anyhow::Error::new(txn::RollbackFailed {
                cause: format!("{cause:#}"),
                rollback: format!("{rb:#}"),
            }),
        }),
    }
}

/// A failing `go vet ./...` can print megabytes; an MCP client needs the head of it.
const MAX_ERROR_OUTPUT: usize = 64 * 1024;

pub(super) fn cap_output(output: &str) -> String {
    if output.len() <= MAX_ERROR_OUTPUT {
        return output.to_string();
    }
    let mut end = MAX_ERROR_OUTPUT;
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[output truncated at 64 KiB]", &output[..end])
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
            bail!("`{cmd}` failed:\n{}", cap_output(&output));
        }
        steps.push(VerificationStep {
            cmd: cmd.clone(),
            output,
        });
    }
    Ok(steps)
}
