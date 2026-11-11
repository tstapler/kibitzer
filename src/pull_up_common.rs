//! Shared engine for `pull-up-method` and `pull-up-constructor-body`: for each supertype
//! with ≥2 direct subtypes, hash the normalised body of each subtype's same-named method
//! (sliced from source via `SymbolNode::line..=end_line`) and report groups of ≥2
//! identical, same-signature bodies. A method with `end_line == None`, an unreadable
//! file, or a trivial (<2 line) body is skipped.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::arch_model::{ArchModel, SymbolNode};
use crate::architecture_checks::ArchFinding;
use crate::hierarchy_checks::{
    is_ignorable_file, methods_of, resolved_extends, subtypes_by_super, type_index,
};

/// Bodies shorter than this many non-blank lines are too generic to flag.
const MIN_BODY_LINES: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemberKind {
    Method,
    Constructor,
}

impl MemberKind {
    fn tag(self) -> &'static str {
        match self {
            MemberKind::Method => "pull-up-method",
            MemberKind::Constructor => "pull-up-constructor-body",
        }
    }
}

/// TS/Kotlin: a method named `constructor`. Java: a method named after its owning type.
pub(crate) fn is_constructor(sym: &SymbolNode) -> bool {
    sym.name == "constructor"
        || (sym.file.extension().is_some_and(|e| e == "java")
            && sym.parent.as_deref() == Some(sym.name.as_str()))
}

/// Java constructors are named after their (differing) owners; group them under one key.
fn member_key(m: &SymbolNode) -> String {
    if is_constructor(m) {
        "constructor".to_string()
    } else {
        m.name.clone()
    }
}

pub(crate) fn find_pull_up_candidates(model: &ArchModel, kind: MemberKind) -> Vec<ArchFinding> {
    let types = type_index(model);
    let pairs = resolved_extends(model, &types);
    let children = subtypes_by_super(&pairs);
    let mut sources = SourceCache::default();
    let mut findings = Vec::new();

    for (&sup_id, subs) in &children {
        let (sup_pkg, sup) = types[sup_id];
        let subs: Vec<_> = subs
            .iter()
            .copied()
            .filter(|s| {
                let (_, st) = types[s];
                st.name != sup.name && !is_ignorable_file(model, &st.file)
            })
            .collect();
        if subs.len() < 2 {
            continue;
        }
        let sup_method_names: Vec<&str> = methods_of(model, sup_pkg, &sup.name)
            .iter()
            .map(|m| m.name.as_str())
            .collect();

        type Key = (String, Vec<(String, String)>, String);
        let mut groups: BTreeMap<Key, Vec<(&str, &SymbolNode)>> = BTreeMap::new();
        for &sub_id in &subs {
            let (pkg, st) = types[sub_id];
            for m in methods_of(model, pkg, &st.name) {
                if is_constructor(m) != (kind == MemberKind::Constructor) {
                    continue;
                }
                let Some(body) = sources.normalised_body(model, m) else {
                    continue;
                };
                groups
                    .entry((member_key(m), m.params.clone(), body))
                    .or_default()
                    .push((sub_id, m));
            }
        }
        for ((name, _, _), members) in groups {
            let mut owners: Vec<&str> = members.iter().map(|(s, _)| *s).collect();
            owners.dedup();
            if owners.len() < 2 {
                continue;
            }
            if kind == MemberKind::Method && sup_method_names.contains(&name.as_str()) {
                continue;
            }
            let (_, first) = members[0];
            findings.push(ArchFinding {
                file: Some(first.file.clone()),
                line: Some(first.line),
                message: format!(
                    "[{}] '{name}' has an identical body in sibling subtypes {} of {sup_id} — \
                     consider pulling it up (Fowler, {})",
                    kind.tag(),
                    owners.join(", "),
                    match kind {
                        MemberKind::Method => "Pull Up Method",
                        MemberKind::Constructor => "Pull Up Constructor Body",
                    }
                ),
                severity_override: None,
                operation: None,
            });
        }
    }
    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    findings
}

#[derive(Default)]
struct SourceCache {
    files: HashMap<PathBuf, Option<Vec<String>>>,
}

impl SourceCache {
    fn lines(&mut self, model: &ArchModel, file: &Path) -> Option<&Vec<String>> {
        self.files
            .entry(file.to_path_buf())
            .or_insert_with(|| {
                std::fs::read_to_string(file)
                    .or_else(|_| std::fs::read_to_string(model.repo_root.join(file)))
                    .ok()
                    .map(|s| s.lines().map(str::to_string).collect())
            })
            .as_ref()
    }

    fn normalised_body(&mut self, model: &ArchModel, m: &SymbolNode) -> Option<String> {
        let end = m.end_line?;
        if end < m.line || m.line == 0 {
            return None;
        }
        let lines = self.lines(model, &m.file)?;
        let slice = lines.get(m.line - 1..end.min(lines.len()))?;
        normalise_body(slice)
    }
}

/// Text between the signature's opening `{` and the last `}`, whitespace-collapsed,
/// comment-only lines dropped, Go receiver name replaced by `RECV`. `None` for a
/// brace-less body (e.g. Kotlin expression body) or one under [`MIN_BODY_LINES`].
fn normalise_body(slice: &[String]) -> Option<String> {
    let text = slice.join("\n");
    let open = text.find('{')?;
    let close = text.rfind('}')?;
    if close <= open {
        return None;
    }
    let mut body = text[open + 1..close].to_string();
    if let Some(recv) = go_receiver_name(slice.first()?) {
        body = replace_ident(&body, recv, "RECV");
    }
    let norm: Vec<String> = body
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| {
            !l.is_empty() && !l.starts_with("//") && !l.starts_with('*') && !l.starts_with("/*")
        })
        .collect();
    (norm.len() >= MIN_BODY_LINES).then(|| norm.join("\n"))
}

/// `func (r *T) M(` -> `r`.
fn go_receiver_name(first_line: &str) -> Option<&str> {
    let rest = first_line.trim_start().strip_prefix("func")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    let name = &rest[..end];
    (!name.is_empty() && name != "_").then_some(name)
}

fn replace_ident(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut ident = String::new();
    let flush = |ident: &mut String, out: &mut String| {
        out.push_str(if ident == from { to } else { ident });
        ident.clear();
    };
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            ident.push(c);
        } else {
            flush(&mut ident, &mut out);
            out.push(c);
        }
    }
    flush(&mut ident, &mut out);
    out
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fresh scratch dir (no "test" path component: `is_test_file` would skip it).
    pub fn scratch_dir() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "kib-pullup-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn go_receiver_names_are_normalised() {
        let a = normalise_body(&lines("func (a *A) Do() {\n\ta.x++\n\ta.y()\n}"));
        let b = normalise_body(&lines("func (b *B) Do() {\n\tb.x++\n\tb.y()\n}"));
        assert!(a.is_some());
        assert_eq!(a, b);
    }

    #[test]
    fn whitespace_and_comments_are_ignored_but_code_differences_are_not() {
        let a = normalise_body(&lines("void m() {\n  // hi\n  a();\n    b();\n}"));
        let b = normalise_body(&lines("void m() {\n  a();\n  b();\n}"));
        let c = normalise_body(&lines("void m() {\n  a();\n  c();\n}"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn trivial_or_braceless_bodies_are_skipped() {
        assert_eq!(normalise_body(&lines("int m() {\n  return 1;\n}")), None);
        assert_eq!(normalise_body(&lines("fun m() = 1")), None);
    }

    #[test]
    fn constructor_detection() {
        use crate::arch_model::SymbolKind;
        use crate::hierarchy_checks::fixtures::sym;
        let mut ts = sym("p", "constructor", SymbolKind::Method, Some("A"));
        assert!(is_constructor(&ts));
        ts.name = "run".into();
        assert!(!is_constructor(&ts));
        let mut j = sym("p", "A", SymbolKind::Method, Some("A"));
        assert!(
            !is_constructor(&j),
            "name==owner outside .java is not a ctor"
        );
        j.file = PathBuf::from("/r/A.java");
        assert!(is_constructor(&j));
    }
}
