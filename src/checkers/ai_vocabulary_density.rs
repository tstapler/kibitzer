use std::path::Path;

use anyhow::Result;
use regex::Regex;
use std::sync::LazyLock;

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Words that show up disproportionately often in LLM-generated prose (the
/// "AI buzzword list" that's circulated widely enough to be a recognizable tell).
/// This is a mechanical proxy for "this paragraph reads like it was written by an LLM
/// on autopilot," not a semantic judgment — technical and domain writing legitimately
/// uses several of these words on their own merits ("robust", "leverage", "utilize"
/// all have ordinary, non-AI-tell uses in engineering prose). That's why this checker
/// is intentionally NOT wired into `config::default_checks()`: it's opt-in only, added
/// to a repo's `.kibitzer/inspect.json` when that repo's authors want the extra scrutiny.
const AI_VOCAB: &[&str] = &[
    "leverage",
    "tapestry",
    "seamless",
    "seamlessly",
    "nuanced",
    "comprehensive",
    "delve",
    "robust",
    "holistic",
    "paradigm",
    "synergy",
    "streamline",
    "streamlined",
    "cutting-edge",
    "unlock",
    "elevate",
    "foster",
    "underscore",
    "underscores",
    "testament",
    "intricate",
    "multifaceted",
    "invaluable",
    "pivotal",
    "harness",
    "bolster",
    "myriad",
    "plethora",
    "realm",
    "landscape",
    "ecosystem",
    "unpack",
    "resonate",
    "illuminate",
    "navigate",
    "empower",
    "transformative",
    "groundbreaking",
    "unprecedented",
    "meticulous",
    "meticulously",
    "profound",
    "vibrant",
    "embark",
    "showcase",
    "encompass",
    "facilitate",
    "endeavor",
    "utilize",
    "utilizing",
];

/// Total buzzword occurrences (not necessarily distinct words) in a paragraph at or
/// above which it's flagged. Three lets a single stray word through without noise
/// while still catching a paragraph that's visibly leaning on the list.
const AI_VOCAB_THRESHOLD: usize = 3;

static WORD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Za-z']+\b").unwrap());

/// Flags a paragraph whose count of AI-buzzword-list words meets
/// [`AI_VOCAB_THRESHOLD`] — a mechanical proxy for "this reads like unedited LLM
/// output" rather than a judgment on any individual word, several of which
/// ("robust", "leverage", "utilize") are perfectly ordinary in technical writing on
/// their own. Deliberately scoped to `Tag::Paragraph` text outside any list, matching
/// [`crate::repetitive_sentences`] and [`crate::paragraph_breaks`]. Not wired into
/// `config::default_checks()` — opt-in only via a repo's `.kibitzer/inspect.json`,
/// since the false-positive rate on legitimate domain writing is too high to run
/// everywhere by default.
pub struct AiVocabularyDensityChecker;

impl Checker for AiVocabularyDensityChecker {
    fn name(&self) -> &str {
        "ai-vocabulary-density"
    }

    fn description(&self) -> &str {
        "flags a paragraph with a high density of AI-buzzword-list vocabulary"
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
    crate::checker::CheckerFactory(|| vec![Box::new(AiVocabularyDensityChecker)])
}

pub fn check_source(body: &str) -> Vec<Finding> {
    crate::markdown_text::check_paragraphs(body, check_paragraph)
}

/// One paragraph's worth of flattened text, checked for AI-buzzword-list density.
/// Returns a finding listing the matched words in the order they appeared once the
/// count reaches [`AI_VOCAB_THRESHOLD`].
fn check_paragraph(line: usize, text: &str) -> Option<Finding> {
    let matches: Vec<&str> = WORD_RE
        .find_iter(text)
        .filter_map(|m| {
            let lower = m.as_str().to_lowercase();
            AI_VOCAB.iter().find(|w| **w == lower).copied()
        })
        .collect();

    if matches.len() < AI_VOCAB_THRESHOLD {
        return None;
    }

    Some(Finding {
        line,
        message: format!(
            "paragraph uses {} AI-buzzword-list words ({}) — consider more direct language",
            matches.len(),
            matches.join(", ")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_paragraph_with_three_or_more_buzzwords() {
        let body = "We should leverage this robust system to unlock a seamless workflow.\n";
        let findings = check_source(body);
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("leverage, robust, unlock, seamless")
        );
    }

    #[test]
    fn allows_paragraph_with_fewer_than_three_buzzwords() {
        let body = "We should leverage this robust system to get the job done.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_list_items() {
        let body = "- We leverage a robust and seamless system here.\n- Another robust leverage seamless line.\n";
        assert!(check_source(body).is_empty());
    }
}
