//! Change-coupling (temporal coupling) analysis: which file pairs change together across
//! git history, independent of any import/call relationship between them. Reimplements the
//! coupling formula from [code-maat](https://github.com/adamtornhill/code-maat) (GPLv3 — not
//! linked against, formula reimplemented directly to avoid copyleft entanglement) and applies
//! CodeScene's published noise-filter thresholds. Batch-only: this is a "look here"
//! prioritization report over repo history, not a per-edit pass/fail check, so it's never
//! wired into `default_checks()` or hook mode.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// A commit touching more files than this is treated as a mass refactor / mechanical
/// rename, not a real co-change signal, and excluded entirely (CodeScene's noise filter).
const MAX_FILES_PER_COMMIT: usize = 50;
/// Minimum total revisions (revA + revB - shared) for a pair to be reported — below this
/// the coupling percentage is too noisy to act on.
const MIN_TOTAL_REVISIONS: u32 = 10;
/// Minimum shared commits for a pair to be reported.
const MIN_SHARED_COMMITS: u32 = 10;
/// Minimum coupling percentage (0.0-1.0) for a pair to be reported.
const MIN_COUPLING: f64 = 0.5;

/// One file pair's temporal coupling score.
#[derive(Debug, Clone, PartialEq)]
pub struct CoupledPair {
    pub file_a: String,
    pub file_b: String,
    pub revisions_a: u32,
    pub revisions_b: u32,
    pub shared_commits: u32,
    /// `shared_commits / (revisions_a + revisions_b - shared_commits)`, in `[0.0, 1.0]`.
    pub coupling: f64,
}

/// Parses `git log --name-only --no-merges` output into one file list per commit, in the
/// format `git_log_commits` produces it. A commit with no changed files (shouldn't happen
/// for `--no-merges` history, but tolerated) yields an empty `Vec`.
fn parse_name_only_log(output: &str) -> Vec<Vec<String>> {
    let mut commits = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in output.lines() {
        if line.starts_with('\u{1}') {
            if let Some(files) = current.take() {
                commits.push(files);
            }
            current = Some(Vec::new());
        } else if !line.is_empty()
            && let Some(files) = current.as_mut()
        {
            files.push(line.to_string());
        }
    }
    if let Some(files) = current.take() {
        commits.push(files);
    }
    commits
}

/// Runs `git log --name-only --no-merges -n <limit>` in `repo_root` and returns the
/// per-commit file lists. Each commit is delimited with a `\u{1}` marker (a byte that never
/// appears in a file path) so commits with zero changed files don't get silently merged
/// into their neighbor.
fn git_log_commits(repo_root: &Path, limit: usize) -> Result<Vec<Vec<String>>> {
    let output = Command::new("git")
        .args([
            "log",
            "--no-merges",
            "--name-only",
            "--pretty=format:\u{1}",
            &format!("-n{limit}"),
        ])
        .current_dir(repo_root)
        .output()
        .context("failed to run git log")?;
    if !output.status.success() {
        bail!(
            "git log exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_name_only_log(&text))
}

/// Computes temporal coupling over `commits` (each a list of files changed in one commit,
/// oldest-history-window-bounded by the caller). Pure function — no git I/O — so the
/// coupling math is unit-testable without a repo fixture. Commits touching more than
/// [`MAX_FILES_PER_COMMIT`] files are dropped before counting (mass-refactor noise).
/// Results are filtered to [`MIN_TOTAL_REVISIONS`]/[`MIN_SHARED_COMMITS`]/[`MIN_COUPLING`]
/// and sorted by coupling percentage, descending.
pub fn compute_coupling(commits: &[Vec<String>]) -> Vec<CoupledPair> {
    let (revisions, shared) = tally_revisions_and_shared_commits(commits);
    let mut pairs = pairs_above_thresholds(&revisions, &shared);
    pairs.sort_by(|a, b| {
        b.coupling
            .partial_cmp(&a.coupling)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairs
}

/// First pass over `commits`: per-file revision counts and per-pair shared-commit counts,
/// skipping empty and mass-refactor commits.
/// Per-file revision counts and per-file-pair shared-commit counts, both keyed by borrowed
/// file paths from the input commits.
type RevisionAndSharedCounts<'a> = (BTreeMap<&'a str, u32>, BTreeMap<(&'a str, &'a str), u32>);

fn tally_revisions_and_shared_commits(commits: &[Vec<String>]) -> RevisionAndSharedCounts<'_> {
    let mut revisions: BTreeMap<&str, u32> = BTreeMap::new();
    let mut shared: BTreeMap<(&str, &str), u32> = BTreeMap::new();

    for commit in commits {
        if commit.is_empty() || commit.len() > MAX_FILES_PER_COMMIT {
            continue;
        }
        let mut files: Vec<&str> = commit.iter().map(String::as_str).collect();
        files.sort_unstable();
        files.dedup();
        for f in &files {
            *revisions.entry(f).or_default() += 1;
        }
        for i in 0..files.len() {
            for j in (i + 1)..files.len() {
                *shared.entry((files[i], files[j])).or_default() += 1;
            }
        }
    }
    (revisions, shared)
}

/// Second pass: turns the raw tallies into [`CoupledPair`]s, dropping anything below
/// [`MIN_TOTAL_REVISIONS`]/[`MIN_SHARED_COMMITS`]/[`MIN_COUPLING`].
fn pairs_above_thresholds<'a>(
    revisions: &BTreeMap<&'a str, u32>,
    shared: &BTreeMap<(&'a str, &'a str), u32>,
) -> Vec<CoupledPair> {
    shared
        .iter()
        .filter_map(|(&(a, b), &shared_commits)| {
            let revisions_a = *revisions.get(a).unwrap_or(&0);
            let revisions_b = *revisions.get(b).unwrap_or(&0);
            let total = revisions_a + revisions_b - shared_commits;
            if total < MIN_TOTAL_REVISIONS || shared_commits < MIN_SHARED_COMMITS {
                return None;
            }
            let coupling = shared_commits as f64 / total as f64;
            if coupling < MIN_COUPLING {
                return None;
            }
            Some(CoupledPair {
                file_a: a.to_string(),
                file_b: b.to_string(),
                revisions_a,
                revisions_b,
                shared_commits,
                coupling,
            })
        })
        .collect()
}

/// Runs the full change-coupling report over `repo_root`'s last `limit` non-merge commits,
/// returning the top `top_n` coupled pairs.
pub fn analyze(repo_root: &Path, limit: usize, top_n: usize) -> Result<Vec<CoupledPair>> {
    let commits = git_log_commits(repo_root, limit)?;
    let mut pairs = compute_coupling(&commits);
    pairs.truncate(top_n);
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commits_of(pairs: &[&[&str]]) -> Vec<Vec<String>> {
        pairs
            .iter()
            .map(|files| files.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn parses_name_only_log_with_marker_delimiters() {
        let log = "\u{1}\na.rs\nb.rs\n\u{1}\nb.rs\nc.rs\n";
        let commits = parse_name_only_log(log);
        assert_eq!(
            commits,
            vec![
                vec!["a.rs".to_string(), "b.rs".to_string()],
                vec!["b.rs".to_string(), "c.rs".to_string()],
            ]
        );
    }

    #[test]
    fn parses_commit_with_no_changed_files() {
        let log = "\u{1}\n\u{1}\na.rs\n";
        let commits = parse_name_only_log(log);
        assert_eq!(
            commits,
            vec![Vec::<String>::new(), vec!["a.rs".to_string()]]
        );
    }

    #[test]
    fn flags_pair_above_all_thresholds() {
        // 10 commits touching both a.rs and b.rs, plus 2 extra commits each touching only
        // one of them: revisions_a=12, revisions_b=12, shared=10, total=14, coupling=0.71.
        let mut commits: Vec<&[&str]> = vec![&["a.rs", "b.rs"]; 10];
        commits.push(&["a.rs"]);
        commits.push(&["a.rs"]);
        commits.push(&["b.rs"]);
        commits.push(&["b.rs"]);
        let pairs = compute_coupling(&commits_of(&commits));
        assert_eq!(pairs.len(), 1);
        let pair = &pairs[0];
        assert_eq!(pair.file_a, "a.rs");
        assert_eq!(pair.file_b, "b.rs");
        assert_eq!(pair.shared_commits, 10);
        assert_eq!(pair.revisions_a, 12);
        assert_eq!(pair.revisions_b, 12);
        assert!((pair.coupling - 10.0 / 14.0).abs() < 1e-9);
    }

    #[test]
    fn drops_pair_below_min_shared_commits() {
        let commits: Vec<&[&str]> = vec![&["a.rs", "b.rs"]; 5];
        let pairs = compute_coupling(&commits_of(&commits));
        assert!(pairs.is_empty());
    }

    #[test]
    fn drops_pair_below_min_coupling_percentage() {
        // Shared-commit count clears the minimum, but each file also picks up 20 solo
        // revisions, driving the coupling percentage well under the 50% floor.
        let mut commits: Vec<&[&str]> = vec![&["a.rs", "b.rs"]; 10];
        for _ in 0..20 {
            commits.push(&["a.rs"]);
        }
        for _ in 0..20 {
            commits.push(&["b.rs"]);
        }
        let pairs = compute_coupling(&commits_of(&commits));
        assert!(pairs.is_empty());
    }

    #[test]
    fn excludes_mass_refactor_commits_from_counting() {
        let huge_commit: Vec<String> = (0..(MAX_FILES_PER_COMMIT + 1))
            .map(|i| format!("f{i}.rs"))
            .collect();
        let mut commits = vec![huge_commit];
        for _ in 0..10 {
            commits.push(vec!["a.rs".to_string(), "b.rs".to_string()]);
        }
        let pairs = compute_coupling(&commits);
        // Only the a.rs/b.rs pair should register — the mass-refactor commit contributes
        // no revisions or shared-commit counts to any file.
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].revisions_a, 10);
    }

    #[test]
    fn sorts_results_by_coupling_descending() {
        // Pair (a,b): 10/10 shared -> coupling 1.0. Pair (c,d): 10 shared + 5 extra each ->
        // total 20, coupling 0.5.
        let ab: Vec<&[&str]> = vec![&["a.rs", "b.rs"]; 10];
        let cd: Vec<&[&str]> = vec![&["c.rs", "d.rs"]; 10];
        let mut commits = commits_of(&ab);
        commits.extend(commits_of(&cd));
        for _ in 0..5 {
            commits.push(vec!["c.rs".to_string()]);
            commits.push(vec!["d.rs".to_string()]);
        }
        let pairs = compute_coupling(&commits);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].file_a, "a.rs");
        assert!((pairs[0].coupling - 1.0).abs() < 1e-9);
    }
}
