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
pub struct SentenceLengthUniformityChecker;

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
        Ok(check_source(ctx.source))
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(SentenceLengthUniformityChecker)])
}

pub fn check_source(body: &str) -> Vec<Finding> {
    crate::markdown_text::check_paragraphs(body, check_paragraph)
}

/// One paragraph's worth of flattened text, checked for a run of sentences whose word
/// counts vary too little to look human-written. Requires at least [`MIN_SENTENCES`]
/// sentences and a mean length of at least [`MIN_MEAN_WORDS`] words before judging
/// variation at all, since short paragraphs and terse fragments are noisy on their own.
fn check_paragraph(line: usize, text: &str) -> Option<Finding> {
    let sentences = split_sentences(text);
    if sentences.len() < MIN_SENTENCES {
        return None;
    }

    let word_counts: Vec<f64> = sentences
        .iter()
        .map(|s| s.split_whitespace().count() as f64)
        .collect();

    let mean = word_counts.iter().sum::<f64>() / word_counts.len() as f64;
    if mean < MIN_MEAN_WORDS {
        return None;
    }

    let variance =
        word_counts.iter().map(|w| (w - mean).powi(2)).sum::<f64>() / word_counts.len() as f64;
    let stddev = variance.sqrt();
    let coefficient_of_variation = stddev / mean;

    if coefficient_of_variation >= MAX_COEFFICIENT_OF_VARIATION {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
