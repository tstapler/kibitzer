//! Shared helpers for the `claude-skill-*` checkers: locating a file inside a
//! `.claude/{skills,agents,commands}` tree, deriving its path-implied id, parsing simple
//! frontmatter, and building the skill-id registry. Semantics port `stapler-scripts/
//! lint-claude-kb` from tstapler/dotfiles; see `docs/claude-skill-checks.md`.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Skill,
    Agent,
    Command,
}

/// Where a file sits inside a `.claude/<kind>/` tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub kind: Kind,
    /// The `.claude` directory (as spelled in the input path, so relative stays relative).
    pub root: PathBuf,
    /// Path segments below `.claude/<kind>/`.
    pub rel: Vec<String>,
}

impl Located {
    /// True for a file Claude Code (or this checker family) treats as a definition: a
    /// skill's `SKILL.md`, an agent file, a command file. Skill support files
    /// (`references/*.md`, `scripts/`) are not definitions and carry no frontmatter rules.
    pub fn is_definition(&self) -> bool {
        let name = self.rel.last().map(String::as_str).unwrap_or("");
        if !name.ends_with(".md") {
            return false;
        }
        match self.kind {
            Kind::Skill => self.is_skill_md() || self.is_flat_skill_file(),
            Kind::Agent => self.rel.len() == 1,
            Kind::Command => !matches!(name, "CLAUDE.md" | "README.md"),
        }
    }

    fn is_skill_md(&self) -> bool {
        let last = self.rel.last().map(String::as_str);
        last == Some("SKILL.md")
            && (self.rel.len() == 2 || (self.rel.len() == 4 && self.rel[1] == "skills"))
    }

    /// A bare `<name>.md` directly under `.claude/skills/`: never discovered by Claude Code.
    /// `README.md` is documentation for the directory, not a skill (apache/cassandra ships one).
    pub fn is_flat_skill_file(&self) -> bool {
        self.kind == Kind::Skill
            && self.rel.len() == 1
            && self.rel[0].ends_with(".md")
            && self.rel[0] != "README.md"
    }

    /// The id `name:` must equal. `None` for commands, and for namespaced sub-skills
    /// (`ns/skills/sub/SKILL.md`), which conventionally omit `name:`.
    pub fn expected_id(&self) -> Option<String> {
        match self.kind {
            Kind::Command => None,
            Kind::Agent => self.rel.first().map(|f| strip_md(f)),
            Kind::Skill if self.is_flat_skill_file() => Some(strip_md(&self.rel[0])),
            Kind::Skill if self.rel.len() == 2 && self.is_skill_md() => Some(self.rel[0].clone()),
            Kind::Skill => None,
        }
    }
}

fn strip_md(file: &str) -> String {
    file.strip_suffix(".md").unwrap_or(file).to_string()
}

/// Finds the innermost `.claude/{skills,agents,commands}` ancestor of `file`.
pub fn locate(file: &Path) -> Option<Located> {
    let comps: Vec<Component> = file.components().collect();
    let names: Vec<String> = comps
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let idx = (0..names.len().saturating_sub(2))
        .rev()
        .find(|&i| names[i] == ".claude" && kind_of(&names[i + 1]).is_some())?;
    let kind = kind_of(&names[idx + 1])?;
    let root: PathBuf = comps[..=idx].iter().collect();
    Some(Located {
        kind,
        root,
        rel: names[idx + 2..].to_vec(),
    })
}

fn kind_of(dir: &str) -> Option<Kind> {
    match dir {
        "skills" => Some(Kind::Skill),
        "agents" => Some(Kind::Agent),
        "commands" => Some(Kind::Command),
        _ => None,
    }
}

/// One `key: value` frontmatter entry with its 1-indexed line in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub value: String,
    pub line: usize,
}

/// Parses the leading `---` block. `None` when there is no block. A block scalar
/// indicator (`>-`, `|`, ...) is dropped and its continuation lines are folded into the
/// value; enough for length/content checks without a YAML dependency.
pub fn parse_frontmatter(text: &str) -> Option<Vec<Entry>> {
    let mut lines = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .enumerate();
    let (_, first) = lines.next()?;
    if first.trim_end() != "---" {
        return None;
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (i, raw) in lines {
        let line = raw;
        if line.trim_end() == "---" {
            return Some(entries);
        }
        if let Some((key, value)) = split_key_value(line) {
            let value = if BLOCK_SCALAR_INDICATORS.contains(&value) {
                ""
            } else {
                value
            };
            entries.push(Entry {
                key: key.to_string(),
                value: value.to_string(),
                line: i + 1,
            });
        } else if line.starts_with([' ', '\t'])
            && !line.trim().is_empty()
            && let Some(last) = entries.last_mut()
        {
            last.value = format!("{} {}", last.value, line.trim()).trim().to_string();
        }
    }
    None // never closed
}

const BLOCK_SCALAR_INDICATORS: [&str; 6] = [">", ">-", ">+", "|", "|-", "|+"];

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once(':')?;
    let mut chars = key.chars();
    let first = chars.next()?;
    let valid = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then(|| (key, unquote(rest.trim())))
}

/// Strips one pair of matching surrounding quotes, or a trailing ` # comment` on an
/// unquoted scalar, so `name: "foo"` compares as `foo`.
fn unquote(value: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return inner;
        }
    }
    if !value.starts_with(['"', '\''])
        && let Some(i) = value.find(" #")
    {
        return value[..i].trim_end();
    }
    value
}

pub fn get<'a>(entries: &'a [Entry], key: &str) -> Option<&'a Entry> {
    entries.iter().find(|e| e.key == key)
}

const REGISTRY_TTL: Duration = Duration::from_secs(2);

type RegistryCache = Mutex<HashMap<PathBuf, (Instant, HashSet<String>)>>;
static REGISTRIES: LazyLock<RegistryCache> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Every known skill id under `<root>/skills`: `name` for `name/SKILL.md` or flat
/// `name.md`, `ns:sub` plus bare `sub` for `ns/skills/sub/SKILL.md`. Memoized per root for
/// a couple of seconds so a batch over N files does one scan, while a long-lived daemon
/// still notices a newly added skill.
pub fn skill_registry(root: &Path) -> HashSet<String> {
    // A relative root means a different directory per working directory, so it can't key a
    // process-wide cache.
    if !root.is_absolute() {
        return scan_skill_ids(&root.join("skills"));
    }
    {
        let cache = REGISTRIES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, ids)) = cache.get(root)
            && at.elapsed() < REGISTRY_TTL
        {
            return ids.clone();
        }
    }
    let ids = scan_skill_ids(&root.join("skills"));
    let mut cache = REGISTRIES.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, (at, _)| at.elapsed() < REGISTRY_TTL);
    cache.insert(root.to_path_buf(), (Instant::now(), ids.clone()));
    ids
}

fn scan_skill_ids(skills_dir: &Path) -> HashSet<String> {
    let mut ids = HashSet::new();
    let Ok(read) = std::fs::read_dir(skills_dir) else {
        return ids;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_file() {
            if name != "README.md" {
                ids.extend(name.strip_suffix(".md").map(str::to_string));
            }
        } else if path.is_dir() {
            add_dir_ids(&path, &name, &mut ids);
        }
    }
    ids
}

fn add_dir_ids(dir: &Path, name: &str, ids: &mut HashSet<String>) {
    if dir.join("SKILL.md").exists() {
        ids.insert(name.to_string());
    }
    let Ok(subs) = std::fs::read_dir(dir.join("skills")) else {
        return;
    };
    for sub in subs
        .flatten()
        .filter(|s| s.path().join("SKILL.md").exists())
    {
        let sub_name = sub.file_name().to_string_lossy().into_owned();
        ids.insert(format!("{name}:{sub_name}"));
        ids.insert(sub_name);
    }
}

/// A fresh empty directory under the system temp dir, for tests that need a real tree.
#[cfg(test)]
pub fn scratch_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "kibitzer-claude-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_and_commented_values_are_unwrapped() {
        let e = parse_frontmatter("---\nname: \"foo\"\ndescription: 'bar'\nx: baz # note\n---\n")
            .unwrap();
        assert_eq!(get(&e, "name").unwrap().value, "foo");
        assert_eq!(get(&e, "description").unwrap().value, "bar");
        assert_eq!(get(&e, "x").unwrap().value, "baz");
    }

    #[test]
    fn bom_and_trailing_space_on_fences_still_parse() {
        let e = parse_frontmatter("\u{feff}---\nname: a\n--- \nbody").unwrap();
        assert_eq!(get(&e, "name").unwrap().value, "a");
    }

    #[test]
    fn locate_classifies_each_tree() {
        let l = locate(Path::new("/r/.claude/skills/foo/SKILL.md")).unwrap();
        assert_eq!(l.kind, Kind::Skill);
        assert_eq!(l.root, Path::new("/r/.claude"));
        assert_eq!(l.expected_id().as_deref(), Some("foo"));
        assert_eq!(
            locate(Path::new(".claude/agents/a.md"))
                .unwrap()
                .expected_id()
                .as_deref(),
            Some("a")
        );
        assert_eq!(
            locate(Path::new(".claude/commands/x/y.md"))
                .unwrap()
                .expected_id(),
            None
        );
    }

    #[test]
    fn namespaced_sub_skill_has_no_expected_id() {
        let l = locate(Path::new(".claude/skills/ns/skills/sub/SKILL.md")).unwrap();
        assert!(l.is_definition());
        assert_eq!(l.expected_id(), None);
    }

    #[test]
    fn locate_rejects_paths_outside_the_trees() {
        assert!(locate(Path::new("docs/SKILL.md")).is_none());
        assert!(locate(Path::new(".claude/CLAUDE.md")).is_none());
        assert!(locate(Path::new(".claude/settings/x.md")).is_none());
    }

    #[test]
    fn support_files_are_not_definitions() {
        let l = locate(Path::new(".claude/skills/foo/references/a.md")).unwrap();
        assert!(!l.is_definition());
        let l = locate(Path::new(".claude/commands/README.md")).unwrap();
        assert!(!l.is_definition());
    }

    #[test]
    fn frontmatter_folds_block_scalars_and_tracks_lines() {
        let fm =
            parse_frontmatter("---\nname: a\ndescription: >-\n  one\n  two\n---\nbody").unwrap();
        assert_eq!(get(&fm, "name").unwrap().line, 2);
        assert_eq!(get(&fm, "description").unwrap().value, "one two");
    }

    #[test]
    fn frontmatter_absent_or_unclosed_is_none() {
        assert!(parse_frontmatter("# title\n").is_none());
        assert!(parse_frontmatter("---\nname: a\n").is_none());
    }
}
