use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::duplicate_code::{
    DuplicateCodeChecker, MIN_BLOCK_LINES, MIN_OCCURRENCES, qualifying_windows,
};

/// Diff-aware counterpart to `kibitzer check duplicates` (`duplicate_code::find_cross_file_duplicates`)
/// — see #28 proposal item 2. That command rescans a whole directory from scratch every
/// time, which is fine for a batch/CI run but too expensive to run on every `PostToolUse`
/// edit. This instead maintains a persistent per-repo index (`DuplicateIndex`) of every
/// file's duplicate-candidate line windows, updated incrementally for just the one file
/// being edited, so a hook firing only pays the cost of re-indexing that file rather than
/// the whole repo.
pub struct CrossFileDuplicateChecker;

impl Checker for CrossFileDuplicateChecker {
    fn name(&self) -> &str {
        "duplicate-code-cross-file"
    }

    fn description(&self) -> &str {
        "flags a block in this file that's already duplicated, near-verbatim, in another file in the repo"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        DuplicateCodeChecker.file_globs()
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let index_path = index_path_for_repo(&discover_repo_root(file));
        let mut index = DuplicateIndex::load(&index_path);
        let findings = index.reindex_file_and_find_duplicates(file, ctx.source);
        index.save(&index_path)?;
        Ok(findings)
    }
}

/// Shared scope for any file with no discoverable `.git` ancestor — grouping these
/// together (rather than, say, each file's own directory) matters because the whole
/// point of this checker is catching duplication *across* directories/packages; scoping
/// a git-less file to just its own directory would silently make that impossible for
/// any multi-directory project that isn't a git repo.
const NO_GIT_ROOT_FALLBACK: &str = "/kibitzer-no-git-root-fallback";

/// Walks up from `file` looking for a `.git` entry (directory for a normal clone, file
/// for a worktree/submodule) to find the repo boundary the index should be scoped to —
/// without this, an unrelated pair of repos sharing a coincidental snippet would report
/// as cross-file duplicates of each other. Falls back to [`NO_GIT_ROOT_FALLBACK`] when no
/// `.git` is found anywhere above `file`.
fn discover_repo_root(file: &Path) -> PathBuf {
    let mut dir = file.parent().unwrap_or(file);
    loop {
        if dir.join(".git").exists() {
            return dir.to_path_buf();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    PathBuf::from(NO_GIT_ROOT_FALLBACK)
}

/// Overrides where the persistent index lives, entirely so `backtest.rs` can point it
/// at a scratch directory: replaying reconstructed (not-necessarily-current) file
/// content from a transcript through this stateful checker would otherwise corrupt the
/// real index a live `PostToolUse` hook reads from afterward.
const INDEX_DIR_OVERRIDE_ENV: &str = "KIBITZER_DUPLICATE_INDEX_DIR";

fn duplicate_index_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(INDEX_DIR_OVERRIDE_ENV) {
        return PathBuf::from(dir);
    }
    crate::cache::default_cache_path()
        .parent()
        .map(|dir| dir.join("duplicate-index"))
        .unwrap_or_else(|| std::env::temp_dir().join("kibitzer-duplicate-index"))
}

/// One index file per repo. Named from a hash of the *canonicalized* repo root, not the
/// raw path text: a naive `/`-to-`_` substitution collides ordinary, non-adversarial
/// paths onto the same filename (e.g. `/home/t/my_project` and `/home/t/my/project`
/// both sanitize to `_home_t_my_project`), which would silently mix two unrelated
/// repos' indexes. Canonicalizing first also means a symlinked path and its resolved
/// target share one index instead of silently fragmenting into two. The hash need not
/// be cryptographic — inputs are this machine's own repo paths, not adversarial — so
/// `DefaultHasher` (deterministic across runs, unlike `HashMap`'s randomized default)
/// is enough. A truncated, sanitized basename is kept as a prefix purely so the
/// directory listing stays human-inspectable; only the hash suffix is load-bearing.
fn index_path_for_repo(repo_root: &Path) -> PathBuf {
    let canonical = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    canonical.hash(&mut hasher);
    let hash = hasher.finish();

    let readable: String = canonical
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();

    let name = if readable.is_empty() {
        format!("{hash:016x}.json")
    } else {
        format!("{readable}-{hash:016x}.json")
    };
    duplicate_index_dir().join(name)
}

/// Persistent, whole-repo reverse index from a duplicate-candidate window's exact
/// (normalized) text to every `(file, 0-indexed start line)` it currently occurs at.
/// `files` tracks which window keys each file currently contributes, so re-indexing a
/// file can cheaply drop its stale entries before inserting the fresh ones. `save`
/// writes via temp-file-then-rename so a crash or a second, concurrent writer can never
/// leave a torn/corrupt file for `load` to stumble over — the remaining risk under true
/// concurrent invocations (e.g. two daemon connection threads editing the same repo at
/// once, see `src/daemon.rs`'s per-connection `Arc<Mutex<Cache>>` for the pattern this
/// index does *not* yet share) is a clean lost update, not corruption: whichever `save`
/// runs last wins outright, and the dropped update's duplicate is simply caught the next
/// time either file is re-indexed. Folding this into daemon-shared, lock-protected state
/// the way `Cache` already is would close that gap; tracked as a follow-up rather than
/// done here.
#[derive(Debug, Default, Serialize, Deserialize)]
struct DuplicateIndex {
    windows: HashMap<String, Vec<(String, usize)>>,
    files: HashMap<String, Vec<String>>,
}

impl DuplicateIndex {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Writes via a uniquely-named temp file plus an atomic rename onto `path`, so a
    /// reader's `load` always sees either the previous complete index or the new one —
    /// never a partial write from a crash or an interleaved concurrent `save`.
    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("json.tmp.{}.{n}", std::process::id()));
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Replaces `file`'s entries in the index with the windows freshly computed from
    /// `source`, then reports every window that (after the update) still occurs at
    /// least `MIN_OCCURRENCES` times across at least two distinct files — the same bar
    /// `duplicate_code::find_cross_file_duplicates` applies, computed incrementally.
    fn reindex_file_and_find_duplicates(&mut self, file: &Path, source: &str) -> Vec<Finding> {
        let file_key = file.to_string_lossy().into_owned();
        self.remove_file(&file_key);
        let windows = file_windows(source);
        self.insert_file(&file_key, &windows);
        self.find_duplicates(&file_key, &windows)
    }

    fn remove_file(&mut self, file_key: &str) {
        let Some(old_keys) = self.files.remove(file_key) else {
            return;
        };
        for key in old_keys {
            if let Some(occurrences) = self.windows.get_mut(&key) {
                occurrences.retain(|(f, _)| f != file_key);
                if occurrences.is_empty() {
                    self.windows.remove(&key);
                }
            }
        }
    }

    fn insert_file(&mut self, file_key: &str, windows: &[(usize, String)]) {
        let mut keys = Vec::with_capacity(windows.len());
        for (line, text) in windows {
            self.windows
                .entry(text.clone())
                .or_default()
                .push((file_key.to_string(), *line));
            keys.push(text.clone());
        }
        self.files.insert(file_key.to_string(), keys);
    }

    /// Collapses shifted-window duplicates from one longer duplicated region into a
    /// single finding each, the same `covered_until` trick `find_duplicate_blocks`
    /// uses — tracked in `file_key`'s own line space since that's the only file this
    /// finding is reported against.
    ///
    /// Known limitation, deliberately not fixed here: nothing prunes a deleted or
    /// renamed file's entries from the index — the `PostToolUse` hook has no
    /// delete/rename signal, only Edit/Write/MultiEdit (`hook.rs`'s `ToolInput`) — so a
    /// stale occurrence can outlive the file it points at. Filtering to occurrences
    /// whose file still exists on disk was tried and reverted: it broke `check
    /// backtest`'s whole premise (replaying historical content "without touching disk"
    /// for files that may since be deleted, e.g. a session's `/tmp` scratch file) by
    /// requiring every *other* occurrence to still physically exist, not just the file
    /// under test. Acceptable for now since this is an advisory-only checker; revisit if
    /// stale duplicate reports against long-gone files become a real complaint.
    fn find_duplicates(&self, file_key: &str, windows: &[(usize, String)]) -> Vec<Finding> {
        let mut findings = Vec::new();
        let mut covered_until = 0usize;
        for (start, text) in windows {
            if *start < covered_until {
                continue;
            }
            if let Some(finding) = self.finding_for_window(file_key, *start, text) {
                findings.push(finding);
                covered_until = start + MIN_BLOCK_LINES;
            }
        }
        findings
    }

    /// One window's worth of `find_duplicates`' logic: threshold checks and message
    /// formatting — split out so `find_duplicates` stays just the `covered_until`
    /// collapsing loop.
    fn finding_for_window(&self, file_key: &str, start: usize, text: &str) -> Option<Finding> {
        let occurrences = &self.windows[text];
        if occurrences.len() < MIN_OCCURRENCES {
            return None;
        }
        let distinct_files: HashSet<&str> = occurrences.iter().map(|(f, _)| f.as_str()).collect();
        if distinct_files.len() < 2 {
            return None;
        }

        let mut other_locations: Vec<String> = occurrences
            .iter()
            .filter(|(f, _)| f != file_key)
            .map(|(f, line)| format!("{f}:{}", line + 1))
            .collect();
        other_locations.sort();
        other_locations.dedup();

        Some(Finding {
            line: start + 1,
            message: format!(
                "{MIN_BLOCK_LINES}-line block also duplicated in {} other file(s) ({}) — \
                 consider extracting a shared function",
                other_locations.len(),
                other_locations.join(", ")
            ),
        })
    }
}

/// Every `MIN_BLOCK_LINES`-line qualifying window in `source`, as `(0-indexed start
/// line, joined normalized text)` — the exact text (not a hash) is used as the index
/// key so two different blocks can never collide onto the same entry.
fn file_windows(source: &str) -> Vec<(usize, String)> {
    let normalized: Vec<String> = source.lines().map(|l| l.trim().to_string()).collect();
    qualifying_windows(&normalized)
        .map(|(start, window)| (start, window.join("\n")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "func doWork(id string) error {\n\
                          \tconn := openConnection(id)\n\
                          \tdefer conn.Close()\n\
                          \tresult := conn.Fetch(id)\n\
                          \tlog.Printf(\"fetched %v\", result)\n\
                          \treturn conn.Validate(result)\n";

    fn other_file_src() -> String {
        format!("package other\n\n{BLOCK}")
    }

    #[test]
    fn flags_a_block_once_it_reaches_three_files() {
        let mut index = DuplicateIndex::default();
        assert!(
            index
                .reindex_file_and_find_duplicates(Path::new("a.go"), &other_file_src())
                .is_empty()
        );
        assert!(
            index
                .reindex_file_and_find_duplicates(Path::new("b.go"), &other_file_src())
                .is_empty()
        );

        let findings = index.reindex_file_and_find_duplicates(Path::new("c.go"), &other_file_src());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 3);
        assert!(findings[0].message.contains("2 other file(s)"));
        assert!(findings[0].message.contains("a.go:3"));
        assert!(findings[0].message.contains("b.go:3"));
    }

    #[test]
    fn does_not_flag_when_only_one_other_file_has_it() {
        let mut index = DuplicateIndex::default();
        index.reindex_file_and_find_duplicates(Path::new("a.go"), &other_file_src());
        let findings = index.reindex_file_and_find_duplicates(Path::new("b.go"), &other_file_src());
        assert!(findings.is_empty());
    }

    #[test]
    fn removing_the_only_file_holding_a_block_drops_the_window_entry_entirely() {
        let mut index = DuplicateIndex::default();
        index.reindex_file_and_find_duplicates(Path::new("a.go"), &other_file_src());
        assert_eq!(index.windows.len(), 1);

        index.reindex_file_and_find_duplicates(Path::new("a.go"), "package other\n");
        assert!(
            index.windows.is_empty(),
            "a window with zero remaining occurrences must be removed, not left as an empty Vec"
        );
    }

    #[test]
    fn does_not_flag_repeats_confined_to_a_single_file() {
        let mut index = DuplicateIndex::default();
        let src = format!("package main\n\n{BLOCK}\n{BLOCK}\n{BLOCK}");
        let findings = index.reindex_file_and_find_duplicates(Path::new("a.go"), &src);
        assert!(findings.is_empty());
    }

    #[test]
    fn re_editing_a_file_to_remove_the_block_drops_its_index_entries() {
        let mut index = DuplicateIndex::default();
        index.reindex_file_and_find_duplicates(Path::new("a.go"), &other_file_src());
        index.reindex_file_and_find_duplicates(Path::new("b.go"), &other_file_src());
        let findings = index.reindex_file_and_find_duplicates(Path::new("c.go"), &other_file_src());
        assert_eq!(findings.len(), 1);

        // c.go is edited to no longer contain the block — it should drop out of the
        // index, taking the finding below MIN_OCCURRENCES again for a.go/b.go.
        index.reindex_file_and_find_duplicates(Path::new("c.go"), "package other\n");
        let findings = index.reindex_file_and_find_duplicates(
            Path::new("a.go"),
            &format!("package other\n\n{BLOCK}extra line to force a re-check\n"),
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn only_flags_once_for_a_longer_duplicate_run() {
        let long_block = "func longWork(id string) error {\n\
                           \tconn := openConnection(id)\n\
                           \tdefer conn.Close()\n\
                           \tresult := conn.Fetch(id)\n\
                           \tlog.Printf(\"fetched %v\", result)\n\
                           \tvalidated := conn.Validate(result)\n\
                           \tif validated == nil {\n\
                           \t\treturn errors.New(\"invalid\")\n\
                           \t}\n\
                           \treturn nil\n";
        let src = format!("package main\n\n{long_block}");
        let mut index = DuplicateIndex::default();
        index.reindex_file_and_find_duplicates(Path::new("a.go"), &src);
        index.reindex_file_and_find_duplicates(Path::new("b.go"), &src);
        let findings = index.reindex_file_and_find_duplicates(Path::new("c.go"), &src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn discover_repo_root_finds_the_nearest_ancestor_git_dir() {
        let base = std::env::temp_dir().join(format!(
            "kibitzer-cross-file-dup-repo-root-{}",
            std::process::id()
        ));
        let nested = base.join("src").join("pkg");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(base.join(".git")).unwrap();

        let found = discover_repo_root(&nested.join("file.go"));

        std::fs::remove_dir_all(&base).ok();
        assert_eq!(found, base);
    }

    #[test]
    fn discover_repo_root_falls_back_to_a_shared_scope_when_no_git_found() {
        // Two unrelated, git-less paths in *different* directories must still land in
        // the same fallback scope — otherwise cross-directory duplication (the whole
        // point of this checker) could never be caught in an ungit'd project.
        let a = Path::new("/kibitzer-test-path-with-no-git-ancestor/pkg1/a.go");
        let b = Path::new("/kibitzer-test-path-with-no-git-ancestor/pkg2/b.go");
        assert_eq!(discover_repo_root(a), discover_repo_root(b));
        assert_eq!(discover_repo_root(a), Path::new(NO_GIT_ROOT_FALLBACK));
    }

    #[test]
    fn index_path_for_repo_does_not_collide_ordinary_paths_that_naive_substitution_would() {
        // Both of these would sanitize to the identical `_home_t_my_project` under a
        // naive `/`-to-`_` substitution — the exact collision the hash suffix exists
        // to prevent. Neither path needs to exist on disk: `canonicalize` fails open
        // (falls back to the raw path) for a nonexistent one, so the hash still differs.
        let a = Path::new("/home/t/my_project");
        let b = Path::new("/home/t/my/project");
        assert_ne!(index_path_for_repo(a), index_path_for_repo(b));
    }

    #[test]
    fn index_path_for_repo_is_stable_across_calls() {
        let root = Path::new("/some/repo/root");
        assert_eq!(index_path_for_repo(root), index_path_for_repo(root));
    }

    /// Sets up a temp `.git` repo with `pkg1/a.go`/`pkg2/b.go`/`pkg3/c.go`, each
    /// containing `BLOCK` under a different package name, for the end-to-end test below.
    fn e2e_fixture_repo() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "kibitzer-cross-file-checker-e2e-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(".git")).unwrap();
        for pkg in ["pkg1", "pkg2", "pkg3"] {
            std::fs::create_dir_all(base.join(pkg)).unwrap();
        }
        std::fs::write(base.join("pkg1/a.go"), format!("package pkg1\n\n{BLOCK}")).unwrap();
        std::fs::write(base.join("pkg2/b.go"), format!("package pkg2\n\n{BLOCK}")).unwrap();
        std::fs::write(base.join("pkg3/c.go"), format!("package pkg3\n\n{BLOCK}")).unwrap();
        base
    }

    fn run_checker(path: &Path) -> Vec<Finding> {
        let source = std::fs::read_to_string(path).unwrap();
        let ctx = CheckContext {
            source: &source,
            tree: None,
        };
        CrossFileDuplicateChecker.check(path, &ctx).unwrap()
    }

    #[test]
    fn checker_check_flags_a_cross_file_duplicate_end_to_end() {
        // Exercises the real `Checker::check()` path (repo-root discovery, the on-disk
        // index, glob/language dispatch via `checker::run_checker`) rather than
        // `DuplicateIndex`'s internal methods directly — the gap the review flagged:
        // without this, `index_path_for_repo`/`duplicate_index_dir()`'s non-override
        // branch had zero direct coverage.
        let _isolation = crate::backtest::DuplicateIndexIsolation::new();
        let base = e2e_fixture_repo();

        assert!(run_checker(&base.join("pkg1/a.go")).is_empty());
        assert!(run_checker(&base.join("pkg2/b.go")).is_empty());
        let findings = run_checker(&base.join("pkg3/c.go"));

        std::fs::remove_dir_all(&base).ok();

        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("2 other file(s)"));
    }
}
