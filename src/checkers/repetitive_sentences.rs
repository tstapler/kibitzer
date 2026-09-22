use std::path::Path;

use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::markdown_text::split_sentences;

/// Sentence-opener words common enough that three in a row reads as monotonous
/// (JMU Writing Center / Purdue OWL "sentence variety" guidance). Anything else is
/// treated as a distinct, non-repeating opener — this catches the common "This..."/
/// "The..."/"It..." run without needing real part-of-speech tagging.
const OPENERS: &[&str] = &[
    "i", "you", "he", "she", "it", "we", "they", "this", "that", "these", "those", "there", "the",
    "a", "an",
];

static FIRST_WORD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z']+").unwrap());

/// Flags 3+ consecutive sentences in one flowing-prose paragraph that open with the same
/// common function word (a pronoun, article, or demonstrative) — a mechanical proxy for
/// "repetitive sentence structure" that doesn't require POS tagging. Deliberately scoped
/// to `Tag::Paragraph` text outside any list, since a parallel bulleted enumeration
/// (independent facts stated in the same grammatical form on purpose) is not this defect
/// — see the originating issue's false-positive guardrail.
pub struct RepetitiveSentencesChecker;

impl Checker for RepetitiveSentencesChecker {
    fn name(&self) -> &str {
        "repetitive-sentence-structure"
    }

    fn description(&self) -> &str {
        "flags 3+ consecutive sentences in a paragraph sharing the same opening word"
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
    crate::checker::CheckerFactory(|| vec![Box::new(RepetitiveSentencesChecker)])
}

pub fn check_source(body: &str) -> Vec<Finding> {
    crate::markdown_text::check_paragraphs(body, check_paragraph)
}

/// One paragraph's worth of flattened text, checked for a 3+ run of sentences sharing
/// the same opener — returns the first such run found (one finding per paragraph is
/// enough to flag it for a human to fix).
fn check_paragraph(line: usize, text: &str) -> Option<Finding> {
    let mut run_opener: Option<&str> = None;
    let mut run_len = 0usize;

    for sentence in split_sentences(text) {
        let opener = sentence_opener(sentence.trim());
        match (opener, run_opener) {
            (Some(o), Some(prev)) if o == prev => {
                run_len += 1;
                if run_len == 3 {
                    return Some(Finding {
                        line,
                        message: format!(
                            "3+ consecutive sentences start with \"{o}\" — vary sentence structure"
                        ),
                    });
                }
            }
            (Some(o), _) => {
                run_opener = Some(o);
                run_len = 1;
            }
            (None, _) => {
                run_opener = None;
                run_len = 0;
            }
        }
    }
    None
}

/// The sentence's opening word, lowercased, if it's one of [`OPENERS`] — anything else
/// returns `None` and breaks a run rather than starting a new trackable one.
fn sentence_opener(sentence: &str) -> Option<&'static str> {
    let word = FIRST_WORD_RE.find(sentence)?.as_str().to_lowercase();
    OPENERS.iter().find(|o| **o == word).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_three_consecutive_same_opener() {
        let body = "This is bad. This is worse. This is worst.\n";
        let findings = check_source(body);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("\"this\""));
    }

    #[test]
    fn allows_varied_openers() {
        let body = "This is fine. It works well. The result holds.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_list_items() {
        let body = "- This is a. This is b. This is c.\n- This is d. This is e. This is f.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn two_in_a_row_is_not_enough() {
        let body = "This is fine. This is also fine. It changes now.\n";
        assert!(check_source(body).is_empty());
    }
}
