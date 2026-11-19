use std::path::Path;

use anyhow::Result;

use super::claude_skill_common::locate;
use crate::checker::{CheckContext, Checker, Finding, Language};

/// Directories whose symlinks legitimately dangle until a package install runs.
const SKIP_SEGMENTS: &[&str] = &["node_modules", ".venv", ".git"];

/// Flags a symlink under `.claude/{skills,agents,commands}` whose target does not
/// resolve. The finding is attributed to the symlink itself (line 1), so it needs a
/// walked path rather than file contents: the runner calls this checker even when the
/// path cannot be read (see [`Checker::inspects_unreadable_paths`]), which is why it is
/// wired batch-only in `default_checks()` and not as a per-edit check.
pub struct ClaudeSkillDanglingSymlinkChecker;

impl Checker for ClaudeSkillDanglingSymlinkChecker {
    fn name(&self) -> &str {
        "claude-skill-dangling-symlink"
    }

    fn description(&self) -> &str {
        "flags dangling symlinks under .claude/{skills,agents,commands}"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        &["**/.claude/**"]
    }

    fn inspects_unreadable_paths(&self) -> bool {
        true
    }

    fn check(&self, file: &Path, _ctx: &CheckContext) -> Result<Vec<Finding>> {
        Ok(dangling_target(file)
            .map(|target| Finding {
                line: 1,
                message: format!("[dangling-symlink] target does not resolve: {target}"),
            })
            .into_iter()
            .collect())
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(ClaudeSkillDanglingSymlinkChecker)])
}

fn dangling_target(file: &Path) -> Option<String> {
    let loc = locate(file)?;
    // Only segments below `.claude/<kind>/`: a repo that itself lives under `.git` or
    // `node_modules` must still be checked.
    let skipped = loc.rel.iter().any(|s| SKIP_SEGMENTS.contains(&s.as_str()));
    if skipped || !file.is_symlink() || file.exists() {
        return None;
    }
    Some(
        std::fs::read_link(file)
            .map(|t| t.display().to_string())
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkers::claude_skill_common::scratch_dir;

    fn run(file: &Path) -> Vec<Finding> {
        let ctx = CheckContext {
            source: "",
            tree: None,
        };
        ClaudeSkillDanglingSymlinkChecker.check(file, &ctx).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_is_reported_against_the_link_itself() {
        let root = scratch_dir("symlink");
        let skills = root.join(".claude/skills");
        std::fs::create_dir_all(&skills).unwrap();
        let link = skills.join("ghost");
        std::os::unix::fs::symlink("../nowhere", &link).unwrap();
        let out = run(&link);
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("../nowhere"));
    }

    #[cfg(unix)]
    #[test]
    fn resolving_symlink_and_regular_file_are_clean() {
        let root = scratch_dir("symlink-ok");
        let skills = root.join(".claude/skills");
        std::fs::create_dir_all(skills.join("real")).unwrap();
        let link = skills.join("alias");
        std::os::unix::fs::symlink("real", &link).unwrap();
        assert!(run(&link).is_empty());
        assert!(run(&skills.join("real")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_inside_node_modules_is_skipped() {
        let root = scratch_dir("symlink-nm");
        let dir = root.join(".claude/skills/x/node_modules");
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("dep");
        std::os::unix::fs::symlink("missing", &link).unwrap();
        assert!(run(&link).is_empty());
    }

    #[test]
    fn path_outside_a_claude_tree_is_ignored() {
        assert!(run(Path::new("docs/whatever")).is_empty());
    }
}
