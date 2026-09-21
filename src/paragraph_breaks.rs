use std::path::Path;

use anyhow::Result;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::markdown_text::{for_each_paragraph, split_sentences};

/// A sentence starting with one of these (case-insensitive) reads as a topic shift —
/// Purdue OWL's "On Paragraphs" guidance (start a new paragraph on a topic shift or when
/// the reader needs a pause).
const SHIFT_WORDS: &[&str] = &[
    "however",
    "separately",
    "meanwhile",
    "additionally",
    "alternatively",
    "in contrast",
    "on the other hand",
    "that said",
];

/// Sentence count above which an unbroken paragraph is long enough that a mid-paragraph
/// topic shift is worth flagging at all — short paragraphs with a "However" mid-sentence
/// are usually fine as one paragraph.
const LONG_PARAGRAPH_SENTENCES: usize = 6;

/// A cheap structural proxy for "this paragraph should probably be split in two":
/// flags a long paragraph (more sentences than [`LONG_PARAGRAPH_SENTENCES`]) that
/// contains a sentence starting with a contrastive/shift transition word after its
/// first sentence. "Topically distinct idea" is a semantic judgment an LLM would need
/// to make precisely — this only catches the mechanical proxy for it, and has a much
/// higher false-positive rate than [`crate::repetitive_sentences`], so it's wired in as
/// Advisory-only in `config::default_checks`, never Blocking.
pub struct ParagraphBreaksChecker;

impl Checker for ParagraphBreaksChecker {
    fn name(&self) -> &str {
        "missing-paragraph-break"
    }

    fn description(&self) -> &str {
        "flags a long paragraph containing a mid-paragraph topic-shift transition"
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
    crate::checker::CheckerFactory(|| vec![Box::new(ParagraphBreaksChecker)])
}

pub fn check_source(body: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for_each_paragraph(body, |p| {
        if let Some(finding) = check_paragraph(p.line, &p.text) {
            findings.push(finding);
        }
    });
    findings
}

fn check_paragraph(line: usize, text: &str) -> Option<Finding> {
    let sentences: Vec<&str> = split_sentences(text).into_iter().map(str::trim).collect();
    if sentences.len() <= LONG_PARAGRAPH_SENTENCES {
        return None;
    }
    let (idx, word) = sentences
        .iter()
        .enumerate()
        .skip(1)
        .find_map(|(i, s)| shift_word(s).map(|w| (i, w)))?;
    Some(Finding {
        line,
        message: format!(
            "long paragraph ({} sentences) shifts topic at sentence {} (\"{word}\") — consider a paragraph break",
            sentences.len(),
            idx + 1
        ),
    })
}

fn shift_word(sentence: &str) -> Option<&'static str> {
    let lower = sentence.to_lowercase();
    SHIFT_WORDS.iter().find(|w| lower.starts_with(*w)).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_long_paragraph_with_shift() {
        let body = "One. Two. Three. Four. Five. Six. However, seven changes topic.\n";
        let findings = check_source(body);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("however"));
    }

    #[test]
    fn allows_short_paragraph_with_shift() {
        let body = "One. Two. However, three changes topic.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn allows_long_paragraph_without_shift() {
        let body = "One. Two. Three. Four. Five. Six. Seven stays on topic.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn ignores_shift_word_in_first_sentence() {
        let body = "However this opens. Two. Three. Four. Five. Six. Seven.\n";
        assert!(check_source(body).is_empty());
    }

    #[test]
    fn version_numbers_do_not_inflate_the_sentence_count() {
        // 4 real sentences — the naive `.`-split this replaced would have split "v1.10",
        // "v1.11", and "v1.12" into 3 extra fake sentences, pushing this paragraph over
        // LONG_PARAGRAPH_SENTENCES and misreporting its length.
        let body = "The client shipped in v1.10, v1.11, and v1.12. The team maintains it. \
                     However, it still gets bug fixes. This remains true today.\n";
        assert!(check_source(body).is_empty());
    }
}
