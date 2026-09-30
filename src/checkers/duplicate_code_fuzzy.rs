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

/// Opt-in near-duplicate counterpart to `duplicate-code` (issue #47): same window-hashing,
/// but the key is computed after replacing literals with a placeholder, so
/// `getUsersByStatus("active")` vs `("pending")` still match. Not in `config::core_checks()`
/// — whether a literal is worth parameterizing is a judgment call, so this is opt-in like
/// the prose checks (`docs/prose-checks.md`); enable via `.kibitzer/inspect.json`.
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

/// Skips common test-file naming conventions across supported languages. Backtesting
/// against `kubernetes/kubernetes` found ~97% of raw findings were `_test.go` table-driven
/// cases — already the Parameterize-Function shape, so flagging them back is backwards.
/// See `docs/duplicate-code-fuzzy.md`.
fn is_test_file(file: &Path) -> bool {
    // Rust integration tests and JS/TS test suites both conventionally live in a `tests/`
    // or `__tests__/` directory rather than following a filename suffix (e.g.
    // `crates/cli/tests/browser_session.rs`, confirmed as a real backtest hit before this
    // check was added — see docs/duplicate-code-fuzzy.md).
    if file
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some("tests") | Some("__tests__")))
    {
        return true;
    }

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

/// Matches a quoted string/char literal or standalone numeric literal — the
/// normalization target. Deliberately simple/line-based (no per-language AST): misses
/// forms like Python triple-quoted strings, which just falls back to requiring an exact
/// match, not a false finding.
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

/// Compares literals from each occurrence of a fuzzy-matched window (same length/position
/// in the common case, since the fuzzy key requires non-literal text to match exactly —
/// but not indexed unchecked below, since a NUL byte in real source could coincide with
/// `LITERAL_PLACEHOLDER` and defeat that guarantee) and reports which position(s) differ.
/// Returns `None` if every occurrence has identical literals (an exact duplicate) or there
/// are none.
fn differing_literals(literal_sets: &[Vec<String>]) -> Option<String> {
    let width = literal_sets.first()?.len();
    if width == 0 {
        return None;
    }
    let diffs: Vec<String> = (0..width)
        .filter_map(|i| {
            let values: BTreeSet<&str> = literal_sets
                .iter()
                .filter_map(|lits| lits.get(i).map(String::as_str))
                .collect();
            (values.len() > 1).then(|| values.into_iter().collect::<Vec<_>>().join(" vs "))
        })
        .collect();
    (!diffs.is_empty()).then(|| diffs.join(", "))
}

/// Rust keeps tests inline in the same file (`#[cfg(test)] mod tests { ... }`), unlike
/// every other language this checker supports, where a test file has its own name
/// `is_test_file` can filter on. Table-driven `#[test]` fixtures are the same
/// false-positive class `is_test_file` exists for, so stop scanning at the first
/// `#[cfg(test)]` line rather than descending into the test module at all.
fn trimmed_lines_before_rust_test_module(source: &str) -> Vec<String> {
    let mut trimmed: Vec<String> = source.lines().map(|l| l.trim().to_string()).collect();
    if let Some(test_mod_start) = trimmed.iter().position(|l| l == "#[cfg(test)]") {
        trimmed.truncate(test_mod_start);
    }
    trimmed
}

/// Fuzzy counterpart to `duplicate_code::find_duplicate_blocks` — same windowing and
/// occurrence-count bar, but grouped by literal-normalized text and reported only when
/// at least one literal actually differs across occurrences.
fn find_fuzzy_duplicate_blocks(source: &str) -> Vec<Finding> {
    let trimmed = trimmed_lines_before_rust_test_module(source);

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
    fn does_not_flag_a_file_in_a_tests_directory() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            block("active"),
            block("pending"),
            block("banned")
        );
        assert!(check_source_at(Path::new("crates/cli/tests/browser_session.rs"), &src).is_empty());
        assert!(check_source_at(Path::new("src/__tests__/thing.ts"), &src).is_empty());
    }

    #[test]
    fn does_not_flag_rust_test_fixtures_after_cfg_test() {
        // Rust tests live inline in the same file under `#[cfg(test)]`, so a `.rs` file's
        // own table-driven `#[test]` fixtures would otherwise trip this checker the same
        // way `_test.go` does for Go — `is_test_file` can't help since there's no separate
        // test filename to exclude.
        let src = format!(
            "fn real_code() {{}}\n\n#[cfg(test)]\nmod tests {{\n{}\n{}\n{}\n}}",
            block("active"),
            block("pending"),
            block("banned")
        );
        assert!(check_source_at(Path::new("thing.rs"), &src).is_empty());
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

    fn limited_block(status: &str, limit: &str) -> String {
        format!(
            "func getUsersByStatus{status}() []User {{\n\
             \trows := db.Query(\"SELECT * FROM users WHERE status = '{status}' LIMIT {limit}\")\n\
             \tvar users []User\n\
             \tfor rows.Next() {{\n\
             \t\tusers = append(users, scanUser(rows))\n\
             \t}}\n\
             \treturn users\n"
        )
    }

    #[test]
    fn reports_each_independently_differing_literal() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            limited_block("active", "10"),
            limited_block("pending", "20"),
            limited_block("banned", "30")
        );
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        // Both the status string and the numeric limit differ — both diffs, not just one.
        assert!(findings[0].message.contains("active"));
        assert!(findings[0].message.contains("10"));
        assert!(findings[0].message.contains(", ")); // two diff entries joined
    }

    fn backtick_block(pattern: &str) -> String {
        format!(
            "func compile{pattern}() *Regexp {{\n\
             \tre := regexp.MustCompile(`{pattern}`)\n\
             \tvar cache []*Regexp\n\
             \tfor range cache {{\n\
             \t\tcache = append(cache, re)\n\
             \t}}\n\
             \treturn re\n"
        )
    }

    #[test]
    fn flags_blocks_differing_only_by_a_backtick_literal() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            backtick_block("^foo$"),
            backtick_block("^bar$"),
            backtick_block("^baz$")
        );
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("^foo$"));
        assert!(findings[0].message.contains("^bar$"));
    }

    fn escaped_quote_block(who: &str) -> String {
        format!(
            "func greet{who}() string {{\n\
             \tmsg := \"say \\\"hi {who}\\\"\"\n\
             \tvar out string\n\
             \tfor range msg {{\n\
             \t\tout += msg\n\
             \t}}\n\
             \treturn out\n"
        )
    }

    #[test]
    fn flags_blocks_differing_only_by_a_literal_containing_an_escaped_quote() {
        let src = format!(
            "package main\n\n{}\n{}\n{}",
            escaped_quote_block("Alice"),
            escaped_quote_block("Bob"),
            escaped_quote_block("Cara")
        );
        let findings = check_source(&src);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("Alice"));
        assert!(findings[0].message.contains("Bob"));
    }

    #[test]
    fn differing_literals_does_not_panic_on_mismatched_widths() {
        // Can't happen through the normal fuzzy-key grouping path (see differing_literals'
        // doc comment), but differing_literals is exercised directly here so a future
        // change to that guarantee fails safe (returns a partial diff) instead of panicking
        // the whole checker run via unchecked indexing.
        let literal_sets = vec![
            vec!["a".to_string(), "b".to_string()],
            vec!["a".to_string()],
        ];
        assert_eq!(differing_literals(&literal_sets), None);
    }
}
