//! Checks on names and paths that come from a plan, which may be hand-edited or foreign.

use std::path::{Component, Path};

const GO_KEYWORDS: [&str; 25] = [
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

/// `.git` and `.kibitzer` hold hooks and check commands; an edit there is code execution.
pub fn is_protected(root: &Path, p: &Path) -> bool {
    p.strip_prefix(root).is_ok_and(|rel| {
        rel.components()
            .any(|c| c.as_os_str() == ".git" || c.as_os_str() == ".kibitzer")
    })
}

/// A valid, non-keyword Go identifier.
pub fn is_go_ident(s: &str) -> bool {
    if GO_KEYWORDS.contains(&s) {
        return false;
    }
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// An import path that can be spliced between double quotes without escaping.
pub fn is_safe_import_path(s: &str) -> bool {
    !s.is_empty()
        && !s
            .chars()
            .any(|c| c == '"' || c == '`' || c == '\\' || c.is_whitespace() || c.is_control())
}

/// True when `p` is under `root` without `..` hops or a symlink that leaves the tree. Checks
/// the deepest existing ancestor, since the destination may not exist yet.
pub fn is_inside(root: &Path, p: &Path) -> bool {
    if !p.starts_with(root) || p.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    let (Ok(canon_root), Some(existing)) =
        (root.canonicalize(), p.ancestors().find(|a| a.exists()))
    else {
        return false;
    };
    existing
        .canonicalize()
        .is_ok_and(|c| c.starts_with(canon_root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_exclude_keywords_and_junk() {
        assert!(is_go_ident("core"));
        assert!(is_go_ident("_x1"));
        assert!(!is_go_ident("func"));
        assert!(!is_go_ident("foo bar"));
        assert!(!is_go_ident("1abc"));
        assert!(!is_go_ident(""));
    }

    #[test]
    fn import_paths_must_be_quotable() {
        assert!(is_safe_import_path("example.com/a/b"));
        assert!(!is_safe_import_path("a\"b"));
        assert!(!is_safe_import_path("a b"));
        assert!(!is_safe_import_path(""));
    }

    #[test]
    fn protected_covers_git_and_kibitzer_only() {
        let root = Path::new("/r");
        assert!(is_protected(root, Path::new("/r/.git/hooks/x")));
        assert!(is_protected(root, Path::new("/r/.kibitzer/inspect.json")));
        assert!(!is_protected(root, Path::new("/r/pkg/a.go")));
    }
}
