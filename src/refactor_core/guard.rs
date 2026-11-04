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

/// A repo-relative plan path must be non-empty, relative, and free of `..`.
pub fn check_plan_rel_path(p: &str) -> anyhow::Result<()> {
    let path = Path::new(p);
    let escapes = path.is_absolute() || path.components().any(|c| c == Component::ParentDir);
    if escapes || p.is_empty() {
        anyhow::bail!("plan path `{p}` is absolute or leaves the repository; refusing to apply");
    }
    Ok(())
}

/// No existing component of `p` under `root` may be a symlink (a not-yet-existing tail is fine).
pub fn check_no_symlink_crossing(root: &Path, p: &str) -> anyhow::Result<()> {
    let mut cur = root.to_path_buf();
    for comp in Path::new(p).components() {
        cur.push(comp);
        match std::fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => {
                anyhow::bail!("plan path `{p}` crosses a symlink; refusing to apply")
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => anyhow::bail!("plan path `{p}` cannot be inspected: {e}; refusing to apply"),
        }
    }
    Ok(())
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

    #[test]
    fn predicates_reject_the_awkward_inputs() {
        for bad in ["a`b", "a\\b", "a\nb", "a\tb", "a\u{7}b"] {
            assert!(!is_safe_import_path(bad), "{bad:?}");
        }
        assert!(is_go_ident("é") && is_go_ident("日本"));
        assert!(
            is_go_ident("_") && is_go_ident("init"),
            "syntactically valid; callers add their own bans"
        );
        let root = Path::new("/r");
        assert!(
            !is_protected(root, Path::new("/elsewhere/.git/x")),
            "outside root is not this predicate's job"
        );
        assert!(
            is_protected(root, Path::new("/r/a/.git/../b")),
            "a .git component anywhere in the path counts"
        );
        assert!(!is_inside(root, Path::new("/r/../etc/passwd")));
    }
}
