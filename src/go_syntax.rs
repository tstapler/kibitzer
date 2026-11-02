//! Tree-sitter-backed Go source queries and byte-range edits used by the refactor
//! operations. Every rewrite is a set of non-overlapping `Edit`s over the original text,
//! so untouched bytes (comments, spacing) are preserved exactly.

use std::collections::{BTreeSet, HashSet};

use tree_sitter::{Node, Parser, Tree};

#[derive(Debug, Clone)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

pub fn apply_edits(src: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
    let mut out = src.to_string();
    for e in edits {
        out.replace_range(e.start..e.end, &e.text);
    }
    out
}

pub fn parse(src: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    parser.parse(src, None)
}

fn text<'a>(n: Node, src: &'a str) -> &'a str {
    &src[n.start_byte()..n.end_byte()]
}

fn children<'t>(n: Node<'t>) -> Vec<Node<'t>> {
    let mut c = n.walk();
    n.children(&mut c).collect()
}

fn walk<'t>(n: Node<'t>, f: &mut dyn FnMut(Node<'t>)) {
    f(n);
    for c in children(n) {
        walk(c, f);
    }
}

/// The package clause identifier, e.g. `foo` or `foo_test`.
pub fn package_name(src: &str) -> Option<String> {
    let tree = parse(src)?;
    let root = tree.root_node();
    children(root)
        .into_iter()
        .find(|n| n.kind() == "package_clause")
        .and_then(|pc| {
            children(pc)
                .into_iter()
                .find(|n| n.kind() == "package_identifier")
        })
        .map(|n| text(n, src).to_string())
}

/// Replace the package clause identifier. `new_name` is the non-test package name; a
/// `_test` suffix on the existing name is preserved.
pub fn rewrite_package_clause(src: &str, new_name: &str) -> Option<String> {
    let tree = parse(src)?;
    let pc = children(tree.root_node())
        .into_iter()
        .find(|n| n.kind() == "package_clause")?;
    let id = children(pc)
        .into_iter()
        .find(|n| n.kind() == "package_identifier")?;
    let suffix = if text(id, src).ends_with("_test") {
        "_test"
    } else {
        ""
    };
    Some(apply_edits(
        src,
        vec![Edit {
            start: id.start_byte(),
            end: id.end_byte(),
            text: format!("{new_name}{suffix}"),
        }],
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportName {
    Implicit,
    Alias(String),
    Dot,
    Blank,
}

#[derive(Debug, Clone)]
pub struct ImportSpec {
    pub path: String,
    pub name: ImportName,
    pub line: usize,
    pub spec_start: usize,
    pub spec_end: usize,
    pub path_start: usize,
    pub path_end: usize,
    /// The spec is the only one in a non-parenthesised `import "x"` declaration.
    pub sole_decl: Option<(usize, usize)>,
}

fn unquote(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '`').to_string()
}

pub fn imports(src: &str) -> Vec<ImportSpec> {
    let Some(tree) = parse(src) else {
        return vec![];
    };
    let mut out = vec![];
    for decl in children(tree.root_node())
        .into_iter()
        .filter(|n| n.kind() == "import_declaration")
    {
        let specs: Vec<Node> = {
            let mut v = vec![];
            walk(decl, &mut |n| {
                if n.kind() == "import_spec" {
                    v.push(n);
                }
            });
            v
        };
        let grouped = children(decl)
            .iter()
            .any(|c| c.kind() == "import_spec_list");
        for s in specs {
            let Some(path) = s.child_by_field_name("path") else {
                continue;
            };
            let name = match s.child_by_field_name("name") {
                None => ImportName::Implicit,
                Some(n) => match n.kind() {
                    "dot" => ImportName::Dot,
                    "blank_identifier" => ImportName::Blank,
                    _ => ImportName::Alias(text(n, src).to_string()),
                },
            };
            out.push(ImportSpec {
                path: unquote(text(path, src)),
                name,
                line: s.start_position().row + 1,
                spec_start: s.start_byte(),
                spec_end: s.end_byte(),
                path_start: path.start_byte(),
                path_end: path.end_byte(),
                sole_decl: (!grouped).then(|| (decl.start_byte(), decl.end_byte())),
            });
        }
    }
    out
}

/// Rewrite `old` to `new` in every import path literal.
pub fn rewrite_import_path(src: &str, old: &str, new: &str) -> String {
    let edits = imports(src)
        .into_iter()
        .filter(|i| i.path == old)
        .map(|i| Edit {
            start: i.path_start,
            end: i.path_end,
            text: format!("\"{new}\""),
        })
        .collect();
    apply_edits(src, edits)
}

/// Swap every import of `old` for `new`; `alias` is added to specs that carry no name so existing
/// `pkg.Sym` selectors stay valid when the package name changes. `None` when no spec matches, so a
/// caller never reports a no-op as a rewrite.
pub fn rewrite_import_path_aliased(
    src: &str,
    old: &str,
    new: &str,
    alias: Option<&str>,
) -> Option<String> {
    let edits: Vec<Edit> = imports(src)
        .into_iter()
        .filter(|i| i.path == old)
        .map(|i| {
            let prefix = match (alias, &i.name) {
                (Some(a), ImportName::Implicit) => format!("{a} "),
                _ => String::new(),
            };
            Edit {
                start: i.path_start,
                end: i.path_end,
                text: format!("{prefix}\"{new}\""),
            }
        })
        .collect();
    (!edits.is_empty()).then(|| apply_edits(src, edits))
}

/// Delete the import spec(s) for `path`.
pub fn remove_import(src: &str, path: &str) -> String {
    let mut edits = vec![];
    for i in imports(src).into_iter().filter(|i| i.path == path) {
        let (s, e) = i.sole_decl.unwrap_or((i.spec_start, i.spec_end));
        // Swallow the whole line(s) so no blank residue remains.
        let line_start = src[..s].rfind('\n').map_or(0, |p| p + 1);
        let line_end = src[e..].find('\n').map_or(src.len(), |p| e + p + 1);
        let only_ws_before = src[line_start..s].trim().is_empty();
        let only_ws_after = src[e..line_end].trim().is_empty();
        if only_ws_before && only_ws_after {
            edits.push(Edit {
                start: line_start,
                end: line_end,
                text: String::new(),
            });
        } else {
            edits.push(Edit {
                start: s,
                end: e,
                text: String::new(),
            });
        }
    }
    drop_empty_import_decls(&apply_edits(src, edits))
}

/// `import ()` left behind by removing the last spec is valid Go but noise; delete it.
fn drop_empty_import_decls(src: &str) -> String {
    let Some(tree) = parse(src) else {
        return src.to_string();
    };
    let mut edits = vec![];
    for d in children(tree.root_node())
        .into_iter()
        .filter(|n| n.kind() == "import_declaration")
    {
        let mut has_spec = false;
        walk(d, &mut |n| has_spec |= n.kind() == "import_spec");
        if !has_spec {
            let end = if src[d.end_byte()..].starts_with("\n\n") {
                d.end_byte() + 2
            } else {
                d.end_byte()
            };
            edits.push(Edit {
                start: d.start_byte(),
                end,
                text: String::new(),
            });
        }
    }
    apply_edits(src, edits)
}

/// Add `import [alias] "path"` unless an import of `path` already exists.
pub fn add_import(src: &str, path: &str, alias: Option<&str>) -> String {
    if imports(src).iter().any(|i| i.path == path) {
        return src.to_string();
    }
    let spec = match alias {
        Some(a) => format!("{a} \"{path}\""),
        None => format!("\"{path}\""),
    };
    let Some(tree) = parse(src) else {
        return src.to_string();
    };
    let root = tree.root_node();
    let decls: Vec<Node> = children(root)
        .into_iter()
        .filter(|n| n.kind() == "import_declaration")
        .collect();
    if let Some(list) = decls.iter().find_map(|d| {
        children(*d)
            .into_iter()
            .find(|c| c.kind() == "import_spec_list")
    }) {
        let close = list.end_byte() - 1; // the ')'
        return apply_edits(
            src,
            vec![Edit {
                start: close,
                end: close,
                text: format!("\t{spec}\n"),
            }],
        );
    }
    if let Some(d) = decls.first() {
        // Single `import "x"`: widen into a group.
        let existing = text(*d, src)
            .trim_start_matches("import")
            .trim()
            .to_string();
        return apply_edits(
            src,
            vec![Edit {
                start: d.start_byte(),
                end: d.end_byte(),
                text: format!("import (\n\t{existing}\n\t{spec}\n)"),
            }],
        );
    }
    let Some(pc) = children(root)
        .into_iter()
        .find(|n| n.kind() == "package_clause")
    else {
        return src.to_string();
    };
    apply_edits(
        src,
        vec![Edit {
            start: pc.end_byte(),
            end: pc.end_byte(),
            text: format!("\n\nimport {spec}"),
        }],
    )
}

/// Package qualifiers used in the file: `x` in `x.Foo` (expression) and `x.T` (type).
pub fn used_qualifiers(src: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(tree) = parse(src) else { return out };
    walk(tree.root_node(), &mut |n| match n.kind() {
        "selector_expression" => {
            if let Some(op) = n.child_by_field_name("operand")
                && op.kind() == "identifier"
            {
                out.insert(text(op, src).to_string());
            }
        }
        "qualified_type" => {
            if let Some(p) = n.child_by_field_name("package") {
                out.insert(text(p, src).to_string());
            }
        }
        _ => {}
    });
    out
}

/// Name a file refers to an import by, when it can be known without compiling the
/// imported package. `None` ⇒ unreliable (never prune on it).
pub fn reliable_import_name(spec: &ImportSpec, known_pkg_name: Option<&str>) -> Option<String> {
    match &spec.name {
        ImportName::Alias(a) => Some(a.clone()),
        ImportName::Dot | ImportName::Blank => None,
        ImportName::Implicit => {
            if let Some(n) = known_pkg_name {
                return Some(n.to_string());
            }
            // Standard-library paths (no dot in the first element) are named by their last
            // element; third-party names (`.../go` -> `jsoniter`) cannot be guessed.
            let first = spec.path.split('/').next().unwrap_or("");
            let last = spec.path.rsplit('/').next().unwrap_or("");
            let is_version = last.len() > 1
                && last.starts_with('v')
                && last[1..].chars().all(|c| c.is_ascii_digit());
            let plain =
                !last.is_empty() && last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            (!first.contains('.') && plain && !is_version).then(|| last.to_string())
        }
    }
}

/// Drop imports whose (reliably known) name is no longer referenced.
pub fn prune_unused_imports(src: &str, known: &dyn Fn(&str) -> Option<String>) -> String {
    let used = used_qualifiers(src);
    let mut out = src.to_string();
    for spec in imports(src) {
        if let Some(name) = reliable_import_name(&spec, known(&spec.path).as_deref())
            && !used.contains(&name)
        {
            out = remove_import(&out, &spec.path);
        }
    }
    out
}

/// Rename the package qualifier `old` → `new` on selectors/qualified types whose member
/// is in `members` (or all members when `None`). Returns `Err(line)` when `old` also
/// appears as an ordinary identifier (a local shadowing the package), which the caller
/// must treat as a blocker rather than guess.
pub fn rewrite_qualifier(
    src: &str,
    old: &str,
    new: &str,
    members: Option<&HashSet<String>>,
) -> Result<String, usize> {
    let Some(tree) = parse(src) else {
        return Ok(src.to_string());
    };
    let mut edits = vec![];
    let mut qualifier_nodes: HashSet<usize> = HashSet::new();
    let mut shadow: Option<usize> = None;
    walk(tree.root_node(), &mut |n| match n.kind() {
        "selector_expression" => {
            let (Some(op), Some(field)) = (
                n.child_by_field_name("operand"),
                n.child_by_field_name("field"),
            ) else {
                return;
            };
            if op.kind() == "identifier" && text(op, src) == old {
                qualifier_nodes.insert(op.id());
                if members.is_none_or(|m| m.contains(text(field, src))) {
                    edits.push(Edit {
                        start: op.start_byte(),
                        end: op.end_byte(),
                        text: new.to_string(),
                    });
                }
            }
        }
        "qualified_type" => {
            if let Some(p) = n.child_by_field_name("package")
                && text(p, src) == old
            {
                let member = n
                    .child_by_field_name("name")
                    .map(|x| text(x, src))
                    .unwrap_or("");
                if members.is_none_or(|m| m.contains(member)) {
                    edits.push(Edit {
                        start: p.start_byte(),
                        end: p.end_byte(),
                        text: new.to_string(),
                    });
                }
            }
        }
        _ => {}
    });
    walk(tree.root_node(), &mut |n| {
        if n.kind() == "identifier"
            && text(n, src) == old
            && !qualifier_nodes.contains(&n.id())
            && shadow.is_none()
        {
            shadow = Some(n.start_position().row + 1);
        }
    });
    if let Some(line) = shadow {
        return Err(line);
    }
    Ok(apply_edits(src, edits))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclKind {
    Func,
    Method,
    Type,
    Const,
    Var,
}

#[derive(Debug, Clone)]
pub struct TopDecl {
    pub kind: DeclKind,
    /// Names introduced (empty for `init`, `_`).
    pub names: Vec<String>,
    /// Receiver base type for methods.
    pub recv: Option<String>,
    /// Byte range including adjacent leading doc comments.
    pub start: usize,
    pub end: usize,
    pub line: usize,
}

fn base_type_name<'a>(n: Node<'a>, src: &str) -> Option<String> {
    match n.kind() {
        "type_identifier" => Some(text(n, src).to_string()),
        _ => children(n).into_iter().find_map(|c| base_type_name(c, src)),
    }
}

pub fn top_decls(src: &str) -> Vec<TopDecl> {
    let Some(tree) = parse(src) else {
        return vec![];
    };
    let root = tree.root_node();
    let kids = children(root);
    let mut out = vec![];
    for (i, n) in kids.iter().enumerate() {
        let (kind, mut names, recv) = match n.kind() {
            "function_declaration" => {
                let nm = n
                    .child_by_field_name("name")
                    .map(|x| text(x, src).to_string());
                (DeclKind::Func, nm.into_iter().collect::<Vec<_>>(), None)
            }
            "method_declaration" => {
                let recv = n
                    .child_by_field_name("receiver")
                    .and_then(|r| base_type_name(r, src));
                (DeclKind::Method, vec![], recv)
            }
            "type_declaration" | "const_declaration" | "var_declaration" => {
                let k = match n.kind() {
                    "type_declaration" => DeclKind::Type,
                    "const_declaration" => DeclKind::Const,
                    _ => DeclKind::Var,
                };
                let mut names = vec![];
                walk(*n, &mut |x| match x.kind() {
                    "type_spec" | "type_alias" => {
                        if let Some(nm) = x.child_by_field_name("name") {
                            names.push(text(nm, src).to_string());
                        }
                    }
                    "const_spec" | "var_spec" => {
                        let mut c = x.walk();
                        for nm in x.children_by_field_name("name", &mut c) {
                            names.push(text(nm, src).to_string());
                        }
                    }
                    _ => {}
                });
                (k, names, None)
            }
            _ => continue,
        };
        names.retain(|x| x != "_");
        // Pull in contiguous doc comments directly above.
        let mut start = n.start_byte();
        let mut j = i;
        while j > 0 {
            let prev = kids[j - 1];
            let own_line = j < 2 || kids[j - 2].end_position().row < prev.start_position().row;
            let adjacent = prev.kind() == "comment"
                && own_line
                && prev.end_position().row + 1 >= kids[j].start_position().row;
            if adjacent {
                start = prev.start_byte();
                j -= 1;
            } else {
                break;
            }
        }
        out.push(TopDecl {
            kind,
            names,
            recv,
            start,
            end: n.end_byte(),
            line: n.start_position().row + 1,
        });
    }
    out
}

/// Top-level declared names (excluding methods, `init`, `_`).
pub fn top_level_names(src: &str) -> BTreeSet<String> {
    top_decls(src)
        .into_iter()
        .flat_map(|d| d.names)
        .filter(|n| n != "init")
        .collect()
}

#[derive(Debug, Clone)]
pub struct IdentRef {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    /// In a composite-literal key position: could be a field name or a value.
    pub ambiguous: bool,
}

/// Unqualified references to any of `names`: identifiers that are not selector fields,
/// not declaration names, and not struct-literal keys (those are flagged `ambiguous`).
pub fn unqualified_refs(src: &str, names: &HashSet<String>) -> Vec<IdentRef> {
    let Some(tree) = parse(src) else {
        return vec![];
    };
    let mut out = vec![];
    walk(tree.root_node(), &mut |n| {
        if !matches!(n.kind(), "identifier" | "type_identifier") {
            return;
        }
        let t = text(n, src);
        if !names.contains(t) {
            return;
        }
        let Some(parent) = n.parent() else { return };
        if parent
            .child_by_field_name("name")
            .is_some_and(|x| x.id() == n.id())
        {
            return; // declaration site
        }
        if matches!(parent.kind(), "keyed_element" | "literal_element") {
            let in_key = if parent.kind() == "literal_element" {
                parent.parent().is_some_and(|g| {
                    g.kind() == "keyed_element" && g.child(0).is_some_and(|k| k.id() == parent.id())
                })
            } else {
                parent.child(0).is_some_and(|k| k.id() == n.id())
            };
            if in_key {
                out.push(IdentRef {
                    name: t.to_string(),
                    start: n.start_byte(),
                    end: n.end_byte(),
                    line: n.start_position().row + 1,
                    ambiguous: true,
                });
                return;
            }
        }
        out.push(IdentRef {
            name: t.to_string(),
            start: n.start_byte(),
            end: n.end_byte(),
            line: n.start_position().row + 1,
            ambiguous: false,
        });
    });
    out
}

pub fn is_generated(src: &str) -> bool {
    src.lines()
        .take(60)
        .any(|l| l.starts_with("// Code generated") && l.contains("DO NOT EDIT"))
}

pub fn has_build_constraint(path: &str, src: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        "aix",
        "android",
        "darwin",
        "dragonfly",
        "freebsd",
        "hurd",
        "illumos",
        "ios",
        "js",
        "linux",
        "nacl",
        "netbsd",
        "openbsd",
        "plan9",
        "solaris",
        "wasip1",
        "windows",
        "zos",
        "386",
        "amd64",
        "amd64p32",
        "arm",
        "armbe",
        "arm64",
        "arm64be",
        "loong64",
        "mips",
        "mipsle",
        "mips64",
        "mips64le",
        "mips64p32",
        "mips64p32le",
        "ppc",
        "ppc64",
        "ppc64le",
        "riscv",
        "riscv64",
        "s390",
        "s390x",
        "sparc",
        "sparc64",
        "wasm",
    ];
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".go")
        .trim_end_matches("_test");
    if stem.contains('_')
        && stem
            .rsplit('_')
            .next()
            .is_some_and(|s| SUFFIXES.contains(&s))
    {
        return true;
    }
    // Constraints live in comments before the package clause (after any license block).
    let Some(tree) = parse(src) else { return false };
    for n in children(tree.root_node()) {
        if n.kind() != "comment" {
            break;
        }
        if text(n, src)
            .lines()
            .any(|l| l.starts_with("//go:build") || l.starts_with("// +build"))
        {
            return true;
        }
    }
    false
}

pub fn imports_cgo(src: &str) -> bool {
    imports(src).iter().any(|i| i.path == "C")
}

/// Interpreted/raw string literal contents outside import specs, with their lines.
pub fn string_literals(src: &str) -> Vec<(String, usize)> {
    let Some(tree) = parse(src) else {
        return vec![];
    };
    let mut out = vec![];
    walk(tree.root_node(), &mut |n| {
        if matches!(
            n.kind(),
            "interpreted_string_literal" | "raw_string_literal"
        ) {
            let in_import = std::iter::successors(n.parent(), |p| p.parent())
                .any(|p| p.kind() == "import_spec");
            if !in_import {
                out.push((unquote(text(n, src)), n.start_position().row + 1));
            }
        }
    });
    out
}

/// Calls/selectors whose behavior is keyed by a type's or function's package path or
/// name at run time, so relocating the code silently changes what they see.
pub const PATH_KEYED_SELECTORS: &[(&str, &str)] = &[
    ("gob", "Register"),
    ("gob", "RegisterName"),
    ("proto", "RegisterType"),
    ("proto", "RegisterFile"),
    ("plugin", "Open"),
    ("plugin", "Lookup"),
    ("runtime", "FuncForPC"),
    ("runtime", "Callers"),
    ("runtime", "Caller"),
    ("reflect", "PkgPath"),
];

/// Line of the first path-keyed registration/reflection use, with a label.
pub fn path_keyed_use(src: &str) -> Option<(usize, String)> {
    let tree = parse(src)?;
    let mut hit = None;
    walk(tree.root_node(), &mut |n| {
        if hit.is_some() || n.kind() != "selector_expression" {
            return;
        }
        let (Some(op), Some(field)) = (
            n.child_by_field_name("operand"),
            n.child_by_field_name("field"),
        ) else {
            return;
        };
        let (o, f) = (text(op, src), text(field, src));
        let known = PATH_KEYED_SELECTORS
            .iter()
            .any(|(po, pf)| *po == o && *pf == f);
        // `t.PkgPath()` on any reflect.Type value
        if known || f == "PkgPath" {
            hit = Some((n.start_position().row + 1, format!("{o}.{f}")));
        }
    });
    hit
}

pub fn uses_go_embed(src: &str) -> bool {
    src.lines().any(|l| l.starts_with("//go:embed"))
}

pub fn has_init_func(src: &str) -> bool {
    top_decls(src)
        .iter()
        .any(|d| d.kind == DeclKind::Func && src[d.start..d.end].contains("func init("))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_clause_keeps_test_suffix() {
        assert_eq!(
            rewrite_package_clause("package foo\n", "bar").unwrap(),
            "package bar\n"
        );
        assert_eq!(
            rewrite_package_clause("package foo_test\n", "bar").unwrap(),
            "package bar_test\n"
        );
    }

    #[test]
    fn import_rewrite_handles_single_grouped_alias() {
        let single = "package a\n\nimport \"x/old\"\n";
        assert!(rewrite_import_path(single, "x/old", "x/new").contains("\"x/new\""));
        let grouped = "package a\n\nimport (\n\t\"fmt\"\n\tal \"x/old\"\n)\n";
        let out = rewrite_import_path(grouped, "x/old", "x/new");
        assert!(out.contains("al \"x/new\"") && out.contains("\"fmt\""));
    }

    #[test]
    fn add_and_remove_import() {
        let src = "package a\n\nimport \"fmt\"\n\nfunc f() { fmt.Println() }\n";
        let added = add_import(src, "x/y", None);
        assert!(
            added.contains("import (") && added.contains("\"x/y\"") && added.contains("\"fmt\"")
        );
        let bare = add_import("package a\n", "x/y", None);
        assert!(bare.contains("import \"x/y\""));
        let removed = remove_import(&added, "x/y");
        assert!(!removed.contains("x/y"));
        assert!(!remove_import(src, "fmt").contains("import"));
    }

    #[test]
    fn qualifier_rewrite_is_member_scoped_and_detects_shadowing() {
        let src = "package a\n\nfunc f() { old.Foo(); old.Bar(); var x old.T; _ = x }\n";
        let members: HashSet<String> = ["Foo".to_string(), "T".to_string()].into();
        let out = rewrite_qualifier(src, "old", "nu", Some(&members)).unwrap();
        assert!(out.contains("nu.Foo()") && out.contains("old.Bar()") && out.contains("nu.T"));
        let shadow = "package a\n\nfunc f(old int) { _ = old }\n";
        assert_eq!(rewrite_qualifier(shadow, "old", "nu", None), Err(3));
    }

    #[test]
    fn top_decls_include_docs_methods_and_groups() {
        let src = "package a\n\n// Doc.\nfunc F() {}\n\ntype T struct{}\n\nfunc (t *T) M() {}\n\nconst (\n\tA = 1\n\tB = 2\n)\n";
        let d = top_decls(src);
        assert_eq!(d.len(), 4);
        assert!(src[d[0].start..d[0].end].starts_with("// Doc."));
        assert_eq!(d[2].recv.as_deref(), Some("T"));
        assert_eq!(d[3].names, vec!["A", "B"]);
    }

    #[test]
    fn unqualified_refs_skip_selectors_and_flag_keys() {
        let names: HashSet<String> = ["Helper".to_string()].into();
        let src = "package a\n\nfunc g() { Helper(); x.Helper(); _ = S{Helper: 1} }\n";
        let refs = unqualified_refs(src, &names);
        assert_eq!(refs.len(), 2);
        assert!(refs.iter().any(|r| r.ambiguous));
    }

    #[test]
    fn markers() {
        assert!(is_generated(
            "// Code generated by x. DO NOT EDIT.\npackage a\n"
        ));
        assert!(has_build_constraint(
            "a.go",
            "//go:build linux\n\npackage a\n"
        ));
        assert!(has_build_constraint("a_linux.go", "package a\n"));
        assert!(!has_build_constraint("a.go", "package a\n"));
        assert!(imports_cgo(
            "package a\n\n// #include <x.h>\nimport \"C\"\n"
        ));
        assert!(has_init_func("package a\n\nfunc init() {}\n"));
    }

    #[test]
    fn trailing_comment_is_not_the_next_decls_doc() {
        let src = "package a\n\nvar Stay = 1 // trailing\nfunc Go() int { return 2 }\n";
        let d = top_decls(src);
        assert!(!src[d[1].start..d[1].end].contains("trailing"));
    }

    #[test]
    fn license_block_does_not_hide_build_constraint() {
        let licensed =
            "/*\nCopyright 2020 Someone.\n\nLicensed.\n*/\n\n//go:build linux\npackage a\n";
        assert!(has_build_constraint("a.go", licensed));
        assert!(has_build_constraint("a_dragonfly.go", "package a\n"));
    }

    #[test]
    fn third_party_import_names_are_not_guessed() {
        let spec = |p: &str| ImportSpec {
            path: p.into(),
            name: ImportName::Implicit,
            line: 1,
            spec_start: 0,
            spec_end: 0,
            path_start: 0,
            path_end: 0,
            sole_decl: None,
        };
        assert_eq!(
            reliable_import_name(&spec("strings"), None).as_deref(),
            Some("strings")
        );
        assert_eq!(
            reliable_import_name(&spec("github.com/json-iterator/go"), None),
            None
        );
    }

    #[test]
    fn dynamic_reference_helpers() {
        let src = "package a\n\nimport \"x/y\"\n\nfunc f() { gob.Register(1); _ = \"x/y.T\"; _ = t.PkgPath() }\n";
        let lits = string_literals(src);
        assert_eq!(lits.len(), 1);
        assert_eq!(lits[0].0, "x/y.T");
        assert_eq!(path_keyed_use(src).unwrap().1, "gob.Register");
        assert!(path_keyed_use("package a\n\nfunc f() {}\n").is_none());
    }

    #[test]
    fn exact_import_match_ignores_prefixes_and_comments() {
        let src = "package a\n\n// \"foo/bar\"\nimport (\n\t\"foo/bar/baz\"\n\tx \"foo/bar\"\n)\n";
        let hits: Vec<_> = imports(src)
            .into_iter()
            .filter(|i| i.path == "foo/bar")
            .collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, ImportName::Alias("x".into()));
    }

    #[test]
    fn aliased_rewrite_leaves_prefix_paths_alone_and_reports_no_match() {
        let src = "package a\n\nimport (\n\t\"foo/bar/baz\"\n\t\"foo/bar\"\n)\n";
        let out = rewrite_import_path_aliased(src, "foo/bar", "x/y", Some("bar")).unwrap();
        assert_eq!(
            out,
            "package a\n\nimport (\n\t\"foo/bar/baz\"\n\tbar \"x/y\"\n)\n"
        );
        assert_eq!(rewrite_import_path_aliased(src, "foo/ba", "x", None), None);
    }

    #[test]
    fn aliased_rewrite_keeps_an_existing_name() {
        let src = "package a\n\nimport b \"foo/bar\"\n";
        let out = rewrite_import_path_aliased(src, "foo/bar", "x/y", Some("bar")).unwrap();
        assert_eq!(out, "package a\n\nimport b \"x/y\"\n");
    }

    #[test]
    fn cgo_and_package_clause_reads() {
        assert!(imports_cgo(
            "package a\n\n/*\n#include <x.h>\n*/\nimport \"C\"\n"
        ));
        assert!(!imports_cgo("package a\n\nimport \"fmt\"\n"));
        assert_eq!(
            package_name("package foo_test\n").as_deref(),
            Some("foo_test")
        );
    }

    #[test]
    fn build_constraint_only_counts_before_the_package_clause() {
        assert!(has_build_constraint(
            "a.go",
            "//go:build linux\n\npackage a\n"
        ));
        assert!(!has_build_constraint(
            "a.go",
            "package a\n\n// +build linux\n"
        ));
        assert!(has_build_constraint("a_linux.go", "package a\n"));
    }
}
