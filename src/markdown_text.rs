//! Shared helpers for native checkers that scan raw markdown prose (as opposed to
//! [`crate::markdown_link_integrity`]'s link/anchor graph): byte-offset-to-line-number
//! conversion, and walking flowing-prose paragraphs (skipping list items) as flattened
//! text.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::checker::Finding;

pub fn line_start_offsets(src: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

pub fn line_for_offset(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(i) => i + 1,
        Err(i) => i,
    }
}

/// One flowing-prose paragraph (not inside a list item): its 1-indexed start line and
/// its text flattened from `Text`/`Code` events, with soft/hard breaks collapsed to a
/// single space.
pub struct Paragraph {
    pub line: usize,
    pub text: String,
}

/// Splits `text` into sentence-like chunks on `.`/`!`/`?`, except a `.` sitting between
/// two digits (`v1.13`, `10.5`) — treated as a decimal/version number, not a sentence
/// boundary. Found necessary backtesting against kubernetes/website: naive splitting on
/// every `.` turned "etcd client is included in Kubernetes v1.13. Previously..." into a
/// fake extra "13." sentence, inflating a paragraph's reported sentence count and
/// shifting where a topic-shift word appeared to land. Not a real sentence tokenizer (no
/// abbreviation list, no quote/parenthetical handling) — good enough for the mechanical
/// structural checks that use it.
pub fn split_sentences(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if !matches!(bytes[i], b'.' | b'!' | b'?') {
            i += 1;
            continue;
        }
        let is_decimal_point = bytes[i] == b'.'
            && i > 0
            && bytes[i - 1].is_ascii_digit()
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit();
        if is_decimal_point {
            i += 1;
            continue;
        }
        let mut end = i + 1;
        while end < bytes.len() && matches!(bytes[end], b'.' | b'!' | b'?') {
            end += 1;
        }
        push_if_not_blank(&mut sentences, &text[start..end]);
        start = end;
        i = end;
    }
    push_if_not_blank(&mut sentences, &text[start..]);
    sentences
}

fn push_if_not_blank<'a>(sentences: &mut Vec<&'a str>, candidate: &'a str) {
    if !candidate.trim().is_empty() {
        sentences.push(candidate);
    }
}

/// Walks `body`'s markdown paragraphs outside any list, invoking `on_paragraph` for
/// each one. List items are excluded because a parallel bulleted enumeration (independent
/// facts stated in the same grammatical form on purpose) is not the kind of prose defect
/// these checkers look for — see the originating issue's false-positive guardrail.
///
/// Parses with `ENABLE_TABLES`: without it, a GFM table has no blank-line separation
/// pulldown-cmark recognizes, so the whole table parses as one giant `Tag::Paragraph`
/// (found backtesting vscode-docs: a troubleshooting table reported as one 30-"sentence"
/// paragraph). With it, table content arrives as `Tag::Table`/`TableCell` events instead,
/// which `in_paragraph`'s gate below already excludes.
pub fn for_each_paragraph(body: &str, mut on_paragraph: impl FnMut(Paragraph)) {
    let line_starts = line_start_offsets(body);
    let parser = Parser::new_ext(body, Options::ENABLE_TABLES);

    let mut list_depth = 0usize;
    let mut in_paragraph = false;
    let mut paragraph_line = 0usize;
    let mut paragraph_text = String::new();

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Paragraph) if list_depth == 0 => {
                in_paragraph = true;
                paragraph_line = line_for_offset(&line_starts, range.start);
                paragraph_text.clear();
            }
            Event::End(TagEnd::Paragraph) if in_paragraph => {
                in_paragraph = false;
                on_paragraph(Paragraph {
                    line: paragraph_line,
                    text: std::mem::take(&mut paragraph_text),
                });
            }
            Event::Text(ref text) | Event::Code(ref text) if in_paragraph => {
                paragraph_text.push_str(text);
                paragraph_text.push(' ');
            }
            Event::SoftBreak | Event::HardBreak if in_paragraph => {
                paragraph_text.push(' ');
            }
            _ => {}
        }
    }
}

/// Runs `check_paragraph` over every flowing-prose paragraph in `body` (via
/// [`for_each_paragraph`]) and collects whatever findings it returns. Every native prose
/// checker in this crate (`repetitive_sentences`, `paragraph_breaks`,
/// `ai_vocabulary_density`, `filler_phrase_density`, `formulaic_ai_openers`,
/// `sentence_length_uniformity`, `em_dash_overuse`) had converged on this exact
/// `check_source` wrapper independently — kibitzer's own `duplicate-code-cross-file`
/// checker flagged the repetition, so it's a shared helper instead of a seventh copy.
pub fn check_paragraphs(
    body: &str,
    mut check_paragraph: impl FnMut(usize, &str) -> Option<Finding>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for_each_paragraph(body, |p| {
        if let Some(finding) = check_paragraph(p.line, &p.text) {
            findings.push(finding);
        }
    });
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_number_dot_is_not_a_sentence_boundary() {
        let sentences = split_sentences("The client shipped in v1.13. It works well.");
        assert_eq!(
            sentences,
            vec!["The client shipped in v1.13.", " It works well."]
        );
    }

    #[test]
    fn trailing_punctuation_run_is_kept_together() {
        let sentences = split_sentences("Wait, really?! Yes.");
        assert_eq!(sentences, vec!["Wait, really?!", " Yes."]);
    }

    #[test]
    fn a_table_is_not_treated_as_one_giant_paragraph() {
        let body = "| Problem | Cause |\n\
                     | --- | --- |\n\
                     | One. Two. Three. | Four. Five. Six. |\n\
                     | Seven. Eight. Nine. | Ten. Eleven. Twelve. |\n";
        let mut paragraphs = Vec::new();
        for_each_paragraph(body, |p| paragraphs.push(p.text));
        assert!(
            paragraphs.is_empty(),
            "table cells should never surface as paragraphs, got: {paragraphs:?}"
        );
    }
}
