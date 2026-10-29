pub mod ai_vocabulary_density;
pub mod comment_quality;
pub mod complexity;
pub mod duplicate_code;
pub mod duplicate_code_fuzzy;
pub mod duplicate_cross_file_checker;
pub mod em_dash_overuse;
pub mod file_size;
pub mod filler_phrase_density;
pub mod formulaic_ai_openers;
pub mod go_blank_imports;
pub mod go_bulk_fetch_linear_scan;
pub mod go_encapsulate_collection;
pub mod go_error_context;
pub mod go_ignored_error;
pub mod go_table_driven_test;
pub mod go_type_switch_density;
pub mod hollow_qualifier_phrasing;
mod import_preamble;
pub mod inline_ignore;
pub mod java_error_context;
pub mod java_ignored_error;
pub mod java_lost_exception_cause;
pub mod java_swallowed_interrupt;
mod magic_literal_context;
pub mod markdown_link_integrity;
pub mod paragraph_breaks;
pub mod primitive_obsession;
pub mod repetitive_sentences;
pub mod rules;
pub mod sentence_length_uniformity;
pub mod test_no_assertion;

/// True for common test-file naming conventions across supported languages. Expects a
/// repo-relative path: a `tests/` component anywhere in an absolute path (e.g. a checkout
/// under `~/tests/`) also matches.
///
/// Origin: Backtesting
/// against `kubernetes/kubernetes` found ~97% of raw findings were `_test.go` table-driven
/// cases — already the Parameterize-Function shape, so flagging them back is backwards.
/// See `docs/duplicate-code-fuzzy.md`.
pub(crate) fn is_test_file(file: &std::path::Path) -> bool {
    // Judge only the part below the working directory: a checkout under `~/test/` or
    // `/tmp/spec/` must not make every file in it a test file.
    let relative = std::env::current_dir().ok().and_then(|cwd| {
        file.strip_prefix(cwd)
            .ok()
            .map(std::path::Path::to_path_buf)
    });
    let file = relative.as_deref().unwrap_or(file);
    // Rust integration tests and JS/TS test suites both conventionally live in a `tests/`
    // or `__tests__/` directory rather than following a filename suffix (e.g.
    // `crates/cli/tests/browser_session.rs`, confirmed as a real backtest hit before this
    // check was added — see docs/duplicate-code-fuzzy.md).
    // Singular `test/`, `spec/` and `testing/` are as common (kubernetes' `test/e2e`,
    // `runtime/testing/`).
    if file.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("tests" | "__tests__" | "test" | "spec" | "testing")
        )
    }) {
        return true;
    }

    let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || lower.ends_with("_tests.py")
        || (lower.starts_with("test_") && lower.ends_with(".py"))
        || [
            ".test.ts",
            ".test.tsx",
            ".test.js",
            ".test.jsx",
            ".spec.ts",
            ".spec.tsx",
            ".spec.js",
            ".spec.jsx",
            ".integrationtest.ts",
            ".integrationtest.tsx",
        ]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
        || (lower.ends_with(".java")
            && (name.ends_with("Test.java")
                || name.ends_with("Tests.java")
                || name.ends_with("TestBase.java")))
        || (lower.ends_with(".kt")
            && (name.ends_with("Test.kt")
                || name.ends_with("Tests.kt")
                || name.ends_with("TestBase.kt")))
}

#[cfg(test)]
mod tests {
    use super::is_test_file;
    use std::path::Path;

    #[test]
    #[allow(non_snake_case)]
    fn is_test_file_should_MatchSingularTestSpecAndTestingDirs_When_AnyComponent() {
        for p in [
            "test/e2e/a.go",
            "spec/models/a.rb",
            "pkg/runtime/testing/types.go",
        ] {
            assert!(is_test_file(Path::new(p)), "{p}");
        }
        assert!(!is_test_file(Path::new("pkg/contest/a.go")));
        assert!(!is_test_file(Path::new("pkg/contesting/a.go")));
        assert!(!is_test_file(Path::new("src/latest/a.go")));
    }

    #[test]
    #[allow(non_snake_case)]
    fn is_test_file_should_IgnoreCwdAncestors_When_PathIsAbsoluteUnderCwd() {
        let cwd = std::env::current_dir().unwrap();
        assert!(!is_test_file(&cwd.join("src/a.go")));
        assert!(is_test_file(&cwd.join("test/a.go")));
    }

    #[test]
    #[allow(non_snake_case)]
    fn is_test_file_should_MatchTestBaseTestsPyAndIntegrationTest_When_NewNames() {
        for p in [
            "src/FooTestBase.java",
            "src/FooTestBase.kt",
            "pkg/foo_tests.py",
            "src/api.integrationTest.ts",
        ] {
            assert!(is_test_file(Path::new(p)), "{p}");
        }
        assert!(!is_test_file(Path::new("src/Foo.java")));
        assert!(!is_test_file(Path::new("pkg/foo.py")));
    }
}
