//! Mechanical-refactor core behind the `suggest_refactors` / `simulate_split` /
//! `plan_split_refactor` / `apply_split_refactor` / `verify_refactor` / `revert_refactor` MCP tools
//! (docs/refactor-tools.md). Pure functions over `ArchModel` plus on-disk plan state under
//! the worktree's git dir, so plans survive the shared-socket server's per-session models.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arch_model::{ArchModel, PackageNode};
use crate::architecture_checks::find_cycles;
use crate::import_graph::ImportGraph;
use crate::refactor_core::exec::{self, EnvPolicy};
use crate::refactor_core::txn;
use crate::refactor_core::{guard, sha256_hex};

/// Cap on any listed collection in a response; counts and a `truncated` flag accompany it.
pub const LIST_CAP: usize = 50;
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PartitionGroup {
    /// Name of the new sub-package (also its directory and Go package name).
    pub name: String,
    /// Names of the package's symbols that move into it.
    pub symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SimulationResult {
    pub feasible: bool,
    pub blocker: String,
    pub violations_resolved: usize,
    pub violations_introduced: usize,
    pub importers_affected: usize,
    pub importer_list: Vec<String>,
    pub importer_list_truncated: bool,
}

fn cap<T>(mut v: Vec<T>) -> (Vec<T>, bool) {
    let truncated = v.len() > LIST_CAP;
    v.truncate(LIST_CAP);
    (v, truncated)
}

fn graph_of(model: &ArchModel) -> ImportGraph {
    let mut graph = ImportGraph::default();
    graph.nodes.extend(model.packages.keys().cloned());
    for e in &model.import_edges {
        graph.nodes.insert(e.from.clone());
        graph.nodes.insert(e.to.clone());
    }
    graph.edges = model.import_edges.clone();
    graph
}

const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

/// Group names become a directory, a Go package clause and an import alias.
fn valid_go_ident(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !GO_KEYWORDS.contains(&name)
}

/// Which group (by index; `partition.len()` = the remainder left in the original package)
/// owns each symbol id of `package`. Methods follow their parent type.
fn assign_groups(
    model: &ArchModel,
    package: &str,
    partition: &[PartitionGroup],
) -> Result<BTreeMap<String, usize>, String> {
    let pkg = model
        .packages
        .get(package)
        .ok_or_else(|| format!("unknown package '{package}'"))?;
    let mut by_name: BTreeMap<&str, usize> = BTreeMap::new();
    let mut names = BTreeSet::new();
    for (i, g) in partition.iter().enumerate() {
        if g.name.is_empty() || !names.insert(g.name.as_str()) {
            return Err(format!("group name '{}' is empty or duplicated", g.name));
        }
        if !valid_go_ident(&g.name) {
            return Err(format!(
                "group name '{}' must match [a-z_][a-z0-9_]* and not be a Go keyword",
                g.name
            ));
        }
        if g.symbols.is_empty() {
            return Err(format!("group '{}' has no symbols", g.name));
        }
        for s in &g.symbols {
            if by_name.insert(s.as_str(), i).is_some() {
                return Err(format!("symbol '{s}' appears in more than one group"));
            }
            if !pkg
                .symbols
                .iter()
                .any(|sym| &sym.name == s && sym.parent.is_none())
            {
                return Err(format!("symbol '{s}' not found in package '{package}'"));
            }
        }
    }
    Ok(pkg
        .symbols
        .iter()
        .map(|sym| {
            let owner = sym.parent.as_deref().unwrap_or(&sym.name);
            (
                sym.id.clone(),
                by_name.get(owner).copied().unwrap_or(partition.len()),
            )
        })
        .collect())
}

fn group_key(package: &str, partition: &[PartitionGroup], idx: usize) -> String {
    match partition.get(idx) {
        Some(g) => format!("{package}/{}", g.name),
        None => package.to_string(),
    }
}

/// Which new groups the calls between `package` and the other end of an import edge touch.
/// `importer` is true when the other end imports `package` (its calls go into `package`).
fn touched_groups(
    model: &ArchModel,
    package: &str,
    partition: &[PartitionGroup],
    owner: &BTreeMap<String, usize>,
    other: &str,
    importer: bool,
) -> BTreeSet<String> {
    let prefix = format!("{other}::");
    model
        .call_edges
        .iter()
        .filter(|c| c.resolved)
        .filter_map(|c| {
            let (near, far) = if importer {
                (&c.from, &c.to)
            } else {
                (&c.to, &c.from)
            };
            near.starts_with(&prefix).then(|| owner.get(far)).flatten()
        })
        .map(|&i| group_key(package, partition, i))
        .collect()
}

/// The import graph after the split, plus the set of packages that imported `package`.
/// Edges touching `package` are narrowed to the groups the call edges reach, falling back to
/// every new package when there is no call evidence.
fn build_after_graph(
    model: &ArchModel,
    package: &str,
    partition: &[PartitionGroup],
    owner: &BTreeMap<String, usize>,
    before: &ImportGraph,
) -> (ImportGraph, BTreeSet<String>) {
    let all_keys: BTreeSet<String> = owner
        .values()
        .map(|&i| group_key(package, partition, i))
        .collect();
    let mut after = ImportGraph {
        nodes: before
            .nodes
            .iter()
            .filter(|n| *n != package)
            .cloned()
            .collect(),
        ..ImportGraph::default()
    };
    after.nodes.extend(all_keys.iter().cloned());
    let mut importers = BTreeSet::new();
    for e in &before.edges {
        let (importer, exporter) = (e.to == package, e.from == package);
        if !importer && !exporter {
            after.edges.push(e.clone());
            continue;
        }
        let other = if importer { &e.from } else { &e.to };
        if importer {
            importers.insert(e.from.clone());
        }
        let touched = touched_groups(model, package, partition, owner, other, importer);
        for k in if touched.is_empty() {
            &all_keys
        } else {
            &touched
        } {
            let mut edge = e.clone();
            if importer {
                edge.to = k.clone();
            } else {
                edge.from = k.clone();
            }
            after.edges.push(edge);
        }
    }
    for c in model.call_edges.iter().filter(|c| c.resolved) {
        if let (Some(&a), Some(&b)) = (owner.get(&c.from), owner.get(&c.to))
            && a != b
        {
            after.edges.push(crate::import_graph::ImportEdge {
                from: group_key(package, partition, a),
                to: group_key(package, partition, b),
                file: c.file.clone(),
                line: c.line,
            });
        }
    }
    (after, importers)
}

/// What-if: re-runs import-cycle detection on the import graph with `package` split per
/// `partition`. Importer edges are narrowed to the groups whose symbols the importer's
/// call edges actually reach, falling back to every new package when there is no call
/// evidence (conservative: may report a cycle that a finer analysis would not).
pub fn simulate_split(
    model: &ArchModel,
    package: &str,
    partition: &[PartitionGroup],
) -> SimulationResult {
    let infeasible = |blocker: String| SimulationResult {
        feasible: false,
        blocker,
        violations_resolved: 0,
        violations_introduced: 0,
        importers_affected: 0,
        importer_list: vec![],
        importer_list_truncated: false,
    };
    let owner = match assign_groups(model, package, partition) {
        Ok(o) => o,
        Err(e) => return infeasible(e),
    };
    let before = graph_of(model);
    let before_cycles = find_cycles(&before).len();
    let (after, importers) = build_after_graph(model, package, partition, &owner, &before);
    let after_cycles = find_cycles(&after).len();
    let introduced = after_cycles.saturating_sub(before_cycles);
    let (importer_list, importer_list_truncated) = cap(importers.iter().cloned().collect());
    SimulationResult {
        feasible: introduced == 0,
        blocker: if introduced == 0 {
            String::new()
        } else {
            format!("partition introduces {introduced} new import cycle(s)")
        },
        violations_resolved: before_cycles.saturating_sub(after_cycles),
        violations_introduced: introduced,
        importers_affected: importers.len(),
        importer_list,
        importer_list_truncated,
    }
}

/// Partition `package` into the connected components of its files (linked by resolved
/// calls between their symbols). `None` when the package has fewer than two components.
pub fn auto_partition(model: &ArchModel, package: &str) -> Option<Vec<PartitionGroup>> {
    let pkg = model.packages.get(package)?;
    let file_of: BTreeMap<&str, &Path> = pkg
        .symbols
        .iter()
        .map(|s| (s.id.as_str(), s.file.as_path()))
        .collect();
    let files: Vec<&Path> = pkg
        .symbols
        .iter()
        .map(|s| s.file.as_path())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut parent: Vec<usize> = (0..files.len()).collect();
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        if p[x] != x {
            let r = find(p, p[x]);
            p[x] = r;
        }
        p[x]
    }
    let idx = |f: &Path| files.iter().position(|x| *x == f);
    for c in model.call_edges.iter().filter(|c| c.resolved) {
        if let (Some(a), Some(b)) = (file_of.get(c.from.as_str()), file_of.get(c.to.as_str()))
            && let (Some(i), Some(j)) = (idx(a), idx(b))
        {
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            parent[ri] = rj;
        }
    }
    let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..files.len() {
        let r = find(&mut parent, i);
        comps.entry(r).or_default().push(i);
    }
    if comps.len() < 2 {
        return None;
    }
    let mut groups: Vec<(usize, PartitionGroup)> = comps
        .values()
        .map(|members| {
            let mut symbols: Vec<String> = pkg
                .symbols
                .iter()
                .filter(|s| s.parent.is_none() && members.iter().any(|&m| files[m] == s.file))
                .map(|s| s.name.clone())
                .collect();
            symbols.sort();
            (
                members[0],
                PartitionGroup {
                    name: String::new(),
                    symbols,
                },
            )
        })
        .filter(|(_, g)| !g.symbols.is_empty())
        .collect();
    groups.sort_by_key(|(first, _)| *first);
    if groups.len() < 2 {
        return None;
    }
    // The largest component stays in the original package so its importers keep working.
    let keep =
        (0..groups.len()).max_by_key(|&i| (groups[i].1.symbols.len(), std::cmp::Reverse(i)))?;
    groups.remove(keep);
    Some(
        groups
            .into_iter()
            .enumerate()
            .map(|(i, (_, mut g))| {
                g.name = format!("part{}", i + 1);
                g
            })
            .collect(),
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub operation: String,
    pub package: String,
    pub partition: Vec<PartitionGroup>,
    pub evidence: SuggestionEvidence,
    pub feasible: bool,
    pub blockers: Vec<String>,
    pub mcp_call: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuggestionEvidence {
    pub cycles_resolved: usize,
    pub importers_affected: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Abstention {
    pub operation: String,
    pub package: String,
    pub reason: String,
}

pub fn plan_call(package: &str, partition: &[PartitionGroup]) -> String {
    format!(
        "plan_split_refactor({})",
        serde_json::json!({"operation":"split_package","args":{"package":package,"partition":partition}})
    )
}

/// Ranks `split_package` operations for packages that sit in an import cycle. Evidence is
/// cycle membership plus the simulated outcome (churn signals are a follow-up); anything
/// without a clean partition or a simulated improvement is reported in `abstained`.
pub fn suggest(model: &ArchModel, scope: &[String]) -> (Vec<Suggestion>, Vec<Abstention>) {
    let in_scope = |p: &str| scope.is_empty() || scope.iter().any(|s| p.starts_with(s.as_str()));
    let cyclic: BTreeSet<String> = find_cycles(&graph_of(model))
        .into_iter()
        .flatten()
        .collect();
    let (mut out, mut abstained) = (vec![], vec![]);
    for package in cyclic
        .iter()
        .filter(|p| model.packages.contains_key(*p) && in_scope(p))
    {
        let abstain = |reason: String| Abstention {
            operation: "split_package".into(),
            package: package.clone(),
            reason,
        };
        let Some(partition) = auto_partition(model, package) else {
            abstained.push(abstain(
                "in an import cycle, but its files form a single connected component: no clean partition to propose".into(),
            ));
            continue;
        };
        let sim = simulate_split(model, package, &partition);
        if sim.feasible && sim.violations_resolved > 0 {
            out.push(Suggestion {
                operation: "split_package".into(),
                package: package.clone(),
                evidence: SuggestionEvidence {
                    cycles_resolved: sim.violations_resolved,
                    importers_affected: sim.importers_affected,
                },
                feasible: true,
                blockers: vec![],
                mcp_call: plan_call(package, &partition),
                partition,
            });
        } else if !sim.feasible {
            abstained.push(abstain(format!(
                "simulated split is not feasible: {}",
                sim.blocker
            )));
        } else {
            abstained.push(abstain(
                "simulated split does not resolve any import cycle; evidence too thin to recommend"
                    .into(),
            ));
        }
    }
    out.sort_by(|a, b| {
        b.evidence
            .cycles_resolved
            .cmp(&a.evidence.cycles_resolved)
            .then(
                a.evidence
                    .importers_affected
                    .cmp(&b.evidence.importers_affected),
            )
            .then(a.package.cmp(&b.package))
    });
    (out, abstained)
}

// ---------------------------------------------------------------------------------
// Plans, persistence, locks, apply/verify/revert
// ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Planned,
    Applied,
    Reverted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedEdit {
    /// Repo-relative path.
    pub file: String,
    /// `write` creates/overwrites with `content`; `delete` removes the file.
    pub kind: String,
    pub description: String,
    #[serde(default)]
    pub content: Option<String>,
    /// File content at plan time (`None` = did not exist); used by revert and rollback.
    #[serde(default)]
    pub original: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorPlan {
    pub plan_id: String,
    /// HEAD commit at plan time; the plan is stale once HEAD moves.
    pub staleness_token: String,
    pub operation: String,
    pub package: String,
    pub edits: Vec<PlannedEdit>,
    pub importers_touched: Vec<String>,
    pub exported_surface_changes: Vec<String>,
    pub verification_steps: Vec<String>,
    /// Every repo-relative path the plan writes; callers compare these across worktrees.
    pub touched_paths: Vec<String>,
    pub status: PlanStatus,
}

#[derive(Debug, Clone)]
pub struct RefactorError {
    pub code: &'static str,
    pub message: String,
}

fn err<T>(code: &'static str, message: impl Into<String>) -> Result<T, RefactorError> {
    Err(RefactorError {
        code,
        message: message.into(),
    })
}

fn git(repo: &Path, args: &[&str]) -> Result<String, RefactorError> {
    let mut cmd = crate::git_cmd::git_command(repo);
    cmd.args(args);
    let Some(out) = crate::git_cmd::bounded_output(cmd, GIT_TIMEOUT) else {
        return err(
            "git_failed",
            format!("git {} timed out or could not start", args.join(" ")),
        );
    };
    if !out.status.success() {
        return err(
            "git_failed",
            format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn head(repo: &Path) -> Result<String, RefactorError> {
    Ok(git(repo, &["rev-parse", "HEAD"])?.trim().to_string())
}

fn state_dir(repo: &Path) -> Result<PathBuf, RefactorError> {
    let dir = git(repo, &["rev-parse", "--git-dir"])?;
    Ok(repo.join(dir.trim()).join("kibitzer-refactor"))
}

fn new_plan_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!(
        "{nanos}:{}:{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    sha256_hex(seed.as_bytes())[..16].to_string()
}

/// Every edit must stay inside the repo root: no absolute paths, `..`, or symlink that leaves it.
fn safe_rel(repo: &Path, rel: &str) -> Result<(), RefactorError> {
    let p = Path::new(rel);
    let outside = guard::check_plan_rel_path(rel).is_err()
        || p.components().any(|c| matches!(c, Component::Prefix(_)))
        || !guard::is_inside(repo, &repo.join(p));
    if outside {
        return err(
            "path_outside_repo",
            format!("edit path '{rel}' is outside the repository root"),
        );
    }
    Ok(())
}

fn plan_path(repo: &Path, id: &str) -> Result<PathBuf, RefactorError> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return err("unknown_plan", format!("unknown plan_id '{id}'"));
    }
    Ok(state_dir(repo)?.join("plans").join(format!("{id}.json")))
}

fn save_plan(repo: &Path, plan: &RefactorPlan) -> Result<(), RefactorError> {
    let path = plan_path(repo, &plan.plan_id)?;
    let io = |e: std::io::Error| RefactorError {
        code: "io",
        message: e.to_string(),
    };
    std::fs::create_dir_all(path.parent().unwrap()).map_err(io)?;
    let json = serde_json::to_string(plan).map_err(|e| RefactorError {
        code: "io",
        message: e.to_string(),
    })?;
    std::fs::write(path, json).map_err(io)
}

pub fn load_plan(repo: &Path, id: &str) -> Result<RefactorPlan, RefactorError> {
    let path = plan_path(repo, id)?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return err("unknown_plan", format!("unknown plan_id '{id}'")),
    };
    serde_json::from_str(&raw).map_err(|e| RefactorError {
        code: "io",
        message: format!("corrupt plan: {e}"),
    })
}

/// Builds and persists a `split_package` plan. Never touches the working tree.
pub fn plan_split(
    model: &ArchModel,
    repo: &Path,
    package: &str,
    partition: &[PartitionGroup],
) -> Result<RefactorPlan, RefactorError> {
    let sim = simulate_split(model, package, partition);
    if !sim.feasible {
        return err("infeasible", sim.blocker);
    }
    let pkg = &model.packages[package];
    let mut edits = vec![];
    let mut surface = vec![];
    for g in partition {
        for file in group_files(pkg, repo, g) {
            edits.extend(file_move_edits(pkg, repo, &file, g)?);
        }
        surface.push(format!(
            "symbols {:?} move to package {package}/{}",
            g.symbols, g.name
        ));
    }
    edits.extend(importer_edits(model, repo, package, partition)?);
    let verification_steps = if repo.join("go.mod").is_file() {
        vec!["go build ./...".to_string()]
    } else {
        vec![]
    };
    let touched_paths = edits.iter().map(|e| e.file.clone()).collect();
    let plan = RefactorPlan {
        plan_id: new_plan_id(),
        staleness_token: head(repo)?,
        operation: "split_package".into(),
        package: package.into(),
        edits,
        importers_touched: sim.importer_list,
        exported_surface_changes: surface,
        verification_steps,
        touched_paths,
        status: PlanStatus::Planned,
    };
    save_plan(repo, &plan)?;
    Ok(plan)
}

fn repo_rel(repo: &Path, f: &Path) -> String {
    f.strip_prefix(repo)
        .unwrap_or(f)
        .to_string_lossy()
        .to_string()
}

/// Repo-relative files that define any of the group's symbols.
fn group_files(pkg: &PackageNode, repo: &Path, g: &PartitionGroup) -> BTreeSet<String> {
    pkg.symbols
        .iter()
        .filter(|s| {
            g.symbols
                .iter()
                .any(|x| x == s.parent.as_deref().unwrap_or(&s.name))
        })
        .map(|s| repo_rel(repo, &s.file))
        .collect()
}

/// The write-to-new-package + delete-original edit pair for one whole-file move.
fn file_move_edits(
    pkg: &PackageNode,
    repo: &Path,
    file: &str,
    g: &PartitionGroup,
) -> Result<[PlannedEdit; 2], RefactorError> {
    safe_rel(repo, file)?;
    if !file.ends_with(".go") {
        return err(
            "unsupported",
            "split_package currently supports Go packages only",
        );
    }
    let other = pkg
        .symbols
        .iter()
        .filter(|s| repo_rel(repo, &s.file) == file)
        .map(|s| s.parent.as_deref().unwrap_or(&s.name))
        .find(|n| !g.symbols.iter().any(|x| x == n));
    if let Some(o) = other {
        return err(
            "file_straddles_groups",
            format!(
                "{file} also defines '{o}', which is outside group '{}'; file-level moves need whole files in one group",
                g.name
            ),
        );
    }
    let original = std::fs::read_to_string(repo.join(file)).map_err(|e| RefactorError {
        code: "io",
        message: format!("reading {file}: {e}"),
    })?;
    let p = Path::new(file);
    let dest = p
        .parent()
        .unwrap_or(Path::new(""))
        .join(&g.name)
        .join(p.file_name().unwrap());
    let dest_s = dest.to_string_lossy().to_string();
    if repo.join(&dest).exists() {
        return err("dest_exists", format!("{dest_s} already exists"));
    }
    Ok([
        PlannedEdit {
            file: dest_s.clone(),
            kind: "write".into(),
            description: format!("move {file} to package {}", g.name),
            content: Some(rewrite_go_package(&original, &g.name)),
            original: None,
        },
        PlannedEdit {
            file: file.to_string(),
            kind: "delete".into(),
            description: format!("remove {file} (moved to {dest_s})"),
            content: None,
            original: Some(original),
        },
    ])
}

/// Rewrites `alias.Sym` selectors in files importing `package` so moved symbols point at
/// their new sub-package, adding the new import and dropping the old one if it went unused.
/// Textual (regex) rewriting: a selector inside a string or comment is rewritten too, and
/// verification is the backstop.
fn importer_edits(
    model: &ArchModel,
    repo: &Path,
    package: &str,
    partition: &[PartitionGroup],
) -> Result<Vec<PlannedEdit>, RefactorError> {
    let mut out = vec![];
    for (file, aliases) in &model.file_import_aliases {
        let Some(alias) = aliases
            .iter()
            .find(|(_, v)| v.as_str() == package)
            .map(|(k, _)| k)
        else {
            continue;
        };
        let rel_file = file
            .strip_prefix(repo)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();
        safe_rel(repo, &rel_file)?;
        let Ok(original) = std::fs::read_to_string(repo.join(&rel_file)) else {
            continue;
        };
        let mut text = original.clone();
        let mut new_imports = vec![];
        for g in partition {
            for sym in &g.symbols {
                let re = regex::Regex::new(&format!(
                    r"\b{}\.{}\b",
                    regex::escape(alias),
                    regex::escape(sym)
                ))
                .map_err(|e| RefactorError {
                    code: "io",
                    message: e.to_string(),
                })?;
                if re.is_match(&text) {
                    text = re
                        .replace_all(&text, format!("{}.{sym}", g.name).as_str())
                        .to_string();
                    let imp = format!("\t{} \"{package}/{}\"", g.name, g.name);
                    if !new_imports.contains(&imp) {
                        new_imports.push(imp);
                    }
                }
            }
        }
        if new_imports.is_empty() {
            continue;
        }
        let still_used = regex::Regex::new(&format!(r"\b{}\.", regex::escape(alias)))
            .unwrap()
            .is_match(&text);
        text = add_go_imports(&text, package, &new_imports, !still_used);
        out.push(PlannedEdit {
            file: rel_file.clone(),
            kind: "write".into(),
            description: format!("update imports/selectors in {rel_file}"),
            content: Some(text),
            original: Some(original),
        });
    }
    Ok(out)
}

fn add_go_imports(src: &str, old_path: &str, new_imports: &[String], drop_old: bool) -> String {
    let quoted = format!("\"{old_path}\"");
    let mut out: Vec<String> = vec![];
    let mut done = false;
    for line in src.lines() {
        let t = line.trim_start();
        if !done && t.starts_with("import (") {
            out.push(line.to_string());
            out.extend(new_imports.iter().cloned());
            done = true;
        } else if !done && t.starts_with("import ") && line.contains(&quoted) {
            out.push("import (".into());
            out.extend(new_imports.iter().cloned());
            if !drop_old {
                out.push(format!("\t{}", t.trim_start_matches("import ").trim()));
            }
            out.push(")".into());
            done = true;
        } else if drop_old && line.contains(&quoted) && !t.starts_with("import") {
            continue;
        } else {
            out.push(line.to_string());
        }
    }
    let mut joined = out.join("\n");
    if src.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

fn rewrite_go_package(src: &str, name: &str) -> String {
    let mut done = false;
    src.split_inclusive('\n')
        .map(|line| {
            if !done && line.trim_start().starts_with("package ") {
                done = true;
                let nl = if line.ends_with('\n') { "\n" } else { "" };
                format!("package {name}{nl}")
            } else {
                line.to_string()
            }
        })
        .collect()
}

// ---- locks: one file per touched path, owned by a plan, valid while HEAD is unchanged ----

fn lock_file(repo: &Path, rel: &str) -> Result<PathBuf, RefactorError> {
    Ok(state_dir(repo)?
        .join("locks")
        .join(&sha256_hex(rel.as_bytes())[..32]))
}

fn acquire_locks(repo: &Path, plan: &RefactorPlan) -> Result<(), RefactorError> {
    let mut held: Vec<PathBuf> = vec![];
    let res = (|| {
        for rel in &plan.touched_paths {
            let lf = lock_file(repo, rel)?;
            std::fs::create_dir_all(lf.parent().unwrap()).map_err(|e| RefactorError {
                code: "io",
                message: e.to_string(),
            })?;
            let body = serde_json::json!({"plan_id": plan.plan_id, "head": plan.staleness_token})
                .to_string();
            for attempt in 0..2 {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&lf)
                {
                    Ok(mut f) => {
                        use std::io::Write;
                        let _ = f.write_all(body.as_bytes());
                        held.push(lf.clone());
                        break;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        let cur: serde_json::Value = std::fs::read_to_string(&lf)
                            .ok()
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_default();
                        let owner = cur["plan_id"].as_str().unwrap_or("");
                        // An unreadable lock body counts as live: never steal on a guess.
                        let live = cur["head"]
                            .as_str()
                            .is_none_or(|h| h == plan.staleness_token);
                        if live || attempt == 1 {
                            return err(
                                "lock_conflict",
                                format!("{rel} is locked by plan '{owner}' from another session"),
                            );
                        }
                        // Stale: owner's HEAD is gone. Rename is atomic, so only one stealer wins.
                        let tomb = lf.with_extension(format!("stale-{}", plan.plan_id));
                        if std::fs::rename(&lf, &tomb).is_ok() {
                            let _ = std::fs::remove_file(&tomb);
                        }
                    }
                    Err(e) => return err("io", e.to_string()),
                }
            }
        }
        Ok(())
    })();
    if res.is_err() {
        for lf in held {
            let _ = std::fs::remove_file(lf);
        }
    }
    res
}

fn release_locks(repo: &Path, plan: &RefactorPlan) {
    for rel in &plan.touched_paths {
        if let Ok(lf) = lock_file(repo, rel) {
            let owned = std::fs::read_to_string(&lf)
                .map(|s| s.contains(&plan.plan_id))
                .unwrap_or(false);
            if owned {
                let _ = std::fs::remove_file(lf);
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Verification {
    pub passed: bool,
    pub skipped: bool,
    pub output: String,
}

fn truncate_at_boundary(s: &mut String, max: usize) {
    if s.len() > max {
        let end = (0..=max)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        s.truncate(end);
    }
}

/// Per verification step; a hung build must not hold the plan locks forever.
const VERIFICATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

pub fn run_verification(repo: &Path, plan: &RefactorPlan) -> Verification {
    if plan.verification_steps.is_empty() {
        return Verification {
            passed: true,
            skipped: true,
            output: "no verification steps for this repo".into(),
        };
    }
    let mut output = String::new();
    for step in &plan.verification_steps {
        let tool = step.split_whitespace().next().unwrap_or("");
        if Command::new(tool).arg("version").output().is_err() {
            output.push_str(&format!("skipped '{step}': '{tool}' not on PATH\n"));
            return Verification {
                passed: true,
                skipped: true,
                output,
            };
        }
        let run = exec::run(&exec::Spec {
            program: "sh",
            args: &["-c".to_string(), step.clone()],
            dir: repo,
            timeout: VERIFICATION_TIMEOUT,
            env: EnvPolicy::Inherit,
        });
        output.push_str(&run.output);
        if !run.ok() {
            truncate_at_boundary(&mut output, 4000);
            return Verification {
                passed: false,
                skipped: false,
                output,
            };
        }
    }
    truncate_at_boundary(&mut output, 4000);
    Verification {
        passed: true,
        skipped: false,
        output,
    }
}

fn paths_refs(plan: &RefactorPlan) -> Vec<&str> {
    plan.touched_paths.iter().map(String::as_str).collect()
}

fn txn_error(e: anyhow::Error) -> RefactorError {
    // A failed rollback leaves the tree possibly dirty and the log on disk: callers must be able
    // to tell it from an ordinary I/O failure.
    let code = if e.downcast_ref::<txn::RollbackFailed>().is_some() {
        "rollback_failed"
    } else {
        "io"
    };
    RefactorError {
        code,
        message: format!("{e:#}"),
    }
}

/// The plan's edits as transaction changes: the writes and deletions `apply` makes.
fn apply_changes(repo: &Path, plan: &RefactorPlan) -> Vec<txn::Change> {
    plan.edits
        .iter()
        .map(|e| txn::Change {
            path: repo.join(&e.file),
            op: if e.kind == "write" {
                txn::Op::Put {
                    bytes: e.content.clone().unwrap_or_default().into_bytes(),
                    mode_from: None,
                }
            } else {
                txn::Op::Delete
            },
        })
        .collect()
}

/// The inverse: put each file back to its plan-time content, or delete what the plan created.
fn revert_changes(repo: &Path, plan: &RefactorPlan) -> Vec<txn::Change> {
    plan.edits
        .iter()
        .map(|e| txn::Change {
            path: repo.join(&e.file),
            op: match &e.original {
                Some(orig) => txn::Op::Put {
                    bytes: orig.clone().into_bytes(),
                    mode_from: None,
                },
                None => txn::Op::Delete,
            },
        })
        .collect()
}

/// Unstages the plan's paths (apply `git add`s them) and drops directories the plan created.
fn reset_index(repo: &Path, plan: &RefactorPlan) {
    for e in plan.edits.iter().filter(|e| e.original.is_none()) {
        if let Some(parent) = repo.join(&e.file).parent() {
            let _ = std::fs::remove_dir(parent); // only if now empty
        }
    }
    let mut args = vec!["reset", "-q", "--"];
    args.extend(paths_refs(plan));
    let _ = git(repo, &args);
}

/// Rolls an applied-but-failed plan back through the transaction, unstages it and releases its
/// locks. `Err` carries the rollback failure (the log is kept for the next recovery).
fn abort_apply(repo: &Path, plan: &RefactorPlan, pending: txn::Pending) -> Result<(), String> {
    let rolled = pending.rollback().map_err(|e| format!("{e:#}"));
    reset_index(repo, plan);
    release_locks(repo, plan);
    rolled
}

#[derive(Debug, Serialize)]
pub struct ApplyOutcome {
    pub applied: bool,
    pub rolled_back: bool,
    pub diff_summary: String,
    pub verification: Verification,
}

pub fn apply(repo: &Path, id: &str) -> Result<ApplyOutcome, RefactorError> {
    // Lock and crash-recover first: a half-applied tree left by a killed process is restored
    // before the HEAD and dirty-tree checks below look at it.
    let session = txn::session(repo).map_err(txn_error)?;
    let mut plan = load_plan(repo, id)?;
    if plan.status != PlanStatus::Planned {
        return err(
            "not_applicable",
            format!("plan '{id}' is {:?}, not planned", plan.status),
        );
    }
    for e in &plan.edits {
        safe_rel(repo, &e.file)?;
    }
    if head(repo)? != plan.staleness_token {
        return err(
            "stale_plan",
            "HEAD changed since plan_split_refactor; plan again",
        );
    }
    acquire_locks(repo, &plan)?;
    // Dirty check runs while the locks are held so two sessions cannot both pass it.
    // The transaction's own lock file lives in `.kibitzer/plans`; it is not the user's change.
    let status = git(
        repo,
        &[
            "status",
            "--porcelain",
            "--",
            ".",
            ":(exclude).kibitzer/plans",
        ],
    );
    match status {
        Ok(s) if s.trim().is_empty() => {}
        Ok(_) => {
            release_locks(repo, &plan);
            return err(
                "dirty_tree",
                "working tree has uncommitted changes; commit or stash first",
            );
        }
        Err(e) => {
            release_locks(repo, &plan);
            return Err(e);
        }
    }
    let pending = match session.begin(
        &plan.plan_id,
        &apply_changes(repo, &plan),
        &txn::real_rename,
    ) {
        Ok(p) => p,
        Err(e) => {
            release_locks(repo, &plan);
            return Err(txn_error(e));
        }
    };
    let mut add = vec!["add", "-A", "--"];
    add.extend(paths_refs(&plan));
    if let Err(e) = git(repo, &add) {
        return Err(match abort_apply(repo, &plan, pending) {
            Ok(()) => e,
            Err(rb) => RefactorError {
                code: e.code,
                message: format!("{}; rollback also failed: {rb}", e.message),
            },
        });
    }
    let mut verification = run_verification(repo, &plan);
    if !verification.passed {
        if let Err(rb) = abort_apply(repo, &plan, pending) {
            verification
                .output
                .push_str(&format!("\nrollback failed: {rb}"));
        }
        return Ok(ApplyOutcome {
            applied: false,
            rolled_back: true,
            diff_summary: String::new(),
            verification,
        });
    }
    plan.status = PlanStatus::Applied;
    if let Err(e) = save_plan(repo, &plan) {
        let _ = abort_apply(repo, &plan, pending);
        return Err(e);
    }
    // Last, so the plan can still be rolled back if saving it failed. A failed commit can leave
    // the log behind, which the next recovery rolls back: the plan must then read Planned again
    // and its locks must not stay held.
    if let Err(e) = pending.commit() {
        plan.status = PlanStatus::Planned;
        let _ = save_plan(repo, &plan);
        reset_index(repo, &plan);
        release_locks(repo, &plan);
        return Err(txn_error(e));
    }
    let diff_summary = git(repo, &["diff", "--cached", "--stat"]).unwrap_or_default();
    Ok(ApplyOutcome {
        applied: true,
        rolled_back: false,
        diff_summary,
        verification,
    })
}

pub fn verify(repo: &Path, id: &str) -> Result<Verification, RefactorError> {
    let plan = load_plan(repo, id)?;
    if plan.status != PlanStatus::Applied {
        return err(
            "not_applicable",
            format!("plan '{id}' is {:?}, not applied", plan.status),
        );
    }
    Ok(run_verification(repo, &plan))
}

pub fn revert(repo: &Path, id: &str) -> Result<Vec<String>, RefactorError> {
    let session = txn::session(repo).map_err(txn_error)?;
    let mut plan = load_plan(repo, id)?;
    if plan.status != PlanStatus::Applied {
        return err(
            "not_applicable",
            format!("plan '{id}' is {:?}, not applied", plan.status),
        );
    }
    if head(repo)? != plan.staleness_token {
        return err(
            "already_committed",
            "HEAD moved since apply; revert with git instead",
        );
    }
    for e in &plan.edits {
        let cur = std::fs::read_to_string(repo.join(&e.file)).ok();
        let expected = if e.kind == "write" {
            e.content.clone()
        } else {
            None
        };
        if cur != expected {
            return err(
                "modified_since_apply",
                format!("{} was modified after apply; not reverting", e.file),
            );
        }
    }
    // The inverse edit set is itself a transaction: all files return to their plan-time content
    // or none does, and a crash mid-revert is recovered like any other.
    session
        .begin(
            &format!("revert-{}", plan.plan_id),
            &revert_changes(repo, &plan),
            &txn::real_rename,
        )
        .and_then(txn::Pending::commit)
        .map_err(txn_error)?;
    reset_index(repo, &plan);
    release_locks(repo, &plan);
    plan.status = PlanStatus::Reverted;
    save_plan(repo, &plan)?;
    Ok(plan.touched_paths)
}

pub fn cap_list<T>(v: Vec<T>) -> (Vec<T>, bool) {
    cap(v)
}

/// Bounded response for `plan_split_refactor`: edit contents stay on disk, lists are capped.
pub fn plan_response(plan: &RefactorPlan) -> serde_json::Value {
    let edits: Vec<_> = plan
        .edits
        .iter()
        .map(|e| serde_json::json!({"file": e.file, "kind": e.kind, "description": e.description}))
        .collect();
    let (edit_list, edits_truncated) = cap(edits);
    let (importers, importers_truncated) = cap(plan.importers_touched.clone());
    let (touched, touched_truncated) = cap(plan.touched_paths.clone());
    serde_json::json!({
        "plan_id": plan.plan_id,
        "staleness_token": plan.staleness_token,
        "operation": plan.operation,
        "edits": edit_list,
        "edit_count": plan.edits.len(),
        "edits_truncated": edits_truncated,
        "importers_touched": importers,
        "importers_truncated": importers_truncated,
        "exported_surface_changes": plan.exported_surface_changes,
        "verification_steps": plan.verification_steps,
        "touched_paths": touched,
        "touched_paths_truncated": touched_truncated,
    })
}

#[cfg(test)]
#[path = "refactor_split_tests.rs"]
mod tests;
