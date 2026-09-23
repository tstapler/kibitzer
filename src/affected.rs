//! `kibitzer architecture affected` — diffs the working tree against a `--base` ref,
//! walks `ArchModel.import_edges` in reverse from the changed files' packages, and
//! returns either the affected package set or a conservative `__ALL__`-style bail-out
//! when the diff touches something with unbounded blast radius. See
//! `project_plans/affected-tests/implementation/plan.md` for the design.
//!
//! Batch-only — never wired into `default_checks()`/hook mode, matching the rest of the
//! `ArchitectureAction` family. v1 is Go-only (see `ArchModel`'s import-graph coverage).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::arch_model::{self, ArchModel, PruneConfig};
use crate::config::AffectedConfig;
use crate::import_graph::ImportEdge;

/// How one path changed between `base` and the working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed { old: PathBuf },
}

/// One changed path, repo-relative (as `git diff --name-status` reports it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: PathBuf,
    pub status: ChangeStatus,
}

/// Why `compute_affected` bailed out to `AffectedResult::BailOut` instead of returning a
/// narrowed package list. Deliberately has no "unresolvable base ref" variant — that
/// case is a hard `Err` from [`resolve_base_sha`], since no diff was ever computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BailOutReason {
    GlobMatched { glob: String, path: PathBuf },
    PackageFullyRemoved { path: PathBuf },
    PackageClauseChanged { path: PathBuf },
    ShallowCloneOrNoCommonHistory,
}

impl std::fmt::Display for BailOutReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BailOutReason::GlobMatched { glob, path } => {
                write!(f, "{} matched bail-out glob '{glob}'", path.display())
            }
            BailOutReason::PackageFullyRemoved { path } => {
                write!(
                    f,
                    "{} was the last file in its package — package fully removed",
                    path.display()
                )
            }
            BailOutReason::PackageClauseChanged { path } => {
                write!(
                    f,
                    "{}'s `package` clause changed without a rename",
                    path.display()
                )
            }
            BailOutReason::ShallowCloneOrNoCommonHistory => {
                write!(
                    f,
                    "base ref shares no common history with HEAD (shallow clone?)"
                )
            }
        }
    }
}

/// Either a narrowed, transitively-closed affected-package list, or a bail-out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AffectedResult {
    Packages(Vec<String>),
    BailOut(BailOutReason),
}

/// Runs `git <args>` with `cwd = repo_root`, returning the raw `Output` — callers decide
/// whether a non-zero exit is a hard error, an `Ok(None)`, or something else (this
/// module has all three shapes), so this stays a thin wrapper rather than baking in one
/// call site's error-handling policy.
fn run_git(repo_root: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))
}

/// Whether `path` has a `.go` extension — the Go-only v1 scope's single-path check, used
/// where a bulk file-list filter (`import_graph::files_for`) doesn't apply.
fn is_go_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("go")
}

/// `path` as a `/`-separated string, matching git's own path convention — needed
/// wherever a `PathBuf` is handed to `git diff -- <path>` or `glob::matches_scope`.
fn git_path_str(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Resolves `repo_root` to the actual git working-tree top-level — matches
/// `hotspots.rs::git_toplevel`'s convention, since every subsequent git call and every
/// path comparison against `ArchModel` needs the true top-level, not a subdirectory a
/// caller happened to pass. A non-zero exit here (not a git repo, `git` binary missing)
/// is a hard error, never a bail-out — there is nothing to compute at all.
fn repo_toplevel(repo_root: &Path) -> Result<PathBuf> {
    let output = run_git(repo_root, &["rev-parse", "--show-toplevel"])?;
    if !output.status.success() {
        bail!(
            "{} is not a git repository: git rev-parse --show-toplevel exited with {}: {}",
            repo_root.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// Resolves `base` to a concrete SHA before any other git call, so a moving branch name
/// (someone pushes to `main` mid-run) can't silently shrink or distort the diff. An
/// unresolvable `base` (typo, unfetched branch) is a **hard error**, not a bail-out —
/// see `plan.md` Story 1.1.1's reconciled exit contract.
pub fn resolve_base_sha(repo_root: &Path, base: &str) -> Result<String> {
    let output = run_git(repo_root, &["rev-parse", base])?;
    if !output.status.success() {
        bail!(
            "base ref '{base}' did not resolve to a commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Parses `git diff --name-status [-M]` output into `ChangedFile`s. Tab-separated:
/// `A\tpath`, `M\tpath`, `D\tpath`, or `R<score>\told\tnew`.
fn parse_name_status(output: &str) -> Vec<ChangedFile> {
    let mut result = Vec::new();
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(code) = parts.next() else { continue };
        let Some(first_char) = code.chars().next() else {
            continue;
        };
        match first_char {
            'A' | 'M' | 'D' => {
                let Some(path) = parts.next() else { continue };
                let status = match first_char {
                    'A' => ChangeStatus::Added,
                    'M' => ChangeStatus::Modified,
                    _ => ChangeStatus::Deleted,
                };
                result.push(ChangedFile {
                    path: PathBuf::from(path),
                    status,
                });
            }
            'R' => {
                if let (Some(old), Some(new)) = (parts.next(), parts.next()) {
                    result.push(ChangedFile {
                        path: PathBuf::from(new),
                        status: ChangeStatus::Renamed {
                            old: PathBuf::from(old),
                        },
                    });
                }
            }
            _ => {}
        }
    }
    result
}

/// `git diff --name-status -M <base_sha>...HEAD` — committed changes since the merge
/// base, with rename detection. `Ok(None)` (not `Err`) on a non-zero exit: this is the
/// ambiguous "no common history" case (e.g. a shallow clone), which `compute_affected`
/// maps to `BailOutReason::ShallowCloneOrNoCommonHistory` rather than a hard error —
/// `base_sha` itself already resolved fine. The discarded stderr is still surfaced (not
/// silently dropped) since a real git failure unrelated to shared history would
/// otherwise be misclassified as "shallow clone" with no diagnostic trail.
fn diff_base_head(repo_root: &Path, base_sha: &str) -> Result<Option<Vec<ChangedFile>>> {
    let output = run_git(
        repo_root,
        &["diff", "--name-status", "-M", &format!("{base_sha}...HEAD")],
    )?;
    if !output.status.success() {
        eprintln!(
            "[kibitzer] git diff --name-status failed (treated as no common history): {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Ok(None);
    }
    Ok(Some(parse_name_status(&String::from_utf8_lossy(
        &output.stdout,
    ))))
}

/// Uncommitted tracked edits (`git diff --name-status HEAD`) plus untracked new files
/// (`git ls-files --others --exclude-standard`, all `Added`) — matching
/// `test-affected.py`'s working-tree-inclusive scope.
fn diff_working_tree(repo_root: &Path) -> Result<Vec<ChangedFile>> {
    let tracked = run_git(repo_root, &["diff", "--name-status", "HEAD"])?;
    if !tracked.status.success() {
        bail!(
            "git diff --name-status HEAD exited with {}: {}",
            tracked.status,
            String::from_utf8_lossy(&tracked.stderr).trim()
        );
    }
    let mut changed = parse_name_status(&String::from_utf8_lossy(&tracked.stdout));

    let untracked = run_git(repo_root, &["ls-files", "--others", "--exclude-standard"])?;
    if !untracked.status.success() {
        bail!(
            "git ls-files --others --exclude-standard exited with {}: {}",
            untracked.status,
            String::from_utf8_lossy(&untracked.stderr).trim()
        );
    }
    for line in String::from_utf8_lossy(&untracked.stdout).lines() {
        if line.is_empty() {
            continue;
        }
        changed.push(ChangedFile {
            path: PathBuf::from(line),
            status: ChangeStatus::Added,
        });
    }
    Ok(changed)
}

/// Every changed path since `base_sha`, unioning committed, uncommitted, and untracked
/// changes with no duplicates (a path in both the committed diff and the working-tree
/// diff keeps the committed entry, since it may carry rename info the working-tree diff
/// wouldn't). `Ok(None)` when `diff_base_head` can't establish common history.
pub fn diff_changed_files(repo_root: &Path, base_sha: &str) -> Result<Option<Vec<ChangedFile>>> {
    let Some(base_changes) = diff_base_head(repo_root, base_sha)? else {
        return Ok(None);
    };
    let working_tree = diff_working_tree(repo_root)?;

    let mut by_path: BTreeMap<PathBuf, ChangedFile> = BTreeMap::new();
    for entry in base_changes {
        by_path.insert(entry.path.clone(), entry);
    }
    for entry in working_tree {
        by_path.entry(entry.path.clone()).or_insert(entry);
    }
    Ok(Some(by_path.into_values().collect()))
}

/// A `Modified` `.go` file's diff hunk (base vs. working tree — covers both a committed
/// and an uncommitted edit) touched its own `package` line and the identifier actually
/// changed. A cheap content-level check on top of the same `git diff` machinery
/// `diff_changed_files` already establishes, not a full tree-sitter re-parse. Non-`.go`
/// files are never checked (Go-only v1 scope).
fn detect_package_clause_change(repo_root: &Path, base_sha: &str, path: &Path) -> Result<bool> {
    if !is_go_path(path) {
        return Ok(false);
    }
    let output = run_git(
        repo_root,
        &["diff", "-U0", base_sha, "--", &git_path_str(path)],
    )?;
    if !output.status.success() {
        bail!(
            "git diff -U0 exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut removed_pkg: Option<&str> = None;
    let mut added_pkg: Option<&str> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("-package ") {
            removed_pkg = Some(rest.trim());
        } else if let Some(rest) = line.strip_prefix("+package ") {
            added_pkg = Some(rest.trim());
        }
    }
    Ok(matches!((removed_pkg, added_pkg), (Some(a), Some(b)) if a != b))
}

/// Inverts `edges` (`from -> to`) into `to -> {from}` — the reverse adjacency
/// `reverse_bfs` walks. A `BTreeSet` value both dedupes multi-edge pairs (two import
/// statements between the same two packages) and gives deterministic iteration order.
fn build_reverse_adjacency(edges: &[ImportEdge]) -> BTreeMap<&str, BTreeSet<&str>> {
    let mut map: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for edge in edges {
        map.entry(edge.to.as_str())
            .or_default()
            .insert(edge.from.as_str());
    }
    map
}

/// Visited-set-guarded BFS from `seeds` over the reverse adjacency, returning the full
/// transitive closure of importers (not just direct ones). The seeds themselves are
/// always included in the result — a changed package is always "affected," even with no
/// importers at all. Terminates on a cyclic import graph (an explicit `visited` set is
/// carried from the first step, not bolted on after the fact).
fn reverse_bfs(
    adjacency: &BTreeMap<&str, BTreeSet<&str>>,
    seeds: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut visited: BTreeSet<String> = seeds.clone();
    let mut queue: VecDeque<String> = seeds.iter().cloned().collect();
    while let Some(node) = queue.pop_front() {
        if let Some(importers) = adjacency.get(node.as_str()) {
            for &importer in importers {
                if visited.insert(importer.to_string()) {
                    queue.push_back(importer.to_string());
                }
            }
        }
    }
    visited
}

/// Maps every `Added`/`Modified`/`Renamed` (new-side path) entry to its package key via
/// the working-tree `ArchModel`, silently skipping any entry with no `file_packages`
/// entry (unsupported extension, or excluded by `SKIP_DIRS` — e.g. `vendor/`). `Deleted`
/// entries and a `Renamed` entry's *old*-side path are handled separately by
/// [`resolve_removed_path`], called directly from `compute_affected`.
fn resolve_changed_packages(changed: &[ChangedFile], model: &ArchModel) -> BTreeSet<String> {
    changed
        .iter()
        .filter(|entry| !matches!(entry.status, ChangeStatus::Deleted))
        .filter_map(|entry| {
            model
                .package_for_file(&model.repo_root.join(&entry.path))
                .map(str::to_string)
        })
        .collect()
}

/// For a `Deleted` path or a `Renamed` entry's old-side path: does the package that used
/// to live at `old_path`'s directory still exist in the working-tree `ArchModel` (some
/// other file in that directory survives)? Returns `Some(key)` if so (added to the seed
/// set as a changed package — no bail-out, its continued existence is directly
/// observable); `None` if the directory has no surviving recognized-language files at
/// all (the package was fully removed — caller bails out per ADR-001).
fn resolve_removed_path(repo_root: &Path, old_path: &Path, model: &ArchModel) -> Option<String> {
    let abs = repo_root.join(old_path);
    let parent = abs.parent()?;
    model
        .packages
        .values()
        .find(|pkg| pkg.files.iter().any(|f| f.parent() == Some(parent)))
        .map(|pkg| pkg.path.clone())
}

/// First bail-out glob (from `globs`, e.g. `AffectedConfig::effective_bail_out_globs()`)
/// matched by any changed path, cheaply checked before any graph-walk work. First-match
/// wins — the specific matched glob is only for the debuggability message on stderr, not
/// decision logic.
fn matches_any_bail_out_glob(
    changed: &[ChangedFile],
    globs: &[String],
) -> Option<(String, PathBuf)> {
    for entry in changed {
        let path_str = git_path_str(&entry.path);
        for glob in globs {
            if crate::glob::matches_scope(&path_str, std::slice::from_ref(glob)) {
                return Some((glob.clone(), entry.path.clone()));
            }
        }
    }
    None
}

/// Orchestrates the whole `affected` computation: diff `base` against the working tree,
/// check for an unbounded-blast-radius bail-out, resolve the changed files to a seed
/// package set, and reverse-BFS the transitive closure of importers. See this module's
/// doc comment and `plan.md`'s Domain Glossary for the full contract; in short:
///
/// - An unresolvable `--base` (or `repo_root` not being a git repo at all) is a hard
///   `Err` — no diff was ever computed, so neither a package list nor `__ALL__` means
///   anything.
/// - A diff that *was* computed but can't safely be narrowed (an unbounded-blast-radius
///   glob match, a fully-removed package, a `package`-clause change without a rename, or
///   no common history with `base`) is `Ok(AffectedResult::BailOut(_))`.
/// - Otherwise, `Ok(AffectedResult::Packages(_))` — possibly empty, a valid, successful
///   answer distinct from a bail-out.
pub fn compute_affected(
    repo_root: &Path,
    base: &str,
    config: &AffectedConfig,
) -> Result<AffectedResult> {
    let repo_root = repo_toplevel(repo_root)?;
    let base_sha = resolve_base_sha(&repo_root, base)?;

    let Some(changed) = diff_changed_files(&repo_root, &base_sha)? else {
        return Ok(AffectedResult::BailOut(
            BailOutReason::ShallowCloneOrNoCommonHistory,
        ));
    };

    if changed.is_empty() {
        return Ok(AffectedResult::Packages(Vec::new()));
    }

    let globs = config.effective_bail_out_globs();
    if let Some((glob, path)) = matches_any_bail_out_glob(&changed, &globs) {
        return Ok(AffectedResult::BailOut(BailOutReason::GlobMatched {
            glob,
            path,
        }));
    }

    for entry in &changed {
        if entry.status == ChangeStatus::Modified
            && detect_package_clause_change(&repo_root, &base_sha, &entry.path)?
        {
            return Ok(AffectedResult::BailOut(
                BailOutReason::PackageClauseChanged {
                    path: entry.path.clone(),
                },
            ));
        }
    }

    // Go-only v1 (this module's doc comment, and requirements.md's explicit scope):
    // scope the `ArchModel` build to `.go` files only, not every file
    // `walk_and_collect_files` finds. Two reasons, not one: (1) a non-Go package key
    // (e.g. JS/TS's directory-path-shaped key from `import_graph.rs::dir_key`, which
    // is an *absolute* filesystem path, not a portable identifier) has no business in
    // `affected`'s stdout contract — a CI wrapper feeding it straight into
    // `go test $PKGS` would choke on a non-Go token; (2) parsing every JS/TS/Python/
    // Java/Kotlin/Rust file in a polyglot repo on every invocation is pure waste when
    // only the Go import graph is ever consulted.
    let all_files = crate::check::walk_and_collect_files(&repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;
    let go_files: Vec<PathBuf> =
        crate::import_graph::files_for(&all_files, crate::checker::Language::Go)
            .into_iter()
            .cloned()
            .collect();
    let model = arch_model::build_model_from_files(
        &repo_root,
        &go_files,
        &PruneConfig {
            include_private: true,
        },
    )?;

    let mut seeds = resolve_changed_packages(&changed, &model);

    for entry in &changed {
        let old_path = match &entry.status {
            ChangeStatus::Deleted => Some(entry.path.clone()),
            ChangeStatus::Renamed { old } => Some(old.clone()),
            ChangeStatus::Added | ChangeStatus::Modified => None,
        };
        // Go-only v1: a deleted/renamed-away non-`.go` file (a doc, a fixture, a
        // config file) was never part of a Go package in the first place — treating
        // it as a `PackageFullyRemoved` candidate would falsely bail out on, say, a
        // deleted markdown file in a Go-free docs directory (`resolve_removed_path`
        // finds no surviving `.go` file there because there never was one), turning
        // an everyday doc edit into an unnecessary `__ALL__`.
        let old_path = old_path.filter(|p| is_go_path(p));
        if let Some(old_path) = old_path {
            match resolve_removed_path(&repo_root, &old_path, &model) {
                Some(key) => {
                    seeds.insert(key);
                }
                None => {
                    return Ok(AffectedResult::BailOut(
                        BailOutReason::PackageFullyRemoved { path: old_path },
                    ));
                }
            }
        }
    }

    let adjacency = build_reverse_adjacency(&model.import_edges);
    let affected = reverse_bfs(&adjacency, &seeds);

    Ok(AffectedResult::Packages(affected.into_iter().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempGitRepo {
        dir: PathBuf,
    }

    impl TempGitRepo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "kibitzer-affected-test-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self::run(&dir, &["init", "-q", "-b", "main"]);
            Self::run(&dir, &["config", "user.email", "test@example.com"]);
            Self::run(&dir, &["config", "user.name", "test"]);
            Self { dir }
        }

        fn run(dir: &Path, args: &[&str]) -> std::process::Output {
            let output = Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        }

        fn write(&self, rel: &str, contents: &str) {
            let path = self.dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
        }

        fn commit_all(&self, msg: &str) {
            Self::run(&self.dir, &["add", "-A"]);
            Self::run(&self.dir, &["commit", "-q", "-m", msg]);
        }

        fn head_sha(&self) -> String {
            String::from_utf8_lossy(&Self::run(&self.dir, &["rev-parse", "HEAD"]).stdout)
                .trim()
                .to_string()
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn go_mod_fixture(repo: &TempGitRepo) {
        repo.write("go.mod", "module example.com/app\n\ngo 1.21\n");
    }

    // --- resolve_base_sha -----------------------------------------------------------

    #[test]
    fn resolve_base_sha_returns_sha_matching_git_rev_parse_when_ref_resolves() {
        let repo = TempGitRepo::new("resolve-ok");
        go_mod_fixture(&repo);
        repo.commit_all("init");
        let expected = repo.head_sha();

        let sha = resolve_base_sha(&repo.dir, "main").unwrap();

        assert_eq!(sha, expected);
    }

    #[test]
    fn resolve_base_sha_returns_err_naming_the_ref_when_ref_is_unresolvable() {
        let repo = TempGitRepo::new("resolve-err");
        go_mod_fixture(&repo);
        repo.commit_all("init");

        let err = resolve_base_sha(&repo.dir, "totally-not-a-ref").unwrap_err();

        assert!(err.to_string().contains("totally-not-a-ref"));
    }

    // --- diff_changed_files -----------------------------------------------------------

    #[test]
    fn diff_changed_files_detects_pure_rename_as_renamed_not_delete_add() {
        let repo = TempGitRepo::new("rename");
        go_mod_fixture(&repo);
        repo.write("a/foo.go", "package a\n\nfunc F() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        std::fs::create_dir_all(repo.dir.join("b")).unwrap();
        TempGitRepo::run(&repo.dir, &["mv", "a/foo.go", "b/foo.go"]);
        repo.commit_all("rename");

        let changed = diff_changed_files(&repo.dir, &base).unwrap().unwrap();

        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].path, PathBuf::from("b/foo.go"));
        assert_eq!(
            changed[0].status,
            ChangeStatus::Renamed {
                old: PathBuf::from("a/foo.go")
            }
        );
    }

    #[test]
    fn diff_changed_files_unions_committed_uncommitted_and_untracked_changes_without_duplicates() {
        let repo = TempGitRepo::new("union");
        go_mod_fixture(&repo);
        repo.write("x/x.go", "package x\n\nfunc X() {}\n");
        repo.write("y/y.go", "package y\n\nfunc Y() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write("x/x.go", "package x\n\nfunc X() { println(1) }\n");
        repo.commit_all("committed change");

        repo.write("y/y.go", "package y\n\nfunc Y() { println(1) }\n");
        repo.write("z/z.go", "package z\n\nfunc Z() {}\n");

        let changed = diff_changed_files(&repo.dir, &base).unwrap().unwrap();

        let paths: BTreeSet<PathBuf> = changed.iter().map(|c| c.path.clone()).collect();
        assert_eq!(
            paths,
            BTreeSet::from([
                PathBuf::from("x/x.go"),
                PathBuf::from("y/y.go"),
                PathBuf::from("z/z.go"),
            ])
        );
    }

    #[test]
    fn diff_changed_files_returns_none_when_base_shares_no_common_history() {
        let repo = TempGitRepo::new("shallow");
        go_mod_fixture(&repo);
        repo.commit_all("init");

        // An orphan branch shares no history with `main` — the same shape as a
        // shallow clone missing `base_sha`'s history.
        TempGitRepo::run(&repo.dir, &["checkout", "-q", "--orphan", "orphan"]);
        repo.write("orphan.go", "package orphan\n");
        repo.commit_all("orphan root");
        let orphan_sha = repo.head_sha();
        TempGitRepo::run(&repo.dir, &["checkout", "-q", "main"]);

        let result = diff_changed_files(&repo.dir, &orphan_sha).unwrap();

        assert!(result.is_none());
    }

    // --- build_reverse_adjacency / reverse_bfs ----------------------------------------

    fn edge(from: &str, to: &str) -> ImportEdge {
        ImportEdge {
            from: from.to_string(),
            to: to.to_string(),
            file: PathBuf::from("f.go"),
            line: 1,
        }
    }

    #[test]
    fn build_reverse_adjacency_groups_multiple_importers_under_one_key() {
        let edges = vec![edge("a", "b"), edge("c", "b")];
        let map = build_reverse_adjacency(&edges);
        assert_eq!(
            map.get("b").cloned().unwrap_or_default(),
            BTreeSet::from(["a", "c"])
        );
    }

    #[test]
    fn build_reverse_adjacency_omits_a_package_with_no_importers() {
        let edges = vec![edge("a", "b")];
        let map = build_reverse_adjacency(&edges);
        assert_eq!(map.get("a"), None);
    }

    #[test]
    fn reverse_bfs_terminates_on_a_two_package_import_cycle_and_visits_both_packages() {
        let edges = vec![edge("a", "b"), edge("b", "a")];
        let adjacency = build_reverse_adjacency(&edges);
        let result = reverse_bfs(&adjacency, &BTreeSet::from(["a".to_string()]));
        assert_eq!(result, BTreeSet::from(["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn reverse_bfs_returns_full_transitive_closure_for_a_linear_import_chain() {
        // a -> b -> c -> d
        let edges = vec![edge("a", "b"), edge("b", "c"), edge("c", "d")];
        let adjacency = build_reverse_adjacency(&edges);
        let result = reverse_bfs(&adjacency, &BTreeSet::from(["d".to_string()]));
        assert_eq!(
            result,
            BTreeSet::from(["a", "b", "c", "d"].map(String::from))
        );
    }

    #[test]
    fn reverse_bfs_returns_only_the_seed_when_seed_package_has_no_importers() {
        let edges = vec![edge("a", "b")];
        let adjacency = build_reverse_adjacency(&edges);
        let result = reverse_bfs(&adjacency, &BTreeSet::from(["a".to_string()]));
        assert_eq!(result, BTreeSet::from(["a".to_string()]));
    }

    // --- detect_package_clause_change -------------------------------------------------

    #[test]
    fn detect_package_clause_change_true_when_package_line_changes_uncommitted() {
        let repo = TempGitRepo::new("pkg-change");
        go_mod_fixture(&repo);
        repo.write("a/a.go", "package foo\n\nfunc F() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write("a/a.go", "package bar\n\nfunc F() {}\n");

        let changed = detect_package_clause_change(&repo.dir, &base, Path::new("a/a.go")).unwrap();
        assert!(changed);
    }

    #[test]
    fn detect_package_clause_change_false_for_an_ordinary_body_edit() {
        let repo = TempGitRepo::new("pkg-no-change");
        go_mod_fixture(&repo);
        repo.write("a/a.go", "package foo\n\nfunc F() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write("a/a.go", "package foo\n\nfunc F() { println(1) }\n");

        let changed = detect_package_clause_change(&repo.dir, &base, Path::new("a/a.go")).unwrap();
        assert!(!changed);
    }

    #[test]
    fn detect_package_clause_change_skipped_for_a_non_go_file() {
        let repo = TempGitRepo::new("pkg-non-go");
        go_mod_fixture(&repo);
        repo.write("README.md", "package unrelated\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write("README.md", "package something-else\n");

        let changed =
            detect_package_clause_change(&repo.dir, &base, Path::new("README.md")).unwrap();
        assert!(!changed);
    }

    // --- compute_affected: end-to-end orchestration -----------------------------------

    #[test]
    fn compute_affected_returns_packages_including_transitive_importer_of_the_changed_file() {
        let repo = TempGitRepo::new("e2e-happy");
        go_mod_fixture(&repo);
        repo.write("widget/widget.go", "package widget\n\nfunc Widget() {}\n");
        repo.write(
            "consumer/consumer.go",
            "package consumer\n\nimport \"example.com/app/widget\"\n\nfunc C() { widget.Widget() }\n",
        );
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write(
            "widget/widget.go",
            "package widget\n\nfunc Widget() { println(1) }\n",
        );
        repo.commit_all("edit widget");

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        match result {
            AffectedResult::Packages(pkgs) => {
                assert!(pkgs.contains(&"example.com/app/widget".to_string()));
                assert!(pkgs.contains(&"example.com/app/consumer".to_string()));
            }
            other => panic!("expected Packages, got {other:?}"),
        }
    }

    #[test]
    fn compute_affected_returns_empty_packages_for_a_clean_diff() {
        let repo = TempGitRepo::new("e2e-empty");
        go_mod_fixture(&repo);
        repo.write("a/a.go", "package a\n\nfunc A() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        assert_eq!(result, AffectedResult::Packages(Vec::new()));
    }

    #[test]
    fn compute_affected_returns_glob_matched_bail_out_when_go_mod_changes() {
        let repo = TempGitRepo::new("e2e-glob");
        go_mod_fixture(&repo);
        repo.write("a/a.go", "package a\n\nfunc A() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write("go.mod", "module example.com/app\n\ngo 1.22\n");
        repo.write("a/a.go", "package a\n\nfunc A() { println(1) }\n");
        repo.commit_all("bump go.mod");

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        assert!(matches!(
            result,
            AffectedResult::BailOut(BailOutReason::GlobMatched { .. })
        ));
    }

    #[test]
    fn compute_affected_returns_package_fully_removed_bail_out_when_last_file_in_a_package_is_deleted()
     {
        let repo = TempGitRepo::new("e2e-removed");
        go_mod_fixture(&repo);
        repo.write("a/only.go", "package a\n\nfunc A() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        std::fs::remove_file(repo.dir.join("a/only.go")).unwrap();
        repo.commit_all("delete a");

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        assert_eq!(
            result,
            AffectedResult::BailOut(BailOutReason::PackageFullyRemoved {
                path: PathBuf::from("a/only.go")
            })
        );
    }

    #[test]
    fn compute_affected_does_not_bail_out_when_a_deleted_file_leaves_a_surviving_sibling_in_its_package()
     {
        let repo = TempGitRepo::new("e2e-sibling-survives");
        go_mod_fixture(&repo);
        repo.write("a/one_of_two.go", "package a\n\nfunc One() {}\n");
        repo.write("a/other.go", "package a\n\nfunc Other() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        std::fs::remove_file(repo.dir.join("a/one_of_two.go")).unwrap();
        repo.commit_all("delete one");

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        match result {
            AffectedResult::Packages(pkgs) => {
                assert!(pkgs.contains(&"example.com/app/a".to_string()));
            }
            other => panic!("expected Packages, got {other:?}"),
        }
    }

    /// Regression test for a real bug caught during Task 4.1.3's real-world validation
    /// against stapler-squad: deleting a Markdown file in a Go-free docs directory
    /// (`docs/bugs/open/BUG-086-....md`) was wrongly resolved as `PackageFullyRemoved`,
    /// because `resolve_removed_path` found no surviving `.go` file at that path — there
    /// never was one, since the deleted file itself was never Go source. Go-only v1
    /// scope means a non-`.go` delete/rename must never even reach the fully-removed
    /// check.
    #[test]
    fn compute_affected_does_not_bail_out_when_a_non_go_file_is_deleted_from_a_go_free_directory() {
        let repo = TempGitRepo::new("e2e-non-go-delete");
        go_mod_fixture(&repo);
        repo.write("a/a.go", "package a\n\nfunc A() {}\n");
        repo.write("docs/bugs/open/BUG-086.md", "# a bug\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        std::fs::remove_file(repo.dir.join("docs/bugs/open/BUG-086.md")).unwrap();
        repo.commit_all("close bug");

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        assert_eq!(result, AffectedResult::Packages(Vec::new()));
    }

    #[test]
    fn compute_affected_returns_package_clause_changed_bail_out_for_a_modified_file_without_git_mv()
    {
        let repo = TempGitRepo::new("e2e-pkg-clause");
        go_mod_fixture(&repo);
        repo.write("a/a.go", "package foo\n\nfunc F() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write("a/a.go", "package bar\n\nfunc F() {}\n");
        repo.commit_all("silent package rename");

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        assert_eq!(
            result,
            AffectedResult::BailOut(BailOutReason::PackageClauseChanged {
                path: PathBuf::from("a/a.go")
            })
        );
    }

    #[test]
    fn compute_affected_returns_shallow_clone_bail_out_when_base_has_no_common_history() {
        let repo = TempGitRepo::new("e2e-shallow");
        go_mod_fixture(&repo);
        repo.commit_all("init");

        TempGitRepo::run(&repo.dir, &["checkout", "-q", "--orphan", "orphan"]);
        repo.write("orphan.go", "package orphan\n");
        repo.commit_all("orphan root");
        let orphan_sha = repo.head_sha();
        TempGitRepo::run(&repo.dir, &["checkout", "-q", "main"]);

        let result = compute_affected(&repo.dir, &orphan_sha, &AffectedConfig::default()).unwrap();

        assert_eq!(
            result,
            AffectedResult::BailOut(BailOutReason::ShallowCloneOrNoCommonHistory)
        );
    }

    #[test]
    fn compute_affected_returns_err_not_bail_out_for_an_unresolvable_base_ref() {
        let repo = TempGitRepo::new("e2e-bad-ref");
        go_mod_fixture(&repo);
        repo.commit_all("init");

        let result = compute_affected(&repo.dir, "does-not-exist-ref", &AffectedConfig::default());

        assert!(result.is_err());
    }

    #[test]
    fn compute_affected_returns_err_when_repo_root_is_not_a_git_repository() {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-affected-not-a-repo-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let result = compute_affected(&dir, "main", &AffectedConfig::default());

        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_err());
    }

    /// Task 4.1.3a: a deliberately constructed multi-`go.mod` fixture mirroring
    /// stapler-squad's real layout (a root module plus `tuitest/`, `tools/scanner/`,
    /// `tools/lint/` nested modules), with the root module's `go.mod` `replace`-ing
    /// `tools/scanner` to a local path — the previously-untested gap the Tech Debt
    /// Disposition flagged (`import_graph.rs`'s `find_go_mod_upward` has no
    /// `replace`-directive parsing).
    ///
    /// Finding: `affected` resolves this correctly. `find_go_mod_upward` never reads
    /// `replace` directives at all — but it doesn't need to here, since kibitzer's
    /// import-edge resolution matches purely on the *literal import path string*
    /// against every package key discovered while walking the repo, independent of
    /// whether a `replace` directive produced that import path. As long as (as here,
    /// and as in stapler-squad's own real `tools/scanner`/`tools/lint` modules) the
    /// `replace`d module's own `go.mod` declares the same module path the importer
    /// writes in its `import` statement, the edge resolves and `reverse_bfs` finds the
    /// root module's dependent package. The gap would only bite for a `replace`
    /// pointing an import path at a module whose own declared `module` name *differs*
    /// from the import path written at the call site (an unusual, not-recommended Go
    /// module-graph shape) — not exercised here; recorded as still-accepted risk in
    /// `docs/affected-validation.md`.
    #[test]
    fn compute_affected_multi_module_fixture_with_replace_directive_finds_the_dependent_package() {
        let repo = TempGitRepo::new("multi-module");
        repo.write(
            "go.mod",
            "module example.com/app\n\ngo 1.21\n\nreplace example.com/app/tools/scanner => ./tools/scanner\n",
        );
        repo.write(
            "main.go",
            "package main\n\nimport \"example.com/app/tools/scanner\"\n\nfunc main() { scanner.Run() }\n",
        );
        repo.write(
            "tuitest/go.mod",
            "module example.com/app/tuitest\n\ngo 1.21\n",
        );
        repo.write("tuitest/tuitest.go", "package tuitest\n\nfunc Run() {}\n");
        repo.write(
            "tools/scanner/go.mod",
            "module example.com/app/tools/scanner\n\ngo 1.21\n",
        );
        repo.write(
            "tools/scanner/scanner.go",
            "package scanner\n\nfunc Run() {}\n",
        );
        repo.write(
            "tools/lint/go.mod",
            "module example.com/app/tools/lint\n\ngo 1.21\n",
        );
        repo.write("tools/lint/lint.go", "package lint\n\nfunc Run() {}\n");
        repo.commit_all("init");
        let base = repo.head_sha();

        repo.write(
            "tools/scanner/scanner.go",
            "package scanner\n\nfunc Run() { println(1) }\n",
        );
        repo.commit_all("edit scanner");

        let result = compute_affected(&repo.dir, &base, &AffectedConfig::default()).unwrap();

        match result {
            AffectedResult::Packages(pkgs) => {
                assert!(pkgs.contains(&"example.com/app/tools/scanner".to_string()));
                assert!(
                    pkgs.contains(&"example.com/app".to_string()),
                    "expected the root module's dependent package to be found via the \
                     replace-directive import, got: {pkgs:?}"
                );
            }
            other => panic!("expected Packages, got {other:?}"),
        }
    }
}
