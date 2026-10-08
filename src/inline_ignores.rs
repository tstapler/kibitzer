//! Inline `kibitzer:ignore <rule>[,<rule>...] -- <reason>` directives: parsing only.
//! Matching and application at the check seam live in later tasks of the
//! inline-ignore-syntax plan.
//!
//! Items here are not yet called from non-test code; later tasks wire them in.
#![allow(dead_code)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tree_sitter::Tree;

use crate::checker::{Finding, GrammarCache, Language};
use crate::config::Severity;
use crate::markdown_text::{line_for_offset, line_start_offsets};
use crate::tree_walk::{comment_kinds, walk_preorder};

const MARKER: &str = "kibitzer:ignore";

/// 1-based line number; tree-sitter's 0-based rows are converted once at the scan boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Line(NonZeroUsize);

impl Line {
    /// Line 0 (a checker that reports "no line") is treated as line 1.
    pub fn new(one_based: usize) -> Self {
        Line(NonZeroUsize::new(one_based).unwrap_or(NonZeroUsize::MIN))
    }

    pub fn get(self) -> usize {
        self.0.get()
    }
}

/// A rule a directive names; `[a-z0-9-]+`, so a doc placeholder like `<rule>` is rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RuleId(String);

fn is_rule_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

impl RuleId {
    pub fn new(text: &str) -> Option<Self> {
        (!text.is_empty() && text.chars().all(is_rule_char)).then(|| RuleId(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeakReason {
    TooShort,
    RuleEcho,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonError {
    Blank,
    Weak(WeakReason),
}

/// The justification after ` -- `. The constructor enforces a mechanical floor: not blank,
/// not an echo of a listed rule id, and more than one word. It stops `-- needed`, not a
/// determined two-word lie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason(String);

fn fold_for_echo(text: &str) -> String {
    text.to_lowercase()
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Reason {
    pub fn new(text: &str, rules: &[RuleId]) -> Result<Self, ReasonError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ReasonError::Blank);
        }
        let folded = fold_for_echo(text);
        if rules.iter().any(|r| fold_for_echo(r.as_str()) == folded) {
            return Err(ReasonError::Weak(WeakReason::RuleEcho));
        }
        if text.split_whitespace().count() < 2 {
            return Err(ReasonError::Weak(WeakReason::TooShort));
        }
        Ok(Reason(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One parsed, well-formed ignore comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    rules: Vec<RuleId>,
    pub reason: Reason,
    pub start_line: Line,
    pub end_line: Line,
    /// No code precedes the comment on its start row.
    pub whole_line: bool,
}

impl Directive {
    /// `None` when `rules` is empty: a directive that names nothing is not representable.
    pub fn new(
        rules: Vec<RuleId>,
        reason: Reason,
        start_line: Line,
        end_line: Line,
        whole_line: bool,
    ) -> Option<Self> {
        (!rules.is_empty()).then_some(Directive {
            rules,
            reason,
            start_line,
            end_line,
            whole_line,
        })
    }

    pub fn rules(&self) -> &[RuleId] {
        &self.rules
    }

    fn placed(mut self, start: Line, end: Line, whole_line: bool) -> Self {
        self.start_line = start;
        self.end_line = end;
        self.whole_line = whole_line;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedReason {
    MissingRule,
    MissingReason,
    WeakReason(WeakReason),
    /// `kibitzer: ignore`, `kibitzer:false-positive`, `kibitzer:allow`, and similar.
    NearMissMarker,
    NotAtCommentStart,
    EmDashSeparator,
    BadRuleList,
}

/// Result of parsing one comment line. A malformed directive never suppresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveParse {
    Valid(Directive),
    Malformed(MalformedReason),
    NotADirective,
}

/// A parse result and the 1-based row of the comment line it came from.
pub type Scanned = (Line, DirectiveParse);

const LEADERS: &[&str] = &["<!--", "///", "//!", "//", "/**", "/*", "#", "*"];

/// Comment text without its leader (`//`, `#`, `/*`, `*`, `<!--`) and trailer (`*/`, `-->`).
fn strip_comment_syntax(line: &str) -> &str {
    let mut text = line.trim();
    if let Some(leader) = LEADERS.iter().find(|l| text.starts_with(**l)) {
        text = &text[leader.len()..];
    }
    let text = text.trim();
    let text = text
        .strip_suffix("*/")
        .or_else(|| text.strip_suffix("-->"))
        .unwrap_or(text);
    text.trim()
}

/// Whether any line of the comment `text` begins (after its leader) with `kibitzer:`.
pub fn is_directive_comment(text: &str) -> bool {
    text.lines()
        .any(|l| strip_comment_syntax(l).starts_with("kibitzer:"))
}

static NEAR_MISS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^kibitzer\s*:\s*(ignore|disable|allow|suppress|false[-_ ]positive)\b").unwrap()
});

static LATER_DIRECTIVE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bkibitzer:ignore\s+[a-z0-9-]+(?:,[a-z0-9-]+)*\s+--\s+\S").unwrap()
});

static NEAR_MISS_ANYWHERE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"kibitzer\s*:\s*(?:ignore|disable|allow|suppress|false[-_ ]positive)\b").unwrap()
});

/// The near-miss marker as written on a source line, for echoing back in the repair message.
pub(crate) fn near_miss_text(line: &str) -> Option<&str> {
    NEAR_MISS_ANYWHERE_RE.find(line).map(|m| m.as_str())
}

/// A directive line as written, normalised for repair messages: the rule list (split on commas
/// and whitespace, rejoined with commas) and the reason text after the separator, if any.
/// Reads the raw source line because the parse result does not carry the text.
pub(crate) fn echo_parts(line: &str) -> (String, String) {
    let rest = line
        .find(MARKER)
        .map_or("", |i| line[i + MARKER.len()..].trim());
    let rest = rest
        .strip_suffix("*/")
        .or_else(|| rest.strip_suffix("-->"))
        .unwrap_or(rest)
        .trim();
    let (rules, reason) = match find_separator(rest) {
        Some(i) => (&rest[..i], rest[i + 2..].trim()),
        None => match rest.find(['\u{2014}', '\u{2013}']) {
            Some(i) => (
                &rest[..i],
                rest[i..]
                    .trim_start_matches(['\u{2014}', '\u{2013}'])
                    .trim(),
            ),
            None => (rest, ""),
        },
    };
    let rules = rules
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|r| !r.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    (rules, reason.to_string())
}

/// Finds a standalone `--` token (whitespace or text edge on both sides).
fn find_separator(rest: &str) -> Option<usize> {
    rest.match_indices("--").map(|(i, _)| i).find(|&i| {
        let before_ok = rest[..i]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        let after_ok = rest[i + 2..].chars().next().is_none_or(char::is_whitespace);
        before_ok && after_ok
    })
}

fn has_dash_separator(rest: &str) -> bool {
    rest.split_whitespace()
        .any(|t| t == "\u{2014}" || t == "\u{2013}")
}

/// `Ok(None)` means the rule text is a doc placeholder (`<rule>`), not a directive.
fn parse_rule_list(text: &str) -> Result<Option<Vec<RuleId>>, MalformedReason> {
    if text.contains(['<', '>']) {
        return Ok(None);
    }
    if text.is_empty() {
        return Err(MalformedReason::MissingRule);
    }
    let mut rules: Vec<RuleId> = Vec::new();
    for element in text.split(',') {
        let rule = RuleId::new(element).ok_or(MalformedReason::BadRuleList)?;
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    Ok(Some(rules))
}

/// Parses one comment line (leader and trailer included or not). The returned directive
/// carries placeholder positions; scanners place it with its real rows.
pub fn parse_comment_line(text: &str) -> DirectiveParse {
    use DirectiveParse::{Malformed, NotADirective, Valid};
    let text = strip_comment_syntax(text);

    let Some(rest) = text
        .strip_prefix(MARKER)
        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
    else {
        if NEAR_MISS_RE.is_match(text) {
            return Malformed(MalformedReason::NearMissMarker);
        }
        if LATER_DIRECTIVE_RE.is_match(text) {
            return Malformed(MalformedReason::NotAtCommentStart);
        }
        return NotADirective;
    };

    let rest = rest.trim();
    let Some(sep) = find_separator(rest) else {
        if rest.is_empty() {
            return Malformed(MalformedReason::MissingRule);
        }
        if has_dash_separator(rest) {
            return Malformed(MalformedReason::EmDashSeparator);
        }
        return match parse_rule_list(rest) {
            Ok(None) => NotADirective,
            _ => Malformed(MalformedReason::MissingReason),
        };
    };

    let rules = match parse_rule_list(rest[..sep].trim()) {
        Ok(Some(rules)) => rules,
        Ok(None) => return NotADirective,
        Err(reason) => return Malformed(reason),
    };
    match Reason::new(&rest[sep + 2..], &rules) {
        Ok(reason) => {
            let one = Line::new(1);
            Directive::new(rules, reason, one, one, true)
                .map_or(Malformed(MalformedReason::MissingRule), Valid)
        }
        Err(ReasonError::Blank) => Malformed(MalformedReason::MissingReason),
        Err(ReasonError::Weak(kind)) => Malformed(MalformedReason::WeakReason(kind)),
    }
}

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
                DirectiveParse::Valid(d) => Some((
                    row,
                    DirectiveParse::Valid(d.placed(row, Line::new(end_row), whole_line)),
                )),
                other => Some((row, other)),
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
                DirectiveParse::Valid(d) => {
                    Some((row, DirectiveParse::Valid(d.placed(row, row, true))))
                }
                other => Some((row, other)),
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

/// Rules whose anchor is not a statement: a directive in the file head also covers them.
pub(crate) const FILE_SCOPE_RULES: &[&str] = &["file-size", "file-complexity"];
pub(crate) const FILE_HEAD_LINES: usize = 10;
/// Diagnostics about directives themselves; suppressing them would hide the repair prompt.
const META_RULES: &[&str] = &[
    "inline-ignore",
    "ignore-syntax",
    "unused-ignore",
    "ignore-volume",
    "blocking-suppressed",
];
/// Checkers whose leading `[x]` prefix is data (a link ref id), not a rule id.
const DYNAMIC_PREFIX_CHECKERS: &[&str] = &["markdown-link-integrity"];

/// The leading `[x]` of a finding message, if any.
fn bracket_prefix(message: &str) -> Option<&str> {
    let rest = message.strip_prefix('[')?;
    rest.split_once(']').map(|(rule, _)| rule)
}

/// Matching never consults a rule table, so a stale table cannot make an ignore fail closed.
pub(crate) fn rule_matches(directive_rule: &RuleId, checker_name: &str, message: &str) -> bool {
    if META_RULES.contains(&directive_rule.as_str()) {
        return false;
    }
    if directive_rule.as_str() == checker_name {
        return true;
    }
    !DYNAMIC_PREFIX_CHECKERS.contains(&checker_name)
        && bracket_prefix(message) == Some(directive_rule.as_str())
}

/// A whole-line directive also covers the row below its last row; a trailing one only its own rows.
pub(crate) fn covers(d: &Directive, finding_line: Line, rule_match: bool) -> bool {
    rule_match
        && ((d.start_line <= finding_line && finding_line <= d.end_line)
            || (d.whole_line && finding_line.get() == d.end_line.get() + 1))
}

/// The rule a finding answers to: its leading `[x]` prefix, or the checker name when it has
/// none or the prefix is data (see `DYNAMIC_PREFIX_CHECKERS`).
pub(crate) fn anchor_rule(checker_name: &str, finding: &Finding) -> RuleId {
    let by_name = || RuleId(checker_name.to_string());
    if DYNAMIC_PREFIX_CHECKERS.contains(&checker_name) {
        return by_name();
    }
    bracket_prefix(&finding.message)
        .and_then(RuleId::new)
        .unwrap_or_else(by_name)
}

/// How a comment is written in a file type: text before and after the directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommentLeader {
    pub open: &'static str,
    pub close: &'static str,
}

/// Leader by grammar language, `.md`, else `#` (the fallback scanner accepts it).
pub(crate) fn comment_leader(path: &Path) -> CommentLeader {
    let (open, close) = match Language::for_path(path) {
        Some(Language::Python) => ("#", ""),
        Some(_) => ("//", ""),
        None if path.extension().is_some_and(|e| e == "md") => ("<!--", " -->"),
        None => ("#", ""),
    };
    CommentLeader { open, close }
}

const HINT_RULE_LIMIT: usize = 6;

/// The footer's one-line syntax teaching. `anchor` is the first shown finding; without it the
/// generic `<rule>` form is rendered and `rule_ids` is ignored.
pub(crate) fn syntax_hint(
    path: &Path,
    anchor: Option<(&RuleId, Line)>,
    rule_ids: &[&RuleId],
) -> String {
    syntax_hint_limited(path, anchor, rule_ids, HINT_RULE_LIMIT)
}

/// As `syntax_hint`, listing at most `max_rules` ids (then `...`); the footer budget shrinks it.
pub(crate) fn syntax_hint_limited(
    path: &Path,
    anchor: Option<(&RuleId, Line)>,
    rule_ids: &[&RuleId],
    max_rules: usize,
) -> String {
    let leader = comment_leader(path);
    let rule_text = anchor.map_or("<rule>", |(rule, _)| rule.as_str());
    let mut hint = format!(
        "Dismiss a judged finding: {} kibitzer:ignore {rule_text} -- <why>{}, ",
        leader.open, leader.close
    );
    match anchor {
        Some((rule, line)) if FILE_SCOPE_RULES.contains(&rule.as_str()) => hint.push_str(&format!(
            "in the first {FILE_HEAD_LINES} lines or on the anchor line (line {})",
            line.get()
        )),
        Some((_, line)) => hint.push_str(&format!(
            "on its own line directly above line {} (or at its end)",
            line.get()
        )),
        None => hint.push_str("on its own line directly above the flagged line (or at its end)"),
    }
    hint.push('.');
    if anchor.is_some() && !rule_ids.is_empty() {
        let shown: Vec<&str> = rule_ids
            .iter()
            .take(max_rules.max(1))
            .map(|r| r.as_str())
            .collect();
        let more = if rule_ids.len() > shown.len() {
            ", ..."
        } else {
            ""
        };
        hint.push_str(&format!(" Rules: {}{more}.", shown.join(", ")));
    }
    hint
}

/// Rule ids that appear as `[id]` message prefixes. Advisory only: it feeds the unknown-rule
/// suggestion and never decides suppression, so a stale entry costs a hint, not an ignore.
/// `known_rules_should_CoverEveryStaticRulePrefix_When_DriftGuardScansCheckerSources` guards drift.
pub(crate) const KNOWN_RULES: &[&str] = &[
    "commented-out-code",
    "over-commented",
    "verbose-comment",
    "long-function",
    "deep-nesting",
    "long-parameter-list",
    "flag-argument",
    "unreachable-code",
    "replace-magic-literal",
    "extract-variable",
    "file-size",
    "single-call-site-delegation",
    "god-class",
    "isp-fat-interface",
    "unreferenced-private-symbol",
    "go-type-switch-density",
    "go-encapsulate-collection",
];

/// `KNOWN_RULES` plus every registered checker name, built once (the registry is fixed per process).
static SUGGESTION_NAMES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut names: Vec<String> = KNOWN_RULES.iter().map(|r| r.to_string()).collect();
    names.extend(
        crate::checker::registry()
            .iter()
            .map(|c| c.name().to_string()),
    );
    names.sort();
    names.dedup();
    names
});

/// Whether `rule` names something kibitzer knows. Meta rules count as known so they are not
/// reported as typos, though they never suppress.
pub(crate) fn known_rule(rule: &str) -> bool {
    META_RULES.contains(&rule) || SUGGESTION_NAMES.iter().any(|n| n == rule)
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let up = row[j + 1];
            row[j + 1] = (diag + usize::from(ca != *cb)).min(up + 1).min(row[j] + 1);
            diag = up;
        }
    }
    row[b.len()]
}

const MAX_SUGGESTION_DISTANCE: usize = 2;
const MIN_PREFIX_SUGGESTION_LEN: usize = 4;

/// The closest known name within edit distance 2, or one the unknown text is a prefix of
/// (`flag-arg` for `flag-argument`; truncation is too far away for edit distance alone).
pub(crate) fn did_you_mean(unknown: &str) -> Option<&'static str> {
    SUGGESTION_NAMES
        .iter()
        .filter_map(|name| {
            let prefix = unknown.len() >= MIN_PREFIX_SUGGESTION_LEN && name.starts_with(unknown);
            let score = if prefix {
                1
            } else {
                levenshtein(unknown, name)
            };
            (score <= MAX_SUGGESTION_DISTANCE && name != unknown).then_some((score, name))
        })
        .min_by_key(|(score, name)| (*score, name.len()))
        .map(|(_, name)| name.as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InlineIgnoreMode {
    #[default]
    Apply,
    /// Raw findings: used by `--no-inline-ignores` and the unused-ignore rerun.
    Disabled,
}

/// Findings dropped inline in one run, of which from a blocking check.
#[derive(Debug, Default)]
pub struct SuppressionCounts {
    pub total: AtomicUsize,
    pub blocking: AtomicUsize,
}

/// (path, content hash, scan result)
type MemoEntry = (PathBuf, u64, Arc<Vec<Scanned>>);

/// Single-entry scan cache: checks for one file run back to back, so one entry gives about one
/// scan per file. Content-hash keyed, so an edited file never returns stale directives.
/// Per-context (never `static`) so parallel tests, the daemon, and LSP cannot interfere.
#[derive(Debug, Default)]
pub struct ScanMemo {
    entry: Mutex<Option<MemoEntry>>,
    pub scans: AtomicUsize,
    pub hash_calls: AtomicUsize,
    /// Hook-path raw reruns performed (Story 2.2.3); the test seam for "no rerun without a directive row".
    pub raw_reruns: AtomicUsize,
}

impl ScanMemo {
    pub(crate) fn scan(&self, path: &Path, source: &str) -> Arc<Vec<Scanned>> {
        self.hash_calls.fetch_add(1, Ordering::Relaxed);
        let mut hasher = DefaultHasher::new();
        source.hash(&mut hasher);
        let hash = hasher.finish();
        let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((p, h, scanned)) = entry.as_ref()
            && p == path
            && *h == hash
        {
            return Arc::clone(scanned);
        }
        self.scans.fetch_add(1, Ordering::Relaxed);
        let scanned = Arc::new(scan_directives(path, source));
        *entry = Some((path.to_path_buf(), hash, Arc::clone(&scanned)));
        scanned
    }
}

/// How one run applies inline ignores. Cloning shares the counter and memo.
#[derive(Debug, Clone, Default)]
pub struct InlineIgnoreContext {
    pub mode: InlineIgnoreMode,
    pub counter: Option<Arc<SuppressionCounts>>,
    pub scan_memo: Arc<ScanMemo>,
}

impl InlineIgnoreContext {
    pub fn disabled() -> Self {
        InlineIgnoreContext {
            mode: InlineIgnoreMode::Disabled,
            ..Default::default()
        }
    }

    /// Same mode, no counter and a fresh memo: a HEAD-baseline replay scans different
    /// content for the same path and must neither be counted nor evict the live entry.
    pub fn without_counter(&self) -> Self {
        InlineIgnoreContext {
            mode: self.mode,
            ..Default::default()
        }
    }
}

/// A finding a directive removed, kept so advisories and counters read one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedFinding {
    pub directive_start: Line,
    pub directive_end: Line,
    pub rule: RuleId,
    pub reason: Reason,
    pub finding_line: Line,
    pub severity: Severity,
}

/// Inline-ignore result carried on `CheckResult`. `shown` lists findings still visible after
/// scoping and `accepted/` (capped), `kept` every finding that survived inline filtering
/// (whole file, capped), `dropped` what directives removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineOutcome {
    #[serde(default)]
    pub shown: Vec<(RuleId, Line)>,
    #[serde(default)]
    pub kept: Vec<(RuleId, Line)>,
    #[serde(default)]
    pub dropped: Vec<DroppedFinding>,
}

impl InlineOutcome {
    pub fn first_anchor(&self) -> Option<(&RuleId, Line)> {
        self.shown.first().map(|(r, l)| (r, *l))
    }

    /// Distinct rule ids of the shown findings, in finding order.
    pub fn rule_ids(&self) -> Vec<&RuleId> {
        let mut ids: Vec<&RuleId> = Vec::new();
        for (rule, _) in &self.shown {
            if !ids.contains(&rule) {
                ids.push(rule);
            }
        }
        ids
    }
}

#[derive(Debug, Default)]
pub struct AppliedIgnores {
    pub kept: Vec<Finding>,
    pub dropped: Vec<DroppedFinding>,
}

/// First valid directive rule that matches `finding` and covers its row (or, for file-scope
/// rules, sits in the file head).
fn covering<'a>(
    directives: &'a [&'a Directive],
    checker_name: &str,
    finding: &Finding,
) -> Option<(&'a Directive, &'a RuleId)> {
    let line = Line::new(finding.line);
    directives.iter().find_map(|d| {
        d.rules().iter().find_map(|r| {
            rule_covers(d, r, checker_name, &finding.message, line).then_some((*d, r))
        })
    })
}

/// The one definition of "this directive rule silences this finding", shared by the apply
/// path and the unused-ignore judgement so the two cannot disagree.
fn rule_covers(
    d: &Directive,
    rule: &RuleId,
    checker_name: &str,
    message: &str,
    line: Line,
) -> bool {
    let matched = rule_matches(rule, checker_name, message);
    let in_head =
        d.start_line.get() <= FILE_HEAD_LINES && FILE_SCOPE_RULES.contains(&rule.as_str());
    covers(d, line, matched) || (matched && in_head)
}

/// A finding as the checker reported it, before any directive was applied. Built from
/// structured findings, never parsed from rendered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawFinding {
    pub line: Line,
    pub checker: String,
    pub rule: RuleId,
    pub message: String,
}

/// Checker-name prefixes or suffixes whose findings carry `rule`, for the rules whose checker
/// name differs from the rule id. `None` means ownership is unknown, so the rule is not judged.
fn owner_matches(rule: &str, checker: &str) -> Option<bool> {
    match rule {
        "commented-out-code" | "over-commented" | "verbose-comment" => {
            Some(checker.starts_with("comment-quality"))
        }
        "long-function"
        | "deep-nesting"
        | "long-parameter-list"
        | "flag-argument"
        | "unreachable-code"
        | "replace-magic-literal"
        | "extract-variable" => Some(checker.starts_with("syntax-rules")),
        "file-size" => Some(checker.ends_with("file-size")),
        _ if crate::checker::lookup(rule).is_some() => Some(checker == rule),
        _ => None,
    }
}

/// Whether any checker could own `rule` by the table; `false` means "judge from first-pass data".
pub(crate) fn has_owner(rule: &str) -> bool {
    owner_matches(rule, "").is_some()
}

/// Owner check against a check name, for choosing which checks to rerun.
pub(crate) fn owned_by(rule: &str, check_name: &str) -> bool {
    owner_matches(rule, check_name) == Some(true)
}

/// Row of the raw finding `rule` answers to that lies nearest `from` (earlier row on a tie).
pub(crate) fn nearest_finding_line(rule: &RuleId, raw: &[RawFinding], from: Line) -> Option<Line> {
    raw.iter()
        .filter(|f| rule_matches(rule, &f.checker, &f.message))
        .map(|f| f.line)
        .min_by_key(|l| (l.get().abs_diff(from.get()), l.get()))
}

pub(crate) fn rows_intersect(d: &Directive, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|&(s, e)| s <= d.end_line.get() && d.start_line.get() <= e)
}

/// Directives that suppress nothing, judged against `raw` (findings before any directive or
/// `accepted/` entry, so a shadowed ignore still counts as used). A rule is judged only when
/// its owning checker is in `ran_checkers`; unowned rules fail open, except a name that is not
/// a known rule or checker at all, which is reported as a probable typo. `only_rows` limits
/// judgement to directives touching those ranges (the hook's changed lines).
pub(crate) fn unused_ignores(
    directives: &[Directive],
    raw: &[RawFinding],
    ran_checkers: &[&str],
    only_rows: Option<&[(usize, usize)]>,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for d in directives {
        if only_rows.is_some_and(|ranges| !rows_intersect(d, ranges)) {
            continue;
        }
        for rule in d.rules() {
            let used = raw
                .iter()
                .any(|f| rule_covers(d, rule, &f.checker, &f.message, f.line));
            if used {
                continue;
            }
            if let Some(message) = unused_message(rule.as_str(), ran_checkers) {
                out.push(Finding {
                    line: d.start_line.get(),
                    message,
                });
            }
        }
    }
    out
}

/// The advisory for a rule that matched no raw finding, or `None` when it cannot be judged.
fn unused_message(rule: &str, ran_checkers: &[&str]) -> Option<String> {
    if known_rule(rule) {
        let judged = ran_checkers
            .iter()
            .any(|c| owner_matches(rule, c) == Some(true));
        return judged.then(|| {
            format!("[unused-ignore] kibitzer:ignore {rule} suppresses nothing - remove it")
        });
    }
    // `[ignore-syntax]` already carries the suggestion for a near miss.
    did_you_mean(rule).is_none().then(|| {
        format!(
            "[unused-ignore] '{rule}' is not a known rule or checker; run 'kibitzer check list' to see valid names"
        )
    })
}

/// Drops findings covered by a valid directive. The early returns run before any hashing,
/// locking, or parsing, so the common no-marker hook path stays a substring search.
pub(crate) fn apply_inline_ignores(
    findings: Vec<Finding>,
    file: &Path,
    source: &str,
    checker_name: &str,
    severity: Severity,
    ctx: &InlineIgnoreContext,
) -> AppliedIgnores {
    if findings.is_empty() || ctx.mode == InlineIgnoreMode::Disabled || !source.contains("kibitzer")
    {
        return AppliedIgnores {
            kept: findings,
            dropped: Vec::new(),
        };
    }
    let scanned = ctx.scan_memo.scan(file, source);
    let directives: Vec<&Directive> = scanned
        .iter()
        .filter_map(|(_, p)| match p {
            DirectiveParse::Valid(d) => Some(d),
            _ => None,
        })
        .collect();
    let mut applied = AppliedIgnores::default();
    for finding in findings {
        match covering(&directives, checker_name, &finding) {
            Some((d, rule)) => applied.dropped.push(DroppedFinding {
                directive_start: d.start_line,
                directive_end: d.end_line,
                rule: rule.clone(),
                reason: d.reason.clone(),
                finding_line: Line::new(finding.line),
                severity,
            }),
            None => applied.kept.push(finding),
        }
    }
    if let Some(counter) = &ctx.counter {
        let n = applied.dropped.len();
        counter.total.fetch_add(n, Ordering::Relaxed);
        if severity == Severity::Blocking {
            counter.blocking.fetch_add(n, Ordering::Relaxed);
        }
    }
    applied
}

// Test names follow the validation plan's should_X_When_Y convention.
#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn rules(names: &[&str]) -> Vec<RuleId> {
        names.iter().map(|n| RuleId::new(n).unwrap()).collect()
    }

    fn valid(text: &str) -> Directive {
        match parse_comment_line(text) {
            DirectiveParse::Valid(d) => d,
            other => panic!("expected Valid for {text:?}, got {other:?}"),
        }
    }

    fn malformed(text: &str) -> MalformedReason {
        match parse_comment_line(text) {
            DirectiveParse::Malformed(m) => m,
            other => panic!("expected Malformed for {text:?}, got {other:?}"),
        }
    }

    fn go_scan(source: &str) -> Vec<Scanned> {
        scan_directives(Path::new("x.go"), source)
    }

    #[test]
    fn parse_comment_line_should_ReturnDirective_When_KibitzerIgnore() {
        let source = "package main\n\n\n\n\n\n// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfunc f() {}\n";
        let scanned = go_scan(source);
        assert_eq!(scanned.len(), 1);
        let expected = Directive::new(
            rules(&["flag-argument"]),
            Reason::new("legacy API, callers pinned", &[]).unwrap(),
            Line::new(7),
            Line::new(7),
            true,
        )
        .unwrap();
        assert_eq!(scanned[0], (Line::new(7), DirectiveParse::Valid(expected)));
    }

    #[test]
    fn parse_comment_line_should_ReturnAllRules_When_CommaList() {
        let d = valid("// kibitzer:ignore a,b -- legacy API, callers pinned");
        assert_eq!(d.rules(), rules(&["a", "b"]).as_slice());
        assert_eq!(d.reason.as_str(), "legacy API, callers pinned");
    }

    #[test]
    fn parse_comment_line_should_ReturnValid_When_ReasonPresent() {
        let d = valid("//   kibitzer:ignore   a,b   --   legacy API, callers pinned   ");
        assert_eq!(d.reason.as_str(), "legacy API, callers pinned");
        assert_eq!(d.rules().len(), 2);
    }

    #[test]
    fn rule_id_new_should_Reject_When_PlaceholderOrUppercase() {
        assert!(RuleId::new("<rule>").is_none());
        assert!(RuleId::new("Foo_Bar").is_none());
        assert!(RuleId::new("").is_none());
        assert!(RuleId::new("flag-argument").is_some());
    }

    #[test]
    fn parse_comment_line_should_ReturnNotADirective_When_RustdocPlaceholder() {
        assert_eq!(
            parse_comment_line("/// kibitzer:ignore <rule> -- <why>"),
            DirectiveParse::NotADirective
        );
    }

    #[test]
    fn parse_comment_line_should_ReturnMalformedMissingReason_When_NoDashDash() {
        assert_eq!(
            malformed("// kibitzer:ignore flag-argument"),
            MalformedReason::MissingReason
        );
    }

    #[test]
    fn parse_comment_line_should_ReturnMalformedMissingReason_When_ReasonEmptyAfterSeparator() {
        assert_eq!(
            malformed("// kibitzer:ignore flag-argument -- "),
            MalformedReason::MissingReason
        );
        assert_eq!(
            malformed("// kibitzer:ignore flag-argument --"),
            MalformedReason::MissingReason
        );
    }

    #[test]
    fn parse_comment_line_should_ReturnMalformedMissingRule_When_OnlyReason() {
        assert_eq!(
            malformed("// kibitzer:ignore -- because"),
            MalformedReason::MissingRule
        );
        assert_eq!(
            malformed("// kibitzer:ignore"),
            MalformedReason::MissingRule
        );
    }

    #[test]
    fn reason_new_should_Reject_When_EmptyOrBlank() {
        assert_eq!(Reason::new("  ", &[]), Err(ReasonError::Blank));
        assert_eq!(Reason::new("", &[]), Err(ReasonError::Blank));
    }

    #[test]
    fn reason_new_should_ClassifyWeakReasons_When_TooShortOrRuleEcho() {
        let r = rules(&["flag-argument"]);
        assert_eq!(
            Reason::new("needed", &r),
            Err(ReasonError::Weak(WeakReason::TooShort))
        );
        for echo in ["flag-argument", "Flag Argument", "FLAG_ARGUMENT"] {
            assert_eq!(
                Reason::new(echo, &r),
                Err(ReasonError::Weak(WeakReason::RuleEcho)),
                "{echo}"
            );
        }
        assert!(Reason::new("legacy API, callers pinned", &r).is_ok());
    }

    #[test]
    fn parse_comment_line_should_ReturnWeakReason_When_NeededOrEcho() {
        assert_eq!(
            malformed("// kibitzer:ignore flag-argument -- needed"),
            MalformedReason::WeakReason(WeakReason::TooShort)
        );
        assert_eq!(
            malformed("// kibitzer:ignore flag-argument -- flag-argument"),
            MalformedReason::WeakReason(WeakReason::RuleEcho)
        );
        assert_eq!(
            malformed("// kibitzer:ignore flag-argument -- Flag Argument"),
            MalformedReason::WeakReason(WeakReason::RuleEcho)
        );
    }

    #[test]
    fn parse_comment_line_should_ReturnMalformed_When_EmDashSeparator() {
        for dash in ["\u{2014}", "\u{2013}"] {
            let line =
                format!("// kibitzer:ignore flag-argument {dash} legacy API, callers pinned");
            assert_eq!(malformed(&line), MalformedReason::EmDashSeparator, "{dash}");
        }
    }

    #[test]
    fn parse_comment_line_should_HandleRuleListEdges_When_CommaVariants() {
        let ok = valid("// kibitzer:ignore a,b -- legacy API, callers pinned");
        assert_eq!(ok.rules(), rules(&["a", "b"]).as_slice());
        let dedup = valid("// kibitzer:ignore a,a -- legacy API, callers pinned");
        assert_eq!(dedup.rules(), rules(&["a"]).as_slice());
        for bad in [
            "a, b -- why not",
            "a,,b -- why not",
            "a, -- why not",
            ",a -- why not",
        ] {
            assert_eq!(
                malformed(&format!("// kibitzer:ignore {bad}")),
                MalformedReason::BadRuleList,
                "{bad}"
            );
        }
        assert_eq!(
            parse_comment_line("// kibitzer:ignore a,<rule> -- why not"),
            DirectiveParse::NotADirective
        );
    }

    #[test]
    fn parse_comment_line_should_ReturnNearMiss_When_MarkerMisspelled() {
        for text in [
            "// kibitzer: ignore foo -- bar",
            "// kibitzer:allow foo -- bar",
            "# kibitzer:false-positive primitive-obsession -- id is opaque",
            "// kibitzer:false_positive foo -- bar",
        ] {
            assert_eq!(malformed(text), MalformedReason::NearMissMarker, "{text}");
        }
    }

    #[test]
    fn parse_comment_line_should_ReturnNotADirective_When_ProseMentionsKibitzer() {
        for text in [
            "// see kibitzer: allow list in docs",
            "// the kibitzer:ignore syntax is documented",
        ] {
            assert_eq!(
                parse_comment_line(text),
                DirectiveParse::NotADirective,
                "{text}"
            );
        }
    }

    #[test]
    fn parse_comment_line_should_ReturnNotAtCommentStart_When_DirectiveFollowsOtherText() {
        for text in [
            "// TODO kibitzer:ignore flag-argument -- legacy API, callers pinned",
            "// legacy: kibitzer:ignore flag-argument -- legacy API, callers pinned",
        ] {
            assert_eq!(
                malformed(text),
                MalformedReason::NotAtCommentStart,
                "{text}"
            );
        }
    }

    #[test]
    fn parse_comment_line_should_StayNotADirective_When_LaterMentionLacksFullGrammar() {
        for text in [
            "// the kibitzer:ignore syntax is documented",
            "// see kibitzer:ignore <rule> -- <why>",
        ] {
            assert_eq!(
                parse_comment_line(text),
                DirectiveParse::NotADirective,
                "{text}"
            );
        }
    }

    #[test]
    fn parse_comment_line_should_StripLeader_When_SlashDoc_Hash_BlockStar_HtmlOpen() {
        for text in [
            "// kibitzer:ignore a -- two words",
            "/// kibitzer:ignore a -- two words",
            "//! kibitzer:ignore a -- two words",
            "# kibitzer:ignore a -- two words",
            "/* kibitzer:ignore a -- two words */",
            "/** kibitzer:ignore a -- two words */",
            " * kibitzer:ignore a -- two words",
            "<!-- kibitzer:ignore a -- two words -->",
        ] {
            let d = valid(text);
            assert_eq!(d.reason.as_str(), "two words", "{text}");
        }
    }

    #[test]
    fn parse_comment_line_should_Tolerate_When_CrlfAndTabs() {
        let d = valid("//\tkibitzer:ignore\ta,b\t--\ttwo words\r");
        assert_eq!(d.rules().len(), 2);
        assert_eq!(d.reason.as_str(), "two words");
    }

    #[test]
    fn is_directive_comment_should_MatchAnyKibitzerLine_When_BlockOrLine() {
        assert!(is_directive_comment("// kibitzer:ignore a -- two words"));
        assert!(is_directive_comment(
            "/*\n * kibitzer:ignore a -- two words\n */"
        ));
        assert!(!is_directive_comment("// ordinary comment"));
    }

    fn fixture(lang: Language) -> (&'static str, &'static str) {
        let line = "kibitzer:ignore flag-argument -- legacy API, callers pinned";
        match lang {
            Language::Go => (
                "package main\n// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfunc f() {}\n",
                line,
            ),
            Language::TypeScript | Language::Tsx | Language::JavaScript => (
                "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfunction f() {}\n",
                line,
            ),
            Language::Python => (
                "# kibitzer:ignore flag-argument -- legacy API, callers pinned\ndef f():\n    pass\n",
                line,
            ),
            Language::Java => (
                "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nclass A {}\n",
                line,
            ),
            Language::Kotlin => (
                "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfun f() {}\n",
                line,
            ),
            Language::Rust => (
                "// kibitzer:ignore flag-argument -- legacy API, callers pinned\nfn f() {}\n",
                line,
            ),
        }
    }

    #[test]
    fn scan_code_comments_should_ParseDirective_When_EachLanguageAllGrammar() {
        let cache = GrammarCache::new();
        for &lang in Language::ALL {
            let (source, _) = fixture(lang);
            let tree = cache.parse(lang, source).unwrap();
            let scanned = scan_code_comments(lang, &tree, source);
            assert_eq!(scanned.len(), 1, "{lang:?}: {scanned:?}");
            let DirectiveParse::Valid(d) = &scanned[0].1 else {
                panic!("{lang:?}: not valid: {scanned:?}");
            };
            assert_eq!(d.rules(), rules(&["flag-argument"]).as_slice(), "{lang:?}");
            assert!(d.whole_line, "{lang:?}");
        }
    }

    #[test]
    fn scan_code_comments_should_ReturnNothing_When_MarkerInStringLiteral() {
        let source = "fn main() {\n    let s = \"// kibitzer:ignore x -- y\";\n}\n";
        assert!(scan_directives(Path::new("a.rs"), source).is_empty());
    }

    #[test]
    fn scan_code_comments_should_RecordWholeLineFalse_When_CommentTrailsCode() {
        let source = "package main\nvar x = f() // kibitzer:ignore a -- two words\n";
        let scanned = go_scan(source);
        let DirectiveParse::Valid(d) = &scanned[0].1 else {
            panic!("{scanned:?}")
        };
        assert!(!d.whole_line);
        assert_eq!(d.start_line, Line::new(2));
    }

    #[test]
    fn scan_code_comments_should_ReportBlockCommentLineRow_When_DirectiveOnInnerLine() {
        let source = format!(
            "{}/*\n * kibitzer:ignore long-method -- generated code\n */\nclass A {{}}\n",
            "\n".repeat(9)
        );
        let scanned = scan_directives(Path::new("A.java"), &source);
        assert_eq!(scanned.len(), 1, "{scanned:?}");
        let DirectiveParse::Valid(d) = &scanned[0].1 else {
            panic!("{scanned:?}")
        };
        assert_eq!(d.start_line, Line::new(11));
        assert_eq!(scanned[0].0, Line::new(11));
    }

    #[test]
    fn scan_code_comments_should_ConvertRowsToOneBased_When_TreeSitterRowSix() {
        let source = format!(
            "{}// kibitzer:ignore a -- two words\nfn f() {{}}\n",
            "\n".repeat(6)
        );
        let scanned = scan_directives(Path::new("a.rs"), &source);
        let DirectiveParse::Valid(d) = &scanned[0].1 else {
            panic!("{scanned:?}")
        };
        assert_eq!(d.start_line, Line::new(7));
    }

    #[test]
    fn scan_markdown_should_ParseDirective_When_HtmlCommentOutsideFence() {
        let source =
            "# T\n\ntext\n\n<!-- kibitzer:ignore em-dash-overuse -- quoted source -->\n\nmore\n";
        let scanned = scan_directives(Path::new("a.md"), source);
        assert_eq!(scanned.len(), 1, "{scanned:?}");
        let DirectiveParse::Valid(d) = &scanned[0].1 else {
            panic!("{scanned:?}")
        };
        assert_eq!(d.start_line, Line::new(5));
        assert_eq!(d.rules(), rules(&["em-dash-overuse"]).as_slice());
        assert!(d.whole_line);
    }

    #[test]
    fn scan_markdown_should_ParseDirective_When_InlineCommentInTableRow() {
        let source = "| a | b |\n|---|---|\n| x | <!-- kibitzer:ignore a -- two words --> |\n";
        let scanned = scan_markdown(source);
        assert_eq!(scanned.len(), 1, "{scanned:?}");
        let DirectiveParse::Valid(d) = &scanned[0].1 else {
            panic!("{scanned:?}")
        };
        assert_eq!(d.start_line, Line::new(3));
        assert!(!d.whole_line);
    }

    #[test]
    fn scan_directives_should_NotListMarker_When_InRustStringAndMarkdownFence() {
        let rust = "const S: &str = \"// kibitzer:ignore a -- two words\";\n";
        assert!(scan_directives(Path::new("a.rs"), rust).is_empty());
        let fenced = "```md\n<!-- kibitzer:ignore a -- two words -->\n```\n";
        assert!(scan_directives(Path::new("a.md"), fenced).is_empty());
        let indented = "text\n\n    <!-- kibitzer:ignore a -- two words -->\n";
        assert!(scan_directives(Path::new("a.md"), indented).is_empty());
    }

    #[test]
    fn scan_leading_comments_should_ParseDirective_When_ShellHashComment() {
        let source = "#!/bin/sh\n# kibitzer:ignore file-size -- vendored script\necho hi\n";
        let scanned = scan_directives(Path::new("x.sh"), source);
        assert_eq!(scanned.len(), 1, "{scanned:?}");
        let DirectiveParse::Valid(d) = &scanned[0].1 else {
            panic!("{scanned:?}")
        };
        assert_eq!(d.start_line, Line::new(2));
        assert!(d.whole_line);
    }

    #[test]
    fn scan_leading_comments_should_ReturnNothing_When_MarkerInsideEchoString() {
        let source = "echo \"# kibitzer:ignore a -- b\"\n";
        assert!(scan_directives(Path::new("x.sh"), source).is_empty());
    }

    #[test]
    fn scan_directives_should_ConstructNoParser_When_SourceLacksKibitzer() {
        let source = "package main\n\nfunc f() {}\n".repeat(3_333);
        let cache = GrammarCache::new();
        let scanned = scan_directives_with_cache(&cache, Path::new("x.go"), &source);
        assert!(scanned.is_empty());
        assert!(format!("{cache:?}").contains("languages_cached: 0"));
    }

    fn dir(rule_names: &[&str], start: usize, end: usize, whole_line: bool) -> Directive {
        Directive::new(
            rules(rule_names),
            Reason::new("legacy api pinned", &[]).unwrap(),
            Line::new(start),
            Line::new(end),
            whole_line,
        )
        .unwrap()
    }

    #[test]
    fn covers_should_BeTrueOnlyAtNextRow_When_WholeLineDirective() {
        let d = dir(&["flag-argument"], 9, 9, true);
        assert!(covers(&d, Line::new(10), true));
        assert!(covers(&d, Line::new(9), true));
        assert!(!covers(&d, Line::new(11), true));
        assert!(!covers(&d, Line::new(10), false));
    }

    #[test]
    fn covers_should_StayOnOwnRow_When_TrailingComment() {
        let d = dir(&["flag-argument"], 9, 9, false);
        assert!(covers(&d, Line::new(9), true));
        assert!(!covers(&d, Line::new(10), true));
    }

    #[test]
    fn covers_should_SpanAllRows_When_BlockComment() {
        let d = dir(&["flag-argument"], 4, 6, true);
        assert!(covers(&d, Line::new(5), true));
        assert!(covers(&d, Line::new(7), true));
        assert!(!covers(&d, Line::new(8), true));
    }

    #[test]
    fn covers_should_TreatLineZeroAsLineOne() {
        let d = dir(&["file-size"], 1, 1, true);
        assert!(covers(&d, Line::new(0), true));
    }

    #[test]
    fn rule_matches_should_MatchCheckerNameOrBracketPrefix() {
        let r = |s: &str| RuleId::new(s).unwrap();
        assert!(rule_matches(
            &r("flag-argument"),
            "syntax-rules-go",
            "[flag-argument] x"
        ));
        assert!(rule_matches(
            &r("primitive-obsession"),
            "primitive-obsession",
            "plain text"
        ));
        assert!(!rule_matches(
            &r("other"),
            "syntax-rules-go",
            "[flag-argument] x"
        ));
        assert!(rule_matches(
            &r("some-new-rule"),
            "unknown-checker",
            "[some-new-rule] m"
        ));
    }

    #[test]
    fn rule_matches_should_BeFalse_When_DynamicPrefixCheckerAndBracketMatch() {
        let r = |s: &str| RuleId::new(s).unwrap();
        let msg = "[foo] used but never defined";
        assert!(!rule_matches(&r("foo"), "markdown-link-integrity", msg));
        assert!(rule_matches(
            &r("markdown-link-integrity"),
            "markdown-link-integrity",
            msg
        ));
    }

    #[test]
    fn rule_matches_should_BeFalse_When_MetaRule() {
        let r = |s: &str| RuleId::new(s).unwrap();
        assert!(!rule_matches(
            &r("ignore-syntax"),
            "inline-ignore",
            "[ignore-syntax] bad"
        ));
        assert!(!rule_matches(&r("unused-ignore"), "unused-ignore", "x"));
        assert!(!rule_matches(
            &r("blocking-suppressed"),
            "inline-ignore",
            "[blocking-suppressed] x"
        ));
    }

    fn finding(line: usize, message: &str) -> Finding {
        Finding {
            line,
            message: message.to_string(),
        }
    }

    const GO_IGNORE_ABOVE_10: &str = "package main\n\n\n\n\n\n\n\n// kibitzer:ignore flag-argument -- legacy api pinned\nfunc f(b bool) {}\n";

    fn apply(
        findings: Vec<Finding>,
        source: &str,
        checker: &str,
        severity: Severity,
        ctx: &InlineIgnoreContext,
    ) -> AppliedIgnores {
        apply_inline_ignores(findings, Path::new("x.go"), source, checker, severity, ctx)
    }

    #[test]
    fn apply_inline_ignores_should_ReturnInputUnchanged_When_FindingsEmptyOrNoSubstring() {
        let ctx = InlineIgnoreContext::default();
        let out = apply(vec![], GO_IGNORE_ABOVE_10, "c", Severity::Advisory, &ctx);
        assert!(out.kept.is_empty() && out.dropped.is_empty());
        let out = apply(
            vec![finding(10, "[flag-argument] x")],
            "package main\nfunc f() {}\n",
            "c",
            Severity::Advisory,
            &ctx,
        );
        assert_eq!(out.kept.len(), 1);
        assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 0);
        assert_eq!(ctx.scan_memo.hash_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn apply_inline_ignores_should_ReturnRaw_When_ModeDisabled() {
        let ctx = InlineIgnoreContext::disabled();
        let out = apply(
            vec![finding(10, "[flag-argument] x")],
            GO_IGNORE_ABOVE_10,
            "c",
            Severity::Advisory,
            &ctx,
        );
        assert_eq!(out.kept.len(), 1);
        assert_eq!(ctx.scan_memo.hash_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn apply_inline_ignores_should_DropAndReport_When_DirectiveCovers() {
        let ctx = InlineIgnoreContext::default();
        let out = apply(
            vec![
                finding(10, "[flag-argument] x"),
                finding(30, "[flag-argument] y"),
            ],
            GO_IGNORE_ABOVE_10,
            "syntax-rules-go",
            Severity::Blocking,
            &ctx,
        );
        assert_eq!(out.kept, vec![finding(30, "[flag-argument] y")]);
        assert_eq!(out.dropped.len(), 1);
        let d = &out.dropped[0];
        assert_eq!(
            (d.directive_start, d.directive_end),
            (Line::new(9), Line::new(9))
        );
        assert_eq!(d.rule.as_str(), "flag-argument");
        assert_eq!(d.reason.as_str(), "legacy api pinned");
        assert_eq!(d.finding_line, Line::new(10));
        assert_eq!(d.severity, Severity::Blocking);
    }

    #[test]
    fn apply_inline_ignores_should_MatchCheckerName_When_NoBracketPrefix() {
        let source =
            "// kibitzer:ignore primitive-obsession -- ids are plain strings\nfn f(a: String) {}\n";
        let out = apply(
            vec![finding(2, "param a is a primitive")],
            source,
            "primitive-obsession",
            Severity::Advisory,
            &InlineIgnoreContext::default(),
        );
        assert!(out.kept.is_empty());
        assert_eq!(out.dropped.len(), 1);
    }

    #[test]
    fn apply_inline_ignores_should_KeepFinding_When_DirectiveTwoLinesAbove() {
        let source = "// kibitzer:ignore flag-argument -- legacy api pinned\n\nfunc f() {}\n";
        let out = apply(
            vec![finding(3, "[flag-argument] x")],
            source,
            "c",
            Severity::Advisory,
            &InlineIgnoreContext::default(),
        );
        assert_eq!(out.kept.len(), 1);
    }

    #[test]
    fn apply_inline_ignores_should_DropOnlyOwnRow_When_TrailingComment() {
        let source =
            "package main\nx := f() // kibitzer:ignore flag-argument -- legacy api\ny := g()\n";
        let out = apply(
            vec![
                finding(2, "[flag-argument] a"),
                finding(3, "[flag-argument] b"),
            ],
            source,
            "c",
            Severity::Advisory,
            &InlineIgnoreContext::default(),
        );
        assert_eq!(out.kept, vec![finding(3, "[flag-argument] b")]);
    }

    #[test]
    fn apply_inline_ignores_should_NeverDrop_When_MetaRuleNamed() {
        let source = "// kibitzer:ignore ignore-syntax -- hide the repair\nbad\n";
        let out = apply(
            vec![finding(2, "[ignore-syntax] bad")],
            source,
            "inline-ignore",
            Severity::Advisory,
            &InlineIgnoreContext::default(),
        );
        assert_eq!(out.kept.len(), 1);
    }

    #[test]
    fn apply_inline_ignores_should_DropAll_When_FileScopeRuleInHead() {
        let mut source = String::from("// kibitzer:ignore file-complexity -- generated tables\n");
        source.push_str(&"x\n".repeat(50));
        let out = apply(
            vec![
                finding(20, "[file-complexity] a"),
                finding(40, "[file-complexity] a"),
            ],
            &source,
            "file-complexity",
            Severity::Advisory,
            &InlineIgnoreContext::default(),
        );
        assert!(out.kept.is_empty());
        let out = apply(
            vec![finding(900, "[file-size] big")],
            "// kibitzer:ignore file-size -- generated tables\n",
            "file-size",
            Severity::Advisory,
            &InlineIgnoreContext::default(),
        );
        assert!(out.kept.is_empty());
    }

    #[test]
    fn apply_inline_ignores_should_NotHeadScope_When_RuleNotFileScope() {
        let out = apply(
            vec![finding(40, "[flag-argument] a")],
            "// kibitzer:ignore flag-argument -- legacy api\n",
            "c",
            Severity::Advisory,
            &InlineIgnoreContext::default(),
        );
        assert_eq!(out.kept.len(), 1);
    }

    #[test]
    fn apply_inline_ignores_should_CountDropped_When_CounterPresent() {
        let counter = Arc::new(SuppressionCounts::default());
        let ctx = InlineIgnoreContext {
            counter: Some(Arc::clone(&counter)),
            ..Default::default()
        };
        apply(
            vec![
                finding(10, "[flag-argument] x"),
                finding(30, "[flag-argument] y"),
            ],
            GO_IGNORE_ABOVE_10,
            "c",
            Severity::Blocking,
            &ctx,
        );
        apply(
            vec![finding(10, "[flag-argument] x")],
            GO_IGNORE_ABOVE_10,
            "c",
            Severity::Advisory,
            &ctx,
        );
        assert_eq!(counter.total.load(Ordering::Relaxed), 2);
        assert_eq!(counter.blocking.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn inline_ignore_context_should_KeepSeparateCounts_When_TwoRunsConcurrent() {
        let run = |n: usize| {
            let counter = Arc::new(SuppressionCounts::default());
            let ctx = InlineIgnoreContext {
                counter: Some(Arc::clone(&counter)),
                ..Default::default()
            };
            std::thread::spawn(move || {
                let source = format!(
                    "{}\nx\n",
                    (0..n)
                        .map(|_| "// kibitzer:ignore flag-argument -- legacy api\nf()")
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                let findings = (0..n)
                    .map(|i| finding(2 + i * 2, "[flag-argument] a"))
                    .collect();
                apply(findings, &source, "c", Severity::Advisory, &ctx);
                counter.total.load(Ordering::Relaxed)
            })
        };
        let (a, b) = (run(2), run(3));
        assert_eq!((a.join().unwrap(), b.join().unwrap()), (2, 3));
    }

    #[test]
    fn without_counter_should_NotCount_When_Replay() {
        let counter = Arc::new(SuppressionCounts::default());
        let ctx = InlineIgnoreContext {
            counter: Some(Arc::clone(&counter)),
            ..Default::default()
        };
        apply(
            vec![finding(10, "[flag-argument] x")],
            GO_IGNORE_ABOVE_10,
            "c",
            Severity::Blocking,
            &ctx.without_counter(),
        );
        assert_eq!(counter.total.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn scan_memo_should_ScanOncePerContentAndPath() {
        let ctx = InlineIgnoreContext::default();
        let f = || vec![finding(10, "[flag-argument] x")];
        for _ in 0..30 {
            apply(f(), GO_IGNORE_ABOVE_10, "c", Severity::Advisory, &ctx);
        }
        assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 1);
        let edited = GO_IGNORE_ABOVE_10.replace("kibitzer:ignore", "removed");
        let edited = format!("{edited}// kibitzer is mentioned\n");
        let out = apply(f(), &edited, "c", Severity::Advisory, &ctx);
        assert_eq!(out.kept.len(), 1);
        assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 2);
        apply_inline_ignores(
            f(),
            Path::new("other.go"),
            GO_IGNORE_ABOVE_10,
            "c",
            Severity::Advisory,
            &ctx,
        );
        assert_eq!(ctx.scan_memo.scans.load(Ordering::Relaxed), 3);
        let held = ctx.scan_memo.entry.lock().unwrap();
        assert_eq!(held.as_ref().unwrap().0, PathBuf::from("other.go"));
    }

    #[test]
    fn inline_outcome_should_ExposeFirstAnchorAndDistinctRules() {
        let r = |s: &str| RuleId::new(s).unwrap();
        let outcome = InlineOutcome {
            shown: vec![
                (r("a"), Line::new(5)),
                (r("b"), Line::new(7)),
                (r("a"), Line::new(9)),
            ],
            ..Default::default()
        };
        assert_eq!(outcome.first_anchor(), Some((&r("a"), Line::new(5))));
        assert_eq!(outcome.rule_ids(), vec![&r("a"), &r("b")]);
        assert_eq!(InlineOutcome::default().first_anchor(), None);
    }

    #[test]
    fn inline_outcome_should_RoundTripJson_When_Populated() {
        let outcome = InlineOutcome {
            shown: vec![(RuleId::new("a").unwrap(), Line::new(5))],
            kept: vec![],
            dropped: vec![DroppedFinding {
                directive_start: Line::new(4),
                directive_end: Line::new(4),
                rule: RuleId::new("a").unwrap(),
                reason: Reason::new("legacy api pinned", &[]).unwrap(),
                finding_line: Line::new(5),
                severity: Severity::Blocking,
            }],
        };
        let json = serde_json::to_string(&outcome).unwrap();
        assert_eq!(
            serde_json::from_str::<InlineOutcome>(&json).unwrap(),
            outcome
        );
    }

    #[test]
    fn anchor_rule_should_UseChecker_When_NoPrefixOrDynamic() {
        assert_eq!(anchor_rule("c", &finding(1, "[a-b] m")).as_str(), "a-b");
        assert_eq!(
            anchor_rule("primitive-obsession", &finding(1, "plain")).as_str(),
            "primitive-obsession"
        );
        assert_eq!(
            anchor_rule(
                "markdown-link-integrity",
                &finding(1, "[foo] used but never defined")
            )
            .as_str(),
            "markdown-link-integrity"
        );
    }

    #[test]
    fn did_you_mean_should_SuggestNearestKnown_When_TypoOrTruncation() {
        assert_eq!(did_you_mean("flag-arg"), Some("flag-argument"));
        assert_eq!(did_you_mean("flag-arguments"), Some("flag-argument"));
        assert_eq!(did_you_mean("deep-nestng"), Some("deep-nesting"));
        assert_eq!(
            did_you_mean("primitive-obsesion"),
            Some("primitive-obsession")
        );
    }

    #[test]
    fn did_you_mean_should_ReturnNone_When_NothingNear() {
        assert_eq!(did_you_mean("made-up-rule"), None);
        assert_eq!(did_you_mean("zzz"), None);
        assert_eq!(did_you_mean("flag-argument"), None);
    }

    #[test]
    fn known_rule_should_AcceptRulesCheckersAndMeta_When_Queried() {
        assert!(known_rule("flag-argument"));
        assert!(known_rule("primitive-obsession"));
        assert!(known_rule("syntax-rules-rust"));
        assert!(known_rule("ignore-syntax"));
        assert!(!known_rule("made-up-rule"));
    }

    #[test]
    fn levenshtein_should_CountEdits() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
    }

    /// Message-prefix ids (`"[id]`) in checker sources outside `#[cfg(test)]`.
    fn static_rule_prefixes() -> Vec<(String, String)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("checkers"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        for top_level in [
            "single_call_site_delegation.rs",
            "god_class.rs",
            "isp_fat_interface.rs",
            "unreferenced_symbols.rs",
        ] {
            files.push(root.join(top_level));
        }
        let prefix = Regex::new(r#""\[([a-z][a-z0-9-]*)\]"#).unwrap();
        let mut found = Vec::new();
        for file in files {
            let name = file.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap();
            for cap in prefix.captures_iter(production) {
                found.push((name.clone(), cap[1].to_string()));
            }
        }
        found
    }

    #[test]
    fn known_rules_should_CoverEveryStaticRulePrefix_When_DriftGuardScansCheckerSources() {
        let found = static_rule_prefixes();
        assert!(found.len() >= 15, "scan found too little: {}", found.len());
        let missing: Vec<_> = found.iter().filter(|(_, id)| !known_rule(id)).collect();
        assert!(
            missing.is_empty(),
            "add these ids to KNOWN_RULES: {missing:?}"
        );
    }

    fn directive_at(rule_list: &[&str], start: usize, end: usize, whole_line: bool) -> Directive {
        Directive::new(
            rules(rule_list),
            Reason::new("legacy api pinned", &[]).unwrap(),
            Line::new(start),
            Line::new(end),
            whole_line,
        )
        .unwrap()
    }

    fn raw(checker: &str, rule: &str, line: usize, message: &str) -> RawFinding {
        RawFinding {
            line: Line::new(line),
            checker: checker.to_string(),
            rule: RuleId::new(rule).unwrap(),
            message: message.to_string(),
        }
    }

    const SYNTAX: &[&str] = &["syntax-rules-go"];

    #[test]
    fn unused_ignores_should_Report_When_NoMatchingFinding() {
        let d = directive_at(&["flag-argument"], 9, 9, true);
        let out = unused_ignores(&[d], &[], SYNTAX, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 9);
        assert_eq!(
            out[0].message,
            "[unused-ignore] kibitzer:ignore flag-argument suppresses nothing - remove it"
        );
    }

    #[test]
    fn unused_ignores_should_BeEmpty_When_DirectiveCoversRawFinding() {
        let d = directive_at(&["flag-argument"], 9, 9, true);
        let f = raw("syntax-rules-go", "flag-argument", 10, "[flag-argument] x");
        assert!(unused_ignores(&[d], &[f], SYNTAX, None).is_empty());
    }

    #[test]
    fn unused_ignores_should_Report_When_FindingTwoRowsBelowWholeLineDirective() {
        let d = directive_at(&["flag-argument"], 9, 9, true);
        let f = raw("syntax-rules-go", "flag-argument", 11, "[flag-argument] x");
        assert_eq!(unused_ignores(&[d], &[f], SYNTAX, None).len(), 1);
    }

    #[test]
    fn unused_ignores_should_BeEmpty_When_AlsoShadowedByAccepted() {
        // Raw findings are taken before `accepted/` is applied, so a shadowed finding is still raw.
        let d = directive_at(&["flag-argument"], 4, 4, true);
        let shadowed = raw("syntax-rules-go", "flag-argument", 5, "[flag-argument] x");
        assert!(unused_ignores(&[d], &[shadowed], SYNTAX, None).is_empty());
    }

    #[test]
    fn unused_ignores_should_SkipRule_When_OwningCheckerDisabledOrDidNotRun() {
        let d = directive_at(&["flag-argument"], 9, 9, true);
        assert!(
            unused_ignores(std::slice::from_ref(&d), &[], &["em-dash-overuse"], None).is_empty()
        );
        assert!(unused_ignores(&[d], &[], &[], None).is_empty());
    }

    #[test]
    fn unused_ignores_should_FailOpen_When_RuleOwnershipUnknown() {
        // `god-class` is a known rule whose emitting checker is outside the registry.
        let d = directive_at(&["god-class"], 9, 9, true);
        assert!(unused_ignores(&[d], &[], SYNTAX, None).is_empty());
    }

    #[test]
    fn unused_ignores_should_JudgeByCheckerName_When_MarkdownLinkIntegrity() {
        let d = directive_at(&["markdown-link-integrity"], 3, 3, true);
        let f = raw(
            "markdown-link-integrity",
            "markdown-link-integrity",
            4,
            "[foo] used but never defined",
        );
        let ran = ["markdown-link-integrity"];
        assert!(unused_ignores(std::slice::from_ref(&d), &[f], &ran, None).is_empty());
        assert_eq!(unused_ignores(&[d], &[], &ran, None).len(), 1);
    }

    #[test]
    fn unused_ignores_should_PointToCheckList_When_RuleIsNeitherKnownNorCheckerName() {
        let d = directive_at(&["made-up-rule"], 9, 9, true);
        let out = unused_ignores(&[d], &[], SYNTAX, None);
        assert_eq!(
            out[0].message,
            "[unused-ignore] 'made-up-rule' is not a known rule or checker; run 'kibitzer check list' to see valid names"
        );
        assert!(!out[0].message.contains("remove it"));
    }

    #[test]
    fn unused_ignores_should_StaySilent_When_UnknownRuleHasNearMatchOrWasUsed() {
        // A near miss is already reported by `[ignore-syntax]`; a built id that matched is used.
        let near = directive_at(&["flag-argumnt"], 9, 9, true);
        assert!(unused_ignores(&[near], &[], SYNTAX, None).is_empty());
        let plugin = directive_at(&["plugin-only-rule"], 9, 9, true);
        let f = raw(
            "plugin-check",
            "plugin-only-rule",
            10,
            "[plugin-only-rule] x",
        );
        assert!(unused_ignores(&[plugin], &[f], SYNTAX, None).is_empty());
    }

    #[test]
    fn unused_ignores_should_ReportEachUnusedRule_When_CommaList() {
        let d = directive_at(&["flag-argument", "long-function"], 9, 9, true);
        let f = raw("syntax-rules-go", "long-function", 10, "[long-function] x");
        let out = unused_ignores(&[d], &[f], SYNTAX, None);
        assert_eq!(out.len(), 1);
        assert!(
            out[0].message.contains("flag-argument"),
            "{}",
            out[0].message
        );
    }

    #[test]
    fn unused_ignores_should_OnlyJudgeTouchedDirectives_When_OnlyRowsGiven() {
        let near = directive_at(&["flag-argument"], 12, 12, true);
        let far = directive_at(&["flag-argument"], 50, 50, true);
        let out = unused_ignores(&[near, far], &[], SYNTAX, Some(&[(12, 12)]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 12);
        let none = directive_at(&["flag-argument"], 50, 50, true);
        assert!(unused_ignores(&[none], &[], SYNTAX, Some(&[(1, 3)])).is_empty());
    }

    #[test]
    fn unused_ignores_should_HonorFileHead_When_FileSizeRule() {
        let d = directive_at(&["file-size"], 1, 1, true);
        let f = raw("go-file-size", "file-size", 900, "[file-size] big");
        assert!(unused_ignores(&[d], &[f], &["go-file-size"], None).is_empty());
    }

    fn rid(name: &str) -> RuleId {
        RuleId::new(name).unwrap()
    }

    #[test]
    fn syntax_hint_should_UseFileLeader_When_NoAnchor() {
        let go = syntax_hint(Path::new("src/foo.go"), None, &[]);
        assert!(go.contains("// kibitzer:ignore <rule> -- <why>"), "{go}");
        let py = syntax_hint(Path::new("x.py"), None, &[]);
        assert!(py.contains("# kibitzer:ignore <rule> -- <why>"), "{py}");
        let md = syntax_hint(Path::new("notes.md"), None, &[]);
        assert!(
            md.contains("<!-- kibitzer:ignore <rule> -- <why> -->"),
            "{md}"
        );
        let other = syntax_hint(Path::new("Makefile"), None, &[]);
        assert!(other.contains("# kibitzer:ignore <rule>"), "{other}");
        assert!(!go.contains("Rules:"), "{go}");
    }

    #[test]
    fn syntax_hint_should_NameExactRow_When_AnchorGiven() {
        let rule = rid("flag-argument");
        let hint = syntax_hint(
            Path::new("src/foo.go"),
            Some((&rule, Line::new(20))),
            &[&rule],
        );
        assert!(hint.contains("above line 20"), "{hint}");
        assert!(
            hint.contains("// kibitzer:ignore flag-argument -- <why>"),
            "{hint}"
        );
        assert!(!hint.contains("false-positive"), "{hint}");
    }

    #[test]
    fn syntax_hint_should_SayFileHead_When_FileScopeRule() {
        let rule = rid("file-size");
        let hint = syntax_hint(Path::new("a.go"), Some((&rule, Line::new(900))), &[&rule]);
        assert!(
            hint.contains("in the first 10 lines or on the anchor line"),
            "{hint}"
        );
    }

    #[test]
    fn syntax_hint_should_ListRulesIncludingPrefixless_When_SecondFindingHasNoPrefix() {
        let a = rid("flag-argument");
        let b = rid("primitive-obsession");
        let hint = syntax_hint(Path::new("a.go"), Some((&a, Line::new(3))), &[&a, &b]);
        assert!(
            hint.contains("Rules: flag-argument, primitive-obsession"),
            "{hint}"
        );
        let first_prefixless = syntax_hint(Path::new("a.go"), Some((&b, Line::new(3))), &[&b, &a]);
        assert!(
            first_prefixless.contains("kibitzer:ignore primitive-obsession -- <why>"),
            "{first_prefixless}"
        );
    }

    #[test]
    fn syntax_hint_should_TruncateRules_When_MoreThanSix() {
        let ids: Vec<RuleId> = (0..8).map(|i| rid(&format!("rule-{i}"))).collect();
        let refs: Vec<&RuleId> = ids.iter().collect();
        let hint = syntax_hint(Path::new("a.go"), Some((&ids[0], Line::new(1))), &refs);
        assert!(hint.contains("rule-5, ..."), "{hint}");
        assert!(!hint.contains("rule-6"), "{hint}");
    }

    #[test]
    fn syntax_hint_limited_should_KeepOneRule_When_LimitIsOne() {
        let a = rid("flag-argument");
        let b = rid("file-size");
        let hint = syntax_hint_limited(Path::new("a.go"), Some((&a, Line::new(3))), &[&a, &b], 1);
        assert!(hint.contains("Rules: flag-argument, ..."), "{hint}");
    }
}
