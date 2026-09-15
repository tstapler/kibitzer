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
use crate::jaccard::jaccard;

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
                crate::union_find::union(&mut parent, i, j);
            }
        }
    }

    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        by_root
            .entry(crate::union_find::root(&mut parent, i))
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
/// `repo_root` right now (`ArchFinding.message` already carries its own `[checker-name]`
/// tag by this codebase's convention, so nothing further tags them here). Runs every
/// `ArchitectureChecker`/`ArchModelChecker` with default config, same as
/// `architecture_assessment`'s whole-repo survey — this function exists so `corroborate`
/// has something to cross-reference against without requiring the caller to already have
/// an `ArchModel`/`ImportGraph` in hand.
fn current_findings(repo_root: &Path) -> Result<(ArchModel, Vec<ArchFinding>)> {
    let (graph, files) = arch_model::collect_repo_files(repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;
    let model = arch_model::build_model(repo_root, &files, &graph, &PruneConfig::default())
        .with_context(|| format!("building architecture model for {}", repo_root.display()))?;
    let config = ArchitectureConfig::default();

    let mut findings: Vec<ArchFinding> = Vec::new();
    for checker in architecture_checks::registry() {
        findings.extend(checker.check(&graph, &config));
    }
    for checker in architecture_checks::model_registry() {
        findings.extend(checker.check(&model, &config));
    }
    Ok((model, findings))
}

/// Repo-root-relative, forward-slash-normalized form of `path` — matches both git log's
/// path format and `shared_files`'s. `path` may be absolute or relative depending on how
/// the caller's `repo_root` was given to `build_model` (e.g. `.` walked into
/// `./src/foo.go`, or an absolute temp-dir path walked into
/// `/tmp/.../src/foo.go`) — `strip_prefix` handles both; falls back to `path` unchanged if
/// it isn't actually under `repo_root` (shouldn't happen for a finding produced from this
/// same model, but avoids silently dropping it if it ever does).
///
/// `corroborate`'s file-match branch needs this: comparing an un-normalized `finding.file`
/// directly against `shared_files` (bare git-relative paths) only coincides by accident,
/// since `ArchFinding.file` carries whatever path form the model was built with.
fn normalize_repo_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// `path`-relative form of every file in every package of `model` — the lookup
/// `corroborate` uses to map a package-level finding's package path back to the files it
/// owns.
fn files_by_package(model: &ArchModel) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pkg in model.packages.values() {
        for file in &pkg.files {
            map.insert(
                normalize_repo_path(&model.repo_root, file),
                pkg.path.clone(),
            );
        }
    }
    map
}

/// Characters that continue a path/identifier segment — used by [`package_mentioned_in`]
/// to tell a real package-boundary match from a partial-segment substring collision.
fn is_segment_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-'
}

/// Whether `pkg` appears in `message` as a whole path-segment sequence, not merely as a
/// substring — a bare `message.contains(pkg)` would let `"internal/db"` match inside
/// `"internal/dbmigrate"`. A match counts only if the character immediately before and
/// after it (when present) isn't itself a segment-continuation character
/// ([`is_segment_char`]); a `/`, `.`, `:`, whitespace, punctuation, or start/end of string
/// all count as a real boundary.
fn package_mentioned_in(message: &str, pkg: &str) -> bool {
    if pkg.is_empty() {
        return false;
    }
    message.match_indices(pkg).any(|(start, matched)| {
        let end = start + matched.len();
        let before_ok = message[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_segment_char(c));
        let after_ok = message[end..]
            .chars()
            .next()
            .is_none_or(|c| !is_segment_char(c));
        before_ok && after_ok
    })
}

/// Cross-references `shared_files` against `findings`: matches by file path
/// ([`normalize_repo_path`]) directly, or by package-name mention ([`package_mentioned_in`])
/// for a package-level finding (`file: None`). Returns the first match in `findings`'
/// order — not the "best" or most specific one, since there's no principled way to rank
/// them here.
fn corroborate(
    repo_root: &Path,
    shared_files: &[&str],
    file_packages: &HashMap<String, String>,
    findings: &[ArchFinding],
) -> Option<String> {
    let shared: HashSet<&str> = shared_files.iter().copied().collect();
    let packages: HashSet<&str> = shared_files
        .iter()
        .filter_map(|f| file_packages.get(*f).map(String::as_str))
        .collect();

    findings.iter().find_map(|finding| {
        let file_match = finding
            .file
            .as_ref()
            .is_some_and(|f| shared.contains(normalize_repo_path(repo_root, f).as_str()));
        let package_match = finding.file.is_none()
            && packages
                .iter()
                .any(|pkg| package_mentioned_in(&finding.message, pkg));
        (file_match || package_match).then(|| finding.message.clone())
    })
}

/// Builds one cluster's evidence packet from its member commit indices.
fn evidence_packet(
    repo_root: &Path,
    members: &[usize],
    bug_fixes: &[TaggedCommit],
    file_packages: &HashMap<String, String>,
    findings: &[ArchFinding],
) -> RootCauseCluster {
    let shared_files = shared_files_of(members, bug_fixes);
    let file_lists: Vec<Vec<String>> = members
        .iter()
        .map(|&i| bug_fixes[i].files.clone())
        .collect();
    let coupled_pairs = change_coupling::compute_coupling_unfiltered(&file_lists);
    let corroborating_finding = corroborate(repo_root, &shared_files, file_packages, findings);

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
/// Clusters are sorted by shared-file count, descending (more shared files is a stronger
/// co-change signal), ties broken by first commit sha for determinism, and truncated to
/// the top `top_n` — the same `--top` convention `change_coupling::analyze` uses.
pub fn analyze(repo_root: &Path, limit: usize, top_n: usize) -> Result<Vec<RootCauseCluster>> {
    let commits = change_coupling::git_log_commits_with_subject(repo_root, limit)
        .with_context(|| format!("reading git log for {}", repo_root.display()))?;
    let bug_fixes: Vec<TaggedCommit> = commits
        .into_iter()
        .filter(|c| change_coupling::is_bug_fix_commit(&c.subject, &c.body))
        .collect();

    let clusters = cluster_bug_fix_commits(&bug_fixes);
    if clusters.is_empty() {
        return Ok(Vec::new());
    }

    let (model, findings) = current_findings(repo_root)?;
    let file_packages = files_by_package(&model);

    let mut out: Vec<RootCauseCluster> = clusters
        .iter()
        .map(|members| evidence_packet(repo_root, members, &bug_fixes, &file_packages, &findings))
        .collect();

    out.sort_by(|a, b| {
        b.shared_files
            .len()
            .cmp(&a.shared_files.len())
            .then_with(|| a.commit_shas.first().cmp(&b.commit_shas.first()))
    });
    out.truncate(top_n);
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
            body: String::new(),
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
        let findings = vec![finding];
        let result = corroborate(Path::new(""), &["a.rs"], &HashMap::new(), &findings);
        assert_eq!(
            result,
            Some("[import-cycles] cycle involves a.rs".to_string())
        );
    }

    /// Regression: `finding.file` isn't a bare git-relative path in practice — it's
    /// whatever form `repo_root` was walked from (absolute repo_root -> absolute
    /// `finding.file`; `.` -> `./...`). `corroborate` used to compare it against
    /// `shared_files` (always bare git-relative) with no normalization, so this case could
    /// never match before the fix — see [`normalize_repo_path`].
    #[test]
    fn corroborate_matches_a_file_level_finding_whose_path_needs_repo_root_stripped() {
        let finding = ArchFinding {
            file: Some(PathBuf::from("/repo/pkg/a.rs")),
            line: Some(3),
            message: "[import-cycles] cycle involves pkg/a.rs".to_string(),
            severity_override: None,
        };
        let findings = vec![finding];
        let result = corroborate(
            Path::new("/repo"),
            &["pkg/a.rs"],
            &HashMap::new(),
            &findings,
        );
        assert!(result.is_some(), "got: {result:?}");
    }

    #[test]
    fn corroborate_matches_a_package_level_finding_via_message_substring() {
        let finding = ArchFinding {
            file: None,
            line: None,
            message: "[instability] pkg/domain is in the zone of pain".to_string(),
            severity_override: None,
        };
        let findings = vec![finding];
        let mut file_packages = HashMap::new();
        file_packages.insert("pkg/domain/a.go".to_string(), "pkg/domain".to_string());

        let result = corroborate(
            Path::new(""),
            &["pkg/domain/a.go"],
            &file_packages,
            &findings,
        );
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
        let findings = vec![finding];
        assert_eq!(
            corroborate(Path::new(""), &["a.rs"], &HashMap::new(), &findings),
            None
        );
    }

    // --- package_mentioned_in (#2: unanchored substring match) ---

    #[test]
    fn package_mentioned_in_does_not_match_a_longer_sibling_package() {
        // "internal/db" must not match inside "internal/dbmigrate".
        assert!(!package_mentioned_in(
            "[instability] internal/dbmigrate is in the zone of pain",
            "internal/db"
        ));
    }

    #[test]
    fn package_mentioned_in_matches_at_a_real_boundary() {
        assert!(package_mentioned_in(
            "[instability] internal/db is in the zone of pain",
            "internal/db"
        ));
        assert!(package_mentioned_in(
            "kibitzer::checker imports kibitzer::comment_quality directly",
            "kibitzer::comment_quality"
        ));
    }

    /// End-to-end regression for the blocker: a real `ImportCycleChecker` violation (real
    /// `ArchModel`, walked from an absolute temp-repo path — the actual path shape the CLI
    /// produces, not an idealized relative string) plus a real bug-fix-commit cluster
    /// touching the cycle's file must corroborate.
    #[test]
    fn analyze_corroborates_a_real_import_cycle_against_a_real_git_repo() {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-root-cause-clusters-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("domain")).unwrap();
        std::fs::create_dir_all(dir.join("handlers")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(
            dir.join("domain/domain.go"),
            "package domain\n\nimport \"fixture/handlers\"\n\nfunc Do() { handlers.Do() }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("handlers/handlers.go"),
            "package handlers\n\nimport \"fixture/domain\"\n\nfunc Do() { domain.Do() }\n",
        )
        .unwrap();

        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "init"]);
        for i in 0..2 {
            std::fs::write(dir.join("domain/domain.go"), format!("// v{i}\npackage domain\n\nimport \"fixture/handlers\"\n\nfunc Do() {{ handlers.Do() }}\n")).unwrap();
            std::fs::write(dir.join("handlers/handlers.go"), format!("// v{i}\npackage handlers\n\nimport \"fixture/domain\"\n\nfunc Do() {{ domain.Do() }}\n")).unwrap();
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", &format!("fix: bug {i}")]);
        }

        let clusters = analyze(&dir, 1000, 20).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(clusters.len(), 1, "got: {clusters:?}");
        let corroboration = clusters[0]
            .corroborating_finding
            .as_deref()
            .unwrap_or_else(|| panic!("expected corroboration, got: {:?}", clusters[0]));
        assert!(
            corroboration.contains("import-cycle"),
            "got: {corroboration}"
        );
    }

    #[test]
    fn analyze_truncates_to_top_n() {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-root-cause-clusters-topn-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();

        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);

        // Cluster 1: 3 shared files across its two commits.
        for i in 0..2 {
            std::fs::write(dir.join("a1.go"), format!("// v{i}\npackage a\n")).unwrap();
            std::fs::write(dir.join("a2.go"), format!("// v{i}\npackage a\n")).unwrap();
            std::fs::write(dir.join("a3.go"), format!("// v{i}\npackage a\n")).unwrap();
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", &format!("fix: cluster a {i}")]);
        }
        // Cluster 2: 1 shared file across its two commits — fewer shared files, so
        // top_n=1 must keep cluster 1 and drop this one.
        for i in 0..2 {
            std::fs::write(dir.join("b1.go"), format!("// v{i}\npackage b\n")).unwrap();
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", &format!("fix: cluster b {i}")]);
        }

        let all = analyze(&dir, 1000, 20).unwrap();
        assert_eq!(all.len(), 2, "got: {all:?}");

        let top_one = analyze(&dir, 1000, 1).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(top_one.len(), 1, "got: {top_one:?}");
        assert_eq!(top_one[0].shared_files.len(), 3, "got: {top_one:?}");
    }

    #[test]
    fn analyze_returns_empty_ok_when_there_are_zero_bug_fix_commits() {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-root-cause-clusters-no-bug-fixes-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();

        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);
        std::fs::write(dir.join("a.go"), "package a\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "feat: add a thing"]);

        let result = analyze(&dir, 1000, 20);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(result.unwrap(), Vec::new());
    }

    #[test]
    fn tied_clusters_sort_deterministically_by_first_commit_sha() {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-root-cause-clusters-tie-break-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();

        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);

        // Two independent clusters, each with 2 shared files across its two commits — a
        // genuine tie on shared_files.len() that only the documented sha tie-break can
        // order.
        for i in 0..2 {
            std::fs::write(dir.join("a1.go"), format!("// v{i}\npackage a\n")).unwrap();
            std::fs::write(dir.join("a2.go"), format!("// v{i}\npackage a\n")).unwrap();
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", &format!("fix: cluster a {i}")]);
        }
        for i in 0..2 {
            std::fs::write(dir.join("b1.go"), format!("// v{i}\npackage b\n")).unwrap();
            std::fs::write(dir.join("b2.go"), format!("// v{i}\npackage b\n")).unwrap();
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", &format!("fix: cluster b {i}")]);
        }

        let first_run = analyze(&dir, 1000, 20).unwrap();
        let second_run = analyze(&dir, 1000, 20).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(first_run.len(), 2, "got: {first_run:?}");
        assert_eq!(
            first_run[0].shared_files.len(),
            first_run[1].shared_files.len(),
            "expected a genuine tie: {first_run:?}"
        );
        assert!(
            first_run[0].commit_shas.first() <= first_run[1].commit_shas.first(),
            "documented tie-break sorts ascending by first commit sha: {first_run:?}"
        );
        assert_eq!(
            first_run, second_run,
            "tie-break must produce the same order on every run"
        );
    }
}
