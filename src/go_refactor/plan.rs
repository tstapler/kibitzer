//! Turns a mutated [`Workspace`] plus operation findings into a reviewable [`MovePlan`].

use std::collections::BTreeSet;
use std::io::Write;
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use super::ops::Findings;
use super::types::{
    BlockingDiagnostic, FileTransform, MovePlan, OperationKind, PlanStep, PlanStepKind,
};
use super::workspace::{Workspace, dir_of};

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// `gofmt` over stdin. `Ok(None)` when gofmt is unavailable.
fn gofmt(src: &str) -> Result<Option<String>, String> {
    let child = Command::new("gofmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else {
        return Ok(None);
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(src.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(Some(String::from_utf8_lossy(&out.stdout).into_owned()))
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Simulation output (#153) reporting infeasibility becomes a blocker.
fn simulation_blockers(sim: &serde_json::Value) -> Vec<BlockingDiagnostic> {
    let mut out = vec![];
    let listed = sim
        .get("blockers")
        .and_then(|v| v.as_array())
        .is_some_and(|b| !b.is_empty());
    if sim.get("feasible").and_then(|v| v.as_bool()) == Some(false) && !listed {
        out.push(BlockingDiagnostic::new(
            "simulation_blocker",
            "feasibility simulation reports the operation is infeasible",
        ));
    }
    if let Some(bs) = sim.get("blockers").and_then(|v| v.as_array()) {
        for b in bs {
            let msg = b
                .get("message")
                .and_then(|m| m.as_str())
                .or_else(|| b.as_str())
                .unwrap_or("simulation blocker");
            out.push(BlockingDiagnostic::new("simulation_blocker", msg));
        }
    }
    out
}

pub fn build_plan(
    ws: &Workspace,
    op: OperationKind,
    repo_root: &str,
    mut f: Findings,
    simulation: Option<serde_json::Value>,
) -> MovePlan {
    if let Some(sim) = &simulation {
        for b in simulation_blockers(sim) {
            f.blockers.push(b);
        }
    }
    let mut steps = vec![];
    let mut gofmt_missing = false;
    let moved_origins: BTreeSet<&String> = ws.moves().values().collect();
    for path in ws.changed_paths() {
        let new = ws.get(&path).map(str::to_string);
        let orig = ws.original(&path).map(str::to_string);
        let Some(mut content) = new else {
            if orig.is_some() && !moved_origins.contains(&path) {
                steps.push(PlanStep {
                    kind: PlanStepKind::DeleteFile,
                    transform: FileTransform {
                        old_path: Some(path.clone()),
                        old_sha256: orig.as_deref().map(|o| sha256_hex(o.as_bytes())),
                        new_path: None,
                        new_content: None,
                    },
                    reason: "file removed".into(),
                });
            }
            continue;
        };
        let origin = ws.moves().get(&path).cloned();
        let basis = origin
            .as_ref()
            .and_then(|o| ws.original(o))
            .or(orig.as_deref());
        if super::go_syntax::parse(&content).is_none_or(|t| t.root_node().has_error()) {
            f.blockers.push(
                BlockingDiagnostic::new("syntax_error", "file does not parse as Go (before or after the rewrite); refusing to plan around it")
                    .at(&path, None),
            );
        }
        let basis_clean = basis.is_none_or(|b| matches!(gofmt(b), Ok(Some(ref x)) if x == b));
        if basis_clean {
            match gofmt(&content) {
                Ok(Some(formatted)) => content = formatted,
                Ok(None) => gofmt_missing = true,
                Err(e) => f.blockers.push(
                    BlockingDiagnostic::new(
                        "format_failed",
                        format!("rewritten file does not parse: {}", e.trim()),
                    )
                    .at(&path, None),
                ),
            }
        }
        let (kind, reason, old_path) = match (&origin, &orig) {
            (Some(o), _) => (
                PlanStepKind::MoveFile,
                format!("moved from {o}"),
                Some(o.clone()),
            ),
            (None, Some(_)) => {
                if basis == Some(content.as_str()) {
                    continue;
                }
                (
                    PlanStepKind::RewriteFile,
                    "import/qualifier/declaration rewrite".to_string(),
                    Some(path.clone()),
                )
            }
            (None, None) => (
                PlanStepKind::CreateFile,
                "new file holding extracted declarations".to_string(),
                None,
            ),
        };
        steps.push(PlanStep {
            kind,
            transform: FileTransform {
                old_sha256: basis.map(|b| sha256_hex(b.as_bytes())),
                old_path,
                new_path: Some(path),
                new_content: Some(content),
            },
            reason,
        });
    }
    if gofmt_missing {
        f.warnings
            .push("gofmt not found on PATH; rewritten files were not formatted".into());
    }
    steps.sort_by_key(key);

    let mut dirs: BTreeSet<String> = f.affected_dirs.clone();
    for s in &steps {
        for p in [&s.transform.old_path, &s.transform.new_path]
            .into_iter()
            .flatten()
        {
            dirs.insert(dir_of(p).to_string());
        }
    }
    let affected_packages = dirs
        .into_iter()
        .filter(|d| !ws.files_in_dir(d).is_empty())
        .map(|d| {
            if d.is_empty() {
                ".".to_string()
            } else {
                format!("./{d}")
            }
        })
        .collect();
    f.blockers.dedup();
    MovePlan {
        operation: op,
        repo_root: repo_root.to_string(),
        module: ws.module.clone(),
        from: f.from,
        to: f.to,
        steps,
        blockers: f.blockers,
        warnings: f.warnings,
        surface_changes: f.surface_changes,
        path_map: f.path_map,
        affected_packages,
        simulation_result: simulation,
    }
}

fn key(s: &PlanStep) -> String {
    s.transform
        .new_path
        .clone()
        .or_else(|| s.transform.old_path.clone())
        .unwrap_or_default()
}
