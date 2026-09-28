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

/// Flags a paragraph whose count of AI-buzzword-list words meets [`threshold`]. A
/// mechanical proxy for "this reads like unedited LLM output" rather than a judgment
/// on any individual word, several of which ("robust", "leverage", "utilize") are
/// perfectly ordinary in technical writing on their own. Deliberately scoped to
/// `Tag::Paragraph` text outside any list, matching [`crate::repetitive_sentences`] and
/// [`crate::paragraph_breaks`]. Not wired into `config::default_checks()` — opt-in only
/// via a repo's `.kibitzer/inspect.json`, since the false-positive rate on legitimate
/// domain writing is too high to run everywhere by default.
///
/// [`AI_VOCAB`]/[`AI_VOCAB_THRESHOLD`] are the defaults; a project can override either
/// via this check's `options` (see [`Checker::configure`]):
/// ```json
/// { "checker": "ai-vocabulary-density", "options": { "words": ["synergy"], "threshold": 2 } }
/// ```
pub struct AiVocabularyDensityChecker {
    words: Vec<String>,
    threshold: usize,
}

impl Default for AiVocabularyDensityChecker {
    fn default() -> Self {
        AiVocabularyDensityChecker {
            words: AI_VOCAB.iter().map(|w| w.to_string()).collect(),
            threshold: AI_VOCAB_THRESHOLD,
        }
    }
}

/// Per-project override shape for [`AiVocabularyDensityChecker::configure`]. Both
/// fields optional — an absent one keeps that field's own default.
#[derive(serde::Deserialize)]
struct Options {
    words: Option<Vec<String>>,
    threshold: Option<usize>,
}

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
        Ok(self.check_source(ctx.source))
    }

    fn configure(&self, options: &serde_json::Value) -> Result<Option<Box<dyn Checker>>> {
        let opts: Options = serde_json::from_value(options.clone())?;
        Ok(Some(Box::new(AiVocabularyDensityChecker {
            words: opts.words.unwrap_or_else(|| self.words.clone()),
            threshold: opts.threshold.unwrap_or(self.threshold),
        })))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(AiVocabularyDensityChecker::default())])
}

impl AiVocabularyDensityChecker {
    fn check_source(&self, body: &str) -> Vec<Finding> {
        crate::markdown_text::check_paragraphs(body, |line, text| self.check_paragraph(line, text))
    }

    /// One paragraph's worth of flattened text, checked for AI-buzzword-list density.
    /// Returns a finding listing the matched words in the order they appeared once the
    /// count reaches `self.threshold`.
    fn check_paragraph(&self, line: usize, text: &str) -> Option<Finding> {
        let matches: Vec<&str> = WORD_RE
            .find_iter(text)
            .filter_map(|m| {
                let lower = m.as_str().to_lowercase();
                self.words.iter().find(|w| **w == lower).map(|w| w.as_str())
            })
            .collect();

        if matches.len() < self.threshold {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the default word list/threshold — the shorthand every non-`configure_*`
    /// test below uses instead of spelling out `AiVocabularyDensityChecker::default()`.
    fn check_source(body: &str) -> Vec<Finding> {
        AiVocabularyDensityChecker::default().check_source(body)
    }

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

    #[test]
    fn configure_overrides_word_list_and_threshold() {
        let configured = AiVocabularyDensityChecker::default()
            .configure(&serde_json::json!({ "words": ["banana"], "threshold": 1 }))
            .unwrap()
            .unwrap();
        let ctx = CheckContext {
            source: "One banana in this otherwise ordinary paragraph.\n",
            tree: None,
        };
        let findings = configured.check(Path::new("doc.md"), &ctx).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("banana"));
    }

    #[test]
    fn configure_keeps_default_word_list_when_only_threshold_is_overridden() {
        let configured = AiVocabularyDensityChecker::default()
            .configure(&serde_json::json!({ "threshold": 1 }))
            .unwrap()
            .unwrap();
        let ctx = CheckContext {
            source: "We should leverage this system.\n",
            tree: None,
        };
        let findings = configured.check(Path::new("doc.md"), &ctx).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn configure_rejects_malformed_options() {
        let err = AiVocabularyDensityChecker::default()
            .configure(&serde_json::json!({ "threshold": "not-a-number" }));
        assert!(err.is_err());
    }
}
