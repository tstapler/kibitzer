use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde::Deserialize;

use super::claude_skill_common::{Located, locate, skill_registry};
use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::markdown_text::{line_for_offset, line_start_offsets};

/// A backtick token immediately followed by the word "skill" ("the `foo-bar` skill",
/// "`foo-bar`'s skill"). Ported from `lint-claude-kb`.
static SKILL_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([A-Za-z][\w:/@.-]*)`'?s?[ \t]+skill\b").expect("static regex"));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    /// Skills that exist but are not vendored under `.claude/skills` (Claude Code
    /// built-ins, installed plugins).
    #[serde(default)]
    external_skills: Vec<String>,
}

/// Flags a backtick token followed by "skill" that names no known skill: the skill-name
/// sibling of `markdown-link-integrity`. Known ids come from scanning `.claude/skills`
/// (`name`, `ns:sub`, bare `sub`); `external_skills` in the check's `options` extends them.
/// A `pkg@name` token is resolved by its `name` part. Files outside
/// `.claude/{skills,agents,commands}`, and trees with no skills at all, are skipped.
#[derive(Default)]
pub struct ClaudeSkillXrefChecker {
    external: HashSet<String>,
}

impl Checker for ClaudeSkillXrefChecker {
    fn name(&self) -> &str {
        "claude-skill-xref"
    }

    fn description(&self) -> &str {
        "flags backtick skill references that name no known skill"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        &["**/.claude/**/*.md"]
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let Some(loc) = locate(file).filter(Located::is_definition) else {
            return Ok(Vec::new());
        };
        let known = skill_registry(&loc.root);
        if known.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.broken_refs(ctx.source, &known))
    }

    fn configure(&self, options: &serde_json::Value) -> Result<Option<Box<dyn Checker>>> {
        let opts: Options = serde_json::from_value(options.clone())?;
        Ok(Some(Box::new(ClaudeSkillXrefChecker {
            external: opts.external_skills.into_iter().collect(),
        })))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(ClaudeSkillXrefChecker::default())])
}

impl ClaudeSkillXrefChecker {
    fn broken_refs(&self, source: &str, known: &HashSet<String>) -> Vec<Finding> {
        let starts = line_start_offsets(source);
        SKILL_REF
            .captures_iter(source)
            .filter_map(|c| {
                let token = &c[1];
                let name = token.rsplit('@').next().unwrap_or(token);
                let resolved = [name, token]
                    .iter()
                    .any(|t| known.contains(*t) || self.external.contains(*t));
                (!resolved).then(|| Finding {
                    line: line_for_offset(&starts, c.get(0).map_or(0, |m| m.start())),
                    message: format!(
                        "[broken-skill-ref] `{token}` skill: no skill named {name:?} found"
                    ),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkers::claude_skill_common::scratch_dir;

    /// A tree with skills `alpha`, flat `flat`, and namespaced `ns:sub`; returns a path to
    /// a command file inside it.
    fn tree() -> std::path::PathBuf {
        let root = scratch_dir("xref");
        let skills = root.join(".claude/skills");
        for rel in ["alpha/SKILL.md", "ns/skills/sub/SKILL.md"] {
            std::fs::create_dir_all(skills.join(rel).parent().unwrap()).unwrap();
            std::fs::write(skills.join(rel), "---\n---\n").unwrap();
        }
        std::fs::write(skills.join("flat.md"), "x").unwrap();
        std::fs::create_dir_all(root.join(".claude/commands")).unwrap();
        root.join(".claude/commands/cmd.md")
    }

    fn run(checker: &ClaudeSkillXrefChecker, file: &Path, src: &str) -> Vec<(usize, String)> {
        let ctx = CheckContext {
            source: src,
            tree: None,
        };
        checker
            .check(file, &ctx)
            .unwrap()
            .into_iter()
            .map(|f| (f.line, f.message))
            .collect()
    }

    #[test]
    fn prose_without_skill_references_is_clean() {
        let f = tree();
        assert!(
            run(
                &Default::default(),
                &f,
                "use `ghost` carefully, then `ghost` again\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn known_flat_and_namespaced_references_resolve() {
        let f = tree();
        let src = "the `alpha` skill, `flat` skill, `ns:sub` skill, `sub`'s skill\n";
        assert!(run(&Default::default(), &f, src).is_empty());
    }

    #[test]
    fn unknown_reference_is_reported_on_its_line() {
        let f = tree();
        let out = run(&Default::default(), &f, "ok\n\nrun the `ghost` skill now\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, 3);
        assert!(out[0].1.contains("[broken-skill-ref] `ghost`"));
    }

    #[test]
    fn pkg_at_name_resolves_by_its_name_part() {
        let f = tree();
        assert!(run(&Default::default(), &f, "`pkg/x@alpha` skill\n").is_empty());
        assert_eq!(
            run(&Default::default(), &f, "`pkg/x@nope` skill\n").len(),
            1
        );
    }

    #[test]
    fn configure_external_skills_suppresses_findings() {
        let f = tree();
        let configured = ClaudeSkillXrefChecker::default()
            .configure(&serde_json::json!({"external_skills": ["security-review"]}))
            .unwrap()
            .expect("configured instance");
        let ctx = CheckContext {
            source: "the `security-review` skill and `ghost` skill\n",
            tree: None,
        };
        let out = configured.check(&f, &ctx).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("`ghost`"));
    }

    #[test]
    fn configure_rejects_malformed_options() {
        assert!(
            ClaudeSkillXrefChecker::default()
                .configure(&serde_json::json!({"external_skill": []}))
                .is_err()
        );
    }

    #[test]
    fn file_outside_a_claude_tree_is_skipped() {
        let ctx = CheckContext {
            source: "the `ghost` skill\n",
            tree: None,
        };
        let out = ClaudeSkillXrefChecker::default()
            .check(Path::new("docs/notes.md"), &ctx)
            .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn tree_with_no_skills_is_skipped() {
        let root = scratch_dir("xref-empty");
        std::fs::create_dir_all(root.join(".claude/commands")).unwrap();
        let f = root.join(".claude/commands/c.md");
        assert!(run(&Default::default(), &f, "the `ghost` skill\n").is_empty());
    }
}
