//! Change-coupling: file pairs that change together across git history, independent of
//! import/call relationships. Reimplements [code-maat](https://github.com/adamtornhill/code-maat)'s
//! formula directly (not linked, to avoid GPLv3 entanglement) using CodeScene's noise-filter
//! thresholds. Batch-only — never wired into `default_checks()`/hook mode.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;

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
/// into their neighbor. `pub(crate)` so `hotspots.rs` can reuse the same git-log plumbing
/// for its churn count instead of re-implementing this parsing.
pub(crate) fn git_log_commits(repo_root: &Path, limit: usize) -> Result<Vec<Vec<String>>> {
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

/// Computes temporal coupling over `commits` (each a list of files changed in one commit).
/// Drops commits over [`MAX_FILES_PER_COMMIT`] (mass-refactor noise) and results below
/// [`MIN_TOTAL_REVISIONS`]/[`MIN_SHARED_COMMITS`]/[`MIN_COUPLING`]; sorts by coupling
/// percentage, descending.
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

/// Same computation as [`compute_coupling`], without CodeScene's whole-repo noise-filter
/// thresholds (`MIN_TOTAL_REVISIONS`/`MIN_SHARED_COMMITS`/`MIN_COUPLING`) — appropriate
/// when `commits` is already a small, pre-filtered subset (e.g. `root_cause_clusters.rs`'s
/// bug-fix-commit clusters, typically a handful of commits), where those repo-scale noise
/// floors would suppress everything. Still drops mass-refactor commits
/// ([`MAX_FILES_PER_COMMIT`]) and sorts by coupling percentage, descending.
pub fn compute_coupling_unfiltered(commits: &[Vec<String>]) -> Vec<CoupledPair> {
    let (revisions, shared) = tally_revisions_and_shared_commits(commits);
    let mut pairs: Vec<CoupledPair> = shared
        .iter()
        .map(|(&(a, b), &shared_commits)| {
            let revisions_a = *revisions.get(a).unwrap_or(&0);
            let revisions_b = *revisions.get(b).unwrap_or(&0);
            let total = revisions_a + revisions_b - shared_commits;
            let coupling = if total == 0 {
                0.0
            } else {
                shared_commits as f64 / total as f64
            };
            CoupledPair {
                file_a: a.to_string(),
                file_b: b.to_string(),
                revisions_a,
                revisions_b,
                shared_commits,
                coupling,
            }
        })
        .collect();
    pairs.sort_by(|a, b| {
        b.coupling
            .partial_cmp(&a.coupling)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    pairs
}

/// One commit, tagged with enough information for `root_cause_clusters.rs` to classify it
/// and cluster it by touched files — unlike [`git_log_commits`]'s plain file lists, this
/// keeps the commit hash, subject line, and body (discarded by that function's
/// `--pretty=format:\u{1}`, which exists only to delimit commits). `body` matters for
/// classification even when `subject` mentions no issue: a squash-merge commonly carries
/// GitHub's auto-appended "Fixes #123" (from the linked PR) only in the body.
#[derive(Debug, Clone, PartialEq)]
pub struct TaggedCommit {
    pub sha: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<String>,
}

/// Parses `git log --name-only --no-merges --pretty=format:'\u{1}%H\u{1}%s'` output: each
/// commit's header line starts with `\u{1}`, followed by its hash, another `\u{1}`, and its
/// subject; subsequent non-empty lines until the next header are its changed files. `body`
/// is left empty here — `git_log_commits_with_subject` fills it in from a second git-log
/// call ([`git_log_full_messages`]) since a multi-line commit body can't be told apart
/// from this format's newline-delimited file list.
fn parse_name_only_log_with_subject(output: &str) -> Vec<TaggedCommit> {
    let mut commits = Vec::new();
    let mut current: Option<TaggedCommit> = None;
    for line in output.lines() {
        if let Some(header) = line.strip_prefix('\u{1}') {
            if let Some(c) = current.take() {
                commits.push(c);
            }
            let mut parts = header.splitn(2, '\u{1}');
            current = Some(TaggedCommit {
                sha: parts.next().unwrap_or_default().to_string(),
                subject: parts.next().unwrap_or_default().to_string(),
                body: String::new(),
                files: Vec::new(),
            });
        } else if !line.is_empty()
            && let Some(c) = current.as_mut()
        {
            c.files.push(line.to_string());
        }
    }
    if let Some(c) = current.take() {
        commits.push(c);
    }
    commits
}

/// Runs `git log --no-merges --pretty=format:'\u{2}%H\u{1}%B'` and returns each commit's
/// full raw message (subject + body, via `%B`) keyed by hash. Uses `\u{2}` (STX) as the
/// commit delimiter and `\u{1}` (SOH) only to separate the hash from the message —
/// distinct control characters, so a multi-paragraph body (which can contain blank lines,
/// unlike the single-line `%s`) never gets misparsed as a new commit boundary. Kept as a
/// separate call from [`git_log_commits_with_subject`]'s `--name-only` one specifically
/// because that one's file-list parsing is newline-delimited and can't coexist with an
/// arbitrary multi-line body in the same output stream.
fn git_log_full_messages(repo_root: &Path, limit: usize) -> Result<HashMap<String, String>> {
    let output = Command::new("git")
        .args([
            "log",
            "--no-merges",
            "--pretty=format:\u{2}%H\u{1}%B",
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
    Ok(text
        .split('\u{2}')
        .filter(|block| !block.is_empty())
        .filter_map(|block| block.split_once('\u{1}'))
        .map(|(sha, message)| (sha.to_string(), message.trim_end().to_string()))
        .collect())
}

/// Runs `git log --name-only --no-merges -n <limit>` in `repo_root`, keeping each commit's
/// hash, subject, and body alongside its file list — the [`TaggedCommit`] sibling of
/// [`git_log_commits`], for callers (`root_cause_clusters.rs`) that need to classify
/// commits by message, not just tally file co-changes.
pub fn git_log_commits_with_subject(repo_root: &Path, limit: usize) -> Result<Vec<TaggedCommit>> {
    let output = Command::new("git")
        .args([
            "log",
            "--no-merges",
            "--name-only",
            "--pretty=format:\u{1}%H\u{1}%s",
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
    let mut commits = parse_name_only_log_with_subject(&text);

    let full_messages = git_log_full_messages(repo_root, limit)?;
    for commit in &mut commits {
        if let Some(message) = full_messages.get(&commit.sha) {
            // %B is "subject\n\nbody..." — strip the subject line back off, since
            // TaggedCommit keeps it separately (from %s) and callers scan subject/body
            // independently.
            commit.body = message
                .strip_prefix(&commit.subject)
                .unwrap_or(message)
                .trim_start_matches('\n')
                .to_string();
        }
    }
    Ok(commits)
}

/// Matches a `fix(es|ed)?`/`clos(e|es|ed)?`/`resolv(e|es|ed)? #123`-style issue reference
/// anywhere in a commit subject or body — the same keywords GitHub auto-links from a PR to
/// an issue (https://docs.github.com/en/issues/tracking-your-work-with-issues/linking-a-pull-request-to-an-issue),
/// which a squash-merge commonly carries only in the body.
static FIX_ISSUE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:fix(?:e[sd])?|clos(?:e[sd])?|resolv(?:e[sd])?)\s+#\d+")
        .expect("FIX_ISSUE_RE is a valid pattern")
});

/// Matches `fix` as a whole word at the start of a subject — covers both the
/// conventional-commit `fix:`/`fix(scope):` prefix and a bare imperative `Fix ...` subject
/// with no colon at all (a convention this very codebase's own history uses, e.g. "Fix
/// cargo fmt and clippy CI failures on master"). Deliberately broad: this also matches
/// something like "Fix typo in README" with no tracked issue — over-tagging a small,
/// harmless commit as a "bug fix" for clustering purposes costs nothing (it only
/// participates in a cluster if it happens to co-touch files with another tagged commit),
/// which is a better trade than under-tagging real fixes that don't follow either stronger
/// convention.
static FIX_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^fix\b").expect("FIX_PREFIX_RE is a valid pattern"));

/// Whether a commit looks like a bug fix: a `fix`-prefixed subject ([`FIX_PREFIX_RE`]), or
/// an explicit issue reference ([`FIX_ISSUE_RE`]) anywhere in the subject or body. A
/// heuristic, not a guarantee — a real bug fix using none of these conventions goes
/// untagged, and "Fixed a subtle race condition" (past tense, no issue number) isn't
/// caught by the bare-prefix check either, only by an issue reference if one's present.
pub fn is_bug_fix_commit(subject: &str, body: &str) -> bool {
    FIX_PREFIX_RE.is_match(subject) || FIX_ISSUE_RE.is_match(subject) || FIX_ISSUE_RE.is_match(body)
}

/// Per-file revision counts and per-file-pair shared-commit counts, both keyed by borrowed
/// file paths from the input commits.
type RevisionAndSharedCounts<'a> = (BTreeMap<&'a str, u32>, BTreeMap<(&'a str, &'a str), u32>);

/// First pass over `commits`: per-file revision counts and per-pair shared-commit counts,
/// skipping empty and mass-refactor commits.
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

/// Per-file revision counts across `commits`, applying the same mass-refactor noise
/// filter as [`tally_revisions_and_shared_commits`] ([`MAX_FILES_PER_COMMIT`]) so a single
/// huge rename/vendor-update commit doesn't inflate every touched file's churn count.
/// A standalone pass rather than reusing `tally_revisions_and_shared_commits` and
/// discarding its pairwise `shared` map: that map costs O(files²) per commit to build,
/// pure overhead for `hotspots.rs`'s single-file churn score.
pub(crate) fn file_revisions(commits: &[Vec<String>]) -> BTreeMap<String, u32> {
    let mut revisions: BTreeMap<String, u32> = BTreeMap::new();
    for commit in commits {
        if commit.is_empty() || commit.len() > MAX_FILES_PER_COMMIT {
            continue;
        }
        let mut files: Vec<&str> = commit.iter().map(String::as_str).collect();
        files.sort_unstable();
        files.dedup();
        for f in files {
            *revisions.entry(f.to_string()).or_default() += 1;
        }
    }
    revisions
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

/// Hard ceiling on `analyze`'s `limit`: `git log`'s output is buffered in memory in full
/// before parsing (see `git_log_commits`), so an uncapped `--limit` on a huge repo would
/// let a user request unbounded memory growth from a single flag.
const MAX_LIMIT: usize = 50_000;

/// Runs the full change-coupling report over `repo_root`'s last `limit` non-merge commits
/// (clamped to [`MAX_LIMIT`]), returning the top `top_n` coupled pairs.
pub fn analyze(repo_root: &Path, limit: usize, top_n: usize) -> Result<Vec<CoupledPair>> {
    let commits = git_log_commits(repo_root, limit.min(MAX_LIMIT))?;
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

    struct TempGitRepo {
        dir: std::path::PathBuf,
    }

    impl TempGitRepo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "kibitzer-change-coupling-test-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self::run(&dir, &["init", "-q"]);
            Self::run(&dir, &["config", "user.email", "test@example.com"]);
            Self::run(&dir, &["config", "user.name", "test"]);
            Self { dir }
        }

        fn run(dir: &Path, args: &[&str]) {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        }

        fn commit_touching(&self, files: &[&str], msg: &str) {
            for f in files {
                std::fs::write(self.dir.join(f), msg).unwrap();
            }
            Self::run(&self.dir, &[&["add"], files].concat());
            Self::run(&self.dir, &["commit", "-q", "-m", msg]);
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Exercises the real `git log` subprocess path (`git_log_commits`) and `analyze`'s
    /// end-to-end composition — every other test in this module drives `compute_coupling`
    /// directly with hand-built commit lists, which never proves real git output actually
    /// parses the way `parse_name_only_log` expects.
    #[test]
    fn analyze_end_to_end_against_a_real_git_repo() {
        let repo = TempGitRepo::new("e2e");
        for i in 0..10 {
            repo.commit_touching(&["a.txt", "b.txt"], &format!("commit {i}"));
        }

        let pairs = analyze(&repo.dir, 1000, 20).unwrap();

        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].file_a, "a.txt");
        assert_eq!(pairs[0].file_b, "b.txt");
        assert_eq!(pairs[0].shared_commits, 10);
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

    // --- compute_coupling_unfiltered ---

    #[test]
    fn compute_coupling_unfiltered_surfaces_a_pair_below_the_whole_repo_noise_floor() {
        // Only 3 shared commits — compute_coupling (MIN_SHARED_COMMITS=10) would drop this
        // entirely, but a small bug-fix-commit cluster legitimately has few commits.
        let commits: Vec<&[&str]> = vec![&["a.rs", "b.rs"]; 3];
        let filtered = compute_coupling(&commits_of(&commits));
        assert!(filtered.is_empty(), "sanity: compute_coupling drops this");

        let unfiltered = compute_coupling_unfiltered(&commits_of(&commits));
        assert_eq!(unfiltered.len(), 1);
        assert_eq!(unfiltered[0].shared_commits, 3);
        assert!((unfiltered[0].coupling - 1.0).abs() < 1e-9);
    }

    #[test]
    fn compute_coupling_unfiltered_still_excludes_mass_refactor_commits() {
        let huge_commit: Vec<String> = (0..(MAX_FILES_PER_COMMIT + 1))
            .map(|i| format!("f{i}.rs"))
            .collect();
        let commits = vec![huge_commit];
        assert!(compute_coupling_unfiltered(&commits).is_empty());
    }

    // --- is_bug_fix_commit ---

    #[test]
    fn is_bug_fix_commit_matches_conventional_commit_prefix() {
        assert!(is_bug_fix_commit(
            "fix: correct off-by-one in pagination",
            ""
        ));
        assert!(is_bug_fix_commit("fix(cache): evict stale entries", ""));
    }

    #[test]
    fn is_bug_fix_commit_matches_a_bare_imperative_fix_subject() {
        // Regression: a real commit in this repo's own history ("Fix cargo fmt and clippy
        // CI failures on master") used no colon at all — the old prefix check
        // (`starts_with("fix:")`/`starts_with("fix(")`) missed it entirely.
        assert!(is_bug_fix_commit(
            "Fix cargo fmt and clippy CI failures on master",
            ""
        ));
    }

    #[test]
    fn is_bug_fix_commit_matches_an_explicit_issue_reference() {
        assert!(is_bug_fix_commit("Fixes #123: pagination cursor wraps", ""));
        assert!(is_bug_fix_commit("fix #45", ""));
        assert!(is_bug_fix_commit(
            "Fixed #7 double-free in cache eviction",
            ""
        ));
    }

    #[test]
    fn is_bug_fix_commit_matches_closes_and_resolves_keywords() {
        assert!(is_bug_fix_commit("Add pagination support", "Closes #88"));
        assert!(is_bug_fix_commit("Add retry logic", "Resolves #9"));
    }

    #[test]
    fn is_bug_fix_commit_scans_the_body_for_a_github_auto_appended_issue_reference() {
        // The shape a squash-merge commonly produces: a subject with no issue reference at
        // all, and GitHub's auto-appended "Fixes #N" (from the linked PR) only in the body.
        let subject = "Add retry logic to the upload client (#42)";
        let body = "* implement retry\n* add tests\n\nFixes #41";
        assert!(is_bug_fix_commit(subject, body));
    }

    #[test]
    fn is_bug_fix_commit_does_not_match_fix_as_part_of_a_longer_word() {
        assert!(!is_bug_fix_commit(
            "Fixture setup for integration tests",
            ""
        ));
        assert!(!is_bug_fix_commit(
            "refactor: simplify fix detection logic",
            ""
        ));
    }

    // --- git_log_commits_with_subject / parse_name_only_log_with_subject ---

    #[test]
    fn parses_tagged_commits_with_hash_subject_and_files() {
        let log = "\u{1}abc123\u{1}fix: bug\na.rs\nb.rs\n\u{1}def456\u{1}feat: thing\nc.rs\n";
        let commits = parse_name_only_log_with_subject(log);
        assert_eq!(
            commits,
            vec![
                TaggedCommit {
                    sha: "abc123".to_string(),
                    subject: "fix: bug".to_string(),
                    body: String::new(),
                    files: vec!["a.rs".to_string(), "b.rs".to_string()],
                },
                TaggedCommit {
                    sha: "def456".to_string(),
                    subject: "feat: thing".to_string(),
                    body: String::new(),
                    files: vec!["c.rs".to_string()],
                },
            ]
        );
    }

    #[test]
    fn git_log_commits_with_subject_against_a_real_git_repo() {
        let repo = TempGitRepo::new("tagged-e2e");
        repo.commit_touching(&["a.rs"], "fix: correct the thing");
        repo.commit_touching(&["b.rs"], "feat: add the other thing");

        let commits = git_log_commits_with_subject(&repo.dir, 1000).unwrap();
        assert_eq!(commits.len(), 2, "got: {commits:?}");
        assert_eq!(commits[0].subject, "feat: add the other thing");
        assert_eq!(commits[0].files, vec!["b.rs".to_string()]);
        assert_eq!(commits[1].subject, "fix: correct the thing");
        assert!(!commits[0].sha.is_empty());
        // The common case: an ordinary commit with no body at all must parse as "", not
        // e.g. a stray newline left over from splitting %B on the subject.
        assert_eq!(commits[0].body, "", "got: {commits:?}");
        assert_eq!(commits[1].body, "", "got: {commits:?}");
    }

    #[test]
    fn git_log_commits_with_subject_captures_a_multi_line_body() {
        let repo = TempGitRepo::new("tagged-body-e2e");
        let status = std::process::Command::new("git")
            .args([
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "Add retry logic to the upload client (#42)",
                "-m",
                "* implement retry\n* add tests\n\nFixes #41",
            ])
            .current_dir(&repo.dir)
            .status()
            .unwrap();
        assert!(status.success());

        let commits = git_log_commits_with_subject(&repo.dir, 1000).unwrap();
        assert_eq!(commits.len(), 1, "got: {commits:?}");
        assert_eq!(
            commits[0].subject,
            "Add retry logic to the upload client (#42)"
        );
        assert!(
            commits[0].body.contains("Fixes #41"),
            "got body: {:?}",
            commits[0].body
        );
        assert!(is_bug_fix_commit(&commits[0].subject, &commits[0].body));
    }
}
