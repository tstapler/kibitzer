use std::path::Path;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Number of em dashes (`—`, U+2014) in one paragraph at or above which it's flagged.
const EM_DASH_THRESHOLD: usize = 3;

/// Flags a paragraph with heavy em-dash use — three or more em dashes (`—`, U+2014; not
/// a hyphen `-` or en-dash `–`). Heavy em-dash use is a commonly cited stylistic tell of
/// LLM-generated prose (see the heuristics in
/// [`puneethkotha/humanizer-workbench`](https://github.com/puneethkotha/humanizer-workbench)).
///
/// **This is a known-risky heuristic.** Heavy em-dash use is also a deliberate, common
/// style choice in well-edited technical writing — this codebase's own doc comments (this
/// one included) use em dashes constantly, following a Pinker/Google-style-guide voice.
/// That means this check has a real risk of flagging good human writing, not just AI
/// writing, and it is intentionally **not** wired into [`crate::config::default_checks`].
/// It's opt-in only, via a project's `.kibitzer/inspect.json`. Before anyone enables it
/// project-wide, backtest it carefully against real corpora — including this very repo's
/// own doc comments, which is expected to be a stress test and likely source of false
/// positives.
pub struct EmDashOveruseChecker;

impl Checker for EmDashOveruseChecker {
    fn name(&self) -> &str {
        "em-dash-overuse"
    }

    fn description(&self) -> &str {
        "flags a paragraph with heavy em-dash use"
    }

    fn language(&self) -> Option<Language> {
        None
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.md"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        Ok(check_source(ctx.source))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(EmDashOveruseChecker)])
}

pub fn check_source(body: &str) -> Vec<Finding> {
    crate::markdown_text::check_paragraphs(body, check_paragraph)
}

/// Counts em dashes (U+2014 only — a hyphen or en-dash doesn't count) in one paragraph's
/// flattened text and flags it once the count reaches [`EM_DASH_THRESHOLD`].
fn check_paragraph(line: usize, text: &str) -> Option<Finding> {
    let count = text.chars().filter(|&c| c == '\u{2014}').count();
    if count < EM_DASH_THRESHOLD {
        return None;
    }
    Some(Finding {
        line,
        message: format!("paragraph uses {count} em dashes — vary punctuation/sentence structure"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_three_or_more_em_dashes() {
        let body = "This is one — this is two — this is three — and this is four.\n";
        let findings = check_source(body);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("3 em dashes"));
    }

    #[test]
    fn allows_zero_to_two_em_dashes() {
        let body = "This has one — and only one dash here otherwise.\n";
        assert!(check_source(body).is_empty());

        let body = "This has one — and this has two — but no more.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn does_not_count_hyphens_or_en_dashes() {
        let body = "A well-known, multi-part, non-trivial sentence with a 2020\u{2013}2021 \
                     range and more well-formed hyphenated-words than you'd expect.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_list_items() {
        let body = "- This — has — three — dashes\n- Another — item — here — too\n";
        assert!(check_source(body).is_empty());
    }
}
