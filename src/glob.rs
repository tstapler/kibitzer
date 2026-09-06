use regex::Regex;

/// Convert a glob pattern (supporting `**`, `*`, `?`) into an anchored regex
/// matched against a `/`-separated relative path.
fn glob_to_regex(pattern: &str) -> Regex {
    let mut out = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    // `**/` matches zero or more path segments; bare `**` matches anything.
                    if chars.peek() == Some(&'/') {
                        chars.next();
                        out.push_str("(.*/)?");
                    } else {
                        out.push_str(".*");
                    }
                } else {
                    out.push_str("[^/]*");
                }
            }
            '?' => out.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            other => out.push(other),
        }
    }
    out.push('$');
    Regex::new(&out).expect("glob_to_regex always produces a valid pattern")
}

/// Returns true if `rel_path` (relative to the repo root, `/`-separated) is in scope:
/// it matches at least one positive pattern (or there are none, gitignore-style — an
/// empty/all-negative `scopes` list starts from "everything") and none of the `!`-
/// prefixed negative patterns. Negative patterns are the supported way to suppress a
/// check for one file/directory while leaving the rest of its scope active — see
/// docs/suppressing-checks.md.
pub fn matches_scope(rel_path: &str, scopes: &[String]) -> bool {
    let mut positives = scopes.iter().filter(|p| !p.starts_with('!')).peekable();
    let included =
        positives.peek().is_none() || positives.any(|pat| glob_to_regex(pat).is_match(rel_path));
    if !included {
        return false;
    }
    !scopes
        .iter()
        .filter_map(|p| p.strip_prefix('!'))
        .any(|pat| glob_to_regex(pat).is_match(rel_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_double_star_prefix() {
        assert!(matches_scope(
            "server/services/foo.go",
            &["**/*.go".to_string()]
        ));
        assert!(matches_scope("foo.go", &["**/*.go".to_string()]));
    }

    #[test]
    fn matches_exact_dir() {
        assert!(matches_scope(
            "web-app/src/components/Button.tsx",
            &["web-app/**/*.tsx".to_string()]
        ));
        assert!(!matches_scope(
            "server/main.go",
            &["web-app/**/*.tsx".to_string()]
        ));
    }

    #[test]
    fn empty_scope_matches_all() {
        assert!(matches_scope("anything/at/all.rs", &[]));
    }

    #[test]
    fn negative_pattern_excludes_a_single_file_from_a_wider_positive_scope() {
        let scope = &["**/*.go".to_string(), "!vendor/generated.go".to_string()];
        assert!(matches_scope("pkg/foo.go", scope));
        assert!(!matches_scope("vendor/generated.go", scope));
    }

    #[test]
    fn negative_pattern_excludes_a_whole_directory() {
        let scope = &["**/*.go".to_string(), "!vendor/**".to_string()];
        assert!(!matches_scope("vendor/pkg/foo.go", scope));
        assert!(matches_scope("pkg/foo.go", scope));
    }

    #[test]
    fn scope_with_only_negative_patterns_matches_everything_except_them() {
        let scope = &["!vendor/**".to_string()];
        assert!(matches_scope("pkg/foo.go", scope));
        assert!(!matches_scope("vendor/foo.go", scope));
    }
}
