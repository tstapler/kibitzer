//! Git-churn × complexity hotspot analysis — the technique behind CodeScene (Adam
//! Tornhill's open-source approach, also documented in the `code-hotspot-analysis`
//! skill). A file that's complex but never touched is a museum piece; a file that's
//! simple but changes constantly is fine; a file that's both is where incidents
//! concentrate. Batch-only — never wired into `default_checks()`/hook mode, same
//! convention as `change_coupling.rs`/`root_cause_clusters.rs`: a "look here"
//! prioritization report, not a pass/fail check. v1 scope is Go-only, matching
//! `primitive_obsession.rs`'s rollout precedent (see #15).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::checker::{GrammarCache, Language};

/// Commits scanned beyond this are almost certainly diluting the churn signal with
/// stale history rather than adding real information — mirrors
/// `change_coupling.rs`'s own analogous cap and rationale.
const MAX_LIMIT: usize = 50_000;

/// One file's hotspot score: revisions × complexity.
#[derive(Debug, Clone, PartialEq)]
pub struct Hotspot {
    pub file: String,
    pub revisions: u32,
    pub complexity: usize,
    pub score: u64,
}

/// `git log --name-only`'s file paths are always relative to the repository's real
/// top-level directory, regardless of `git`'s working directory — unlike
/// `change_coupling.rs`'s callers, which only ever print those paths as strings,
/// `analyze` below needs to actually open each file, so it must resolve the true
/// top-level once and join against *that*, not against `path` itself. Without this, a
/// `--path` pointing at a subdirectory silently resolves every file to a nonexistent
/// location and the report comes back empty instead of scoped to that subdirectory.
fn git_toplevel(path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .context("failed to run git rev-parse --show-toplevel")?;
    if !output.status.success() {
        bail!(
            "git rev-parse --show-toplevel exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&output.stdout).trim(),
    ))
}

/// Runs the hotspot report over `path`'s last `limit` non-merge commits (clamped to
/// [`MAX_LIMIT`]), scoring every currently-existing `.go` file touched in that window by
/// `revisions × complexity` and returning the top `top_n`, highest score first. `path`
/// may be the repo root or any subdirectory inside it — results are always scoped to
/// files under `path`, since the real repo top-level (see [`git_toplevel`]) is resolved
/// separately from the directory results are filtered to. A file's complexity is the sum
/// of cyclomatic complexity across its function/method declarations (see
/// `complexity::total_complexity`). Generated files (`file_size::is_generated`) and files
/// no longer present on disk (deleted or renamed since) are skipped — churn history for a
/// file that isn't there to fix isn't actionable.
pub fn analyze(path: &Path, limit: usize, top_n: usize) -> Result<Vec<Hotspot>> {
    let toplevel = git_toplevel(path)?;
    let commits = crate::change_coupling::git_log_commits(path, limit.min(MAX_LIMIT))?;
    let revisions = crate::change_coupling::file_revisions(&commits);

    // Scope to files under `path`: canonicalize both sides so a relative `path` (or one
    // containing `.`/`..`) still compares correctly against `toplevel`, which
    // `git rev-parse` always returns as an absolute, fully-resolved path.
    let scope = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let mut hotspots: Vec<Hotspot> = revisions
        .into_iter()
        .filter_map(|(file, revs)| score_file(&toplevel, &scope, file, revs))
        .collect();

    hotspots.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.file.cmp(&b.file)));
    hotspots.truncate(top_n);
    Ok(hotspots)
}

/// Scores one churned `file` (repo-root-relative, as `git log` emits it), or `None` if
/// it's not a `.go` file, falls outside `scope`, no longer exists on disk, is generated,
/// or fails to parse — see [`analyze`]'s doc comment for why each of those is skipped
/// rather than erroring.
fn score_file(toplevel: &Path, scope: &Path, file: String, revs: u32) -> Option<Hotspot> {
    if !file.ends_with(".go") {
        return None;
    }
    let full_path = toplevel.join(&file);
    if !full_path.starts_with(scope) {
        return None;
    }
    let source = std::fs::read_to_string(&full_path).ok()?;
    if crate::file_size::is_generated(&source) {
        return None;
    }
    // A fresh `GrammarCache` per file: its cache key is `Language` alone, so reusing one
    // instance across files of the same language would silently return the previous
    // file's parse tree (see `GrammarCache::parse`'s doc comment).
    let grammar = GrammarCache::new();
    let tree = grammar.parse(Language::Go, &source).ok()?;
    let complexity = crate::complexity::total_complexity(tree.root_node(), source.as_bytes());
    if complexity == 0 {
        return None;
    }
    let score = u64::from(revs) * complexity as u64;
    Some(Hotspot {
        file,
        revisions: revs,
        complexity,
        score,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempGitRepo {
        dir: std::path::PathBuf,
    }

    impl TempGitRepo {
        fn new(name: &str) -> Self {
            let dir = crate::test_support::unique_temp_dir(&format!("hotspots-test-{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            Self::run(&dir, &["init", "-q"]);
            Self::run(&dir, &["config", "user.email", "test@example.com"]);
            Self::run(&dir, &["config", "user.name", "test"]);
            Self { dir }
        }

        fn run(dir: &Path, args: &[&str]) {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        }

        fn write_and_commit(&self, rel_path: &str, content: &str, message: &str) {
            let path = self.dir.join(rel_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, content).unwrap();
            Self::run(&self.dir, &["add", rel_path]);
            Self::run(&self.dir, &["commit", "-q", "-m", message]);
        }

        fn remove_and_commit(&self, rel_path: &str) {
            std::fs::remove_file(self.dir.join(rel_path)).unwrap();
            Self::run(&self.dir, &["rm", "-q", rel_path]);
            Self::run(&self.dir, &["commit", "-q", "-m", "remove"]);
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const COMPLEX_GO: &str = "package foo\n\nfunc Complex(n int) int {\n\
        if n > 0 {\n\
            for i := 0; i < n; i++ {\n\
                if i%2 == 0 {\n\
                    n += i\n\
                } else if i%3 == 0 {\n\
                    n -= i\n\
                }\n\
            }\n\
        }\n\
        return n\n\
    }\n";

    const SIMPLE_GO: &str = "package foo\n\nfunc Simple() int {\n\treturn 1\n}\n";

    #[test]
    fn ranks_a_frequently_changed_complex_file_above_a_rarely_changed_simple_one() {
        let repo = TempGitRepo::new("ranking");
        // `complex.go` churns 3x, `simple.go` only once. Each revision's content must
        // actually differ — `git commit` fails on a no-op commit.
        for i in 0..3 {
            let content = format!("{COMPLEX_GO}\n// revision {i}\n");
            repo.write_and_commit("complex.go", &content, &format!("edit {i}"));
        }
        repo.write_and_commit("simple.go", SIMPLE_GO, "add simple");

        let hotspots = analyze(&repo.dir, 1000, 20).unwrap();
        let files: Vec<&str> = hotspots.iter().map(|h| h.file.as_str()).collect();
        assert_eq!(files, vec!["complex.go", "simple.go"]);
        assert!(hotspots[0].score > hotspots[1].score);
        assert_eq!(hotspots[0].revisions, 3);
        assert_eq!(hotspots[1].revisions, 1);
    }

    #[test]
    fn skips_non_go_files_and_files_deleted_since() {
        let repo = TempGitRepo::new("skip");
        repo.write_and_commit("README.md", "# hi\n", "add readme");
        repo.write_and_commit("gone.go", SIMPLE_GO, "add gone");
        repo.remove_and_commit("gone.go");

        let hotspots = analyze(&repo.dir, 1000, 20).unwrap();
        assert!(hotspots.is_empty());
    }

    #[test]
    fn skips_generated_files() {
        let repo = TempGitRepo::new("generated");
        let generated = format!("// Code generated by foo; DO NOT EDIT.\n{SIMPLE_GO}");
        repo.write_and_commit("generated.go", &generated, "add generated");

        let hotspots = analyze(&repo.dir, 1000, 20).unwrap();
        assert!(hotspots.is_empty());
    }

    #[test]
    fn respects_top_n() {
        let repo = TempGitRepo::new("top-n");
        repo.write_and_commit("a.go", COMPLEX_GO, "add a");
        repo.write_and_commit("b.go", COMPLEX_GO, "add b");

        let hotspots = analyze(&repo.dir, 1000, 1).unwrap();
        assert_eq!(hotspots.len(), 1);
    }

    /// Regression test: `git log --name-only`'s paths are always relative to the repo's
    /// real top-level, not to whatever directory `analyze` was pointed at. An earlier
    /// version joined those paths directly against the passed-in `path`, which silently
    /// resolved every file to a nonexistent location — and produced an empty report
    /// instead of an error or correct subdirectory scoping — whenever `path` was a
    /// subdirectory rather than the repo root.
    #[test]
    fn scopes_correctly_when_path_is_a_subdirectory() {
        let repo = TempGitRepo::new("subdir-scope");
        repo.write_and_commit("sub/a.go", COMPLEX_GO, "add sub/a.go");
        repo.write_and_commit("other/b.go", COMPLEX_GO, "add other/b.go");

        let scoped = analyze(&repo.dir.join("sub"), 1000, 20).unwrap();
        let files: Vec<&str> = scoped.iter().map(|h| h.file.as_str()).collect();
        assert_eq!(files, vec!["sub/a.go"]);

        let unscoped = analyze(&repo.dir, 1000, 20).unwrap();
        assert_eq!(unscoped.len(), 2);
    }
}
