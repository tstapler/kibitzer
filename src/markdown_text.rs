//! Shared helpers for native checkers that scan raw markdown prose (as opposed to
//! [`crate::markdown_link_integrity`]'s link/anchor graph): byte-offset-to-line-number
//! conversion, and walking flowing-prose paragraphs and headings (skipping list items) as
//! flattened text.

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

/// Returns `body` with a leading `---` ... `---`/`...` YAML front-matter block replaced by
/// spaces (newlines kept, so byte offsets and line numbers are unchanged). Only a block
/// that starts on line 1 and is closed counts; anything else is returned as-is.
fn mask_front_matter(body: &str) -> String {
    let mut lines = body.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return body.to_string();
    }
    let mut end = body.find('\n').map_or(body.len(), |i| i + 1);
    for line in lines {
        end += line.len();
        if matches!(line.trim_end(), "---" | "...") {
            let blanked: String = body[..end]
                .chars()
                .flat_map(|c| {
                    let pad = if c == '\n' { '\n' } else { ' ' };
                    std::iter::repeat_n(pad, c.len_utf8())
                })
                .collect();
            return blanked + &body[end..];
        }
    }
    body.to_string()
}

/// Which kind of flowing-prose block [`for_each_block`] reached.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Paragraph,
    Heading,
}

/// Walks paragraphs and headings (ATX and setext) outside any list, in document order,
/// calling `on_block(kind, line, flattened_text)`. List items are skipped: parallel
/// bullets are deliberate, not a prose defect.
///
/// `ENABLE_TABLES` keeps a GFM table from parsing as one giant paragraph. A leading YAML
/// front-matter block is blanked first (see [`mask_front_matter`]); pulldown-cmark's own
/// metadata option would also swallow text between mid-document `---` rules.
pub fn for_each_block(body: &str, mut on_block: impl FnMut(BlockKind, usize, &str)) {
    let masked = mask_front_matter(body);
    let body = masked.as_str();
    let line_starts = line_start_offsets(body);
    let parser = Parser::new_ext(body, Options::ENABLE_TABLES);

    let mut list_depth = 0usize;
    let mut current: Option<(BlockKind, usize)> = None;
    let mut text = String::new();

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Paragraph) if list_depth == 0 => {
                current = Some((
                    BlockKind::Paragraph,
                    line_for_offset(&line_starts, range.start),
                ));
                text.clear();
            }
            Event::Start(Tag::Heading { .. }) if list_depth == 0 => {
                current = Some((
                    BlockKind::Heading,
                    line_for_offset(&line_starts, range.start),
                ));
                text.clear();
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_)) => {
                if let Some((kind, line)) = current.take() {
                    on_block(kind, line, &text);
                }
            }
            Event::Text(ref t) | Event::Code(ref t) if current.is_some() => {
                text.push_str(t);
                text.push(' ');
            }
            Event::SoftBreak | Event::HardBreak if current.is_some() => text.push(' '),
            _ => {}
        }
    }
}

/// Walks `body`'s flowing-prose paragraphs outside any list; see [`for_each_block`].
pub fn for_each_paragraph(body: &str, mut on_paragraph: impl FnMut(Paragraph)) {
    for_each_block(body, |kind, line, text| {
        if kind == BlockKind::Paragraph {
            on_paragraph(Paragraph {
                line,
                text: text.to_string(),
            });
        }
    });
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

// Test names follow the validation plan's should_X_When_Y convention.
#[cfg(test)]
#[allow(non_snake_case)]
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

    fn headings(body: &str) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for_each_block(body, |kind, line, text| {
            if kind == BlockKind::Heading {
                let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push((line, collapsed));
            }
        });
        out
    }

    #[test]
    fn for_each_block_should_ReportSetextHeadingAtItsFirstLine_When_Underlined() {
        assert_eq!(
            headings("intro\n\nThe primitive\n=====\n"),
            vec![(3, "The primitive".to_string())]
        );
    }

    #[test]
    fn for_each_block_should_SkipHeading_When_InsideListItem() {
        assert!(headings("- # inside a list\n").is_empty());
    }

    #[test]
    fn for_each_block_should_FlattenInlineMarkup_When_HeadingHasEmphasisAndCode() {
        assert_eq!(
            headings("## The *primitive*, `x`, generically\n"),
            vec![(1, "The primitive , x , generically".to_string())]
        );
    }

    #[test]
    fn for_each_block_should_YieldDocumentOrder_When_HeadingsAndParagraphsInterleave() {
        let mut kinds = Vec::new();
        for_each_block("# H\n\npara\n\n## H2\n", |kind, _, _| kinds.push(kind));
        assert_eq!(
            kinds,
            vec![BlockKind::Heading, BlockKind::Paragraph, BlockKind::Heading]
        );
    }

    #[test]
    fn for_each_paragraph_should_KeepParagraphsBetweenRules_When_DashesAppearMidDocument() {
        let body =
            "Intro para.\n\n---\nSlide one text, here.\n\nSecond para.\n\n---\n\nTail para.\n";
        let mut paragraphs = Vec::new();
        for_each_paragraph(body, |p| paragraphs.push(p.text.trim().to_string()));
        assert_eq!(paragraphs.len(), 4, "got: {paragraphs:?}");
    }

    #[test]
    fn for_each_paragraph_should_SkipFrontMatter_When_FileStartsWithYaml() {
        let body = "---\ntitle: a. b.\n\nnote: c. d.\n---\n\nBody.\n";
        let mut paragraphs = Vec::new();
        for_each_paragraph(body, |p| {
            paragraphs.push((p.line, p.text.trim().to_string()))
        });
        assert_eq!(paragraphs, vec![(7, "Body.".to_string())]);
    }

    #[test]
    fn for_each_paragraph_should_KeepLineNumbers_When_FrontMatterHasNonAscii() {
        let body = "---\ntitle: caf\u{e9} \u{2014} x\n---\n\nBody here.\n";
        let mut lines = Vec::new();
        for_each_paragraph(body, |p| lines.push(p.line));
        assert_eq!(lines, vec![5]);
    }

    fn prose_findings(source: &str) -> Vec<(String, usize, String)> {
        ["repetitive-sentence-structure", "missing-paragraph-break"]
            .iter()
            .flat_map(|name| {
                crate::checker::run_checker_configured(
                    name,
                    std::path::Path::new("a.md"),
                    source,
                    None,
                )
                .unwrap()
                .into_iter()
                .map(move |f| (name.to_string(), f.line, f.message))
            })
            .collect()
    }

    #[test]
    fn prose_checks_should_BeIdentical_When_DirectiveHtmlCommentAdded() {
        let paragraph = "This is bad. This is worse. This is worst. It ends. It goes on. The rest follows. Another one here. And a last one.";
        let plain = format!("# Title\n\n{paragraph}\n");
        let baseline = prose_findings(&plain);
        assert!(
            baseline
                .iter()
                .any(|f| f.0 == "repetitive-sentence-structure"),
            "fixture must trigger the check: {baseline:?}"
        );
        let directive = "<!-- kibitzer:ignore missing-paragraph-break -- long on purpose -->";
        for variant in [
            format!("# Title\n\n{paragraph}\n{directive}\n"),
            format!("# Title\n\n{paragraph}\n\n{directive}\n"),
            format!("# Title\n\n{directive}\n{paragraph}\n"),
        ] {
            let with_comment = prose_findings(&variant);
            let strip_line = |v: &[(String, usize, String)]| {
                v.iter()
                    .map(|f| (f.0.clone(), f.2.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                strip_line(&with_comment),
                strip_line(&baseline),
                "{variant:?}"
            );
        }
    }
}
