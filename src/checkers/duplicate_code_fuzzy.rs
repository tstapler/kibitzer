use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::checkers::duplicate_code::{
    DuplicateCodeChecker, MIN_BLOCK_LINES, MIN_OCCURRENCES, qualifying_windows,
};
use crate::checkers::file_size::is_generated;

/// Opt-in near-duplicate counterpart to `duplicate-code` (issue #47) — same window-hashing
/// approach, but the window key is computed after replacing every string/numeric literal
/// with a placeholder, so two blocks that are copy-pasted except for one literal (e.g.
/// `getUsersByStatus("active")` vs `getUsersByStatus("pending")`) still match. Not wired
/// into `config::core_checks()`: unlike exact duplication, "these two blocks differ only by
/// a literal" is a judgment call about whether that literal is worth parameterizing, so
/// this stays opt-in the same way the backtested prose checks in `docs/prose-checks.md` do
/// — enable it per-repo via `.kibitzer/inspect.json` (see `docs/suppressing-checks.md`).
pub struct FuzzyDuplicateCodeChecker;

impl Checker for FuzzyDuplicateCodeChecker {
    fn name(&self) -> &str {
        "duplicate-code-fuzzy"
    }

    fn description(&self) -> &str {
        "flags blocks that are copy-pasted except for a differing literal — a candidate for Parameterize Function"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        DuplicateCodeChecker.file_globs()
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if is_generated(ctx.source) || is_test_file(file) {
            return Ok(Vec::new());
        }
        Ok(find_fuzzy_duplicate_blocks(ctx.source))
    }
}

/// Skips this checker's supported languages' common test-file naming conventions.
/// Backtesting against `kubernetes/kubernetes` (`docs/backtest-repos.md`) found ~97%
/// of raw findings landed in `_test.go` table-driven test cases — those are *already*
/// the shared-body-plus-varying-literal shape Parameterize Function recommends moving
/// toward, so flagging them back as "extract a shared function" is backwards, not a
/// genuine catch. See `docs/duplicate-code-fuzzy.md`.
fn is_test_file(file: &Path) -> bool {
    let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
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
        ]
        .iter()
        .any(|suffix| lower.ends_with(suffix))
        || (lower.ends_with(".java")
            && (name.ends_with("Test.java") || name.ends_with("Tests.java")))
        || (lower.ends_with(".kt") && (name.ends_with("Test.kt") || name.ends_with("Tests.kt")))
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(FuzzyDuplicateCodeChecker)])
}

/// Matches a quoted string/char literal (double, single, or backtick-quoted) or a
/// standalone numeric literal — the placeholder-normalization target. Deliberately
/// simple/line-based, like the rest of this checker family's window hashing (no
/// per-language AST): it can miss language-specific literal forms (e.g. Python
/// triple-quoted strings spanning a line boundary), which just means those blocks fall
/// back to needing an exact match instead of a fuzzy one, not a false finding.
static LITERAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`[^`]*`|\b\d+(?:\.\d+)?\b"#).unwrap()
});

const LITERAL_PLACEHOLDER: &str = "\u{0}LIT\u{0}";

fn normalize_literals(line: &str) -> String {
    LITERAL_RE
        .replace_all(line, LITERAL_PLACEHOLDER)
        .into_owned()
}

fn extract_literals(window: &[String]) -> Vec<String> {
    window
        .iter()
        .flat_map(|line| LITERAL_RE.find_iter(line).map(|m| m.as_str().to_string()))
        .collect()
}

/// Compares the literals extracted from each occurrence of a fuzzy-matched window
/// (guaranteed same length and position, since the fuzzy key requires the surrounding
/// non-literal text to match exactly) and reports which position(s) actually differ, as
/// `"value" vs "value"` pairs. Returns `None` when every occurrence has identical
/// literals (an exact duplicate, already covered by `duplicate-code`) or the block has
/// no literals to normalize at all.
fn differing_literals(literal_sets: &[Vec<String>]) -> Option<String> {
    let width = literal_sets.first()?.len();
    if width == 0 {
        return None;
    }
    let diffs: Vec<String> = (0..width)
        .filter_map(|i| {
            let values: BTreeSet<&str> = literal_sets.iter().map(|lits| lits[i].as_str()).collect();
            (values.len() > 1).then(|| values.into_iter().collect::<Vec<_>>().join(" vs "))
        })
        .collect();
    (!diffs.is_empty()).then(|| diffs.join(", "))
}

/// Fuzzy counterpart to `duplicate_code::find_duplicate_blocks` — same windowing and
/// occurrence-count bar, but grouped by literal-normalized text and reported only when
/// at least one literal actually differs across occurrences.
fn find_fuzzy_duplicate_blocks(source: &str) -> Vec<Finding> {
    let trimmed: Vec<String> = source.lines().map(|l| l.trim().to_string()).collect();

    let mut occurrences: HashMap<String, Vec<usize>> = HashMap::new();
    for (start, window) in qualifying_windows(&trimmed) {
        let fuzzy_key = window
            .iter()
            .map(|l| normalize_literals(l))
            .collect::<Vec<_>>()
            .join("\n");
        occurrences.entry(fuzzy_key).or_default().push(start);
    }

    let mut groups: Vec<Vec<usize>> = occurrences
        .into_values()
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
        if let Some(finding) = fuzzy_finding_for_group(&trimmed, &starts) {
            findings.push(finding);
        }
        covered_until = first + MIN_BLOCK_LINES;
    }

    findings
}

/// Builds the finding for one fuzzy-matched group, or `None` when every occurrence's
/// literals turn out identical (an exact duplicate, not a genuine near-duplicate).
fn fuzzy_finding_for_group(trimmed: &[String], starts: &[usize]) -> Option<Finding> {
    let literal_sets: Vec<Vec<String>> = starts
        .iter()
        .map(|&s| extract_literals(&trimmed[s..s + MIN_BLOCK_LINES]))
        .collect();
    let diffs = differing_literals(&literal_sets)?;

    let lines: Vec<String> = starts.iter().map(|s| (s + 1).to_string()).collect();
    Some(Finding {
        line: starts[starts.len() - 1] + 1,
        message: format!(
            "{MIN_BLOCK_LINES}-line block repeated {} times, differing only by {diffs} (lines {}) — consider extracting a shared function parameterized on the differing value",
            starts.len(),
            lines.join(", ")
        ),
    })
}

#[cfg(test)]
fn check_source(src: &str) -> Vec<Finding> {
    check_source_at(Path::new("<source>.go"), src)
}

#[cfg(test)]
fn check_source_at(file: &Path, src: &str) -> Vec<Finding> {
    let ctx = CheckContext {
        source: src,
        tree: None,
    };
    FuzzyDuplicateCodeChecker.check(file, &ctx).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(status: &str) -> String {
        format!(
            "func getUsersByStatus{status}() []User {{\n\
             \trows := db.Query(\"SELECT * FROM users WHERE status = '{status}'\")\n\
             \tvar users []User\n\
             \tfor rows.Next() {{\n\
             \t\tusers = append(users, scanUser(rows))\n\
             \t}}\n\
             \treturn users\n"
        )
    }

    #[test]
    fn flags_blocks_differing_only_by_a_literal() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            block("active"),
            block("pending"),
            block("banned")
        );
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("differing only by"));
        assert!(findings[0].message.contains("active"));
        assert!(findings[0].message.contains("pending"));
        assert!(findings[0].message.contains("banned"));
    }

    #[test]
    fn does_not_flag_exact_duplicates() {
        // Same literal every time — an exact duplicate, `duplicate-code`'s finding, not
        // this checker's.
        let one = block("active");
        let src = format!("package main\n\n{one}\n{one}\n{one}");
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_below_min_occurrences() {
        let src = format!("package main\n\n{}\n{}", block("active"), block("pending"));
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_blocks_with_no_literals() {
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tvalidated := conn.Validate(result)\n\
                      \treturn validated\n";
        let src = format!("package main\n\n{block}\n{block}\n{block}");
        assert!(check_source(&src).is_empty());
    }

    #[test]
    fn does_not_flag_a_go_test_file() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            block("active"),
            block("pending"),
            block("banned")
        );
        assert!(check_source_at(Path::new("thing_test.go"), &src).is_empty());
    }

    #[test]
    fn does_not_flag_a_python_test_file() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            block("active"),
            block("pending"),
            block("banned")
        );
        assert!(check_source_at(Path::new("test_thing.py"), &src).is_empty());
        assert!(check_source_at(Path::new("thing_test.py"), &src).is_empty());
    }

    #[test]
    fn does_not_flag_a_js_spec_file() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            block("active"),
            block("pending"),
            block("banned")
        );
        assert!(check_source_at(Path::new("thing.spec.ts"), &src).is_empty());
    }
}
