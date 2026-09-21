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
pub fn for_each_paragraph(body: &str, mut on_paragraph: impl FnMut(Paragraph)) {
    let line_starts = line_start_offsets(body);
    let parser = Parser::new_ext(body, Options::empty());

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
