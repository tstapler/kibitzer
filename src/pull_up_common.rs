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
    go_subtype_is_foreign, is_ignorable_file, methods_of, resolved_extends, subtypes_by_super,
    type_index,
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

/// A method whose line range lies inside another function/method of the same package and
/// file is a member of an anonymous or local class; the model attributes it to the
/// enclosing named class, which would make a pull-up finding meaningless.
fn is_nested(pkg: &crate::arch_model::PackageNode, m: &SymbolNode) -> bool {
    let Some(end) = m.end_line else {
        return false;
    };
    pkg.symbols.iter().any(|o| {
        o.id != m.id
            && o.file == m.file
            && matches!(
                o.kind,
                crate::arch_model::SymbolKind::Method | crate::arch_model::SymbolKind::Function
            )
            && o.end_line
                .is_some_and(|oe| o.line <= m.line && oe >= end && (o.line, oe) != (m.line, end))
    })
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
                let (spkg, st) = types[s];
                st.name != sup.name
                    && !is_ignorable_file(model, &st.file)
                    && !go_subtype_is_foreign(spkg, st, sup_pkg)
            })
            .collect();
        if subs.len() < 2 {
            continue;
        }
        let total_direct_subtypes = children[sup_id].len();
        let sup_method_names: Vec<&str> = methods_of(model, sup_pkg, &sup.name)
            .iter()
            .map(|m| m.name.as_str())
            .collect();

        type Key = (String, Vec<(String, String)>, String, String);
        let mut groups: BTreeMap<Key, Vec<(&str, &SymbolNode)>> = BTreeMap::new();
        for &sub_id in &subs {
            let (pkg, st) = types[sub_id];
            for m in methods_of(model, pkg, &st.name) {
                if is_constructor(m) != (kind == MemberKind::Constructor) || is_nested(pkg, m) {
                    continue;
                }
                let Some((sig, body)) = sources.signature_and_body(model, m) else {
                    continue;
                };
                if uses_upper_snake_constant(&body) {
                    continue;
                }
                groups
                    .entry((member_key(m), m.params.clone(), sig, body))
                    .or_default()
                    .push((sub_id, m));
            }
        }
        let sup_fields = model.struct_field_names.get(sup_id);
        for ((name, _, _, body), members) in groups {
            let mut owners: Vec<&str> = members.iter().map(|(s, _)| *s).collect();
            owners.dedup();
            if owners.len() < 2 {
                continue;
            }
            // Same-named "siblings" are copies of one class (vendored/forked), and their
            // by-name supertype resolution is unreliable.
            let mut names: Vec<&str> = owners.iter().map(|o| types[o].1.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            if names.len() != owners.len() {
                continue;
            }
            // A method only a minority of subtypes share points at a generic base
            // (`Disposable`), not a missing common method.
            if kind == MemberKind::Method && owners.len() * 2 <= total_direct_subtypes {
                continue;
            }
            if kind == MemberKind::Method && sup_method_names.contains(&name.as_str()) {
                continue;
            }
            let is_go = members[0].1.file.extension().is_some_and(|e| e == "go");
            if is_go && !go_body_pullable(&body, sup_fields, &sup_method_names) {
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

    /// `(parameter-list text, normalised body)`. The text is needed because `params` is only
    /// populated for Go/TS/JS: without it, same-text Java constructors taking different
    /// parameter *types* would compare equal. Go gets `""` (its receiver differs by design).
    fn signature_and_body(
        &mut self,
        model: &ArchModel,
        m: &SymbolNode,
    ) -> Option<(String, String)> {
        let body = self.normalised_body(model, m)?;
        let sig = if m.file.extension().is_some_and(|e| e == "go") {
            String::new()
        } else {
            let lines = self.lines(model, &m.file)?;
            signature_params_text(lines.get(m.line - 1..m.end_line?.min(lines.len()))?)
        };
        Some((sig, body))
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

/// Whitespace-collapsed text of the first balanced `( ... )` in the signature lines (up to
/// the opening-brace line).
fn signature_params_text(slice: &[String]) -> String {
    let end = slice
        .iter()
        .position(|l| l.trim_end().ends_with('{'))
        .unwrap_or(slice.len().saturating_sub(1));
    let text = slice[..=end.min(slice.len().saturating_sub(1))].join(" ");
    let Some(start) = text.find('(') else {
        return String::new();
    };
    let mut depth = 0usize;
    let mut out = String::new();
    for c in text[start..].chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        out.push(c);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A `FOO_BAR` token: a constant that may resolve to a different value in each sibling.
fn uses_upper_snake_constant(body: &str) -> bool {
    body.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|t| {
            t.len() >= 3
                && t.contains('_')
                && t.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
}

/// The lines between the signature's opening-brace line (the first line ending in `{`, so
/// a `{` inside a return type like Go's `interface{}` is not mistaken for it) and the
/// closing `}` line, whitespace-collapsed, comment-only lines dropped, Go receiver name
/// replaced by `RECV`. `None` for a brace-less body (e.g. Kotlin expression body), a
/// one-line `{ ... }` body, or one under [`MIN_BODY_LINES`].
fn normalise_body(slice: &[String]) -> Option<String> {
    let open = slice.iter().position(|l| l.trim_end().ends_with('{'))?;
    let last = slice.len().checked_sub(1)?;
    if open >= last || slice[last].trim() != "}" {
        return None;
    }
    let mut body = slice[open + 1..last].join("\n");
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

/// A Go base type cannot see the struct that embeds it, so a text-identical body only moves
/// up if every use of the receiver (normalised to `RECV`) is `RECV.<member>` with `member`
/// declared on the supertype itself; a bare `RECV` or an outer-only member means the same
/// text binds to different concrete types in each sibling.
fn go_body_pullable(
    body: &str,
    sup_fields: Option<&std::collections::BTreeSet<String>>,
    sup_methods: &[&str],
) -> bool {
    let mut rest = body;
    while let Some(i) = rest.find("RECV") {
        let before_ok = !rest[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_');
        let after = &rest[i + 4..];
        rest = after;
        if !before_ok || after.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let Some(member_text) = after.strip_prefix('.') else {
            return false;
        };
        let end = member_text
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(member_text.len());
        let member = &member_text[..end];
        if !(sup_fields.is_some_and(|f| f.contains(member)) || sup_methods.contains(&member)) {
            return false;
        }
    }
    true
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
    fn brace_in_a_go_return_type_is_not_the_body_opener() {
        let one_liner = normalise_body(&lines("func (m *M) Value() interface{} {\n\treturn m\n}"));
        assert_eq!(
            one_liner, None,
            "a one-statement body stays too small to flag"
        );
    }

    #[test]
    fn go_bodies_are_pullable_only_through_supertype_declared_members() {
        let fields = std::collections::BTreeSet::from(["n".to_string()]);
        assert!(go_body_pullable(
            "RECV.n++\nRECV.log()",
            Some(&fields),
            &["log"]
        ));
        assert!(
            !go_body_pullable("RECV.Opts.mark()", Some(&fields), &["log"]),
            "outer-only member"
        );
        assert!(!go_body_pullable("return RECV", None, &[]), "bare receiver");
        assert!(
            go_body_pullable("x := 1\nuse(x)", None, &[]),
            "no receiver use"
        );
        assert!(
            go_body_pullable("MYRECV.q()\nRECVX.z()", None, &[]),
            "not the receiver token"
        );
    }

    #[test]
    fn signature_params_text_captures_types_and_ignores_layout() {
        let a = signature_params_text(&lines(
            "  private A(JavaDriverClient client,\n     String q)\n  {",
        ));
        let b = signature_params_text(&lines("  private B(SimpleClient client, String q) {"));
        assert_eq!(a, "(JavaDriverClient client, String q");
        assert_ne!(a, b);
    }

    #[test]
    fn upper_snake_constants_are_detected() {
        assert!(uses_upper_snake_constant("use(DEFAULT_URL)"));
        assert!(!uses_upper_snake_constant("use(defaultUrl, HTTP, a_b)"));
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
