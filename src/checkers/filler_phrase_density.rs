use std::path::Path;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Filler/hedge phrases that pad prose without adding information — common in
/// AI-generated and corporate writing. Lowercase, multi-word so substring matching
/// doesn't false-positive against ordinary shorter words.
const FILLER_PHRASES: &[&str] = &[
    "it is worth noting that",
    "it is important to note that",
    "it should be noted that",
    "in today's fast-paced world",
    "in today's digital age",
    "at the end of the day",
    "due to the fact that",
    "for all intents and purposes",
    "needless to say",
    "as previously mentioned",
    "moving forward",
    "to sum up",
    "in summary",
    "that being said",
    "with that said",
    "all things considered",
    "it goes without saying",
    "last but not least",
    "first and foremost",
    "in essence",
    "in a nutshell",
    "at its core",
];

/// Total filler-phrase occurrences in a paragraph at or above which it's flagged.
const FILLER_PHRASE_THRESHOLD: usize = 2;

/// A cheap structural proxy for "this paragraph is padded with hedge/filler language":
/// flags a paragraph whose flattened text contains [`FILLER_PHRASE_THRESHOLD`] or more
/// occurrences of phrases from [`FILLER_PHRASES`]. Plain substring counting, not
/// word-boundary-aware regex — these phrases are long and specific enough that a false
/// substring match inside another word is not a realistic concern. Judging whether a
/// phrase is filler in context is more subjective and domain-dependent than the purely
/// structural checks in [`crate::repetitive_sentences`] and [`crate::paragraph_breaks`],
/// so this is intentionally NOT wired into `config::default_checks` — it's opt-in only,
/// via a repo's `.kibitzer/inspect.json`.
pub struct FillerPhraseDensityChecker;

impl Checker for FillerPhraseDensityChecker {
    fn name(&self) -> &str {
        "filler-phrase-density"
    }

    fn description(&self) -> &str {
        "flags a paragraph with a high density of filler/hedge phrases"
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
    crate::checker::CheckerFactory(|| vec![Box::new(FillerPhraseDensityChecker)])
}

pub fn check_source(body: &str) -> Vec<Finding> {
    crate::markdown_text::check_paragraphs(body, check_paragraph)
}

/// Counts filler-phrase occurrences in `text` (checked lowercase) and flags the
/// paragraph if the total meets [`FILLER_PHRASE_THRESHOLD`], listing the matched
/// phrases in the order [`FILLER_PHRASES`] defines them.
fn check_paragraph(line: usize, text: &str) -> Option<Finding> {
    let lower = text.to_lowercase();
    let mut matched = Vec::new();
    let mut total = 0usize;

    for phrase in FILLER_PHRASES {
        let count = lower.matches(phrase).count();
        if count > 0 {
            total += count;
            matched.push(*phrase);
        }
    }

    if total < FILLER_PHRASE_THRESHOLD {
        return None;
    }

    Some(Finding {
        line,
        message: format!(
            "paragraph uses {total} filler phrases ({}) — say it more directly",
            matched.join(", ")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_paragraph_with_two_or_more_filler_phrases() {
        let body = "It is worth noting that this matters. Needless to say, it works.\n";
        let findings = check_source(body);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("2 filler phrases"));
        assert!(findings[0].message.contains("it is worth noting that"));
        assert!(findings[0].message.contains("needless to say"));
    }

    #[test]
    fn allows_paragraph_with_at_most_one_filler_phrase() {
        let body = "It is worth noting that this matters. The rest of this paragraph is plain.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_list_items() {
        let body = "- It is worth noting that this matters. Needless to say, it works.\n\
                     - At the end of the day, first and foremost, it still works.\n";
        assert!(check_source(body).is_empty());
    }
}
