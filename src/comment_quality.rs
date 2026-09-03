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
    ("seamlessly", "state the fact instead of reaching for marketing language"),
    ("powerful", "state the fact instead of reaching for marketing language"),
    ("robust", "state the fact instead of reaching for marketing language"),
    ("enterprise-grade", "state the fact instead of reaching for marketing language"),
    ("leverage", "say what actually happens instead of a vaguer synonym"),
    ("utilize", "say what actually happens instead of a vaguer synonym"),
    ("it's worth noting", "cut the filler and state the fact directly"),
    ("as mentioned above", "cut the filler and state the fact directly"),
    ("note that", "cut the filler and state the fact directly"),
    ("needless to say", "cut the filler and state the fact directly"),
    ("designed to improve", "only document what the code actually does, not the intent behind it"),
    ("supports future", "only document what the code actually does, not speculative future use"),
    ("used by", "this belongs in the commit message, not a comment that outlives its caller"),
    ("added for the", "this belongs in the commit message, not a comment describing why it was added"),
    ("this fix", "this belongs in the commit message, not a comment referencing the change"),
    ("this pr", "this belongs in the commit message, not a comment referencing the change"),
    ("handles the case from issue", "this belongs in the commit message, not a comment referencing an issue"),
];

fn comment_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Go | Language::Python | Language::TypeScript | Language::Tsx | Language::JavaScript => {
            &["comment"]
        }
        Language::Java | Language::Kotlin => &["line_comment", "block_comment"],
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
        for comment in &comments {
            let text = comment.utf8_text(ctx.source.as_bytes()).unwrap_or("");
            check_verbose_phrases(*comment, text, &mut findings);
            check_commented_out_code(*comment, text, &mut findings);
        }

        let cfg = rules::lang_config(self.lang);
        walk_declarations_for_proportionality(tree.root_node(), &cfg, kinds, &mut findings);

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

fn check_verbose_phrases(comment: Node, text: &str, findings: &mut Vec<Finding>) {
    let lower = text.to_lowercase();
    for (phrase, reason) in BANNED_PHRASES {
        if lower.contains(phrase) {
            findings.push(Finding {
                line: comment.start_position().row + 1,
                message: format!("[verbose-comment] contains \"{phrase}\" — {reason}"),
            });
        }
    }
}

fn check_commented_out_code(comment: Node, text: &str, findings: &mut Vec<Finding>) {
    let start_row = comment.start_position().row;
    for (offset, raw_line) in text.lines().enumerate() {
        let stripped = strip_comment_markers(raw_line);
        if stripped.is_empty() {
            continue;
        }
        if looks_like_code(stripped) {
            findings.push(Finding {
                line: start_row + offset + 1,
                message: "[commented-out-code] this line looks like dead code, not a comment — delete it or explain why it's kept".to_string(),
            });
        }
    }
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

/// Conservative code-shape heuristic: a semicolon/brace terminator, or a call/assignment
/// expression with no spaces where a sentence would have them. Deliberately biased
/// toward missing real commented-out code over flagging prose (see
/// `docs/reporting-false-positives.md` for how to report a miss the other way).
fn looks_like_code(text: &str) -> bool {
    if text.ends_with(';') || text.ends_with('{') || text == "}" || text.ends_with("});") {
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
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
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
        && lhs.chars().all(|c| c.is_alphanumeric() || "_.[]$".contains(c))
        && !rhs.is_empty()
}

fn walk_declarations_for_proportionality(
    node: Node,
    cfg: &rules::LangRuleConfig,
    comment_kinds: &[&str],
    findings: &mut Vec<Finding>,
) {
    if cfg.function_kinds.contains(&node.kind()) {
        check_proportionality(node, cfg, comment_kinds, findings);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_declarations_for_proportionality(child, cfg, comment_kinds, findings);
    }
}

fn check_proportionality(
    decl: Node,
    cfg: &rules::LangRuleConfig,
    comment_kinds: &[&str],
    findings: &mut Vec<Finding>,
) {
    let Some(body) = (cfg.body_finder)(decl) else {
        return;
    };
    let body_total_lines = body.end_position().row - body.start_position().row + 1;

    let mut comment_rows = BTreeSet::new();
    collect_comment_rows(body, comment_kinds, &mut comment_rows);
    let body_comment_lines = comment_rows.len();

    let leading_rows = leading_comment_rows(decl, comment_kinds);
    let leading_start_line = leading_rows.iter().next().map(|row| row + 1);

    let total_comment_lines = body_comment_lines + leading_rows.len();
    let body_code_lines = body_total_lines.saturating_sub(body_comment_lines);

    if total_comment_lines < MIN_COMMENT_LINES_FOR_RATIO || body_code_lines == 0 {
        return;
    }
    if (total_comment_lines as f64) < COMMENT_TO_CODE_RATIO * (body_code_lines as f64) {
        return;
    }

    findings.push(Finding {
        line: leading_start_line.unwrap_or(decl.start_position().row + 1),
        message: format!(
            "[over-commented] {total_comment_lines} comment lines over a {body_code_lines}-line function body — looks like the comment restates the code instead of explaining why"
        ),
    });
}

fn collect_comment_rows(node: Node, comment_kinds: &[&str], rows: &mut BTreeSet<usize>) {
    if comment_kinds.contains(&node.kind()) {
        for row in node.start_position().row..=node.end_position().row {
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
/// while each one stays contiguous with the one after it.
fn leading_comment_rows(decl: Node, comment_kinds: &[&str]) -> BTreeSet<usize> {
    let mut rows = BTreeSet::new();
    let mut next_start_row = decl.start_position().row;
    let mut cursor = decl;
    while let Some(sibling) = cursor.prev_sibling() {
        if !comment_kinds.contains(&sibling.kind()) {
            break;
        }
        if sibling.end_position().row + 1 != next_start_row {
            break;
        }
        for row in sibling.start_position().row..=sibling.end_position().row {
            rows.insert(row);
        }
        next_start_row = sibling.start_position().row;
        cursor = sibling;
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
        assert!(findings.iter().any(|f| f.message.contains("[verbose-comment]") && f.message.contains("leverage")));
    }

    #[test]
    fn flags_commented_out_code() {
        let src = "package main\n\nfunc F() {\n\t// x = doSomething(1, 2);\n}\n";
        let findings = run(Language::Go, src);
        assert!(findings.iter().any(|f| f.message.contains("[commented-out-code]")));
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
        assert!(findings.iter().any(|f| f.message.contains("[over-commented]")), "findings: {findings:?}");
    }

    #[test]
    fn does_not_flag_proportionate_explanation_over_short_body() {
        let src = "package main\n\n// Retry calls fn up to attempts times, waiting delay between failures.\n// Returns the first successful result, or the last error if every attempt\n// fails — callers that need cancellation should wrap fn themselves, since\n// Retry does not accept a context.\nfunc Retry(fn func() (int, error)) (int, error) {\n\tresult, err := fn()\n\treturn result, err\n}\n";
        let findings = run(Language::Go, src);
        assert!(
            !findings.iter().any(|f| f.message.contains("[over-commented]")),
            "findings: {findings:?}"
        );
    }
}
