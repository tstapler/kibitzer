//! The one-line syntax teaching shown in footers and hook context.

use std::path::Path;

use super::types::*;
use crate::checker::Language;

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

pub(crate) const HINT_RULE_LIMIT: usize = 6;

/// Longer ids (a finding's `[prefix]` is free text) would blow the footer budget, so the hint
/// falls back to the generic `<rule>` form and omits them from the `Rules:` list.
const MAX_HINT_RULE_CHARS: usize = 64;

fn fits_hint(rule: &RuleId) -> bool {
    rule.as_str().len() <= MAX_HINT_RULE_CHARS
}

/// A directive as written in a comment, with `<why>` standing for the reason.
fn directive_example(open: &str, rule_text: &str, close: &str) -> String {
    format!("{open} {MARKER} {rule_text} -- <why>{close}")
}

/// The batch (`kibitzer run`) footer: one generic line, since no single file's comment leader applies.
pub(crate) fn batch_syntax_hint() -> String {
    format!(
        "[kibitzer] to dismiss a finding you judged acceptable: {} (docs/suppressing-checks.md)",
        directive_example("<comment>", "<rule>", "")
    )
}

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
    let rule_text = anchor
        .map(|(rule, _)| rule)
        .filter(|rule| fits_hint(rule))
        .map_or("<rule>", RuleId::as_str);
    let mut hint = format!(
        "Dismiss a judged finding: {}, ",
        directive_example(leader.open, rule_text, leader.close)
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
    if anchor.is_some() && rule_ids.iter().any(|r| fits_hint(r)) {
        let listable: Vec<&RuleId> = rule_ids.iter().copied().filter(|r| fits_hint(r)).collect();
        let shown: Vec<&str> = listable
            .iter()
            .take(max_rules.max(1))
            .map(|r| r.as_str())
            .collect();
        let more = if listable.len() > shown.len() {
            ", ..."
        } else {
            ""
        };
        hint.push_str(&format!(" Rules: {}{more}.", shown.join(", ")));
    }
    hint
}
