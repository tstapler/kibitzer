use std::path::Path;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::markdown_text::split_sentences;

/// Minimum sentence count before uniformity is worth judging at all — a paragraph with
/// only a few sentences can land on near-identical lengths by chance.
const MIN_SENTENCES: usize = 6;

/// Mean word count below which sentences are trivially short — a run of terse fragments
/// (list-like prose, a glossary, short asides) naturally has low variation without being
/// a genuine AI-writing tell.
const MIN_MEAN_WORDS: f64 = 4.0;

/// Coefficient-of-variation (stddev / mean) ceiling below which sentence lengths read as
/// suspiciously uniform.
const MAX_COEFFICIENT_OF_VARIATION: f64 = 0.2;

/// Flags a paragraph whose sentences are all suspiciously close to the same length —
/// near-identical sentence length across many consecutive sentences is one of several
/// statistical tells used by AI-generated-text detectors (borrowed from analyzing
/// [puneethkotha/humanizer-workbench](https://github.com/puneethkotha/humanizer-workbench)'s
/// heuristics). Human writing naturally varies sentence length more — a run of sentences
/// that all land within a narrow band reads as mechanical even when no single sentence is
/// individually wrong. Purely structural/mechanical (word counts only, no word lists), but
/// NOT wired into `config::default_checks` — opt-in only, via a repo's
/// `.kibitzer/inspect.json`. Backtesting against kubernetes/website found this heuristic
/// flagging a deliberately parallel enumeration ("Signers must not... Signers should...
/// Signers should...") with "vary sentence length," which would have actively broken the
/// intentional parallelism — a real false-positive risk in exactly the kind of structured
/// technical documentation this checker is meant to help, not just vocabulary/phrase-list
/// noise on unrelated domain writing.
///
/// [`MIN_SENTENCES`]/[`MIN_MEAN_WORDS`]/[`MAX_COEFFICIENT_OF_VARIATION`] are the
/// defaults; a project can override any of them via this check's `options` (see
/// [`Checker::configure`]) — e.g. to raise `min_sentences` and cut down on false
/// positives against short, deliberately parallel enumerations:
/// ```json
/// { "checker": "sentence-length-uniformity", "options": { "min_sentences": 10 } }
/// ```
pub struct SentenceLengthUniformityChecker {
    min_sentences: usize,
    min_mean_words: f64,
    max_coefficient_of_variation: f64,
}

impl Default for SentenceLengthUniformityChecker {
    fn default() -> Self {
        SentenceLengthUniformityChecker {
            min_sentences: MIN_SENTENCES,
            min_mean_words: MIN_MEAN_WORDS,
            max_coefficient_of_variation: MAX_COEFFICIENT_OF_VARIATION,
        }
    }
}

/// Per-project override shape for [`SentenceLengthUniformityChecker::configure`]. Every
/// field optional — an absent one keeps that field's own default.
#[derive(serde::Deserialize)]
struct Options {
    min_sentences: Option<usize>,
    min_mean_words: Option<f64>,
    max_coefficient_of_variation: Option<f64>,
}

impl Checker for SentenceLengthUniformityChecker {
    fn name(&self) -> &str {
        "sentence-length-uniformity"
    }

    fn description(&self) -> &str {
        "flags a paragraph whose sentences are all suspiciously close to the same length"
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
        Ok(Some(Box::new(SentenceLengthUniformityChecker {
            min_sentences: opts.min_sentences.unwrap_or(self.min_sentences),
            min_mean_words: opts.min_mean_words.unwrap_or(self.min_mean_words),
            max_coefficient_of_variation: opts
                .max_coefficient_of_variation
                .unwrap_or(self.max_coefficient_of_variation),
        })))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(SentenceLengthUniformityChecker::default())])
}

impl SentenceLengthUniformityChecker {
    fn check_source(&self, body: &str) -> Vec<Finding> {
        crate::markdown_text::check_paragraphs(body, |line, text| self.check_paragraph(line, text))
    }

    /// One paragraph's worth of flattened text, checked for a run of sentences whose
    /// word counts vary too little to look human-written. Requires at least
    /// `self.min_sentences` sentences and a mean length of at least
    /// `self.min_mean_words` words before judging variation at all, since short
    /// paragraphs and terse fragments are noisy on their own.
    fn check_paragraph(&self, line: usize, text: &str) -> Option<Finding> {
        let sentences = split_sentences(text);
        if sentences.len() < self.min_sentences {
            return None;
        }

        let word_counts: Vec<f64> = sentences
            .iter()
            .map(|s| s.split_whitespace().count() as f64)
            .collect();

        let mean = word_counts.iter().sum::<f64>() / word_counts.len() as f64;
        if mean < self.min_mean_words {
            return None;
        }

        let variance =
            word_counts.iter().map(|w| (w - mean).powi(2)).sum::<f64>() / word_counts.len() as f64;
        let stddev = variance.sqrt();
        let coefficient_of_variation = stddev / mean;

        if coefficient_of_variation >= self.max_coefficient_of_variation {
            return None;
        }

        Some(Finding {
            line,
            message: format!(
                "{} sentences average {:.1} words each with very low length variation (CV {:.2}) — vary sentence length",
                sentences.len(),
                mean,
                coefficient_of_variation
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the default thresholds — the shorthand every non-`configure_*` test below
    /// uses instead of spelling out `SentenceLengthUniformityChecker::default()`.
    fn check_source(body: &str) -> Vec<Finding> {
        SentenceLengthUniformityChecker::default().check_source(body)
    }

    #[test]
    fn flags_uniform_sentence_lengths() {
        let body = "This is one sentence here. \
                     That is another sentence too. \
                     Here is a third sentence now. \
                     There goes a fourth sentence here. \
                     Now comes a fifth sentence too. \
                     Then follows a sixth sentence now. \
                     Finally the seventh sentence ends.\n";
        let findings = check_source(body);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("vary sentence length"));
    }

    #[test]
    fn allows_varied_sentence_lengths() {
        let body = "Short one. \
                     This one runs quite a bit longer than the others around it. \
                     Four words here. \
                     This sentence is also considerably longer than its short neighbors. \
                     Five words in this one. \
                     This one, too, stretches out across quite a few more words than usual.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn too_few_sentences_is_not_flagged() {
        let body = "This is one sentence here. \
                     That is another sentence too. \
                     Here is a third sentence now.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_list_items() {
        let body = "- This is one sentence here. That is another sentence too. Here is a third sentence now. There goes a fourth sentence here. Now comes a fifth sentence too. Then follows a sixth sentence now.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn configure_overrides_min_sentences() {
        let configured = SentenceLengthUniformityChecker::default()
            .configure(&serde_json::json!({ "min_sentences": 10 }))
            .unwrap()
            .unwrap();
        let body = "This is one sentence here. \
                     That is another sentence too. \
                     Here is a third sentence now. \
                     There goes a fourth sentence here. \
                     Now comes a fifth sentence too. \
                     Then follows a sixth sentence now. \
                     Finally the seventh sentence ends.\n";
        let ctx = CheckContext {
            source: body,
            tree: None,
        };
        // 7 sentences clears the default MIN_SENTENCES (6) but not the raised 10.
        assert!(
            configured
                .check(Path::new("doc.md"), &ctx)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn configure_rejects_malformed_options() {
        let err = SentenceLengthUniformityChecker::default()
            .configure(&serde_json::json!({ "min_sentences": "not-a-number" }));
        assert!(err.is_err());
    }
}
