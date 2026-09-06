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
        "used by",
        "this belongs in the commit message, not a comment that outlives its caller",
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
            in_fence = check_commented_out_code(*comment, text, in_fence, &mut findings);
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
    in_fence: bool,
    findings: &mut Vec<Finding>,
) -> bool {
    let start_row = comment.start_position().row;
    let mut in_fence = in_fence;
    for (offset, raw_line) in text.lines().enumerate() {
        let stripped = strip_comment_markers(raw_line);
        if stripped.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || stripped.is_empty() {
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
/// command;") both carry those characters in ordinary prose. Only `{}[]=+*/&|!` survive
/// as the punctuation set — narrower still, at the further cost of no longer flagging a
/// punctuation-free statement like a bare `return;`/`break;`, or a real call expression
/// mentioned only via this branch (already independently caught by
/// `is_call_expression`'s stricter shape check below, so nothing is actually lost there).
fn looks_like_code(text: &str) -> bool {
    if text.ends_with('{') || text == "}" || text.ends_with("});") {
        return true;
    }
    if text.ends_with(';') && text.chars().any(|c| "{}[]=+*/&|!".contains(c)) {
        return true;
    }
    is_call_expression(text) || is_assignment(text)
}

fn is_call_expression(text: &str) -> bool {
    let core = text.strip_suffix(';').unwrap_or(text);
    let Some(open) = core.find('(') else {
        return false;
    };
    if !core.ends_with(')') {
        return false;
    }
    let name = &core[..open];
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
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
    let lhs = text[..pos].trim();
    let rhs = text[pos + 1..].trim();
    !lhs.is_empty()
        && !lhs.contains(' ')
        && lhs
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && lhs
            .chars()
            .all(|c| c.is_alphanumeric() || "_.[]$".contains(c))
        && !rhs.is_empty()
        // A real single-statement assignment's RHS doesn't itself contain another bare
        // `=` — a second one signals a narrative computation explanation instead (a
        // real backtest finding: "FQDN=15 + 1(dot) + 55 = 71 chars" and
        // "OOMScoreAdj = 1000 - (...) = 869" both have a valid-looking `lhs`, but their
        // `rhs` re-derives a value through a second `=`, which no single Go/Rust/etc.
        // assignment statement does).
        && !rhs.contains('=')
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
    if (total_comment_lines as f64) < COMMENT_TO_CODE_RATIO * (body_code_lines as f64) {
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
}
