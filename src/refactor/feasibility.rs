//! Read-only feasibility check: builds the post-move package import graph in memory and
//! reports cycles, `internal/` violations, cgo, and warnings. Never writes to the tree.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::{BlockingReason, FeasibilityReport, RefactorOperation, Warning, go_syntax};
use crate::arch_model::{ArchModel, looks_generated};

/// An operation resolved against the model: absolute moved-file set plus what remains.
pub(crate) struct Resolved {
    pub moved: BTreeSet<PathBuf>,
    pub whole_package: bool,
}

pub(crate) fn absolutize(model: &ArchModel, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        model.repo_root.join(path)
    }
}

/// Resolves `op.files` against the source package. A file the model skipped (generated) is
/// still accepted if it is a `.go` file next to the package's modeled files.
pub(crate) fn resolve(
    model: &ArchModel,
    op: &RefactorOperation,
) -> Result<Resolved, Vec<BlockingReason>> {
    let Some(pkg) = model.packages.get(&op.source_package) else {
        return Err(vec![BlockingReason::UnknownSourcePackage {
            package: op.source_package.clone(),
        }]);
    };
    let modeled: BTreeSet<PathBuf> = pkg.files.iter().map(|f| absolutize(model, f)).collect();
    let dirs: BTreeSet<PathBuf> = modeled
        .iter()
        .filter_map(|f| f.parent().map(Path::to_path_buf))
        .collect();
    if op.files.is_empty() {
        return Ok(Resolved {
            moved: modeled,
            whole_package: true,
        });
    }
    let mut moved = BTreeSet::new();
    let mut bad = Vec::new();
    for f in &op.files {
        let abs = absolutize(model, f);
        let beside = abs.parent().is_some_and(|d| dirs.contains(d))
            && abs.extension().is_some_and(|e| e == "go")
            && abs.is_file();
        if modeled.contains(&abs) || beside {
            moved.insert(abs);
        } else {
            bad.push(BlockingReason::FileNotInPackage { file: f.clone() });
        }
    }
    if !bad.is_empty() {
        return Err(bad);
    }
    let whole_package = modeled.iter().all(|f| moved.contains(f));
    Ok(Resolved {
        moved,
        whole_package,
    })
}

/// Go's `internal` rule: `a/b/internal/c` is importable only from within `a/b`.
fn internal_allows(importer: &str, imported: &str) -> bool {
    let idx = if let Some(i) = imported.rfind("/internal/") {
        i
    } else if let Some(stripped) = imported.strip_suffix("/internal") {
        stripped.len()
    } else {
        return true;
    };
    let parent = &imported[..idx];
    importer == parent || importer.starts_with(&format!("{parent}/"))
}

type Adjacency = BTreeMap<String, BTreeSet<String>>;

/// The package graph after the move. Edges follow files: an import made from a moved file
/// now originates at the target. Resolved call edges decide the new cross-package edges a
/// partial move creates. Type-only uses are invisible to the call graph, so the verify step
/// (not this check) is the backstop for them.
fn post_move_graph(model: &ArchModel, op: &RefactorOperation, r: &Resolved) -> Adjacency {
    let mut adj: Adjacency = BTreeMap::new();
    let mut add = |from: &str, to: &str| {
        if from != to {
            adj.entry(from.to_string())
                .or_default()
                .insert(to.to_string());
        }
    };
    for e in &model.import_edges {
        let from = if r.moved.contains(&absolutize(model, &e.file)) {
            op.target_package.as_str()
        } else {
            e.from.as_str()
        };
        let to = if r.whole_package && e.to == op.source_package {
            op.target_package.as_str()
        } else {
            e.to.as_str()
        };
        add(from, to);
    }
    if r.whole_package {
        return adj;
    }
    let sym_file = symbol_files(model, op);
    for c in model.call_edges.iter().filter(|c| c.resolved) {
        let callee_moved = sym_file.get(&c.to).is_some_and(|f| r.moved.contains(f));
        let callee_remaining = sym_file.get(&c.to).is_some_and(|f| !r.moved.contains(f));
        let caller_moved = r.moved.contains(&absolutize(model, &c.file));
        let caller_pkg = package_of_symbol(&c.from);
        if callee_moved && !caller_moved {
            add(caller_pkg, &op.target_package);
        } else if callee_remaining && caller_moved {
            add(&op.target_package, &op.source_package);
        }
    }
    adj
}

fn package_of_symbol(id: &str) -> &str {
    id.split("::").next().unwrap_or(id)
}

/// Symbol id -> absolute file, for the source package's symbols.
pub(crate) fn symbol_files(model: &ArchModel, op: &RefactorOperation) -> BTreeMap<String, PathBuf> {
    model
        .packages
        .get(&op.source_package)
        .map(|p| {
            p.symbols
                .iter()
                .map(|s| (s.id.clone(), absolutize(model, &s.file)))
                .collect()
        })
        .unwrap_or_default()
}

/// A cycle through `start`, as `start -> ... -> start`, if one exists.
fn cycle_through(adj: &Adjacency, start: &str) -> Option<Vec<String>> {
    fn dfs<'a>(
        adj: &'a Adjacency,
        node: &'a str,
        start: &str,
        path: &mut Vec<&'a str>,
        seen: &mut BTreeSet<&'a str>,
    ) -> bool {
        for next in adj.get(node).into_iter().flatten() {
            if next == start {
                return true;
            }
            if !seen.insert(next.as_str()) {
                continue;
            }
            path.push(next);
            if dfs(adj, next, start, path, seen) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = vec![start];
    let mut seen = BTreeSet::new();
    if dfs(adj, start, start, &mut path, &mut seen) {
        let mut cycle: Vec<String> = path.into_iter().map(String::from).collect();
        cycle.push(start.to_string());
        Some(cycle)
    } else {
        None
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// cgo is blocking; build tags and generated files only warn.
fn scan_moved_files(r: &Resolved) -> (Vec<BlockingReason>, Vec<Warning>) {
    let mut blocking = Vec::new();
    let mut tagged = Vec::new();
    let mut generated = Vec::new();
    for f in &r.moved {
        let Some(src) = read(f) else { continue };
        if go_syntax::imports_c(&src) {
            blocking.push(BlockingReason::CgoPackage { file: f.clone() });
        }
        if go_syntax::has_build_constraint(&src) {
            tagged.push(f.clone());
        }
        if looks_generated(&src) {
            generated.push(f.clone());
        }
    }
    let mut warnings = Vec::new();
    if !tagged.is_empty() {
        warnings.push(Warning::BuildTaggedFiles { files: tagged });
    }
    if !generated.is_empty() {
        warnings.push(Warning::GeneratedFiles { files: generated });
    }
    (blocking, warnings)
}

fn internal_violations(adj: &Adjacency, target: &str) -> Vec<BlockingReason> {
    adj.iter()
        .flat_map(|(from, tos)| tos.iter().map(move |to| (from, to)))
        .filter(|(from, to)| (*from == target || *to == target) && !internal_allows(from, to))
        .map(|(from, to)| BlockingReason::InternalVisibilityViolation {
            importer: from.clone(),
            target: to.clone(),
        })
        .collect()
}

/// Reports whether `op` is feasible. Pure with respect to the tree: only reads files.
pub fn check(model: &ArchModel, op: &RefactorOperation) -> FeasibilityReport {
    let resolved = match resolve(model, op) {
        Ok(r) => r,
        Err(blocking) => {
            return FeasibilityReport {
                feasible: false,
                blocking,
                warnings: Vec::new(),
                touched_files: Vec::new(),
                importers_affected: Vec::new(),
            };
        }
    };
    let (mut blocking, warnings) = scan_moved_files(&resolved);
    let adj = post_move_graph(model, op, &resolved);
    if let Some(cycle_path) = cycle_through(&adj, &op.target_package) {
        blocking.push(BlockingReason::ImportCycle { cycle_path });
    }
    blocking.extend(internal_violations(&adj, &op.target_package));
    blocking.extend(
        mixed_importers(model, op, &resolved)
            .into_iter()
            .map(|file| BlockingReason::MixedImporter { file }),
    );

    let importers = importer_files(model, op, &resolved);
    let mut touched: BTreeSet<PathBuf> = resolved.moved.clone();
    touched.extend(importers.keys().cloned());
    let importers_affected: BTreeSet<String> = importers.values().cloned().collect();
    FeasibilityReport {
        feasible: blocking.is_empty(),
        blocking,
        warnings,
        touched_files: touched.into_iter().collect(),
        importers_affected: importers_affected.into_iter().collect(),
    }
}

/// Files outside the moved set that import the source package and must be rewritten, mapped
/// to their package. A whole-package move rewrites every importer; a partial move rewrites
/// only files whose resolved calls hit moved symbols.
pub(crate) fn importer_files(
    model: &ArchModel,
    op: &RefactorOperation,
    r: &Resolved,
) -> BTreeMap<PathBuf, String> {
    let sym_file = symbol_files(model, op);
    let uses_moved: BTreeSet<PathBuf> = model
        .call_edges
        .iter()
        .filter(|c| c.resolved && sym_file.get(&c.to).is_some_and(|f| r.moved.contains(f)))
        .map(|c| absolutize(model, &c.file))
        .collect();
    model
        .import_edges
        .iter()
        .filter(|e| e.to == op.source_package && e.from != op.source_package)
        .map(|e| (absolutize(model, &e.file), e.from.clone()))
        .filter(|(f, _)| !r.moved.contains(f))
        .filter(|(f, _)| r.whole_package || uses_moved.contains(f))
        .collect()
}

/// Importer files that call both moved and remaining symbols (partial moves only).
pub(crate) fn mixed_importers(
    model: &ArchModel,
    op: &RefactorOperation,
    r: &Resolved,
) -> BTreeSet<PathBuf> {
    if r.whole_package {
        return BTreeSet::new();
    }
    let sym_file = symbol_files(model, op);
    let mut moved_use = BTreeSet::new();
    let mut kept_use = BTreeSet::new();
    for c in model.call_edges.iter().filter(|c| c.resolved) {
        let Some(f) = sym_file.get(&c.to) else {
            continue;
        };
        let caller = absolutize(model, &c.file);
        if r.moved.contains(f) {
            moved_use.insert(caller);
        } else {
            kept_use.insert(caller);
        }
    }
    moved_use.intersection(&kept_use).cloned().collect()
}
