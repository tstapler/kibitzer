//! Shared helpers for native checkers that scan raw markdown prose (as opposed to
//! [`crate::markdown_link_integrity`]'s link/anchor graph): byte-offset-to-line-number
//! conversion, and walking flowing-prose paragraphs (skipping list items) as flattened
//! text.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

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

#[cfg(test)]
mod tests {
    use super::*;

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
