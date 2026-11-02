//! `kibitzer refactor go …`: atomic, verified, behavior-preserving Go package/file moves.
//! Design: docs/go-pkg-move-ops.md.

mod atomic_apply;
mod go_syntax;
mod ops;
mod plan;
mod policy;
mod types;
mod verify;
mod workspace;

#[cfg(test)]
mod golden_tests;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

pub use types::{ApplyStatus, MovePlan};

use atomic_apply::ApplyOptions;
use ops::{Cluster, Findings, Selection};
use types::OperationKind;
use workspace::Workspace;

#[derive(Subcommand)]
pub enum RefactorAction {
    /// Go refactoring operations.
    Go {
        #[command(subcommand)]
        action: GoRefactorAction,
    },
}

#[derive(Args)]
pub struct CommonArgs {
    /// Root of the Go module (must contain go.mod).
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    /// Write the changes (default: print the plan and change nothing).
    #[arg(long)]
    apply: bool,
    /// Write the plan JSON here instead of stdout.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Feasibility-simulation JSON to embed; a reported blocker forbids apply.
    #[arg(long)]
    simulation: Option<PathBuf>,
    /// Shell command run after writing and before verification (e.g. a code generator).
    #[arg(long)]
    post_apply: Option<String>,
    /// Verify only the plan's affected packages instead of `./...`.
    #[arg(long)]
    affected_only: bool,
}

#[derive(Subcommand)]
pub enum GoRefactorAction {
    /// Move files of one package into another (new or existing) package.
    MoveFiles {
        /// Files to move, relative to the repo root.
        #[arg(long = "src", required = true, num_args = 1..)]
        src: Vec<String>,
        /// Destination import path.
        #[arg(long)]
        dest_pkg: String,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Rename a package / change its import path, rewriting every importer.
    RenamePkg {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        /// New package name (default: derived from the new path when the old name followed its directory).
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Extract declarations (plus the private helpers only they use) into a new package.
    ExtractDecls {
        #[arg(long)]
        from: String,
        #[arg(long, value_delimiter = ',', required = true)]
        decls: Vec<String>,
        #[arg(long)]
        to: String,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Split a package by file clusters: `{"clusters":[{"to":"import/path","files":["a.go"]}]}`.
    SplitPkg {
        #[arg(long)]
        pkg: String,
        #[arg(long, visible_alias = "focus-json")]
        partition: PathBuf,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Update repo policy (inspect.json scopes, guard configs) for a previously computed plan.
    SyncPolicy {
        /// Plan JSON from another subcommand.
        #[arg(long)]
        plan: PathBuf,
        #[command(flatten)]
        common: CommonArgs,
    },
    /// Apply a saved plan (idempotent: a second apply is a no-op or a clear error).
    Apply {
        #[arg(long)]
        plan: PathBuf,
        #[command(flatten)]
        common: CommonArgs,
    },
}

#[derive(serde::Deserialize)]
struct Partition {
    clusters: Vec<PartitionCluster>,
}

#[derive(serde::Deserialize)]
struct PartitionCluster {
    to: String,
    files: Vec<String>,
}

/// Plan a move-files operation (also the entry point golden tests use).
pub fn plan_move_files(
    root: &Path,
    srcs: &[String],
    dest: &str,
    sim: Option<serde_json::Value>,
) -> Result<MovePlan> {
    let mut ws = Workspace::load(root)?;
    let mut f = Findings::default();
    let mut by_dir: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for s in srcs {
        by_dir
            .entry(workspace::dir_of(s).to_string())
            .or_default()
            .push(workspace::base_of(s).to_string());
    }
    if by_dir.len() > 1 {
        f.blockers.push(types::BlockingDiagnostic::new(
            "multiple_source_packages",
            "move-files takes files from exactly one package; run it once per package",
        ));
    }
    for (dir, files) in by_dir {
        ops::partial_move(&mut ws, &mut f, &dir, dest, &Selection::Files(files));
    }
    Ok(plan::build_plan(
        &ws,
        OperationKind::MoveFiles,
        &root.to_string_lossy(),
        f,
        sim,
    ))
}

pub fn plan_rename_pkg(
    root: &Path,
    from: &str,
    to: &str,
    name: Option<&str>,
    sim: Option<serde_json::Value>,
) -> Result<MovePlan> {
    let mut ws = Workspace::load(root)?;
    let mut f = Findings::default();
    ops::rename_pkg(&mut ws, &mut f, from, to, name);
    Ok(plan::build_plan(
        &ws,
        OperationKind::RenamePkg,
        &root.to_string_lossy(),
        f,
        sim,
    ))
}

pub fn plan_extract_decls(
    root: &Path,
    from: &str,
    decls: &[String],
    to: &str,
    sim: Option<serde_json::Value>,
) -> Result<MovePlan> {
    let mut ws = Workspace::load(root)?;
    let mut f = Findings::default();
    match ws.dir_of_import(from) {
        Some(dir) => {
            ops::partial_move(&mut ws, &mut f, &dir, to, &Selection::Decls(decls.to_vec()))
        }
        None => f.blockers.push(types::BlockingDiagnostic::new(
            "outside_module",
            format!("{from} is outside module {}", ws.module),
        )),
    }
    Ok(plan::build_plan(
        &ws,
        OperationKind::ExtractDecls,
        &root.to_string_lossy(),
        f,
        sim,
    ))
}

pub fn plan_split_pkg(
    root: &Path,
    pkg: &str,
    clusters: &[(String, Vec<String>)],
    sim: Option<serde_json::Value>,
) -> Result<MovePlan> {
    let mut ws = Workspace::load(root)?;
    let mut f = Findings::default();
    let cs: Vec<Cluster> = clusters
        .iter()
        .map(|(to, files)| Cluster {
            to: to.clone(),
            files: files.clone(),
        })
        .collect();
    ops::split_pkg(&mut ws, &mut f, pkg, &cs);
    Ok(plan::build_plan(
        &ws,
        OperationKind::SplitPkg,
        &root.to_string_lossy(),
        f,
        sim,
    ))
}

pub fn plan_sync_policy(root: &Path, source: &MovePlan) -> Result<MovePlan> {
    policy::sync_policy(root, source)
}

pub fn apply_plan(
    root: &Path,
    plan: &MovePlan,
    post_apply: Option<String>,
    affected_only: bool,
) -> Result<types::ApplyResult> {
    let policy_only = plan.operation == OperationKind::SyncPolicy;
    atomic_apply::apply(
        plan,
        root,
        &ApplyOptions {
            verify: !policy_only,
            affected_only,
            post_apply,
            inject_failure_after_step: None,
        },
    )
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn emit(value: &impl serde::Serialize, out: &Option<PathBuf>) -> Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    match out {
        Some(p) => std::fs::write(p, format!("{json}\n"))
            .with_context(|| format!("writing {}", p.display()))?,
        None => println!("{json}"),
    }
    Ok(())
}

pub fn run_refactor(action: RefactorAction) -> Result<ExitCode> {
    let RefactorAction::Go { action } = action;
    let sim_of = |c: &CommonArgs| -> Result<Option<serde_json::Value>> {
        c.simulation.as_deref().map(read_json).transpose()
    };
    let (plan, common) = match action {
        GoRefactorAction::MoveFiles {
            src,
            dest_pkg,
            common,
        } => (
            plan_move_files(&common.repo, &src, &dest_pkg, sim_of(&common)?)?,
            common,
        ),
        GoRefactorAction::RenamePkg {
            from,
            to,
            name,
            common,
        } => (
            plan_rename_pkg(&common.repo, &from, &to, name.as_deref(), sim_of(&common)?)?,
            common,
        ),
        GoRefactorAction::ExtractDecls {
            from,
            decls,
            to,
            common,
        } => (
            plan_extract_decls(&common.repo, &from, &decls, &to, sim_of(&common)?)?,
            common,
        ),
        GoRefactorAction::SplitPkg {
            pkg,
            partition,
            common,
        } => {
            let p: Partition = read_json(&partition)?;
            let clusters: Vec<_> = p.clusters.into_iter().map(|c| (c.to, c.files)).collect();
            (
                plan_split_pkg(&common.repo, &pkg, &clusters, sim_of(&common)?)?,
                common,
            )
        }
        GoRefactorAction::SyncPolicy { plan, common } => {
            let source: MovePlan = read_json(&plan)?;
            (plan_sync_policy(&common.repo, &source)?, common)
        }
        GoRefactorAction::Apply { plan, common } => (read_json(&plan)?, common),
    };
    if !common.apply {
        emit(&plan, &common.out)?;
        return Ok(if plan.blockers.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(2)
        });
    }
    let result = apply_plan(
        &common.repo,
        &plan,
        common.post_apply.clone(),
        common.affected_only,
    )?;
    emit(&result, &common.out)?;
    Ok(if result.status == ApplyStatus::RolledBack {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    })
}
