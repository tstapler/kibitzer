use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Minimum number of consecutive lines a duplicated block must span before flagging —
/// short repeats (a closing brace, a single `return nil`) are normal, not copy-paste.
const MIN_BLOCK_LINES: usize = 6;
/// Minimum combined trimmed-line length a block must have, filtering out blocks that
/// are mostly blank or single-token lines shared by coincidence rather than by copying.
const MIN_BLOCK_CHARS: usize = 60;

/// Flags blocks of code duplicated elsewhere in the same file — a lightweight,
/// language-agnostic clone detector (line-window hashing, no AST) in the spirit of
/// `dupl`/`jscpd`. See the "Duplicated code blocks" entry in `docs/check-ideas.md`.
pub struct DuplicateCodeChecker;

impl Checker for DuplicateCodeChecker {
    fn name(&self) -> &str {
        "duplicate-code"
    }

    fn description(&self) -> &str {
        "flags blocks of code duplicated elsewhere in the same file"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        &[
            "**/*.go",
            "**/*.ts",
            "**/*.tsx",
            "**/*.js",
            "**/*.jsx",
            "**/*.py",
            "**/*.java",
            "**/*.kt",
        ]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        Ok(find_duplicate_blocks(ctx.source))
    }
}

/// Slides a `MIN_BLOCK_LINES`-line window over `source`'s trimmed lines, hashing each
/// window's text. A window whose text was already seen earlier in the file — and that
/// isn't just the tail of a duplicate block already flagged — is reported at its second
/// occurrence, pointing back at the first. Windows spanning a blank line, or too short
/// on total content, are skipped so incidental repeats (blank padding, lone `}`) don't
/// fire.
fn find_duplicate_blocks(source: &str) -> Vec<Finding> {
    let normalized: Vec<String> = source.lines().map(|l| l.trim().to_string()).collect();
    if normalized.len() < MIN_BLOCK_LINES {
        return Vec::new();
    }

    let mut first_seen: HashMap<&[String], usize> = HashMap::new();
    let mut findings = Vec::new();
    let mut flagged_until = 0usize;

    for start in 0..=(normalized.len() - MIN_BLOCK_LINES) {
        let window = &normalized[start..start + MIN_BLOCK_LINES];
        if window.iter().any(|l| l.is_empty()) {
            continue;
        }
        let total_chars: usize = window.iter().map(|l| l.len()).sum();
        if total_chars < MIN_BLOCK_CHARS {
            continue;
        }

        match first_seen.get(window) {
            Some(&prev_start) if start >= flagged_until => {
                findings.push(Finding {
                    line: start + 1,
                    message: format!(
                        "{MIN_BLOCK_LINES}-line block duplicates lines {}-{} — consider extracting a shared function",
                        prev_start + 1,
                        prev_start + MIN_BLOCK_LINES
                    ),
                });
                flagged_until = start + MIN_BLOCK_LINES;
            }
            None => {
                first_seen.insert(window, start);
            }
            _ => {}
        }
    }

    findings
}

#[cfg(test)]
fn check_source(src: &str) -> Vec<Finding> {
    let ctx = CheckContext {
        source: src,
        tree: None,
    };
    DuplicateCodeChecker
        .check(Path::new("<source>"), &ctx)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_duplicated_block() {
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        let src = format!("package main\n\n{block}\n{block}");
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("duplicates lines"));
    }

    #[test]
    fn does_not_flag_short_duplicate_blocks() {
        let block = "\tconn := open()\n\tconn.Close()\n";
        let src = format!("package main\n\n{block}\n{block}");
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_trivial_repeated_lines() {
        // Six short, low-content lines (closing braces / blank-ish) repeated twice —
        // below MIN_BLOCK_CHARS, so shouldn't count as meaningful duplication.
        let block = "}\n}\n}\n}\n}\n}\n";
        let src = format!("package main\n\n{block}\n{block}");
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_when_no_duplication() {
        let src = "package main\n\nfunc a() {\n\tfmt.Println(\"a\")\n}\n\nfunc b() {\n\tfmt.Println(\"b\")\n}\n";
        assert!(check_source(src).is_empty());
    }

    #[test]
    fn does_not_flag_block_spanning_a_blank_line() {
        let src = "package main\n\nfunc a() {\n\tx := 1\n\n\ty := 2\n\tz := 3\n\tw := 4\n}\n\nfunc b() {\n\tx := 1\n\n\ty := 2\n\tz := 3\n\tw := 4\n}\n";
        assert!(check_source(src).is_empty());
    }

    #[test]
    fn only_flags_once_for_a_longer_duplicate_run() {
        let block = "func longWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \tvalidated := conn.Validate(result)\n\
                      \tif validated == nil {\n\
                      \t\treturn errors.New(\"invalid\")\n\
                      \t}\n\
                      \treturn nil\n";
        let src = format!("package main\n\n{block}\n{block}");
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn reports_second_occurrence_line_number() {
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        let src = format!("package main\n\n{block}\n{block}");
        let findings = check_source(&src);
        // First occurrence starts at line 3 (1-indexed); the block is 6 lines, plus a
        // blank separator line, so the second occurrence starts at line 10.
        assert_eq!(findings[0].line, 10);
    }
}
