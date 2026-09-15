//! Multi-bug root-cause clustering (#38's "bigger feature," extending the `reflect-and-fix`
//! Claude Code skill): groups bug-fix commits (`change_coupling::is_bug_fix_commit`) that
//! touch overlapping files, computes their temporal-coupling strength via
//! `change_coupling::compute_coupling_unfiltered` restricted to that subset, and
//! cross-references each cluster's shared files against kibitzer's own static findings
//! (import-cycles/layering/coupling/component-deps/instability/dip-concrete-coupling) for
//! corroboration.
//!
//! Deterministic — no ML, no commit-message semantic analysis beyond
//! `is_bug_fix_commit`'s conventional-commit/issue-reference heuristic, no clustering
//! beyond connected-components over a Jaccard-overlap graph. Output is an "evidence
//! packet" per cluster (commits, shared files, coupling stats, corroborating finding if
//! any) — input for a human or an LLM to name the shared root cause and decide whether to
//! act on it, never asserted as fact here: a cluster with co-change but no static
//! corroboration could just as well be a shared test fixture or one engineer's habitual
//! pairing, not a real architectural defect.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};

use crate::arch_model::{self, ArchModel, PruneConfig};
use crate::architecture_checks::{self, ArchFinding};
use crate::change_coupling::{self, CoupledPair, TaggedCommit};
use crate::config::ArchitectureConfig;

/// Minimum Jaccard overlap of touched files between two bug-fix commits before they're
/// considered connected for clustering. No stronger literature citation than "meaningfully
/// overlapping," same status as this codebase's other threshold constants.
const MIN_COMMIT_FILE_JACCARD: f64 = 0.3;

/// A cluster's shared files (files touched by 2+ of its commits) — the ones actually
/// implicated in the co-change pattern, since not every file in every commit needs to be
/// shared for commits to cluster together transitively (see [`cluster_bug_fix_commits`]).
const MIN_SHARERS_FOR_SHARED_FILE: usize = 2;

/// One cluster's evidence packet.
#[derive(Debug, Clone, PartialEq)]
pub struct RootCauseCluster {
    pub commit_shas: Vec<String>,
    pub commit_subjects: Vec<String>,
    /// Files touched by 2+ of this cluster's commits, sorted.
    pub shared_files: Vec<String>,
    /// Temporal coupling among `shared_files` and any other file these commits touched,
    /// computed via `change_coupling::compute_coupling_unfiltered` restricted to this
    /// cluster's own commits.
    pub coupled_pairs: Vec<CoupledPair>,
    /// `Some(description)` when an existing static architecture finding also implicates
    /// one of `shared_files` (or, for a package-level finding with no single file, a
    /// package that owns one) — stronger evidence this is a real architectural defect, not
    /// just co-change. `None` means co-change only.
    pub corroborating_finding: Option<String>,
}

fn jaccard(a: &HashSet<&str>, b: &HashSet<&str>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        a.intersection(b).count() as f64 / union as f64
    }
}

fn union_find_root(parent: &mut [usize], x: usize) -> usize {
    if parent[x] != x {
        parent[x] = union_find_root(parent, parent[x]);
    }
    parent[x]
}

fn union_find_join(parent: &mut [usize], a: usize, b: usize) {
    let ra = union_find_root(parent, a);
    let rb = union_find_root(parent, b);
    if ra != rb {
        parent[ra] = rb;
    }
}

/// Groups `commits` (already filtered to bug fixes) into connected components by
/// [`MIN_COMMIT_FILE_JACCARD`]-overlapping touched-file sets — transitive, so commit A and
/// commit C can end up in the same cluster via a shared intermediate commit B even if A
/// and C themselves share nothing. Returns each cluster as a list of indices into
/// `commits`, clusters with a single member dropped (a "cluster" of one commit has no
/// co-change pattern to report).
fn cluster_bug_fix_commits(commits: &[TaggedCommit]) -> Vec<Vec<usize>> {
    let file_sets: Vec<HashSet<&str>> = commits
        .iter()
        .map(|c| c.files.iter().map(String::as_str).collect())
        .collect();
    let n = commits.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for i in 0..n {
        for j in (i + 1)..n {
            if jaccard(&file_sets[i], &file_sets[j]) >= MIN_COMMIT_FILE_JACCARD {
                union_find_join(&mut parent, i, j);
            }
        }
    }

    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        by_root
            .entry(union_find_root(&mut parent, i))
            .or_default()
            .push(i);
    }
    by_root
        .into_values()
        .filter(|members| members.len() > 1)
        .collect()
}

/// Files touched by at least [`MIN_SHARERS_FOR_SHARED_FILE`] of `members`' commits.
fn shared_files_of<'a>(members: &[usize], commits: &'a [TaggedCommit]) -> Vec<&'a str> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for &i in members {
        let mut seen = HashSet::new();
        for f in &commits[i].files {
            if seen.insert(f.as_str()) {
                *counts.entry(f.as_str()).or_default() += 1;
            }
        }
    }
    let mut files: Vec<&str> = counts
        .into_iter()
        .filter(|(_, n)| *n >= MIN_SHARERS_FOR_SHARED_FILE)
        .map(|(f, _)| f)
        .collect();
    files.sort_unstable();
    files
}

/// Every architecture finding kibitzer's own checkers produce against the repo at
/// `repo_root` right now, tagged with the checker name that produced it (`ArchFinding`
/// itself carries no checker-name field). Runs every `ArchitectureChecker`/
/// `ArchModelChecker` with default config, same as `architecture_assessment`'s whole-repo
/// survey — this function exists so `corroborate` has something to cross-reference against
/// without requiring the caller to already have an `ArchModel`/`ImportGraph` in hand.
fn current_findings(repo_root: &Path) -> Result<(ArchModel, Vec<(&'static str, ArchFinding)>)> {
    let (graph, files) = arch_model::collect_repo_files(repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;
    let model = arch_model::build_model(repo_root, &files, &graph, &PruneConfig::default())
        .with_context(|| format!("building architecture model for {}", repo_root.display()))?;
    let config = ArchitectureConfig::default();

    let mut findings: Vec<(&'static str, ArchFinding)> = Vec::new();
    for checker in architecture_checks::registry() {
        for finding in checker.check(&graph, &config) {
            findings.push((checker_static_name(checker.name()), finding));
        }
    }
    for checker in architecture_checks::model_registry() {
        for finding in checker.check(&model, &config) {
            findings.push((checker_static_name(checker.name()), finding));
        }
    }
    Ok((model, findings))
}

/// `ArchitectureChecker::name`/`ArchModelChecker::name` return `&str` borrowed from the
/// boxed checker (dropped at the end of the loop it's called in), but every real
/// implementation actually returns a `'static` string literal — this leaks nothing new,
/// it just re-asserts that fact so the name can outlive the checker box.
fn checker_static_name(name: &str) -> &'static str {
    match name {
        "import-cycles" => "import-cycles",
        "layering" => "layering",
        "coupling" => "coupling",
        "component-deps" => "component-deps",
        "package-size" => "package-size",
        "instability" => "instability",
        "dip-concrete-coupling" => "dip-concrete-coupling",
        "lcom" => "lcom",
        _ => "unknown-checker",
    }
}

/// Repo-root-relative path (forward-slash-normalized, matching git's own path format) for
/// every file in every package of `model` — the lookup `corroborate` uses to map a
/// package-level finding's package path back to the files it owns.
fn files_by_package(model: &ArchModel) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pkg in model.packages.values() {
        for file in &pkg.files {
            let rel = file
                .strip_prefix(&model.repo_root)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            map.insert(rel, pkg.path.clone());
        }
    }
    map
}

/// Cross-references `shared_files` against `findings`: a finding with a `file` matching
/// one of `shared_files` exactly corroborates directly; a package-level finding (`file:
/// None` — `instability`/`dip-concrete-coupling`/most `coupling`/`component-deps`
/// findings) corroborates if its message mentions the package that owns one of
/// `shared_files` (per `file_packages`) — messages always embed the package path
/// literally, so a substring match is a legitimate, if imperfect, cross-reference; a
/// coincidental substring match is possible but unlikely given package paths are
/// multi-segment. Returns the first match found, in `findings`' order — not the
/// "best" or most specific one, since there's no principled way to rank them here.
/// `ArchFinding.message` is already bracket-tagged with its checker name by this
/// codebase's own convention (`[import-cycles] ...`, `[instability] ...`), so it's
/// returned as-is rather than tagged a second time — `checker` (the `&'static str` this
/// finding was collected under) is only needed to make that convention explicit here, not
/// to build a new prefix.
fn corroborate(
    shared_files: &[&str],
    file_packages: &HashMap<String, String>,
    findings: &[(&'static str, ArchFinding)],
) -> Option<String> {
    let shared: HashSet<&str> = shared_files.iter().copied().collect();
    let packages: HashSet<&str> = shared_files
        .iter()
        .filter_map(|f| file_packages.get(*f).map(String::as_str))
        .collect();

    findings.iter().find_map(|(_checker, finding)| {
        let file_match = finding
            .file
            .as_ref()
            .is_some_and(|f| shared.contains(f.to_string_lossy().replace('\\', "/").as_str()));
        let package_match =
            finding.file.is_none() && packages.iter().any(|pkg| finding.message.contains(pkg));
        (file_match || package_match).then(|| finding.message.clone())
    })
}

/// Builds one cluster's evidence packet from its member commit indices.
fn evidence_packet(
    members: &[usize],
    bug_fixes: &[TaggedCommit],
    file_packages: &HashMap<String, String>,
    findings: &[(&'static str, ArchFinding)],
) -> RootCauseCluster {
    let shared_files = shared_files_of(members, bug_fixes);
    let file_lists: Vec<Vec<String>> = members
        .iter()
        .map(|&i| bug_fixes[i].files.clone())
        .collect();
    let coupled_pairs = change_coupling::compute_coupling_unfiltered(&file_lists);
    let corroborating_finding = corroborate(&shared_files, file_packages, findings);

    RootCauseCluster {
        commit_shas: members.iter().map(|&i| bug_fixes[i].sha.clone()).collect(),
        commit_subjects: members
            .iter()
            .map(|&i| bug_fixes[i].subject.clone())
            .collect(),
        shared_files: shared_files.into_iter().map(str::to_string).collect(),
        coupled_pairs,
        corroborating_finding,
    }
}

/// Runs the full root-cause-clustering recipe over `repo_root`'s last `limit` non-merge
/// commits: tags bug fixes, clusters them by touched-file overlap, computes each
/// cluster's own temporal coupling, and cross-references against current static findings.
/// Clusters are returned sorted by shared-file count, descending (more shared files is a
/// stronger co-change signal), ties broken by first commit sha for determinism.
pub fn analyze(repo_root: &Path, limit: usize) -> Result<Vec<RootCauseCluster>> {
    let commits = change_coupling::git_log_commits_with_subject(repo_root, limit)
        .with_context(|| format!("reading git log for {}", repo_root.display()))?;
    let bug_fixes: Vec<TaggedCommit> = commits
        .into_iter()
        .filter(|c| change_coupling::is_bug_fix_commit(&c.subject))
        .collect();

    let clusters = cluster_bug_fix_commits(&bug_fixes);
    if clusters.is_empty() {
        return Ok(Vec::new());
    }

    let (model, findings) = current_findings(repo_root)?;
    let file_packages = files_by_package(&model);

    let mut out: Vec<RootCauseCluster> = clusters
        .iter()
        .map(|members| evidence_packet(members, &bug_fixes, &file_packages, &findings))
        .collect();

    out.sort_by(|a, b| {
        b.shared_files
            .len()
            .cmp(&a.shared_files.len())
            .then_with(|| a.commit_shas.first().cmp(&b.commit_shas.first()))
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn commit(sha: &str, subject: &str, files: &[&str]) -> TaggedCommit {
        TaggedCommit {
            sha: sha.to_string(),
            subject: subject.to_string(),
            files: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn clusters_bug_fix_commits_that_share_files() {
        let commits = vec![
            commit("1", "fix: a", &["a.rs", "b.rs"]),
            commit("2", "fix: b", &["a.rs", "b.rs"]),
            commit("3", "fix: c", &["z.rs"]),
        ];
        let clusters = cluster_bug_fix_commits(&commits);
        assert_eq!(clusters.len(), 1, "got: {clusters:?}");
        let mut members = clusters[0].clone();
        members.sort_unstable();
        assert_eq!(members, vec![0, 1]);
    }

    #[test]
    fn drops_singleton_clusters() {
        let commits = vec![
            commit("1", "fix: a", &["a.rs"]),
            commit("2", "fix: b", &["z.rs"]),
        ];
        assert!(cluster_bug_fix_commits(&commits).is_empty());
    }

    #[test]
    fn shared_files_of_requires_at_least_two_sharers() {
        let commits = vec![
            commit("1", "fix: a", &["a.rs", "only-in-1.rs"]),
            commit("2", "fix: b", &["a.rs", "only-in-2.rs"]),
        ];
        let shared = shared_files_of(&[0, 1], &commits);
        assert_eq!(shared, vec!["a.rs"]);
    }

    #[test]
    fn corroborate_matches_a_file_level_finding() {
        // The message is already bracket-tagged by this codebase's own convention —
        // corroborate must return it verbatim, not tag it a second time.
        let finding = ArchFinding {
            file: Some(PathBuf::from("a.rs")),
            line: Some(3),
            message: "[import-cycles] cycle involves a.rs".to_string(),
            severity_override: None,
        };
        let findings = vec![("import-cycles", finding)];
        let result = corroborate(&["a.rs"], &HashMap::new(), &findings);
        assert_eq!(
            result,
            Some("[import-cycles] cycle involves a.rs".to_string())
        );
    }

    #[test]
    fn corroborate_matches_a_package_level_finding_via_message_substring() {
        let finding = ArchFinding {
            file: None,
            line: None,
            message: "[instability] pkg/domain is in the zone of pain".to_string(),
            severity_override: None,
        };
        let findings = vec![("instability", finding)];
        let mut file_packages = HashMap::new();
        file_packages.insert("pkg/domain/a.go".to_string(), "pkg/domain".to_string());

        let result = corroborate(&["pkg/domain/a.go"], &file_packages, &findings);
        assert!(result.is_some(), "got: {result:?}");
    }

    #[test]
    fn corroborate_returns_none_when_nothing_matches() {
        let finding = ArchFinding {
            file: Some(PathBuf::from("unrelated.rs")),
            line: None,
            message: "cycle involves unrelated.rs".to_string(),
            severity_override: None,
        };
        let findings = vec![("import-cycles", finding)];
        assert_eq!(corroborate(&["a.rs"], &HashMap::new(), &findings), None);
    }
}
