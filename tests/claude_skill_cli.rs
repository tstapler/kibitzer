//! End-to-end: `kibitzer run` over a real `.claude` tree reaches the three claude-skill-*
//! checkers through the actual glob matching and file walkers, which unit tests bypass.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tree(label: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(label);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".claude/skills/good")).unwrap();
    std::fs::create_dir_all(dir.join(".claude/skills/bad")).unwrap();
    std::fs::write(
        dir.join(".claude/skills/good/SKILL.md"),
        "---\nname: good\ndescription: Does a thing.\n---\nSee the `ghost-skill` skill.\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(".claude/skills/bad/SKILL.md"),
        "---\nname: other\ndescription: Mismatched.\n---\n",
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("../nowhere", dir.join(".claude/skills/dangling")).unwrap();
    dir
}

fn init_git(dir: &Path) {
    let ok = |args: &[&str]| {
        assert!(
            Command::new("git")
                .current_dir(dir)
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    };
    ok(&["init", "-q"]);
    ok(&["add", "-A"]);
    ok(&["commit", "-q", "-m", "x"]);
}

fn run(dir: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kibitzer"))
        .arg("run")
        .arg(dir)
        .output()
        .expect("spawn kibitzer");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn assert_all_three_fire(output: &str) {
    assert!(output.contains("[name-mismatch]"), "{output}");
    assert!(output.contains("[broken-skill-ref]"), "{output}");
    assert!(output.contains("[dangling-symlink]"), "{output}");
    assert!(output.contains("skills/dangling"), "{output}");
}

#[cfg(unix)]
#[test]
fn walk_fallback_reports_all_three_checks() {
    assert_all_three_fire(&run(&tree("claude-skill-walk")));
}

#[cfg(unix)]
#[test]
fn git_listing_reports_all_three_checks() {
    let dir = tree("claude-skill-git");
    init_git(&dir);
    assert_all_three_fire(&run(&dir));
}
