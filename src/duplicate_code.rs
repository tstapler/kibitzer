use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Minimum number of consecutive lines a duplicated block must span before flagging —
/// short repeats (a closing brace, a single `return nil`) are normal, not copy-paste.
/// `pub(crate)`: shared with `duplicate_cross_file_checker`'s incremental index, which
/// applies the exact same windowing bar to a single edited file at a time.
pub(crate) const MIN_BLOCK_LINES: usize = 6;
/// Minimum combined trimmed-line length a block must have, filtering out blocks that
/// are mostly blank or single-token lines shared by coincidence rather than by copying.
const MIN_BLOCK_CHARS: usize = 60;
/// Minimum number of times a block must occur before flagging. A backtest against a
/// real transcript corpus showed two occurrences alone produces mostly benign,
/// individually-defensible repetition (e.g. a handful of near-identical test-fixture
/// calls); three or more is a much stronger copy-paste signal. `pub(crate)`: see
/// `MIN_BLOCK_LINES`.
pub(crate) const MIN_OCCURRENCES: usize = 3;

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
            "**/*.rs",
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

/// One location a cross-file duplicate block occurs at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossFileOccurrence {
    pub file: PathBuf,
    pub line: usize,
}

/// A block of code duplicated across two or more files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossFileDuplicate {
    pub occurrences: Vec<CrossFileOccurrence>,
}

/// Cross-file counterpart to `find_duplicate_blocks` — see #28. Same window-hashing
/// approach and thresholds, pooled across every file in `files` instead of scoped to
/// one, and only reported once a group spans at least two distinct files: a block that
/// only repeats within a single file is `duplicate-code`'s finding, not this one's.
pub fn find_cross_file_duplicates(files: &[(PathBuf, String)]) -> Vec<CrossFileDuplicate> {
    let normalized: Vec<Vec<String>> = files
        .iter()
        .map(|(_, src)| src.lines().map(|l| l.trim().to_string()).collect())
        .collect();
    let occurrences = index_cross_file_windows(&normalized);
    group_cross_file_duplicates(occurrences, files)
}

/// Slides a `MIN_BLOCK_LINES`-line window over every file's normalized lines, indexing
/// each distinct window's text against every `(file index, start line)` it occurs at —
/// the multi-file counterpart to `find_duplicate_blocks`'s single-file occurrence map.
/// Borrows its window keys from `normalized`, which the caller must keep alive at least
/// as long as the returned map.
fn index_cross_file_windows(normalized: &[Vec<String>]) -> HashMap<&[String], Vec<(usize, usize)>> {
    let mut occurrences: HashMap<&[String], Vec<(usize, usize)>> = HashMap::new();
    for (file_idx, lines) in normalized.iter().enumerate() {
        for (start, window) in qualifying_windows(lines) {
            occurrences
                .entry(window)
                .or_default()
                .push((file_idx, start));
        }
    }
    occurrences
}

/// Returns the `MIN_BLOCK_LINES`-line window at `start`, or `None` if it spans a blank
/// line or falls short of `MIN_BLOCK_CHARS` — the same "not meaningful duplication"
/// filter `find_duplicate_blocks` applies. `pub(crate)`: see `MIN_BLOCK_LINES`.
pub(crate) fn qualifying_window(lines: &[String], start: usize) -> Option<&[String]> {
    let window = &lines[start..start + MIN_BLOCK_LINES];
    if window.iter().any(|l| l.is_empty()) {
        return None;
    }
    let total_chars: usize = window.iter().map(|l| l.len()).sum();
    if total_chars < MIN_BLOCK_CHARS {
        return None;
    }
    Some(window)
}

/// Every `MIN_BLOCK_LINES`-line qualifying window in `lines`, as `(0-indexed start,
/// window slice)` — the shared "slide a window and filter" loop every caller that scans
/// a file's lines for duplicate candidates builds on (`index_cross_file_windows` here,
/// and `duplicate_cross_file_checker`'s incremental per-file index).
pub(crate) fn qualifying_windows(lines: &[String]) -> impl Iterator<Item = (usize, &[String])> {
    let starts = if lines.len() < MIN_BLOCK_LINES {
        0..0
    } else {
        0..(lines.len() - MIN_BLOCK_LINES + 1)
    };
    starts.filter_map(move |start| qualifying_window(lines, start).map(|w| (start, w)))
}

/// Filters `occurrences` down to groups worth reporting (at least `MIN_OCCURRENCES`,
/// spanning at least 2 files), then collapses shifted-window duplicates from one
/// longer duplicated region into a single `CrossFileDuplicate` each — the multi-file
/// counterpart to `find_duplicate_blocks`'s single `covered_until` cursor, tracked per
/// file here since each file's copy of a region starts at its own line number.
fn group_cross_file_duplicates(
    occurrences: HashMap<&[String], Vec<(usize, usize)>>,
    files: &[(PathBuf, String)],
) -> Vec<CrossFileDuplicate> {
    let mut groups: Vec<Vec<(usize, usize)>> = occurrences
        .into_values()
        .filter(|locs| locs.len() >= MIN_OCCURRENCES && spans_multiple_files(locs))
        .collect();
    groups.sort();

    let mut covered_until: HashMap<usize, usize> = HashMap::new();
    let mut duplicates = Vec::new();
    for locs in groups {
        let already_covered = locs
            .iter()
            .any(|(file_idx, start)| *start < *covered_until.get(file_idx).unwrap_or(&0));
        if already_covered {
            continue;
        }

        duplicates.push(to_cross_file_duplicate(&locs, files));
        for (file_idx, start) in &locs {
            let until = covered_until.entry(*file_idx).or_insert(0);
            *until = (*until).max(start + MIN_BLOCK_LINES);
        }
    }
    duplicates
}

fn spans_multiple_files(locs: &[(usize, usize)]) -> bool {
    locs.iter()
        .map(|(file_idx, _)| file_idx)
        .collect::<HashSet<_>>()
        .len()
        >= 2
}

fn to_cross_file_duplicate(
    locs: &[(usize, usize)],
    files: &[(PathBuf, String)],
) -> CrossFileDuplicate {
    CrossFileDuplicate {
        occurrences: locs
            .iter()
            .map(|(file_idx, start)| CrossFileOccurrence {
                file: files[*file_idx].0.clone(),
                line: start + 1,
            })
            .collect(),
    }
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

    const BLOCK: &str = "func doWork(id string) error {\n\
                          \tconn := openConnection(id)\n\
                          \tdefer conn.Close()\n\
                          \tresult := conn.Fetch(id)\n\
                          \tlog.Printf(\"fetched %v\", result)\n\
                          \treturn conn.Validate(result)\n";

    fn cross_file_fixture(occurrence_files: &[&str]) -> Vec<CrossFileDuplicate> {
        let files: Vec<(PathBuf, String)> = occurrence_files
            .iter()
            .map(|name| (PathBuf::from(name), format!("package main\n\n{BLOCK}")))
            .collect();
        find_cross_file_duplicates(&files)
    }

    #[test]
    fn flags_block_duplicated_across_three_files() {
        let dups = cross_file_fixture(&["a.go", "b.go", "c.go"]);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].occurrences.len(), 3);
        let files: Vec<&str> = dups[0]
            .occurrences
            .iter()
            .map(|o| o.file.to_str().unwrap())
            .collect();
        assert_eq!(files, vec!["a.go", "b.go", "c.go"]);
        assert!(dups[0].occurrences.iter().all(|o| o.line == 3));
    }

    #[test]
    fn does_not_flag_below_min_occurrences_even_across_files() {
        let dups = cross_file_fixture(&["a.go", "b.go"]);
        assert!(dups.is_empty());
    }

    #[test]
    fn does_not_flag_block_repeated_only_within_one_file() {
        // Three occurrences, but all in the same file — that's `duplicate-code`'s
        // (single-file) finding, not this cross-file checker's.
        let src = format!("package main\n\n{BLOCK}\n{BLOCK}\n{BLOCK}");
        let files = vec![(PathBuf::from("a.go"), src)];
        assert!(find_cross_file_duplicates(&files).is_empty());
    }

    #[test]
    fn only_flags_once_for_a_longer_cross_file_duplicate_run() {
        let long_block = "func longWork(id string) error {\n\
                           \tconn := openConnection(id)\n\
                           \tdefer conn.Close()\n\
                           \tresult := conn.Fetch(id)\n\
                           \tlog.Printf(\"fetched %v\", result)\n\
                           \tvalidated := conn.Validate(result)\n\
                           \tif validated == nil {\n\
                           \t\treturn errors.New(\"invalid\")\n\
                           \t}\n\
                           \treturn nil\n";
        let files: Vec<(PathBuf, String)> = ["a.go", "b.go", "c.go"]
            .iter()
            .map(|name| (PathBuf::from(name), format!("package main\n\n{long_block}")))
            .collect();
        let dups = find_cross_file_duplicates(&files);
        assert_eq!(dups.len(), 1);
    }
}
