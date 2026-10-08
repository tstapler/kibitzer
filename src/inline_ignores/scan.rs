//! Finding directive comments in source text, with a single-entry memo per run.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use tree_sitter::Tree;

use super::parse::parse_comment_line;
use super::types::*;
use crate::checker::{GrammarCache, Language};
use crate::markdown_text::{line_for_offset, line_start_offsets};
use crate::tree_walk::{comment_kinds, walk_preorder};

/// True when only whitespace precedes `byte` on its line.
fn only_whitespace_before(source: &str, byte: usize) -> bool {
    let line_start = source[..byte].rfind('\n').map_or(0, |i| i + 1);
    source[line_start..byte].trim().is_empty()
}

/// Parses each line of `text` (starting on 1-based `first_row`), placing directives with
/// `end_row` and `whole_line`.
fn scan_text_lines(text: &str, first_row: usize, end_row: usize, whole_line: bool) -> Vec<Scanned> {
    text.split('\n')
        .enumerate()
        .filter_map(|(i, line)| {
            let row = Line::new(first_row + i);
            match parse_comment_line(line) {
                DirectiveParse::NotADirective => None,
                DirectiveParse::Valid(d) => Some(Scanned {
                    row,
                    parse: DirectiveParse::Valid(d.placed(row, Line::new(end_row), whole_line)),
                }),
                other => Some(Scanned { row, parse: other }),
            }
        })
        .collect()
}

/// Directives in the real comment nodes of a parsed file; string literals never match.
pub fn scan_code_comments(lang: Language, tree: &Tree, source: &str) -> Vec<Scanned> {
    let kinds = comment_kinds(lang);
    let mut out = Vec::new();
    walk_preorder(tree.root_node(), &mut |node| {
        if !kinds.contains(&node.kind()) {
            return true;
        }
        let text = node.utf8_text(source.as_bytes()).unwrap_or("");
        let first_row = node.start_position().row + 1;
        let end_row = node.end_position().row + 1;
        let whole_line = only_whitespace_before(source, node.start_byte());
        out.extend(scan_text_lines(text, first_row, end_row, whole_line));
        false
    });
    out
}

/// Directives in Markdown HTML comments; fenced and indented code arrive as text events,
/// so quoted examples never match.
pub fn scan_markdown(source: &str) -> Vec<Scanned> {
    use pulldown_cmark::{Event, Options, Parser};
    let line_starts = line_start_offsets(source);
    let parser = Parser::new_ext(source, Options::ENABLE_TABLES).into_offset_iter();
    let mut out = Vec::new();
    for (event, range) in parser {
        if let Event::Html(html) | Event::InlineHtml(html) = event {
            let first_row = line_for_offset(&line_starts, range.start);
            let end_row = first_row + html.trim_end().matches('\n').count();
            let whole_line = only_whitespace_before(source, range.start);
            out.extend(scan_text_lines(&html, first_row, end_row, whole_line));
        }
    }
    out
}

static LEADING_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?://|#|;|--)\s*(kibitzer:.*)$").unwrap());

/// Whole-line comments only, for files with no grammar: a trailing `echo "# kibitzer:..."`
/// is code, not a comment.
pub fn scan_leading_comments(source: &str) -> Vec<Scanned> {
    source
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let body = LEADING_COMMENT_RE.captures(line)?.get(1)?.as_str();
            let row = Line::new(i + 1);
            match parse_comment_line(body) {
                DirectiveParse::NotADirective => None,
                DirectiveParse::Valid(d) => Some(Scanned {
                    row,
                    parse: DirectiveParse::Valid(d.placed(row, row, true)),
                }),
                other => Some(Scanned { row, parse: other }),
            }
        })
        .collect()
}

/// Scans `source` for directives, parsing nothing when it lacks the substring `kibitzer`.
pub fn scan_directives_with_cache(cache: &GrammarCache, path: &Path, source: &str) -> Vec<Scanned> {
    if !source.contains("kibitzer") {
        return Vec::new();
    }
    if let Some(lang) = Language::for_path(path) {
        return match cache.parse(lang, source) {
            Ok(tree) => scan_code_comments(lang, &tree, source),
            Err(_) => scan_leading_comments(source),
        };
    }
    if path.extension().and_then(|e| e.to_str()) == Some("md") {
        return scan_markdown(source);
    }
    scan_leading_comments(source)
}

pub fn scan_directives(path: &Path, source: &str) -> Vec<Scanned> {
    scan_directives_with_cache(&GrammarCache::new(), path, source)
}

#[derive(Debug)]
pub(super) struct MemoEntry {
    pub(super) path: PathBuf,
    content_hash: u64,
    scanned: Arc<Vec<Scanned>>,
}

/// Single-entry scan cache: checks for one file run back to back, so one entry gives about one
/// scan per file. Content-hash keyed, so an edited file never returns stale directives.
/// Per-context (never `static`) so parallel tests, the daemon, and LSP cannot interfere.
#[derive(Debug, Default)]
pub struct ScanMemo {
    pub(super) entry: Mutex<Option<MemoEntry>>,
    /// Test-only counters: the seam for "one scan per file" and "no rerun without a directive row".
    #[cfg(test)]
    pub scans: AtomicUsize,
    #[cfg(test)]
    pub hash_calls: AtomicUsize,
    #[cfg(test)]
    pub raw_reruns: AtomicUsize,
}

impl ScanMemo {
    pub(crate) fn scan(&self, path: &Path, source: &str) -> Arc<Vec<Scanned>> {
        #[cfg(test)]
        self.hash_calls.fetch_add(1, Ordering::Relaxed);
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        let hash = hasher.finish();
        // Held across the scan on purpose: a concurrent caller for the same file waits, then hits the memo.
        let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(hit) = entry.as_ref()
            && hit.path == path
            && hit.content_hash == hash
        {
            return Arc::clone(&hit.scanned);
        }
        #[cfg(test)]
        self.scans.fetch_add(1, Ordering::Relaxed);
        let scanned = Arc::new(scan_directives(path, source));
        *entry = Some(MemoEntry {
            path: path.to_path_buf(),
            content_hash: hash,
            scanned: Arc::clone(&scanned),
        });
        scanned
    }
}
