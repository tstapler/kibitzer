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

/// `comment-too-long` fires when a declaration's leading doc comment alone (not
/// counting scattered inline comments inside the body) exceeds this many lines,
/// regardless of how large the function it documents is. Independent of
/// `COMMENT_TO_CODE_RATIO`, which only catches a comment that's large *relative to a
/// small function* — a comment can clear this absolute bound while still being
/// "proportionate" to a large function body, which is exactly the shape that let three
/// multi-paragraph WHY comments ship on a real PR undetected (see
/// docs/comment-quality-false-positives.md). Set to 5 to match the ceiling already
/// enforced by `/code:review`'s Code Quality Agent and CLAUDE.md's "Short beats long"
/// rule, so a comment that passes here won't get bounced by a human/LLM reviewer for
/// length alone.
const MAX_LEADING_COMMENT_LINES: usize = 5;

const FINDING_VERBOSE_COMMENT: &str = "[verbose-comment]";
const FINDING_COMMENTED_OUT_CODE: &str = "[commented-out-code]";
const FINDING_OVER_COMMENTED: &str = "[over-commented]";
const FINDING_COMMENT_TOO_LONG: &str = "[comment-too-long]";

#[path = "comment_quality_phrases.rs"]
mod phrases;

#[path = "comment_quality_detectors.rs"]
mod detectors;
use detectors::{
    FenceState, check_commented_out_code, check_verbose_phrases, collect_comments, looks_like_code,
    strip_comment_markers,
};

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
        "flags verbose/marketing comment language, commented-out code, comments \
         disproportionate to the code they annotate, and leading comments that are too \
         long in absolute terms regardless of proportion (see docs/comment-quality.md)"
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
        let mut fence = FenceState::Outside;
        for comment in &comments {
            let text = comment.utf8_text(ctx.source.as_bytes()).unwrap_or("");
            check_verbose_phrases(*comment, text, &mut findings);
            fence = check_commented_out_code(*comment, text, fence, &mut findings);
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
    let leading_nodes = leading_comment_nodes(decl, comment_kinds, src);
    let counts = comment_line_counts(decl, body, comment_kinds, &leading_nodes);

    if counts.total_comment_lines == 0 || counts.body_code_lines == 0 {
        return;
    }

    // Delegating bodies only exempt the ratio check (near-zero denominator); `# Safety`
    // exempts both, since that kind of comment can legitimately run long regardless of size.
    let safety_exempt = has_safety_section(&leading_nodes, src);
    if !safety_exempt {
        check_absolute_length(counts.leading_comment_lines, counts.finding_line, findings);
    }
    if safety_exempt || is_delegating_single_statement_body(body, src) {
        return;
    }

    check_comment_to_code_ratio(
        decl,
        cfg,
        (counts.total_comment_lines, counts.body_code_lines),
        counts.finding_line,
        findings,
    );
}

struct CommentLineCounts {
    leading_comment_lines: usize,
    total_comment_lines: usize,
    body_code_lines: usize,
    finding_line: usize,
}

/// Gathers the raw line counts `check_absolute_length`/`check_comment_to_code_ratio`
/// need, in one place, so `check_proportionality` doesn't have to.
fn comment_line_counts(
    decl: Node,
    body: Node,
    comment_kinds: &[&str],
    leading_nodes: &[Node],
) -> CommentLineCounts {
    let body_total_lines = body.end_position().row - body.start_position().row + 1;

    let mut comment_rows = BTreeSet::new();
    collect_comment_rows(body, comment_kinds, &mut comment_rows);
    let body_comment_lines = comment_rows.len();

    let leading_rows = leading_comment_rows(leading_nodes);
    let leading_comment_lines = leading_rows.len();
    let finding_line = leading_rows
        .iter()
        .next()
        .map(|row| row + 1)
        .unwrap_or(decl.start_position().row + 1);

    CommentLineCounts {
        leading_comment_lines,
        total_comment_lines: body_comment_lines + leading_comment_lines,
        body_code_lines: body_total_lines.saturating_sub(body_comment_lines),
        finding_line,
    }
}

/// `[comment-too-long]` — see `MAX_LEADING_COMMENT_LINES`'s doc comment for why this is
/// independent of `check_comment_to_code_ratio`'s ratio.
fn check_absolute_length(
    leading_comment_lines: usize,
    finding_line: usize,
    findings: &mut Vec<Finding>,
) {
    if leading_comment_lines > MAX_LEADING_COMMENT_LINES {
        findings.push(Finding {
            line: finding_line,
            message: format!(
                "{FINDING_COMMENT_TOO_LONG} leading comment is {leading_comment_lines} lines, over the {MAX_LEADING_COMMENT_LINES}-line ceiling — split into root cause / accepted tradeoff / pointer to a test, or trim to the one fact a reviewer would ask for"
            ),
        });
    }
}

/// `[over-commented]` — see `COMMENT_TO_CODE_RATIO`'s doc comment for the ratio/param-
/// count rationale.
fn check_comment_to_code_ratio(
    decl: Node,
    cfg: &rules::LangRuleConfig,
    (total_comment_lines, body_code_lines): (usize, usize),
    finding_line: usize,
    findings: &mut Vec<Finding>,
) {
    if total_comment_lines < MIN_COMMENT_LINES_FOR_RATIO {
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
    findings.push(Finding {
        line: finding_line,
        message: format!(
            "{FINDING_OVER_COMMENTED} {total_comment_lines} comment lines over a {body_code_lines}-line function body — looks like the comment restates the code instead of explaining why"
        ),
    });
}

/// Whether `body` (already known non-empty by the caller) contains exactly one
/// statement that delegates elsewhere (a call, method chain, or struct construction)
/// rather than a self-contained computation (`a + b`). AST-based (named-child count),
/// not line-count-based, so it also catches the common "brace on its own line" style,
/// not just a body crammed onto one source line.
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

/// Descends through single-child "statement container" wrapper nodes (Go's `block`
/// wraps a `statement_list`, Kotlin's `function_body` wraps a `block` — see
/// `rules.rs::kotlin_body`) to the level holding the function's statements as named
/// children. A no-op for every other grammar, whose body node already holds them.
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

/// tree-sitter-rust's `line_comment` span runs through its trailing newline, unlike
/// every other grammar here, so its `end_position()` lands one row past its own last
/// line. Normalizing that back keeps the row-adjacency math below grammar-independent.
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

/// Comment nodes immediately preceding `decl` with no blank-line gap, walked backward,
/// stopping (without including the code-like one) at a block that looks like
/// commented-out code rather than documentation — otherwise a run of leftover dead-code
/// stubs before a real function gets folded into that function's ratio (a real
/// backtest finding).
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
#[path = "comment_quality_tests.rs"]
mod tests;
