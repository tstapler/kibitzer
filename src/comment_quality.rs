use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::rules;

/// `over-commented` fires when a declaration's attached comment lines are at least this
/// multiple of its body's code-line count. A well-justified "why" comment can easily run
/// as long as the short function it documents (ratio ~1.0) — this only fires once
/// comments meaningfully outweigh the code, which is what actually signals restatement
/// rather than explanation. Tuned against `examples/*/comment-good.*`: a 4-line doc
/// comment over a 4-line body (ratio 1.0) must NOT fire; see
/// examples/python/comment-good.py's `retry`.
const COMMENT_TO_CODE_RATIO: f64 = 2.0;
/// ...and only once the comment block clears this many lines — keeps a short doc
/// comment over a trivial function from firing.
const MIN_COMMENT_LINES_FOR_RATIO: usize = 4;
/// `COMMENT_TO_CODE_RATIO` grows by this much per parameter beyond
/// `PARAM_COUNT_RATIO_BASELINE` — a function with several parameters (especially
/// primitive/boolean ones `primitive-obsession`/`long-parameter-list` already flag)
/// legitimately needs more explanation regardless of body length, since the type
/// system carries none of their semantics. Unlike the punctuation/word-boundary fixes
/// nearby, this specific increment has no real-world example directly requiring it —
/// see docs/comment-quality-false-positives.md for what backtest evidence does and
/// doesn't support here; treat it as a starting hypothesis like `COMMENT_TO_CODE_RATIO`
/// itself, not a derived constant.
const PARAM_COUNT_RATIO_BONUS: f64 = 0.25;
const PARAM_COUNT_RATIO_BASELINE: usize = 2;

/// Marketing filler, hedge words, and invented-rationale phrases that add nothing a
/// reader couldn't already see, plus the task/fix/caller-referencing anti-pattern
/// (comments should describe the code, not the change that produced it — those belong
/// in the commit message instead, where they don't rot as the code moves on).
/// Matched case-insensitively as a substring, so keep entries lowercase.
const BANNED_PHRASES: &[(&str, &str)] = &[
    (
        "seamlessly",
        "state the fact instead of reaching for marketing language",
    ),
    (
        "powerful",
        "state the fact instead of reaching for marketing language",
    ),
    (
        "robust",
        "state the fact instead of reaching for marketing language",
    ),
    (
        "enterprise-grade",
        "state the fact instead of reaching for marketing language",
    ),
    // The next two roots' inflected forms are listed as separate entries rather than
    // matched by a stem, per `contains_whole_phrase`'s word-boundary check below — a
    // real backtest finding: whole-word matching (needed so a short attribution
    // phrase doesn't fire inside an unrelated longer word) would otherwise also
    // reject a common third-person present-tense phrasing as "root plus more
    // letters," losing a legitimate hit.
    (
        "leverage",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "leverages",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "leveraging",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "leveraged",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "utilize",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "utilizes",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "utilizing",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "utilized",
        "say what actually happens instead of a vaguer synonym",
    ),
    (
        "it's worth noting",
        "cut the filler and state the fact directly",
    ),
    (
        "as mentioned above",
        "cut the filler and state the fact directly",
    ),
    ("note that", "cut the filler and state the fact directly"),
    (
        "needless to say",
        "cut the filler and state the fact directly",
    ),
    (
        "designed to improve",
        "only document what the code actually does, not the intent behind it",
    ),
    (
        "supports future",
        "only document what the code actually does, not speculative future use",
    ),
    (
        "added for the",
        "this belongs in the commit message, not a comment describing why it was added",
    ),
    (
        "this fix",
        "this belongs in the commit message, not a comment referencing the change",
    ),
    (
        "this pr",
        "this belongs in the commit message, not a comment referencing the change",
    ),
    (
        "handles the case from issue",
        "this belongs in the commit message, not a comment referencing an issue",
    ),
    // Bureaucratic wordy-filler phrases below, hand-picked from Vale's write-good
    // `TooWordy.yml` style pack (https://vale.sh/, vale-styles/write-good) — only the
    // unambiguous multi-word constructs that have no legitimate short form in a
    // technical comment. Deliberately NOT importing that pack's full list (or its
    // `Weasel.yml`): both are calibrated for narrative/bureaucratic prose and flag
    // ordinary technical vocabulary ("eliminate", "employ", "currently", "correctly")
    // that reads fine in a code comment — wholesale import would trade precision for
    // coverage in the wrong direction for this checker.
    (
        "in order to",
        "say \"to\" instead — \"in order to\" is always wordy filler",
    ),
    ("due to the fact that", "say \"because\" instead"),
    ("because of the fact that", "say \"because\" instead"),
    ("by virtue of the fact that", "say \"because\" instead"),
    ("in spite of the fact that", "say \"although\" instead"),
    ("in the event that", "say \"if\" instead"),
    ("with regard to", "say \"about\" instead"),
    ("with regards to", "say \"about\" instead"),
    ("for the purpose of", "say \"to\" or \"for\" instead"),
    (
        "it is important to note that",
        "cut the filler and state the fact directly",
    ),
];

/// `"used by"` was previously a plain `BANNED_PHRASES` entry, but a bare substring
/// match also caught the ubiquitous, encouraged Godoc idiom "X is used by Y"
/// ("PodSubnet is the subnet used by pods.") — a 40-finding corpus sample found this
/// was the single largest false-positive mechanism (~15/40 hits; see
/// docs/comment-quality-false-positives.md). The phrase is only meant to catch a
/// comment naming a specific, impermanent caller/PR/issue ("used by the fix in PR
/// #123"), so it now also requires a PR/issue-number token (`#123`) in the same
/// comment — the one thing that actually distinguishes that pattern from ordinary
/// consumer documentation.
const USED_BY_REASON: &str =
    "this belongs in the commit message, not a comment that outlives its caller";

fn contains_issue_number_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    (0..bytes.len()).any(|i| bytes[i] == b'#' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
}

fn comment_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Go
        | Language::Python
        | Language::TypeScript
        | Language::Tsx
        | Language::JavaScript => &["comment"],
        Language::Java | Language::Kotlin | Language::Rust => &["line_comment", "block_comment"],
    }
}

pub struct CommentQualityChecker {
    lang: Language,
}

impl CommentQualityChecker {
    pub fn new(lang: Language) -> Self {
        Self { lang }
    }

    fn name_for(lang: Language) -> &'static str {
        match lang {
            Language::Go => "comment-quality-go",
            Language::TypeScript => "comment-quality-typescript",
            Language::Tsx => "comment-quality-tsx",
            Language::JavaScript => "comment-quality-javascript",
            Language::Python => "comment-quality-python",
            Language::Java => "comment-quality-java",
            Language::Kotlin => "comment-quality-kotlin",
            Language::Rust => "comment-quality-rust",
        }
    }
}

impl Checker for CommentQualityChecker {
    fn name(&self) -> &str {
        Self::name_for(self.lang)
    }

    fn description(&self) -> &str {
        "flags verbose/marketing comment language, commented-out code, and comments \
         disproportionate to the code they annotate (see docs/comment-quality.md)"
    }

    fn language(&self) -> Option<Language> {
        Some(self.lang)
    }

    fn file_globs(&self) -> &[&str] {
        rules::lang_config(self.lang).file_globs
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("comment-quality checker requires a parsed tree")?;
        let kinds = comment_kinds(self.lang);
        let mut findings = Vec::new();

        let mut comments = Vec::new();
        collect_comments(tree.root_node(), kinds, &mut comments);
        // Threaded across every comment node in document order (not reset per-node) so
        // a fenced code example spanning several consecutive single-line nodes — Rust's
        // `///` lines are each their own node, verified via `to_sexp()` — is tracked as
        // one fence, not re-opened/closed per line. See `check_commented_out_code`.
        let mut in_fence = false;
        for comment in &comments {
            let text = comment.utf8_text(ctx.source.as_bytes()).unwrap_or("");
            check_verbose_phrases(*comment, text, &mut findings);
            in_fence =
                check_commented_out_code(*comment, text, ctx.source, in_fence, &mut findings);
        }

        let cfg = rules::lang_config(self.lang);
        walk_declarations_for_proportionality(
            tree.root_node(),
            &cfg,
            kinds,
            ctx.source.as_bytes(),
            &mut findings,
        );

        Ok(findings)
    }
}

fn collect_comments<'a>(node: Node<'a>, kinds: &[&str], out: &mut Vec<Node<'a>>) {
    if kinds.contains(&node.kind()) {
        out.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_comments(child, kinds, out);
    }
}

/// Whether `haystack` contains `needle` as a whole word/phrase, not merely as a
/// substring — a real backtest finding (docs/comment-quality-false-positives.md): the
/// short banned phrase `"this pr"` matched inside ordinary words like "this
/// **pr**events"/"this **pr**operly"/"this **pr**ocess" with a plain `.contains()`.
/// Only the two outer boundaries are checked (not `needle`'s internal spaces), so a
/// multi-word phrase still matches as a unit.
fn contains_whole_phrase(haystack: &str, needle: &str) -> bool {
    let mut search_start = 0;
    while let Some(rel_pos) = haystack.get(search_start..).and_then(|s| s.find(needle)) {
        let pos = search_start + rel_pos;
        let before_ok = haystack[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = haystack[pos + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        search_start = pos + 1;
    }
    false
}

fn check_verbose_phrases(comment: Node, text: &str, findings: &mut Vec<Finding>) {
    let lower = text.to_lowercase();
    for (phrase, reason) in BANNED_PHRASES {
        if contains_whole_phrase(&lower, phrase) {
            findings.push(Finding {
                line: comment.start_position().row + 1,
                message: format!("[verbose-comment] contains \"{phrase}\" — {reason}"),
            });
        }
    }
    if contains_whole_phrase(&lower, "used by") && contains_issue_number_token(&lower) {
        findings.push(Finding {
            line: comment.start_position().row + 1,
            message: format!("[verbose-comment] contains \"used by\" — {USED_BY_REASON}"),
        });
    }
}

/// Scans one comment node's lines for dead code, skipping anything inside a fenced
/// (```` ``` ````) code example — a real backtest finding: Rust doc comments routinely
/// embed real, intentional usage examples this way, which `looks_like_code` is
/// (correctly, for its actual purpose) built to recognize as code-shaped. `in_fence`
/// carries the fence state in from the previous comment node and returns the state
/// after this one, so a fence spanning several consecutive single-line nodes (Rust's
/// `///` lines are each their own node) is tracked as one fence, not reset per node.
fn check_commented_out_code(
    comment: Node,
    text: &str,
    src: &str,
    in_fence: bool,
    findings: &mut Vec<Finding>,
) -> bool {
    let start_row = comment.start_position().row;
    let mut in_fence = in_fence;
    // A trailing comment is only "trailing" (i.e. sharing a physical row with other
    // code) on the comment node's own first line — see `is_brace_closing_annotation`.
    let skip_brace_closing_annotation = is_brace_closing_annotation(comment, src);
    let baseline_indent = block_comment_baseline_indent(text);
    for (offset, raw_line) in text.lines().enumerate() {
        let stripped = strip_comment_markers(raw_line);
        if stripped.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || stripped.is_empty() {
            continue;
        }
        if offset == 0 && skip_brace_closing_annotation {
            continue;
        }
        if is_indented_code_example_line(raw_line, offset, baseline_indent) {
            continue;
        }
        if looks_like_code(stripped) {
            findings.push(Finding {
                line: start_row + offset + 1,
                message: "[commented-out-code] this line looks like dead code, not a comment — delete it or explain why it's kept".to_string(),
            });
        }
    }
    in_fence
}

/// Godoc's real "this is a code example" signal (https://pkg.go.dev/go/doc/comment):
/// a line indented further than the surrounding paragraph — not a triple-backtick
/// fence, which `go doc` doesn't render specially at all. A real backtest finding
/// (docs/comment-quality-false-positives.md): both an indented callback-signature
/// example in a `/* */` doc comment and an indented method-chain example in `//`
/// lines were misread as dead code by the fence-only exclusion.
///
/// Two shapes carry the signal depending on which physical line this is: a
/// `//`/`///`/`//!`-prefixed line where the marker is followed by a tab or 2+ spaces
/// before its content (gofmt normalizes ordinary doc-comment prose to a single space
/// after the marker; more than that is deliberate) — this needs no baseline, since
/// the indentation shows up right after the marker on every such line — or, inside a
/// `/* */` block comment (whose lines all share one comment node), a continuation
/// line indented further than `baseline_indent`.
fn is_indented_code_example_line(
    raw_line: &str,
    offset: usize,
    baseline_indent: Option<usize>,
) -> bool {
    let trimmed = raw_line.trim_start();
    for marker in ["///", "//!", "//"] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return rest.starts_with('\t') || rest.starts_with("  ");
        }
    }
    // No `//`-family marker on this physical line: either the block comment's own
    // opening line (its leading whitespace is the comment's own source indentation,
    // not comment-internal indentation — not a useful signal here) or a bare
    // continuation line.
    if offset == 0 {
        return false;
    }
    let Some(baseline) = baseline_indent else {
        return false;
    };
    if trimmed.is_empty() {
        return false;
    }
    let indent = raw_line.chars().take_while(|c| c.is_whitespace()).count();
    indent > baseline
}

/// The minimum leading-whitespace width among a `/* */` block comment's own
/// continuation lines (everything but the opening `/*` line, and any line that
/// itself carries a `//`-family marker) — used as the "normal paragraph indentation"
/// baseline `is_indented_code_example_line` compares against. Taking the minimum
/// (rather than assuming 0) correctly tolerates a block comment that's uniformly
/// indented to match its surrounding source code: every line then shares that same
/// minimum, so none of them reads as "indented further than the rest." `None` when
/// the comment has no such continuation line (e.g. a single-line `//` comment, whose
/// own indentation check doesn't need a baseline at all).
fn block_comment_baseline_indent(text: &str) -> Option<usize> {
    text.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .filter(|line| !line.trim_start().starts_with("//"))
        .map(|line| line.chars().take_while(|c| c.is_whitespace()).count())
        .min()
}

/// Whether `comment` is a trailing comment whose only preceding content on its own
/// physical row is a bare closing brace (optionally followed by `;`/`,`, as in a
/// `}(...)` expression statement or an array-of-structs entry) — the
/// `} // ConstructName(args)` idiom common in deeply-nested code, where the comment
/// names the construct several lines above whose closing brace this is. Call-syntax
/// text in such a comment (`is_call_expression`) is syntactically identical to real
/// commented-out code; this is a positional signal instead, checked once against the
/// row the comment node actually starts on rather than against its text — a real
/// backtest finding, see docs/comment-quality-false-positives.md.
fn is_brace_closing_annotation(comment: Node, src: &str) -> bool {
    let start = comment.start_position();
    let Some(row_text) = src.lines().nth(start.row) else {
        return false;
    };
    let Some(before) = row_text.get(..start.column) else {
        return false;
    };
    before.trim().trim_end_matches([';', ',']).trim_end() == "}"
}

/// Strips the leading comment-marker noise (`//`, `///`, `//!`, `#`, `/*`, `*/`, a
/// continuation `*`) so the remaining text can be judged as prose vs. code on its own.
fn strip_comment_markers(line: &str) -> &str {
    let mut s = line.trim();
    for prefix in ["///", "//!", "//", "/**", "/*", "*/", "#!", "#"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim();
            break;
        }
    }
    if let Some(rest) = s.strip_prefix('*') {
        // Javadoc/KDoc-style continuation line (`* foo`) — but not a `**foo` operator
        // line, which would already have been caught by the `/**` prefix above.
        s = rest.trim();
    }
    s.trim_end_matches("*/").trim()
}

/// Conservative code-shape heuristic: a brace terminator, a semicolon terminator
/// alongside punctuation no ordinary English clause would carry, or a call/assignment
/// expression with no spaces where a sentence would have them. Deliberately biased
/// toward missing real commented-out code over flagging prose (see
/// `docs/reporting-false-positives.md` for how to report a miss the other way).
///
/// A bare `ends_with(';')` check (SonarQube's S125 has the same documented gap) treats
/// any semicolon-terminated clause as code, so a doc comment written as a semicolon-
/// separated bullet list ("- validates input;") reads as "ends in `;`" and misfires —
/// see docs/comment-quality-false-positives.md. Requiring an unambiguous code-only
/// punctuation character alongside the trailing `;` (not `.`/`,`, both common in prose)
/// narrows that, but a real-world backtest (see docs/comment-quality-false-positives.md)
/// found `(`/`)` and `<`/`>` are *not* unambiguous either: an Apache license header
/// ("Licensed under ... (the \"License\");") and a spec-quoting blockquote ("> ... the
/// command;") both carry those characters in ordinary prose. A later real-world
/// instance (see docs/comment-quality-false-positives.md) found a bare `/` is not
/// unambiguous either: a `/`-separated prose list of names ("tags/auto_labels/
/// ocr_text are unindexed;") reads as an enumeration far more often than as division.
/// Only `{}[]=+*&|!` survive as the punctuation set — narrower still, at the further
/// cost of no longer flagging a punctuation-free statement like a bare
/// `return;`/`break;`, or a bare division/call expression mentioned only via this
/// branch (a real call is still independently caught by `is_call_expression`'s
/// stricter shape check below, so nothing is lost there; a bare division statement
/// with no other code-only punctuation is rare enough as dead code, vs. common enough
/// as prose, to accept the same way the other exclusions above were).
fn looks_like_code(text: &str) -> bool {
    if is_proto_field_declaration(text) {
        return false;
    }
    if text.ends_with('{') || text == "}" || text.ends_with("});") {
        return true;
    }
    if text.ends_with(';') && text.chars().any(|c| "{}[]=+*&|!".contains(c)) {
        return true;
    }
    is_call_expression(text) || is_assignment(text)
}

/// A quoted Protocol Buffers IDL field declaration (`optional bool alpha_enum =
/// 1060;`), a real backtest finding (docs/comment-quality-false-positives.md) —
/// generated `.pb.go` files sometimes quote the source `.proto` field for
/// documentation, which otherwise matches `looks_like_code`'s semicolon-plus-`=`
/// branch. No real Go/Rust/etc. statement starts with a bare `optional`/`repeated`/
/// `required` keyword followed by a type name, so this is scoped narrowly (exactly
/// `<cardinality> <type> <field> = <digits>;`) to avoid also swallowing real
/// commented-out code that happens to start with one of those words.
fn is_proto_field_declaration(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    let [cardinality, ty, field, eq, num] = words[..] else {
        return false;
    };
    matches!(cardinality, "optional" | "repeated" | "required")
        && eq == "="
        && is_plain_identifier(ty)
        && is_plain_identifier(field)
        && num
            .strip_suffix(';')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn is_plain_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// A call (`foo(...)`, `a.b.c(...)`) spanning the WHOLE trimmed line, not just its
/// first paren group — requires the parens immediately after `name` to be balanced,
/// with nothing (beyond an already-stripped trailing `;`) following the matching
/// close. A real backtest finding (docs/comment-quality-false-positives.md): a
/// multi-term formula like `LendableCL(i) = round(NominalCL(i) * pct(i)/100.0)` has
/// alnum text before its first `(` and ends in `)`, so the old check — which only
/// looked at the first `(` and the last `)` — misread it as one call. The
/// ` = round(...)` that follows the first call's own matching close paren is what
/// actually makes it a formula/assignment, not a call.
fn is_call_expression(text: &str) -> bool {
    let core = text.strip_suffix(';').unwrap_or(text);
    let Some(open) = core.find('(') else {
        return false;
    };
    let name = &core[..open];
    if name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return false;
    }
    let Some(close) = matching_close_paren(core, open) else {
        return false;
    };
    core[close + 1..].trim().is_empty()
}

/// Byte index of the `)` matching the `(` at `open`, respecting nesting — `None` if
/// unbalanced. Byte-indexed scanning is safe here since `(`/`)` are single-byte ASCII
/// and never appear as a continuation byte of a multi-byte UTF-8 character.
fn matching_close_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, &b) in s.as_bytes().iter().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn is_assignment(text: &str) -> bool {
    let Some(pos) = text.find('=') else {
        return false;
    };
    let bytes = text.as_bytes();
    let before = pos.checked_sub(1).and_then(|i| bytes.get(i));
    let after = bytes.get(pos + 1);
    // Exclude `==`, `!=`, `<=`, `>=`, `=>` — comparisons/arrows, not assignments.
    if matches!(after, Some(b'=' | b'>')) || matches!(before, Some(b'=' | b'!' | b'<' | b'>')) {
        return false;
    }
    is_assignment_lhs(text[..pos].trim()) && is_assignment_rhs(text[pos + 1..].trim())
}

/// A real assignment's LHS is a non-empty, dotted-path-shaped identifier.
/// `true`/`false`/`nil` can never legitimately sit on an assignment's LHS (`true = ...`
/// doesn't compile) — only the character class was checked before, which let doc
/// shorthand like "true = no matching session" (explaining what a bool field's value
/// means) through as a fake "assignment" — a real backtest finding
/// (docs/comment-quality-false-positives.md).
fn is_assignment_lhs(lhs: &str) -> bool {
    !lhs.is_empty()
        && !lhs.contains(' ')
        && !matches!(lhs, "true" | "false" | "nil")
        && lhs
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && lhs
            .chars()
            .all(|c| c.is_alphanumeric() || "_.[]$".contains(c))
}

/// A real single-statement assignment's RHS doesn't itself contain another bare `=` —
/// a second one signals a narrative computation explanation instead (a real backtest
/// finding: "FQDN=15 + 1(dot) + 55 = 71 chars" and "OOMScoreAdj = 1000 - (...) = 869"
/// both have a valid-looking `lhs`, but their `rhs` re-derives a value through a
/// second `=`, which no single Go/Rust/etc. assignment statement does). Nor does it
/// contain a sentence boundary — a period followed by more prose (`cmd.WaitDelay = 2 *
/// time.Second. This analyzer enforces that rule`) signals a quoted code fragment
/// resuming into unrelated prose, not a real standalone assignment statement (a real
/// backtest finding, same doc). A genuine RHS never contains ". " — qualified access
/// like `time.Second` has no space after the dot.
fn is_assignment_rhs(rhs: &str) -> bool {
    !rhs.is_empty() && !rhs.contains('=') && !rhs.contains(". ")
}

fn walk_declarations_for_proportionality(
    node: Node,
    cfg: &rules::LangRuleConfig,
    comment_kinds: &[&str],
    src: &[u8],
    findings: &mut Vec<Finding>,
) {
    if cfg.function_kinds.contains(&node.kind()) {
        check_proportionality(node, cfg, comment_kinds, src, findings);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_declarations_for_proportionality(child, cfg, comment_kinds, src, findings);
    }
}

fn check_proportionality(
    decl: Node,
    cfg: &rules::LangRuleConfig,
    comment_kinds: &[&str],
    src: &[u8],
    findings: &mut Vec<Finding>,
) {
    let Some(body) = (cfg.body_finder)(decl) else {
        return;
    };
    let body_total_lines = body.end_position().row - body.start_position().row + 1;

    let mut comment_rows = BTreeSet::new();
    collect_comment_rows(body, comment_kinds, &mut comment_rows);
    let body_comment_lines = comment_rows.len();

    let leading_nodes = leading_comment_nodes(decl, comment_kinds, src);
    let leading_rows = leading_comment_rows(&leading_nodes);
    let leading_start_line = leading_rows.iter().next().map(|row| row + 1);

    let total_comment_lines = body_comment_lines + leading_rows.len();
    let body_code_lines = body_total_lines.saturating_sub(body_comment_lines);

    if total_comment_lines < MIN_COMMENT_LINES_FOR_RATIO || body_code_lines == 0 {
        return;
    }
    if is_delegating_single_statement_body(body, src) {
        // A real backtest (docs/comment-quality-false-positives.md) found every
        // sampled false positive had exactly this shape: a body that's one
        // statement delegating to something else (a call, a method chain, a
        // struct/object construction) — Go's `return &LimitedWriter{w, n}`, Java's
        // `return CompactionManager.instance.performSSTableRewrite(...)`, Rust's
        // `self.node.first_child_ref().map(Into::into)`. The comment in every case
        // documented behavior that lives in the callee/constructed type, or an
        // invariant the signature can't express — never a restatement of the one
        // line actually visible here, no matter how long the comment ran.
        return;
    }
    let param_count = (cfg.params_finder)(decl)
        .map(|params| (cfg.param_counter)(params))
        .unwrap_or(0);
    let effective_ratio = COMMENT_TO_CODE_RATIO
        + PARAM_COUNT_RATIO_BONUS * param_count.saturating_sub(PARAM_COUNT_RATIO_BASELINE) as f64;
    if (total_comment_lines as f64) < effective_ratio * (body_code_lines as f64) {
        return;
    }
    if has_safety_section(&leading_nodes, src) {
        return;
    }

    findings.push(Finding {
        line: leading_start_line.unwrap_or(decl.start_position().row + 1),
        message: format!(
            "[over-commented] {total_comment_lines} comment lines over a {body_code_lines}-line function body — looks like the comment restates the code instead of explaining why"
        ),
    });
}

/// Whether `body` (already known non-empty by the caller) contains exactly one
/// statement, and that statement delegates elsewhere (a call, a method chain, or a
/// struct/object construction) rather than being a self-contained computation over
/// the function's own parameters (`a + b`, `x > 0`). AST-based (named-child count),
/// not line-count-based — a line-count check would only catch a body crammed onto one
/// source line (Go's `func F() { return G() }` style) and miss the far more common
/// "brace on its own line" formatting, which is exactly the shape most of the real
/// false positives this exists to fix are written in.
fn is_delegating_single_statement_body(body: Node, src: &[u8]) -> bool {
    let container = statement_container(body);
    if container.named_child_count() != 1 {
        return false;
    }
    let Some(stmt) = container.named_child(0) else {
        return false;
    };
    let Ok(text) = stmt.utf8_text(src) else {
        return false;
    };
    is_delegating_text(text)
}

/// Descends through pure single-child "statement container" wrapper nodes to the
/// level that actually holds the function's statement(s) as named children — two
/// grammars here wrap differently, both verified via `to_sexp()`: Go's `block` always
/// wraps a `statement_list` one level down (verified for both a one- and a
/// two-statement body: `(block (statement_list (expression_statement ...)
/// (expression_statement ...)))`), and Kotlin's `function_body` (unlike every other
/// grammar here) wraps a `block` one level down rather than being the statement
/// container itself (see `rules.rs::kotlin_body`'s doc comment for the same
/// positional-vs-field-based grammar quirk this mirrors). Every other grammar's body
/// node already holds its statement(s) as direct named children, so this is a no-op
/// for them.
fn statement_container(mut node: Node) -> Node {
    loop {
        if node.named_child_count() != 1 {
            return node;
        }
        let Some(child) = node.named_child(0) else {
            return node;
        };
        if matches!(child.kind(), "statement_list" | "block") {
            node = child;
        } else {
            return node;
        }
    }
}

/// A call (`foo(...)`, `a.b.c(...)`, a chained `a.b().c()`) or a struct/object
/// construction (`Type{...}`, `&Type{...}`) both indicate delegation to something
/// whose own behavior isn't visible in the text given — as opposed to a
/// self-contained computation over the function's own parameters.
fn is_delegating_text(text: &str) -> bool {
    let text = text.trim();
    let text = text.strip_prefix("return ").unwrap_or(text).trim();
    let text = text.strip_prefix('&').unwrap_or(text);
    text.contains('(') || (text.contains('{') && text.trim_end_matches(';').ends_with('}'))
}

/// `# Safety` is Rust's standard doc-comment heading for justifying an `unsafe fn`'s
/// invariants — a real backtest finding: this idiom naturally produces a short
/// function with a long justification, which the ratio otherwise reads as
/// restatement when it's the opposite (the comment carries information the
/// signature alone can't). Checked as a plain lowercase substring, not Rust-gated,
/// since no other language's real doc comments coincidentally contain this exact
/// heading.
fn has_safety_section(leading_nodes: &[Node], src: &[u8]) -> bool {
    leading_nodes.iter().any(|node| {
        node.utf8_text(src)
            .is_ok_and(|t| t.to_lowercase().contains("# safety"))
    })
}

/// A single-line comment's `end_position()` sometimes lands at column 0 of the row
/// *after* its own last line, rather than the end of its own line — verified for
/// tree-sitter-rust's `line_comment`, whose reported span runs through its trailing
/// newline (unlike every other grammar this checker covers, where a single-line
/// comment's end position stays on its own row). Normalizing back to the comment's own
/// last row keeps the row-adjacency math below (used to detect a contiguous leading
/// comment block, and to count comment lines inside a body) grammar-independent.
fn comment_end_row(node: Node) -> usize {
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row {
        end.row - 1
    } else {
        end.row
    }
}

fn collect_comment_rows(node: Node, comment_kinds: &[&str], rows: &mut BTreeSet<usize>) {
    if comment_kinds.contains(&node.kind()) {
        for row in node.start_position().row..=comment_end_row(node) {
            rows.insert(row);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_comment_rows(child, comment_kinds, rows);
    }
}

/// Comment nodes immediately preceding `decl` with no blank-line gap, walked backward
/// while each one stays contiguous with the one after it — stopping (without including
/// the code-like one) at a block whose own text looks like commented-out code rather
/// than documentation. A real backtest finding: a run of leftover commented-out
/// function stubs immediately before a real, unrelated function was getting folded
/// into that function's "leading doc comment," inflating its comment-to-code ratio.
fn leading_comment_nodes<'a>(decl: Node<'a>, comment_kinds: &[&str], src: &[u8]) -> Vec<Node<'a>> {
    let mut nodes = Vec::new();
    let mut next_start_row = decl.start_position().row;
    let mut cursor = decl;
    while let Some(sibling) = cursor.prev_sibling() {
        if !comment_kinds.contains(&sibling.kind()) {
            break;
        }
        if comment_end_row(sibling) + 1 != next_start_row {
            break;
        }
        let Ok(text) = sibling.utf8_text(src) else {
            break;
        };
        if text
            .lines()
            .any(|line| looks_like_code(strip_comment_markers(line)))
        {
            break;
        }
        nodes.push(sibling);
        next_start_row = sibling.start_position().row;
        cursor = sibling;
    }
    nodes
}

fn leading_comment_rows(leading_nodes: &[Node]) -> BTreeSet<usize> {
    let mut rows = BTreeSet::new();
    for node in leading_nodes {
        for row in node.start_position().row..=comment_end_row(*node) {
            rows.insert(row);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checker::{GrammarCache, run_checker_with_cache};
    use std::path::PathBuf;

    fn run(lang: Language, source: &str) -> Vec<Finding> {
        let checker = CommentQualityChecker::new(lang);
        let cache = GrammarCache::new();
        run_checker_with_cache(&checker, &PathBuf::from("f"), source, &cache).unwrap()
    }

    #[test]
    fn flags_marketing_language() {
        let src = "package main\n\n// leverage this seamlessly\nfunc F() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[verbose-comment]") && f.message.contains("leverage"))
        );
    }

    #[test]
    fn flags_wordy_filler_phrase() {
        let src = "package main\n\n// We check this in order to validate the input.\nfunc F() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[verbose-comment]")
                    && f.message.contains("in order to")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn flags_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// x = doSomething(1, 2);\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]"))
        );
    }

    /// Regression guard for docs/comment-quality-false-positives.md's first entry:
    /// SonarQube's S125 has the same documented gap (a bare `ends_with(';')` treats any
    /// semicolon-terminated clause as code), and a semicolon-separated bullet list is a
    /// common real doc-comment style.
    #[test]
    fn does_not_flag_a_semicolon_terminated_bullet_list() {
        let src = "package main\n\n// Normalize does three things:\n// - validates input;\n// - normalizes casing;\n// - returns the result;\nfunc Normalize(s string) string {\n\treturn s\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn does_not_flag_ordinary_prose_comment() {
        let src = "package main\n\n// Parse validates the input and returns an error if it's malformed.\nfunc Parse() {}\n";
        let findings = run(Language::Go, src);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn flags_over_commented_function() {
        let src = "package main\n\n// This function adds two numbers together.\n// It takes a and b as parameters.\n// It returns the sum of a and b.\n// It never returns anything else.\n// It has no side effects.\n// There is nothing more to say about it.\nfunc Add(a, b int) int {\n\treturn a + b\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn does_not_flag_proportionate_explanation_over_short_body() {
        let src = "package main\n\n// Retry calls fn up to attempts times, waiting delay between failures.\n// Returns the first successful result, or the last error if every attempt\n// fails — callers that need cancellation should wrap fn themselves, since\n// Retry does not accept a context.\nfunc Retry(fn func() (int, error)) (int, error) {\n\tresult, err := fn()\n\treturn result, err\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_flags_marketing_language() {
        let src = "/// This seamlessly leverages a robust approach.\nfn f() {}\n";
        let findings = run(Language::Rust, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[verbose-comment]") && f.message.contains("leverage")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_flags_commented_out_code() {
        let src = "fn f() {\n    // x = do_something(1, 2);\n}\n";
        let findings = run(Language::Rust, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_does_not_flag_ordinary_prose_comment() {
        let src = "/// Parses the input and returns an error if it's malformed.\nfn parse() {}\n";
        let findings = run(Language::Rust, src);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn rust_flags_over_commented_function() {
        let src = "/// This function adds two numbers together.\n/// It takes a and b as parameters.\n/// It returns the sum of a and b.\n/// It never returns anything else.\n/// It has no side effects.\n/// There is nothing more to say about it.\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let findings = run(Language::Rust, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_checks_impl_methods_too() {
        let src =
            "struct S;\nimpl S {\n    /// This seamlessly does the thing.\n    fn m(&self) {}\n}\n";
        let findings = run(Language::Rust, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[verbose-comment]")),
            "findings: {findings:?}"
        );
    }

    // --- Regression tests from the 2026-09-06 Kubernetes/Cassandra/Servo backtest ---

    #[test]
    fn short_banned_phrase_does_not_match_inside_an_unrelated_longer_word() {
        let src = "// This prevents the race and properly handles the process.\nfunc f() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings.iter().any(|f| f.message.contains("\"this pr\"")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn short_banned_phrase_still_matches_as_its_own_word() {
        let src = "// For purpose of this PR, report only the failure count.\nfunc f() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings.iter().any(|f| f.message.contains("\"this pr\"")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn license_header_style_comment_is_not_flagged_as_commented_out_code() {
        let src = "// Licensed under the Apache License, Version 2.0 (the \"License\");\npackage main\n\nfunc f() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn spec_quoting_blockquote_ending_in_semicolon_is_not_flagged_as_commented_out_code() {
        let src = "// > or if the command is the fontSize command;\nfunc f() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn narrative_arithmetic_comment_is_not_flagged_as_commented_out_code() {
        let src = "// FQDN=15 + 1(dot) + 55 = 71 chars\nfunc f() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn fenced_doc_comment_code_example_is_not_flagged_as_commented_out_code() {
        let src = "/// Example:\n///\n/// ```\n/// let x = f(1, 2);\n/// ```\nfunc f() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn adjacent_commented_out_stub_is_not_folded_into_the_next_functions_ratio() {
        // The two `//`-commented function stubs immediately above `Real` look like
        // dead code, not `Real`'s own leading doc comment — they must not inflate
        // `Real`'s comment-to-code ratio (a real backtest finding against Servo).
        let src = "// fn Dead1() { return 1; }\n// fn Dead2() { return 2; }\nfunc Real() int {\n\treturn 3\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_safety_doc_section_is_exempt_from_over_commented() {
        let src = "/// Derefs a raw pointer.\n///\n/// # Safety\n///\n/// The caller must ensure the pointer is non-null, properly aligned, and\n/// points to a live, initialized value of type `T` for the duration of the\n/// borrow — violating any of these is immediate undefined behavior.\npub unsafe fn deref<T>(p: *const T) -> &'static T {\n    &*p\n}\n";
        let findings = run(Language::Rust, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    // --- Regression tests for the two 2026-09-06 backtest-informed enforcements:
    // delegating-single-statement-body exemption, and parameter-count-scaled ratio ---

    #[test]
    fn delegating_body_with_struct_construction_is_exempt_from_over_commented() {
        // Mirrors kubernetes/kubernetes's pkg/kubelet/util/ioutils/ioutils.go
        // LimitWriter almost verbatim.
        let src = "// LimitWriter is a copy of the standard library ioutils.LimitReader,\n// applied to the writer interface.\n// LimitWriter returns a Writer that writes to w\n// but stops with EOF after n bytes.\n// The underlying implementation is a *LimitedWriter.\nfunc LimitWriter(w Writer, n int64) Writer { return &LimitedWriter{w, n} }\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn delegating_body_with_qualified_multi_arg_call_is_exempt_from_over_commented() {
        // Mirrors apache/cassandra's ColumnFamilyStore.sstablesRewrite: several
        // opaque boolean/numeric parameters, a thorough per-parameter explanation,
        // over a body that's one delegating call — spread across multiple lines
        // (unlike the crammed-one-line test above) to prove the exemption is
        // AST-based (named-child count), not line-count-based.
        let src = "// Rewrite rewrites all SSTables according to specified parameters.\n//\n// skipIfCurrentVersion, if true, rewrites only SSTables older than current.\n// skipIfNewerThanTimestamp excludes SSTables created after this timestamp.\n// skipIfCompressionMatches, if true, rewrites only SSTables whose compression differs.\nfunc Rewrite(skipIfCurrentVersion bool, skipIfNewerThanTimestamp int64, skipIfCompressionMatches bool, jobs int) error {\n\treturn other.PerformRewrite(skipIfCurrentVersion, skipIfNewerThanTimestamp, skipIfCompressionMatches, jobs)\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_delegating_method_chain_is_exempt_from_over_commented() {
        // Mirrors servo/servo's ServoLayoutNode::dangerous_first_child: a method
        // chain (not a bare call or brace construction) as the sole statement.
        let src = "/// Get the first child of this node.\n///\n/// This node should never be exposed directly to the layout interface, as\n/// that may allow mutating a node that is being laid out on another thread.\npub(super) unsafe fn dangerous_first_child(&self) -> Option<Self> {\n    self.node.first_child_ref().map(Into::into)\n}\n";
        let findings = run(Language::Rust, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn multi_statement_body_is_not_exempt_even_when_the_last_statement_delegates() {
        // Mirrors apache/cassandra's ColumnFamilyStore.addSSTable: a precondition
        // check PLUS a delegating call is two statements, not one — the
        // single-statement gate must not treat this as "just a delegation" the way
        // it correctly does for a bare one-statement body.
        // Body is 4 lines (open-brace-with-signature, two statements, close brace),
        // so 8 comment lines are needed to clear the flat 2x ratio (8 >= 2.0*4).
        let src = "// Add validates and adds the given item to the store.\n// This should be called after ensuring the item's checksum matches, since\n// items with mismatched checksums silently corrupt the on-disk index and\n// there is no way to detect this after the fact — the corruption surfaces\n// only much later, in an unrelated request against unrelated data, by\n// which point the original cause is impossible to trace back.\n// This line and the next exist only to reach the required comment count.\n// Final padding line.\nfunc Add(item Item) {\n\tvalidate(item)\n\tstore.Add(item)\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn higher_parameter_count_raises_the_over_commented_threshold() {
        // 6 parameters, a single non-delegating statement (plain arithmetic — no
        // call/construction shape, so the delegating-body exemption correctly
        // doesn't apply here). At the flat 2x ratio this would fire, since 7
        // comment lines over a 3-line body clears a 2x threshold of 6. Scaled for
        // 6 params (2 plus a 0.25 bonus per param over the baseline of 2, giving
        // 3x here) it must not, since 7 no longer clears a 3x threshold of 9.
        // Body is 3 lines (open-brace-with-signature, one return, close brace); 7
        // comment lines clear the flat 2x threshold of 6 but not the scaled 3x
        // threshold of 9.
        let src = "// f validates a, b, c, d, e, and g against their expected ranges before use,\n// since callers frequently pass swapped or stale values here and the\n// resulting corruption is silent until much later in an unrelated request.\n// This line and the next two exist only to reach the required comment count.\n// Padding line two.\n// Padding line three.\n// Padding line four.\nfunc f(a, b, c, d, e, g int) int {\n\treturn a + b + c + d + e + g\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    // --- Regression tests for the two 2026-09-09 stelekit backtest findings ---

    /// Regression guard for docs/comment-quality-false-positives.md's first 2026-09-09
    /// log entry — built from the real stelekit occurrence
    /// (`QueryPlanAuditTest.kt:45`), not just a synthetic case. The slashes are an
    /// "either/or" enumeration of column names, not division.
    #[test]
    fn slash_separated_prose_list_ending_in_semicolon_is_not_flagged_as_commented_out_code() {
        let src = "fun test() {\n    // asset_index LIKE search — tags/auto_labels/ocr_text are unindexed text columns;\n    assertTrue(true)\n}\n";
        let findings = run(Language::Kotlin, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// A genuine division expression is still caught: unlike the bare-`/` prose case
    /// above, this is a real assignment statement (`is_assignment` matches regardless
    /// of the narrowed punctuation set, which only affects the semicolon-only branch
    /// of `looks_like_code`).
    #[test]
    fn division_assignment_expression_is_still_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// total = width / height;\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Regression guard for docs/comment-quality-false-positives.md's second
    /// 2026-09-09 log entry — built from the real stelekit occurrence (`App.kt:1929`),
    /// not just a synthetic case. The comment names the already-live
    /// `CompositionLocalProvider(...)` call several lines above whose closing brace
    /// this is, not a commented-out call.
    #[test]
    fn brace_closing_annotation_comment_is_not_flagged_as_commented_out_code() {
        let src = "fun App() {\n    CompositionLocalProvider(LocalWindowSizeClass) {\n        Text(\"hi\")\n    } // CompositionLocalProvider(LocalWindowSizeClass)\n}\n";
        let findings = run(Language::Kotlin, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// A genuine trailing commented-out call is still caught when it doesn't follow a
    /// bare closing brace — the positional exemption only fires when the closing brace
    /// is the *only* other code on the row.
    #[test]
    fn trailing_call_comment_after_real_code_is_still_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\tx := 1 // doSomething(x, y)\n\t_ = x\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn low_parameter_count_does_not_get_a_ratio_bonus() {
        // Same shape as the test above but with only 2 parameters (at the
        // baseline, so no bonus applies) and the same comment/body line counts —
        // this must still fire at the plain 2.0x ratio.
        let src = "// f validates a and b against their expected ranges before use,\n// since callers frequently pass swapped or stale values here and the\n// resulting corruption is silent until much later in an unrelated request.\n// This line and the next two exist only to reach the required comment count.\n// Padding line two.\n// Padding line three.\n// Padding line four.\nfunc f(a, b int) int {\n\treturn a + b + a + b + a + b\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }

    // --- Regression tests for the five 2026-09-12 backtest findings ---

    /// Fix 1 — regression guard for the real kubernetes/kubernetes
    /// (`cmd/kubeadm/app/apis/kubeadm/v1/types.go:292`) and stapler-squad
    /// (`config/types.go:83`) occurrences: ordinary Godoc consumer documentation with
    /// no caller/PR/issue reference must not trip the "used by" banned phrase.
    #[test]
    fn ordinary_godoc_used_by_sentence_is_not_flagged() {
        let src = "package main\n\n// PodSubnet is the subnet used by pods.\ntype T struct{}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings.iter().any(|f| f.message.contains("\"used by\"")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn stapler_squad_style_used_by_consumer_doc_is_not_flagged() {
        let src = "package main\n\n// defaultSessionRetentionDays is used by RetentionDaysOrDefault to\n// determine session cleanup.\ntype T struct{}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings.iter().any(|f| f.message.contains("\"used by\"")),
            "findings: {findings:?}"
        );
    }

    /// Fix 1 — the phrase still catches the pattern it was meant for: a comment
    /// naming a specific, impermanent caller/PR/issue.
    #[test]
    fn used_by_with_issue_number_is_still_flagged() {
        let src =
            "package main\n\n// This workaround is used by the fix in issue #456.\nfunc f() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings.iter().any(|f| f.message.contains("\"used by\"")),
            "findings: {findings:?}"
        );
    }

    /// Fix 2 — regression guard for the real kubernetes/kubernetes
    /// (`vendor/.../ginkgo/v2/core_dsl.go:720`) occurrence: an indented callback-
    /// signature example inside a `/* */` doc comment, with no backtick fence, is a
    /// real Godoc code example.
    #[test]
    fn indented_block_comment_code_example_is_not_flagged_as_commented_out_code() {
        let src = "package main\n\n/*\nThe callback can have any of the following signatures:\n\n\tfunc(ctx context.Context, data []byte)\n\nEither of these is accepted.\n*/\nfunc F() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 2 — regression guard for the real stapler-squad
    /// (`session/ent/classificationanalytics_create.go:439`) occurrence: an indented,
    /// unfenced method-chain example in `//` lines.
    #[test]
    fn tab_indented_line_comment_code_example_is_not_flagged_as_commented_out_code() {
        let src = "package main\n\n// Example usage:\n//\n//\tClient.Create().\n//\t\tExec(ctx)\nfunc F() {}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 2 — a genuinely uniform-indented block comment (matching its own source
    /// indentation, not an internal code example) must not have that indentation
    /// mistaken for a code example.
    #[test]
    fn uniformly_indented_block_comment_is_not_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t/*\n\tThis whole comment is indented to match the\n\tsurrounding code, not to mark a code example.\n\t*/\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 3 — regression guard for the real kubernetes/kubernetes
    /// (`.../exemptprioritylevelconfiguration.go:50`) occurrence: a multi-term
    /// formula using call-like notation for its variables, not a real call.
    #[test]
    fn math_formula_with_call_like_notation_is_not_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// LendableCL(i) = round( NominalCL(i) * lendablePercent(i)/100.0 )\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 3 — a genuine call expression (including one with nested parens) is still
    /// caught.
    #[test]
    fn nested_call_expression_is_still_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// Foo(Bar(1), Baz(2))\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 4 — regression guard for the real kubernetes/kubernetes
    /// (`vendor/.../csi/csi.pb.go:7644`) occurrence: a quoted `.proto` field
    /// declaration, not commented-out Go.
    #[test]
    fn proto_idl_field_declaration_is_not_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// optional bool alpha_enum = 1060;\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 4 — a real commented-out assignment that happens to start with
    /// "optional"-shaped text (but isn't proto field syntax) is still caught.
    #[test]
    fn non_proto_assignment_starting_with_cardinality_word_is_still_flagged() {
        let src = "package main\n\nfunc F() {\n\t// optionalFlag = computeDefault();\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 5(a) — regression guard for the real stapler-squad
    /// (`tools/lint/norawexec/analyzer.go:13`) occurrence: a quoted code fragment
    /// followed by unrelated prose in the same comment.
    #[test]
    fn quoted_fragment_followed_by_prose_is_not_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// cmd.WaitDelay = 2 * time.Second. This analyzer enforces that rule.\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 5(b) — regression guard for the real stapler-squad
    /// (`gen/proto/go/session/v1/insights.pb.go:41`) occurrence: a bare boolean
    /// literal used as doc shorthand, not a real LHS.
    #[test]
    fn boolean_literal_doc_shorthand_is_not_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// true = no matching session\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }

    /// Fix 5 — a genuine single-line assignment statement is still caught.
    #[test]
    fn genuine_simple_assignment_is_still_flagged_as_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// total = 5\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[commented-out-code]")),
            "findings: {findings:?}"
        );
    }
}
