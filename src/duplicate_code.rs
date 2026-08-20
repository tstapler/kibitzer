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
/// Minimum number of times a block must occur before flagging. A backtest against a
/// real transcript corpus showed two occurrences alone produces mostly benign,
/// individually-defensible repetition (e.g. a handful of near-identical test-fixture
/// calls); three or more is a much stronger copy-paste signal.
const MIN_OCCURRENCES: usize = 3;

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

/// Slides a `MIN_BLOCK_LINES`-line window over `source`'s trimmed lines, grouping
/// windows by identical text to find every start position each distinct block occurs
/// at. Windows spanning a blank line, or too short on total content, are skipped so
/// incidental repeats (blank padding, lone `}`) don't fire.
///
/// A block only produces a finding once it occurs at least `MIN_OCCURRENCES` times.
/// Blocks longer than `MIN_BLOCK_LINES` produce several overlapping windows (one per
/// shifted start) that all repeat in lockstep — reported once via `covered_until`,
/// which skips any candidate group whose first occurrence falls inside a block already
/// reported, the same way the block's later lines would.
fn find_duplicate_blocks(source: &str) -> Vec<Finding> {
    let normalized: Vec<String> = source.lines().map(|l| l.trim().to_string()).collect();
    if normalized.len() < MIN_BLOCK_LINES {
        return Vec::new();
    }

    let mut occurrences: HashMap<&[String], Vec<usize>> = HashMap::new();
    for start in 0..=(normalized.len() - MIN_BLOCK_LINES) {
        let window = &normalized[start..start + MIN_BLOCK_LINES];
        if window.iter().any(|l| l.is_empty()) {
            continue;
        }
        let total_chars: usize = window.iter().map(|l| l.len()).sum();
        if total_chars < MIN_BLOCK_CHARS {
            continue;
        }
        occurrences.entry(window).or_default().push(start);
    }

    let mut groups: Vec<&Vec<usize>> = occurrences
        .values()
        .filter(|starts| starts.len() >= MIN_OCCURRENCES)
        .collect();
    groups.sort_by_key(|starts| starts[0]);

    let mut findings = Vec::new();
    let mut covered_until = 0usize;
    for starts in groups {
        let first = starts[0];
        if first < covered_until {
            continue;
        }
        let lines: Vec<String> = starts.iter().map(|s| (s + 1).to_string()).collect();
        findings.push(Finding {
            line: starts[starts.len() - 1] + 1,
            message: format!(
                "{MIN_BLOCK_LINES}-line block repeated {} times (lines {}) — consider extracting a shared function",
                starts.len(),
                lines.join(", ")
            ),
        });
        covered_until = first + MIN_BLOCK_LINES;
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
    fn does_not_flag_a_block_repeated_only_twice() {
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        let src = format!("package main\n\n{block}\n{block}");
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn flags_duplicated_block() {
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        let src = format!("package main\n\n{block}\n{block}\n{block}");
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("repeated 3 times"));
    }

    #[test]
    fn does_not_flag_short_duplicate_blocks() {
        let block = "\tconn := open()\n\tconn.Close()\n";
        let src = format!("package main\n\n{block}\n{block}\n{block}");
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_trivial_repeated_lines() {
        // Six short, low-content lines (closing braces / blank-ish) repeated three
        // times — below MIN_BLOCK_CHARS, so shouldn't count as meaningful duplication.
        let block = "}\n}\n}\n}\n}\n}\n";
        let src = format!("package main\n\n{block}\n{block}\n{block}");
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
        // The block is 10 lines, longer than MIN_BLOCK_LINES (6), so it produces
        // several overlapping shifted windows that all repeat in lockstep across the
        // three copies — they must collapse into a single finding, not one per shift.
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
        let src = format!("package main\n\n{block}\n{block}\n{block}");
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn reports_last_occurrence_line_number_and_lists_all_occurrences() {
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        let src = format!("package main\n\n{block}\n{block}\n{block}");
        let findings = check_source(&src);
        // Occurrences start at (1-indexed) lines 3, 10, 17; the finding points at the
        // last one and lists all three in its message.
        assert_eq!(findings[0].line, 17);
        assert!(findings[0].message.contains("lines 3, 10, 17"));
    }
}
