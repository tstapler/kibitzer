use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

use super::claude_skill_common::{Entry, Kind, Located, get, locate, parse_frontmatter};
use crate::checker::{CheckContext, Checker, Finding, Language};

const NAME_MAX_CHARS: usize = 64;
const DESCRIPTION_MAX_CHARS: usize = 1024;

/// Frontmatter keys that are real and documented but make a skill unreachable right now.
/// `paths:` -- https://github.com/anthropics/claude-code/issues/49835: a `paths:`-scoped
/// skill is absent from discovery and fails explicit `Skill` invocation until Claude
/// touches a matching file. Re-check when that issue ships a fix; drop the entry then.
const BROKEN_KEYS: &[(&str, &str)] = &[(
    "paths",
    "stays absent from skill discovery until Claude touches a matching file (anthropics/claude-code#49835)",
)];

/// Skill frontmatter fields per https://code.claude.com/docs/en/skills#frontmatter-reference
/// (snapshot, Claude Code 2.1.296) plus the Agent Skills spec fields. Agents and commands
/// have their own sets and are not key-checked.
const SKILL_KEYS: &[&str] = &[
    "name",
    "description",
    "when_to_use",
    "argument-hint",
    "arguments",
    "disable-model-invocation",
    "user-invocable",
    "allowed-tools",
    "disallowed-tools",
    "model",
    "effort",
    "context",
    "agent",
    "background",
    "hooks",
    "paths",
    "shell",
    "metadata",
    "license",
    "compatibility",
];

const UNKNOWN_KEY_HINTS: &[(&str, &str)] = &[
    (
        "tools",
        "not a skill field (agents use it); use `allowed-tools:`, which pre-approves rather than restricts",
    ),
    (
        "prompt",
        "not a skill field; put the instructions in the markdown body",
    ),
];

static TAG_OPEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<([a-zA-Z][\w-]*)\b[^>]*>").expect("static regex"));

/// A paired `<tag>...</tag>`, not a bare `<placeholder>`: angle-bracket placeholders are
/// ordinary prose in skill text, and only structural markup trips Anthropic's rule.
fn has_xml_tag_pair(text: &str) -> bool {
    TAG_OPEN.captures_iter(text).any(|c| {
        let end = c.get(0).map_or(0, |m| m.end());
        text[end..].contains(&format!("</{}>", &c[1]))
    })
}

/// Validates the frontmatter of Claude Code skills, agents, and commands: `name:` matching
/// the path-implied id and Anthropic's documented limits, `description:` presence and
/// limits, keys that are unknown or currently broken, and a bare `<name>.md` directly
/// under `.claude/skills/` (never discovered). Ports `lint-claude-kb` from tstapler/dotfiles.
/// Files outside `.claude/{skills,agents,commands}` and files with no frontmatter block
/// are skipped.
pub struct ClaudeSkillFrontmatterChecker;

impl Checker for ClaudeSkillFrontmatterChecker {
    fn name(&self) -> &str {
        "claude-skill-frontmatter"
    }

    fn description(&self) -> &str {
        "flags invalid Claude Code skill/agent/command frontmatter"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        &["**/.claude/**/*.md"]
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        Ok(locate(file)
            .filter(Located::is_definition)
            .map(|loc| check_source(&loc, ctx.source))
            .unwrap_or_default())
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(ClaudeSkillFrontmatterChecker)])
}

fn finding(line: usize, id: &str, detail: impl std::fmt::Display) -> Finding {
    Finding {
        line,
        message: format!("[{id}] {detail}"),
    }
}

fn check_source(loc: &Located, source: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    if loc.is_flat_skill_file() {
        out.push(invalid_location(loc));
    }
    let Some(fm) = parse_frontmatter(source) else {
        return out;
    };
    if get(&fm, "description").is_none_or(|e| e.value.is_empty()) {
        out.push(finding(
            1,
            "missing-description",
            "has frontmatter but no description: field",
        ));
    }
    if let Some(name) = get(&fm, "name").filter(|e| !e.value.is_empty()) {
        out.extend(name_findings(loc, name));
    }
    if loc.kind != Kind::Agent
        && let Some(desc) = get(&fm, "description").filter(|e| !e.value.is_empty())
    {
        out.extend(description_findings(desc));
    }
    if loc.kind == Kind::Skill {
        out.extend(key_findings(&fm));
    }
    out.sort_by_key(|f| f.line);
    out
}

fn invalid_location(loc: &Located) -> Finding {
    let stem = loc.rel[0].strip_suffix(".md").unwrap_or(&loc.rel[0]);
    let dup = loc.root.join("skills").join(stem).join("SKILL.md").exists();
    let detail = if dup {
        format!("dead duplicate: {stem}/SKILL.md already exists and is what Claude Code loads")
    } else {
        format!(
            "never discoverable: Claude Code only loads `<name>/SKILL.md` directories; move to {stem}/SKILL.md"
        )
    };
    finding(1, "invalid-skill-location", detail)
}

fn name_findings(loc: &Located, name: &Entry) -> Vec<Finding> {
    let mut out = Vec::new();
    let value = name.value.as_str();
    if let Some(expected) = loc.expected_id()
        && value != expected
    {
        out.push(finding(
            name.line,
            "name-mismatch",
            format!("frontmatter name: {value:?} != expected {expected:?}"),
        ));
    }
    let chars = value.chars().count();
    if chars > NAME_MAX_CHARS {
        out.push(finding(
            name.line,
            "name-too-long",
            format!("{chars} chars > {NAME_MAX_CHARS}-char max"),
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        out.push(finding(
            name.line,
            "name-invalid-chars",
            format!("{value:?} must be lowercase letters/numbers/hyphens only"),
        ));
    }
    if has_xml_tag_pair(value) {
        out.push(finding(
            name.line,
            "name-xml-tag",
            format!("{value:?} contains an XML-like tag"),
        ));
    }
    out
}

fn description_findings(desc: &Entry) -> Vec<Finding> {
    let mut out = Vec::new();
    let chars = desc.value.chars().count();
    if chars > DESCRIPTION_MAX_CHARS {
        out.push(finding(
            desc.line,
            "description-too-long",
            format!("{chars} chars > {DESCRIPTION_MAX_CHARS}-char max"),
        ));
    }
    if has_xml_tag_pair(&desc.value) {
        out.push(finding(
            desc.line,
            "description-xml-tag",
            "description contains a paired XML-like tag",
        ));
    }
    out
}

fn key_findings(fm: &[Entry]) -> Vec<Finding> {
    let mut out = Vec::new();
    for e in fm {
        if let Some((_, why)) = BROKEN_KEYS.iter().find(|(k, _)| *k == e.key) {
            out.push(finding(
                e.line,
                "broken-frontmatter-key",
                format!("`{}:` {why}; remove it", e.key),
            ));
        }
        if !SKILL_KEYS.contains(&e.key.as_str()) {
            let hint = UNKNOWN_KEY_HINTS.iter().find(|(k, _)| *k == e.key).map_or(
                "ignored by Claude Code; move it under `metadata:` or delete it",
                |(_, h)| h,
            );
            out.push(finding(
                e.line,
                "unknown-frontmatter-key",
                format!("`{}:` {hint}", e.key),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(path: &str, src: &str) -> Vec<String> {
        let ctx = CheckContext {
            source: src,
            tree: None,
        };
        ClaudeSkillFrontmatterChecker
            .check(Path::new(path), &ctx)
            .unwrap()
            .into_iter()
            .map(|f| format!("{}:{}", f.line, f.message))
            .collect()
    }

    const SKILL: &str = ".claude/skills/foo/SKILL.md";

    #[test]
    fn clean_skill_passes() {
        assert!(run(SKILL, "---\nname: foo\ndescription: does foo\n---\nbody").is_empty());
    }

    #[test]
    fn name_mismatch_reports_the_name_line() {
        let out = run(SKILL, "---\nname: bar\ndescription: d\n---\n");
        assert_eq!(out.len(), 1);
        assert!(out[0].starts_with("2:[name-mismatch]"), "{out:?}");
    }

    #[test]
    fn namespaced_sub_skill_may_omit_or_choose_name() {
        let p = ".claude/skills/ns/skills/sub/SKILL.md";
        assert!(run(p, "---\ndescription: d\n---\n").is_empty());
        assert!(run(p, "---\nname: anything\ndescription: d\n---\n").is_empty());
    }

    #[test]
    fn missing_description_anchors_to_line_one() {
        let out = run(SKILL, "---\nname: foo\n---\n");
        assert_eq!(
            out,
            ["1:[missing-description] has frontmatter but no description: field"]
        );
    }

    #[test]
    fn no_frontmatter_block_is_skipped() {
        assert!(run(SKILL, "# just prose\n").is_empty());
    }

    #[test]
    fn name_limits() {
        let long = "a".repeat(65);
        let out = run(SKILL, &format!("---\nname: {long}\ndescription: d\n---\n"));
        assert!(out.iter().any(|m| m.contains("[name-too-long]")));
        let ok = "a".repeat(64);
        assert!(
            run(SKILL, &format!("---\nname: {ok}\ndescription: d\n---\n"))
                .iter()
                .all(|m| !m.contains("[name-too-long]"))
        );
        let out = run(SKILL, "---\nname: Foo_Bar\ndescription: d\n---\n");
        assert!(out.iter().any(|m| m.contains("[name-invalid-chars]")));
        let out = run(SKILL, "---\nname: <b>x</b>\ndescription: d\n---\n");
        assert!(out.iter().any(|m| m.contains("[name-xml-tag]")));
    }

    #[test]
    fn description_limits_and_placeholder_edge_case() {
        let long = "a".repeat(1025);
        let out = run(
            SKILL,
            &format!("---\nname: foo\ndescription: {long}\n---\n"),
        );
        assert!(out.iter().any(|m| m.contains("[description-too-long]")));
        let ok = "a".repeat(1024);
        assert!(run(SKILL, &format!("---\nname: foo\ndescription: {ok}\n---\n")).is_empty());
        let out = run(SKILL, "---\nname: foo\ndescription: <x>a</x>\n---\n");
        assert!(out.iter().any(|m| m.contains("[description-xml-tag]")));
        // A bare `<placeholder>` is not an XML pair.
        assert!(
            run(
                SKILL,
                "---\nname: foo\ndescription: use <project> here\n---\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn agents_are_exempt_from_description_limits() {
        let src = "---\nname: a\ndescription: <example>x</example>\n---\n";
        assert!(run(".claude/agents/a.md", src).is_empty());
        assert!(!run(".claude/commands/a.md", src).is_empty());
    }

    #[test]
    fn broken_and_unknown_keys() {
        let out = run(
            SKILL,
            "---\nname: foo\ndescription: d\npaths: x\nversion: 1\ntools: y\n---\n",
        );
        assert!(
            out.iter()
                .any(|m| m.starts_with("4:[broken-frontmatter-key]"))
        );
        assert!(
            out.iter()
                .any(|m| m.starts_with("5:[unknown-frontmatter-key]"))
        );
        assert!(out.iter().any(|m| m.contains("allowed-tools")));
    }

    #[test]
    fn unknown_keys_are_not_checked_on_agents() {
        assert!(
            run(
                ".claude/agents/a.md",
                "---\nname: a\ndescription: d\ntools: Read\n---\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn bare_md_under_skills_is_invalid_location_even_without_frontmatter() {
        let out = run(".claude/skills/foo.md", "# no frontmatter\n");
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("[invalid-skill-location]"), "{out:?}");
    }

    #[test]
    fn readme_under_skills_is_documentation_not_a_skill() {
        assert!(run(".claude/skills/README.md", "# Skills\n").is_empty());
    }

    #[test]
    fn files_outside_the_trees_and_support_files_are_ignored() {
        let bad = "---\nname: Bad Name\n---\n";
        assert!(run(".claude/CLAUDE.md", bad).is_empty());
        assert!(run("docs/SKILL.md", bad).is_empty());
        assert!(run(".claude/skills/foo/references/x.md", bad).is_empty());
    }
}
