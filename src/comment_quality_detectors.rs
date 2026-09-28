use super::{FINDING_COMMENTED_OUT_CODE, FINDING_VERBOSE_COMMENT, phrases::BANNED_PHRASES};
use crate::checker::Finding;
use tree_sitter::Node;

pub(super) fn collect_comments<'a>(node: Node<'a>, kinds: &[&str], out: &mut Vec<Node<'a>>) {
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
/// short pull-request-reference entry in `BANNED_PHRASES` matched inside ordinary words
/// like "this prevents"/"this properly"/"this process" with a plain `.contains()`. Only
/// the two outer boundaries are checked, so a multi-word phrase still matches as a unit.
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

pub(super) fn check_verbose_phrases(comment: Node, text: &str, findings: &mut Vec<Finding>) {
    let lower = text.to_lowercase();
    for (phrase, reason) in BANNED_PHRASES {
        if contains_whole_phrase(&lower, phrase) {
            findings.push(Finding {
                line: comment.start_position().row + 1,
                message: format!("{FINDING_VERBOSE_COMMENT} contains \"{phrase}\" — {reason}"),
            });
        }
    }
}

/// Whether a fenced (```` ``` ````) code example is open at the end of one comment
/// node, threaded into the next — see `check_commented_out_code`'s doc comment for why
/// this must span nodes rather than reset per-node.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FenceState {
    Inside,
    Outside,
}

impl FenceState {
    fn toggled(self) -> Self {
        match self {
            FenceState::Inside => FenceState::Outside,
            FenceState::Outside => FenceState::Inside,
        }
    }
}

/// Scans one comment node's lines for dead code, skipping anything inside a fenced
/// code example — Rust doc comments routinely embed real usage examples this way,
/// which `looks_like_code` would otherwise flag. `fence` carries the fence state in
/// from the previous comment node so one spanning several consecutive single-line
/// nodes (Rust's `///` lines are each their own node) is tracked as one fence.
pub(super) fn check_commented_out_code(
    comment: Node,
    text: &str,
    fence: FenceState,
    findings: &mut Vec<Finding>,
) -> FenceState {
    let start_row = comment.start_position().row;
    let mut fence = fence;
    for (offset, raw_line) in text.lines().enumerate() {
        let stripped = strip_comment_markers(raw_line);
        if stripped.starts_with("```") {
            fence = fence.toggled();
            continue;
        }
        if fence == FenceState::Inside || stripped.is_empty() {
            continue;
        }
        if looks_like_code(stripped) {
            findings.push(Finding {
                line: start_row + offset + 1,
                message: format!(
                    "{FINDING_COMMENTED_OUT_CODE} this line looks like dead code, not a comment — delete it or explain why it's kept"
                ),
            });
        }
    }
    fence
}

/// Strips the leading comment-marker noise (`//`, `///`, `//!`, `#`, `/*`, `*/`, a
/// continuation `*`) so the remaining text can be judged as prose vs. code on its own.
pub(super) fn strip_comment_markers(line: &str) -> &str {
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

/// Conservative code-shape heuristic, deliberately biased toward missing real
/// commented-out code over flagging prose. The `{}[]=+*/&|!` punctuation set backing
/// the semicolon branch below is narrower than it looks — it's survived multiple
/// real-world false-positive backtests (license headers, blockquotes, bullet lists);
/// see docs/comment-quality-false-positives.md for why each broader alternative failed.
pub(super) fn looks_like_code(text: &str) -> bool {
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
