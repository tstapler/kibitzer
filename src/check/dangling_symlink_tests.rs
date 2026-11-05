use std::os::unix::fs::symlink;
use std::path::Path;

use super::dangling_claude_symlinks;
use crate::checkers::claude_skill_common::scratch_dir;

fn mkdir(p: &Path) {
    std::fs::create_dir_all(p).unwrap();
}

#[test]
fn finds_every_kind_in_nested_roots_and_ignores_the_rest() {
    let d = scratch_dir("dangling-discovery");
    for kind in ["skills", "agents", "commands"] {
        mkdir(&d.join(format!(".claude/{kind}")));
        symlink("../nowhere", d.join(format!(".claude/{kind}/dead"))).unwrap();
    }
    mkdir(&d.join("pkg/sub/.claude/skills"));
    symlink("/nonexistent", d.join("pkg/sub/.claude/skills/only-link")).unwrap();
    std::fs::write(d.join(".claude/skills/real.md"), "x").unwrap();
    symlink("real.md", d.join(".claude/skills/live")).unwrap();
    mkdir(&d.join(".claude/other"));
    symlink("../nowhere", d.join(".claude/other/dead")).unwrap();
    mkdir(&d.join(".claude/skills/node_modules"));
    symlink("../nowhere", d.join(".claude/skills/node_modules/dead")).unwrap();

    let got: Vec<String> = dangling_claude_symlinks(&d)
        .iter()
        .map(|p| p.strip_prefix(&d).unwrap().display().to_string())
        .collect();
    assert_eq!(
        got,
        [
            ".claude/agents/dead",
            ".claude/commands/dead",
            ".claude/skills/dead",
            "pkg/sub/.claude/skills/only-link",
        ]
    );
}

#[test]
fn running_inside_a_claude_tree_is_scoped_to_the_requested_directory() {
    let d = scratch_dir("dangling-scope");
    mkdir(&d.join(".claude/skills"));
    mkdir(&d.join(".claude/commands"));
    symlink("../nowhere", d.join(".claude/skills/dead")).unwrap();
    symlink("../nowhere", d.join(".claude/commands/dead")).unwrap();

    let got = dangling_claude_symlinks(&d.join(".claude/skills"));
    assert_eq!(got, [d.join(".claude/skills/dead")]);
}

#[test]
fn a_directory_symlink_loop_terminates_without_phantom_findings() {
    let d = scratch_dir("dangling-loop");
    mkdir(&d.join(".claude/skills/a"));
    for n in ["l1", "l2", "l3"] {
        symlink("..", d.join(format!(".claude/skills/a/{n}"))).unwrap();
    }
    assert!(dangling_claude_symlinks(&d).is_empty());
}
