use std::path::Path;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Beyond this many lines, a file is flagged as a candidate to split. No official
/// convention exists for most of these languages, but 500 lines is the threshold most
/// style guides (e.g. Uber's Go guide) converge on before a single file gets hard to
/// hold in your head at once — applied uniformly across languages rather than tuning
/// a separate number per language nobody's validated yet.
const MAX_FILE_LINES: usize = 500;

/// True if `source` carries a generated-code marker: a line whose first token after a
/// `//` or `#` comment lead is `Code generated ... DO NOT EDIT.`, conventionally within
/// the first few lines (the exact form Go tooling standardized on,
/// <https://go.dev/s/generatedcode>, but widely reused verbatim by generators — protoc,
/// sqlc, mockery, etc. — targeting other `//`- or `#`-comment languages too). Every
/// mainstream linter skips generated files by default — flagging one as a candidate to
/// split is nonsensical (and pure noise) since nobody hand-edits it. Only checked
/// against the first 20 lines: real generated-code headers are always near the top, so
/// this avoids scanning a file's full content just to learn it should be skipped.
pub fn is_generated(source: &str) -> bool {
    source.lines().take(20).any(|line| {
        let trimmed = line.trim_start();
        let after_comment = trimmed
            .strip_prefix("//")
            .or_else(|| trimmed.strip_prefix('#'));
        match after_comment {
            Some(rest) => {
                let rest = rest.trim_start();
                rest.starts_with("Code generated ") && rest.trim_end().ends_with("DO NOT EDIT.")
            }
            None => false,
        }
    })
}

struct LangConfig {
    name: &'static str,
    file_globs: &'static [&'static str],
}

fn lang_config(lang: Language) -> LangConfig {
    match lang {
        Language::Go => LangConfig {
            name: "go-file-size",
            file_globs: &["**/*.go"],
        },
        Language::TypeScript => LangConfig {
            name: "typescript-file-size",
            file_globs: &["**/*.ts"],
        },
        Language::Tsx => LangConfig {
            name: "tsx-file-size",
            file_globs: &["**/*.tsx"],
        },
        Language::JavaScript => LangConfig {
            name: "javascript-file-size",
            file_globs: &["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"],
        },
        Language::Python => LangConfig {
            name: "python-file-size",
            file_globs: &["**/*.py"],
        },
        Language::Java => LangConfig {
            name: "java-file-size",
            file_globs: &["**/*.java"],
        },
        Language::Kotlin => LangConfig {
            name: "kotlin-file-size",
            file_globs: &["**/*.kt", "**/*.kts"],
        },
        Language::Rust => LangConfig {
            name: "rust-file-size",
            file_globs: &["**/*.rs"],
        },
    }
}

/// Flags files over [`MAX_FILE_LINES`] lines as candidates to split into smaller files.
/// One instance per [`Language`] (see `checker::registry`) — doesn't need a parsed tree,
/// a raw line count is enough, so `language()` stays `None` for every instance regardless
/// of which language's files it's scoped to via `file_globs()`.
pub struct FileSizeChecker {
    lang: Language,
}

impl FileSizeChecker {
    pub fn new(lang: Language) -> Self {
        Self { lang }
    }
}

impl Checker for FileSizeChecker {
    fn name(&self) -> &str {
        lang_config(self.lang).name
    }

    fn description(&self) -> &str {
        "flags files over a line-count threshold as candidates to split"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        lang_config(self.lang).file_globs
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if is_generated(ctx.source) {
            return Ok(Vec::new());
        }
        let lines = ctx.source.lines().count();
        if lines <= MAX_FILE_LINES {
            return Ok(Vec::new());
        }
        Ok(vec![Finding {
            line: lines,
            message: format!(
                "[file-size] file spans {lines} lines (over {MAX_FILE_LINES}) — consider splitting it into smaller files"
            ),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(lang: Language, src: &str) -> Vec<Finding> {
        let ctx = CheckContext {
            source: src,
            tree: None,
        };
        FileSizeChecker::new(lang)
            .check(Path::new("<source>"), &ctx)
            .unwrap()
    }

    #[test]
    fn flags_file_over_threshold() {
        let src = "package main\n".repeat(MAX_FILE_LINES + 1);
        let findings = check_source(Language::Go, &src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("consider splitting"));
        assert_eq!(findings[0].line, MAX_FILE_LINES + 1);
    }

    #[test]
    fn allows_file_at_threshold() {
        let src = "package main\n".repeat(MAX_FILE_LINES);
        assert!(check_source(Language::Go, &src).is_empty());
    }

    #[test]
    fn allows_small_file() {
        assert!(check_source(Language::Go, "package main\n").is_empty());
    }

    #[test]
    fn skips_generated_file_even_over_threshold() {
        let mut src = "// Code generated by protoc-gen-go. DO NOT EDIT.\n".to_string();
        src.push_str(&"package main\n".repeat(MAX_FILE_LINES + 1));
        assert!(check_source(Language::Go, &src).is_empty());
    }

    #[test]
    fn is_generated_requires_the_exact_marker() {
        assert!(is_generated(
            "// Code generated by mockery v2.20.0. DO NOT EDIT.\npackage main\n"
        ));
        assert!(!is_generated("// hand-written, not generated\npackage main\n"));
        assert!(!is_generated("// Code generated but this comment is different\n"));
    }

    #[test]
    fn is_generated_recognizes_hash_comment_languages_too() {
        assert!(is_generated(
            "# Code generated by sqlc. DO NOT EDIT.\nimport foo\n"
        ));
    }

    #[test]
    fn is_generated_only_checked_near_the_top() {
        let mut src = "package main\n".repeat(25);
        src.push_str("// Code generated by protoc-gen-go. DO NOT EDIT.\n");
        assert!(!is_generated(&src));
    }

    #[test]
    fn kotlin_file_over_threshold_is_flagged_with_the_kotlin_check_name() {
        let src = "fun f() {}\n".repeat(MAX_FILE_LINES + 1);
        let findings = check_source(Language::Kotlin, &src);
        assert_eq!(findings.len(), 1);
        assert_eq!(FileSizeChecker::new(Language::Kotlin).name(), "kotlin-file-size");
        assert_eq!(
            FileSizeChecker::new(Language::Kotlin).file_globs(),
            &["**/*.kt", "**/*.kts"]
        );
    }

    #[test]
    fn every_language_gets_a_distinct_checker_name() {
        let langs = [
            Language::Go,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
            Language::Java,
            Language::Kotlin,
            Language::Rust,
        ];
        let checkers: Vec<FileSizeChecker> = langs.iter().map(|&lang| FileSizeChecker::new(lang)).collect();
        let names: std::collections::HashSet<&str> = checkers.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), langs.len());
    }
}
