//! Inline `kibitzer:ignore <rule>[,<rule>...] -- <reason>` directives: parsing only.
//! Matching and application at the check seam live in later tasks of the
//! inline-ignore-syntax plan.
//!
//! Items here are not yet called from non-test code; later tasks wire them in.
#![allow(dead_code)]

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use tree_sitter::Tree;

use crate::checker::{GrammarCache, Language};
use crate::markdown_text::{line_for_offset, line_start_offsets};
use crate::tree_walk::{comment_kinds, walk_preorder};

const MARKER: &str = "kibitzer:ignore";

/// 1-based line number; tree-sitter's 0-based rows are converted once at the scan boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
const FILE_SCOPE_RULES: &[&str] = &["file-size", "file-complexity"];
const FILE_HEAD_LINES: usize = 10;
/// Diagnostics about directives themselves; suppressing them would hide the repair prompt.
const META_RULES: &[&str] = &[
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
        assert!(rule_matches(&r("flag-argument"), "syntax-rules-go", "[flag-argument] x"));
        assert!(rule_matches(&r("primitive-obsession"), "primitive-obsession", "plain text"));
        assert!(!rule_matches(&r("other"), "syntax-rules-go", "[flag-argument] x"));
        assert!(rule_matches(&r("some-new-rule"), "unknown-checker", "[some-new-rule] m"));
    }

    #[test]
    fn rule_matches_should_BeFalse_When_DynamicPrefixCheckerAndBracketMatch() {
        let r = |s: &str| RuleId::new(s).unwrap();
        let msg = "[foo] used but never defined";
        assert!(!rule_matches(&r("foo"), "markdown-link-integrity", msg));
        assert!(rule_matches(&r("markdown-link-integrity"), "markdown-link-integrity", msg));
    }

    #[test]
    fn rule_matches_should_BeFalse_When_MetaRule() {
        let r = |s: &str| RuleId::new(s).unwrap();
        assert!(!rule_matches(&r("ignore-syntax"), "inline-ignore", "[ignore-syntax] bad"));
        assert!(!rule_matches(&r("unused-ignore"), "unused-ignore", "x"));
    }
}
