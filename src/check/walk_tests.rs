use super::*;

#[test]
fn nested_git_roots_are_scanned_with_their_own_ignores() {
    let dir = scratch_dir("nested");
    git(&dir, &["init", "-q"]);
    write(&dir, "module/main.go", "package main\n");
    let nested = dir.join("module");
    git(&nested, &["init", "-q"]);
    write(&nested, ".gitignore", "ignored.go\n");
    write(&nested, "ignored.go", "package ignored\n");
    git(&nested, &["add", "."]);
    git(
        &nested,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "initial",
        ],
    );
    for staged in [false, true] {
        if staged {
            git(&dir, &["add", "module"]);
        }
        let names = rel_names(&dir, &walk_and_collect_files(&dir).unwrap());
        assert_eq!(names, ["module/.gitignore", "module/main.go"]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn git_walk_does_not_follow_symlinks_outside_root() {
    let dir = scratch_dir("links");
    let outside = scratch_dir("outside");
    git(&dir, &["init", "-q"]);
    write(&outside, "a.go", "package a\n");
    std::os::unix::fs::symlink(outside.join("a.go"), dir.join("link.go")).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("linked")).unwrap();
    assert!(walk_and_collect_files(&dir).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(outside);
}

fn git(dir: &Path, args: &[&str]) {
    let status = git_command(dir).args(args).status().unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kibitzer-walk-test-{}-{name}-{}",
        std::process::id(),
        TMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn rel_names(dir: &Path, files: &[PathBuf]) -> Vec<String> {
    files
        .iter()
        .map(|f| f.strip_prefix(dir).unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn walk_and_collect_files_skips_gitignored_dirs_but_keeps_untracked_files() {
    let dir = scratch_dir("gitignore");
    git(&dir, &["init", "-q"]);
    write(&dir, ".gitignore", "worktrees/\n");
    write(&dir, "main.go", "package main\n");
    write(&dir, ".claude/worktrees/agent-1/copy.go", "package main\n");
    git(&dir, &["add", "main.go", ".gitignore"]);
    write(&dir, "untracked.go", "package main\n");

    let files = walk_and_collect_files(&dir).unwrap();
    let names = rel_names(&dir, &files);

    assert_eq!(names, [".gitignore", "main.go", "untracked.go"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn walk_and_collect_files_still_skips_skip_dirs_when_tracked() {
    let dir = scratch_dir("skipdirs");
    git(&dir, &["init", "-q"]);
    write(&dir, "main.go", "package main\n");
    write(&dir, "vendor/dep/dep.go", "package dep\n");
    git(&dir, &["add", "."]);

    let names = rel_names(&dir, &walk_and_collect_files(&dir).unwrap());

    assert_eq!(names, ["main.go"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn walk_and_collect_files_omits_tracked_files_deleted_from_work_tree() {
    let dir = scratch_dir("deleted");
    git(&dir, &["init", "-q"]);
    write(&dir, "gone.go", "package main\n");
    write(&dir, "kept.go", "package main\n");
    git(&dir, &["add", "."]);
    std::fs::remove_file(dir.join("gone.go")).unwrap();

    let names = rel_names(&dir, &walk_and_collect_files(&dir).unwrap());

    assert_eq!(names, ["kept.go"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn walk_and_collect_files_falls_back_to_plain_walk_outside_a_work_tree() {
    let dir = scratch_dir("nogit");
    write(&dir, "a.go", "package a\n");
    write(&dir, "node_modules/x.go", "package x\n");

    let names = rel_names(&dir, &walk_and_collect_files(&dir).unwrap());

    assert_eq!(names, ["a.go"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn walk_and_collect_files_walks_a_dir_that_is_itself_gitignored() {
    let repo = scratch_dir("ignored-parent");
    git(&repo, &["init", "-q"]);
    write(&repo, ".gitignore", "snapshot/\n");
    write(&repo, "snapshot/a.go", "package a\n");

    let dir = repo.join("snapshot");
    let names = rel_names(&dir, &walk_and_collect_files(&dir).unwrap());

    assert_eq!(names, ["a.go"]);
    let _ = std::fs::remove_dir_all(&repo);
}
