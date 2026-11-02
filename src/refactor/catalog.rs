//! Go operation catalog: what mechanical operations a package supports, and the plan for a
//! MoveFiles/ExtractPackage operation. Planning reads files; it never writes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use super::feasibility::{self, Resolved};
use super::{
    AvailableOperation, Edit, OperationKind, RefactorOperation, RefactorPlan, TouchedFile,
};
use crate::arch_model::ArchModel;
use crate::go_syntax::{self, ImportName};

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with("_test.go"))
}

/// Operations available for `package`, or `[]` when none is supported (non-Go, unknown,
/// single-file, or cgo packages). Never an error.
pub fn list_operations_for_package(model: &ArchModel, package: &str) -> Vec<AvailableOperation> {
    let Some(pkg) = model.packages.get(package) else {
        return Vec::new();
    };
    let sources: Vec<PathBuf> = pkg
        .files
        .iter()
        .map(|f| feasibility::absolutize(model, f))
        .filter(|f| f.extension().is_some_and(|e| e == "go") && !is_test_file(f))
        .collect();
    let has_cgo = sources
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .any(|src| go_syntax::imports_cgo(&src));
    if sources.len() < 2 || has_cgo {
        return Vec::new();
    }
    let source_package = package.to_string();
    vec![
        AvailableOperation {
            kind: OperationKind::MoveFiles,
            source_package: source_package.clone(),
            description: format!("Move selected files out of {package} into another package"),
        },
        AvailableOperation {
            kind: OperationKind::ExtractPackage,
            source_package,
            description: format!("Extract selected files of {package} into a new package"),
        },
    ]
}

fn module_name(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("go.mod")).ok()?;
    text.lines().find_map(|l| {
        l.trim()
            .strip_prefix("module ")
            .map(|m| m.trim().to_string())
    })
}

/// `(module path, module dir)` of the nearest `go.mod` above `start`, within the repo.
fn go_module(model: &ArchModel, start: &Path) -> Option<(String, PathBuf)> {
    let mut dir = start;
    loop {
        if let Some(module) = module_name(dir) {
            return Some((module, dir.to_path_buf()));
        }
        if dir == model.repo_root {
            return None;
        }
        dir = dir.parent().filter(|p| p.starts_with(&model.repo_root))?;
    }
}

fn target_dir(model: &ArchModel, op: &RefactorOperation, r: &Resolved) -> Result<PathBuf> {
    let first = r.moved.iter().next().context("no files to move")?;
    let (module, root) = go_module(model, first.parent().unwrap_or(first))
        .context("no go.mod found above the source package")?;
    let rest = op
        .target_package
        .strip_prefix(&module)
        .filter(|r| r.is_empty() || r.starts_with('/'))
        .with_context(|| format!("target {} is outside module {module}", op.target_package))?;
    let rel = rest.trim_start_matches('/');
    if rel.split('/').any(|seg| seg == ".." || seg == ".") || rest.contains("//") {
        bail!("target {} has an invalid path segment", op.target_package);
    }
    Ok(root.join(rel))
}

fn target_package_name(op: &RefactorOperation, dir: &Path) -> String {
    let existing = std::fs::read_dir(dir).ok().and_then(|entries| {
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "go") && !is_test_file(p))
            .collect();
        files.sort();
        files
            .into_iter()
            .find_map(|p| go_syntax::package_name(&std::fs::read_to_string(p).ok()?))
    });
    existing.unwrap_or_else(|| {
        let last = op.target_package.rsplit('/').next().unwrap_or("pkg");
        last.replace('-', "_")
    })
}

fn read_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Plans a MoveFiles/ExtractPackage operation. Errors when the operation is infeasible or
/// would need qualifier rewriting this version doesn't do (a partial move whose moved and
/// remaining symbols call each other).
pub fn plan_move_files(model: &ArchModel, op: &RefactorOperation) -> Result<RefactorPlan> {
    let report = feasibility::check(model, op);
    if !report.feasible {
        bail!(
            "operation is not feasible: {}",
            serde_json::to_string(&report.blocking)?
        );
    }
    let resolved = feasibility::resolve(model, op).map_err(|b| anyhow::anyhow!("{b:?}"))?;
    reject_entangled(model, op, &resolved)?;

    let dir = target_dir(model, op, &resolved)?;
    let new_name = target_package_name(op, &dir);
    let (mut edits, moves, old_name) = move_edits(&resolved, &dir, &new_name)?;
    edits.extend(import_edits(model, op, &resolved, &old_name, &new_name)?);
    edits.extend(moves);

    let mut touched = Vec::new();
    for path in &report.touched_files {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        touched.push(TouchedFile {
            path: path.clone(),
            sha256: sha256_hex(&bytes),
        });
    }
    let mut plan = RefactorPlan {
        id: String::new(),
        operation: op.clone(),
        edits,
        verification_cmds: vec!["go build ./...".into(), "go vet ./...".into()],
        touched,
        warnings: report.warnings,
    };
    plan.id = sha256_hex(serde_json::to_string(&plan)?.as_bytes());
    Ok(plan)
}

/// Package-clause rewrites and file moves for the moved files, plus the source package name.
fn move_edits(r: &Resolved, dir: &Path, new_name: &str) -> Result<(Vec<Edit>, Vec<Edit>, String)> {
    let mut decls = Vec::new();
    let mut moves = Vec::new();
    let mut old_name = String::new();
    for file in &r.moved {
        let old =
            go_syntax::package_name(&read_file(file)?).context("file has no package clause")?;
        let external_test = old.ends_with("_test");
        let want = if external_test {
            format!("{new_name}_test")
        } else {
            new_name.to_string()
        };
        if !external_test {
            old_name.clone_from(&old);
        }
        if old != want {
            decls.push(Edit::ReplacePackageDecl {
                file: file.clone(),
                old_name: old,
                new_name: want,
            });
        }
        let name = file.file_name().context("file has no name")?;
        moves.push(Edit::MoveFile {
            from: file.clone(),
            to: dir.join(name),
        });
    }
    Ok((decls, moves, old_name))
}

/// Import-path swaps in every importer; an alias keeps `pkg.Sym` selectors valid when the
/// package name changes.
fn import_edits(
    model: &ArchModel,
    op: &RefactorOperation,
    r: &Resolved,
    old_name: &str,
    new_name: &str,
) -> Result<Vec<Edit>> {
    let mut edits = Vec::new();
    for file in feasibility::importer_files(model, op, r).into_keys() {
        let src = read_file(&file)?;
        let Some(site) = go_syntax::imports(&src)
            .into_iter()
            .find(|i| i.path == op.source_package)
        else {
            continue;
        };
        let has_alias = site.name != ImportName::Implicit;
        let alias = (old_name != new_name && !has_alias && !old_name.is_empty())
            .then(|| old_name.to_string());
        edits.push(Edit::ReplaceImportPath {
            file,
            old_path: op.source_package.clone(),
            new_path: op.target_package.clone(),
            alias,
        });
    }
    Ok(edits)
}

/// A partial move where moved and remaining symbols call each other leaves unqualified
/// references on both sides of the new package boundary.
fn reject_entangled(model: &ArchModel, op: &RefactorOperation, r: &Resolved) -> Result<()> {
    if r.whole_package {
        return Ok(());
    }
    let sym_file = feasibility::symbol_files(model, op);
    let crossing = model.call_edges.iter().filter(|c| c.resolved).any(|c| {
        let (Some(callee), caller) = (sym_file.get(&c.to), sym_file.get(&c.from)) else {
            return false;
        };
        caller.is_some_and(|caller| r.moved.contains(caller) != r.moved.contains(callee))
    });
    if crossing {
        bail!(
            "moved and remaining symbols of {} call each other; qualifier rewriting is not supported yet",
            op.source_package
        );
    }
    Ok(())
}
