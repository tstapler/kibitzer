//! Tree-sitter reads of the two Go constructs the refactor engine rewrites: the package
//! clause and import specs. Regexes would match comments and path prefixes.

use tree_sitter::{Node, Parser};

fn parse(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    parser.parse(source, None)
}

/// The identifier in the file's `package` clause.
pub fn package_name(source: &str) -> Option<String> {
    let tree = parse(source)?;
    let root = tree.root_node();
    let mut cursor = root.walk();
    let clause = root
        .children(&mut cursor)
        .find(|n| n.kind() == "package_clause")?;
    let mut inner = clause.walk();
    let ident = clause
        .children(&mut inner)
        .find(|n| n.kind() == "package_identifier")?;
    Some(source[ident.byte_range()].to_string())
}

/// One import spec whose path equals `wanted` exactly (no prefix matches).
#[derive(Debug, PartialEq, Eq)]
pub struct ImportSite {
    pub has_alias: bool,
}

pub fn find_imports(source: &str, wanted: &str) -> Vec<ImportSite> {
    let Some(tree) = parse(source) else {
        return Vec::new();
    };
    let mut sites = Vec::new();
    collect(tree.root_node(), source, wanted, &mut sites);
    sites
}

fn collect(node: Node, source: &str, wanted: &str, out: &mut Vec<ImportSite>) {
    if node.kind() == "import_spec" {
        collect_spec(node, source, wanted, out);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, source, wanted, out);
    }
}

fn collect_spec(spec: Node, source: &str, wanted: &str, out: &mut Vec<ImportSite>) {
    let Some(path) = spec.child_by_field_name("path") else {
        return;
    };
    let text = &source[path.byte_range()];
    if text.trim_matches(|c| c == '"' || c == '`') != wanted {
        return;
    }
    out.push(ImportSite {
        has_alias: spec.child_by_field_name("name").is_some(),
    });
}

/// Rewrites the identifier in the `package` clause. `None` when there is no clause.
pub fn replace_package_name(source: &str, new_name: &str) -> Option<String> {
    let tree = parse(source)?;
    let root = tree.root_node();
    let mut cursor = root.walk();
    let clause = root
        .children(&mut cursor)
        .find(|n| n.kind() == "package_clause")?;
    let mut inner = clause.walk();
    let ident = clause
        .children(&mut inner)
        .find(|n| n.kind() == "package_identifier")?;
    let mut out = source.to_string();
    out.replace_range(ident.byte_range(), new_name);
    Some(out)
}

/// Swaps every import spec whose path equals `old` for `new`, adding `alias` to specs that
/// have none. `None` when no spec matches, so a caller never reports a no-op as a rewrite.
pub fn replace_import_path(
    source: &str,
    old: &str,
    new: &str,
    alias: Option<&str>,
) -> Option<String> {
    let tree = parse(source)?;
    let mut specs = Vec::new();
    collect_specs(tree.root_node(), &mut specs);
    // Back to front so earlier byte offsets stay valid while splicing.
    let mut sites: Vec<(usize, usize, bool)> = specs
        .iter()
        .filter_map(|spec| {
            let path = spec.child_by_field_name("path")?;
            let text = &source[path.byte_range()];
            (text.trim_matches(|c| c == '"' || c == '`') == old).then(|| {
                (
                    path.start_byte(),
                    path.end_byte(),
                    spec.child_by_field_name("name").is_some(),
                )
            })
        })
        .collect();
    if sites.is_empty() {
        return None;
    }
    sites.sort_unstable();
    let mut out = source.to_string();
    for (start, end, has_alias) in sites.into_iter().rev() {
        let prefix = match alias {
            Some(a) if !has_alias => format!("{a} "),
            _ => String::new(),
        };
        out.replace_range(start..end, &format!("{prefix}\"{new}\""));
    }
    Some(out)
}

fn collect_specs<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    if node.kind() == "import_spec" {
        out.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_specs(child, out);
    }
}

/// True when the file declares `import "C"` (cgo).
pub fn imports_c(source: &str) -> bool {
    !find_imports(source, "C").is_empty()
}

/// True when a `//go:build` or `// +build` constraint appears before the package clause.
pub fn has_build_constraint(source: &str) -> bool {
    for line in source.lines() {
        let t = line.trim();
        if t.starts_with("package ") {
            return false;
        }
        if t.starts_with("//go:build") || t.starts_with("// +build") {
            return true;
        }
    }
    false
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn package_name_should_ReadClause_When_TestPackage() {
        assert_eq!(
            package_name("package foo_test\n").as_deref(),
            Some("foo_test")
        );
    }

    #[test]
    fn find_imports_should_NotMatchPrefixOrComment_When_PathIsPrefixOfAnother() {
        let src = "package a\n\n// \"foo/bar\"\nimport (\n\t\"foo/bar/baz\"\n\tx \"foo/bar\"\n)\n";
        let sites = find_imports(src, "foo/bar");
        assert_eq!(sites, vec![ImportSite { has_alias: true }]);
    }

    #[test]
    fn replace_import_path_should_LeavePrefixPathAlone_When_RewritingExactPath() {
        let src = "package a\n\nimport (\n\t\"foo/bar/baz\"\n\t\"foo/bar\"\n)\n";
        let out = replace_import_path(src, "foo/bar", "x/y", Some("bar")).unwrap();
        assert_eq!(
            out,
            "package a\n\nimport (\n\t\"foo/bar/baz\"\n\tbar \"x/y\"\n)\n"
        );
        assert_eq!(replace_import_path(src, "foo/ba", "x", None), None);
    }

    #[test]
    fn imports_c_should_DetectCgo_When_ImportC() {
        assert!(imports_c(
            "package a\n\n/*\n#include <x.h>\n*/\nimport \"C\"\n"
        ));
        assert!(!imports_c("package a\n\nimport \"fmt\"\n"));
    }

    #[test]
    fn has_build_constraint_should_OnlyLookBeforePackage() {
        assert!(has_build_constraint("//go:build linux\n\npackage a\n"));
        assert!(!has_build_constraint("package a\n\n// +build linux\n"));
    }
}
