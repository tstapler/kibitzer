use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use pulldown_cmark::{BrokenLinkCallback, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::checker::{CheckContext, Checker, Finding, Language};

/// Checks a markdown document for three shapes of link/anchor breakage:
///   - a reference-style use (`[label][ref-id]`) with no matching `[ref-id]: target`
///     definition anywhere in the document
///   - a reference definition with zero uses
///   - a dead heading anchor, either an in-doc link (`[text](#slug)`) or a reference
///     definition pointing at `#slug` or `other.md#slug`
///
/// Parses with `pulldown-cmark` rather than regex, so reference-label matching gets
/// CommonMark's case-insensitive/whitespace-normalized semantics for free, and heading
/// anchors are slugified from the *rendered* heading text (inline code/emphasis/links
/// resolved to their visible text) rather than raw markup. Content inside fenced code
/// blocks or inline code spans is never treated as a real link or definition, because
/// pulldown-cmark's tokenizer does not parse link syntax there.
///
/// ## Multi-step edits: reference used before its definition exists
///
/// An agent adding a reference-style link (`[label][ref-id]`) and its `[ref-id]:
/// target` definition as two separate edits will trip this check between the two
/// edits — this is a real, expected transient state, not a false positive to special-
/// case away in this checker. It's handled generically by [`crate::cache::Cache::apply_grace`]:
/// under a live per-edit trigger (anything other than `"batch"`), the first time a
/// given `(file, check)` pair fails, the result is downgraded from Blocking to
/// Advisory with a "first occurrence this edit sequence" message; it only escalates
/// back to Blocking if the *same* file is still failing the check on a later touch.
///
/// This grace state lives in the check-runner's `Cache`, keyed by `{file}::{check
/// name}`. It survives across edits only for as long as that `Cache` instance does:
///
/// - **With a `kibitzer daemon` running** (the normal setup — see `README.md`), every
///   `hook`/`run` call is served by the same long-lived `Arc<Mutex<Cache>>`, so grace
///   state persists correctly across edits regardless of diff-scoping. Verified via
///   `kibitzer daemon start` plus two successive `kibitzer hook` calls against the same
///   still-failing file: first call → Advisory/exit 0 ("first occurrence..."), second
///   call → Blocking/exit 2.
/// - **Without a daemon** (`run_checks_smart`'s fallback path in `src/daemon.rs`), each
///   `hook` invocation is a fresh process that reloads `Cache` from disk, and disk
///   persistence (`Cache::save`) only triggers when `changed_lines` is `None` — so a
///   diff-scoped per-edit call never writes grace state back to disk. A still-failing
///   violation then stays Advisory forever on that path; it never falsely blocks, but
///   also never self-escalates until the daemon runs or a batch check touches the file.
pub struct MarkdownLinkIntegrityChecker;

impl Checker for MarkdownLinkIntegrityChecker {
    fn name(&self) -> &str {
        "markdown-link-integrity"
    }

    fn description(&self) -> &str {
        "flags broken markdown reference-style links and dead heading anchors"
    }

    fn language(&self) -> Option<Language> {
        // Not tree-sitter-based — scans the raw markdown source directly.
        None
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.md"]
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        check_source(file, ctx.source)
    }
}

pub fn check_source(path: &Path, body: &str) -> Result<Vec<Finding>> {
    let line_starts = line_start_offsets(body);

    // Force the parser to still emit Link/Image events for dangling references (as
    // *Unknown link types) instead of silently rendering them as plain text, so a single
    // pass over events can detect both uses and dangling uses.
    let callback =
        |_broken: pulldown_cmark::BrokenLink| Some((CowStr::Borrowed(""), CowStr::Borrowed("")));
    // ENABLE_TASKLISTS: without it, a GFM task-list marker at the start of a list item
    // (`- [x] done`) isn't recognized as a `TaskListMarker` and falls back to generic
    // link syntax instead — `[x]` parses as a dangling shortcut-reference link (`[ ]`,
    // being whitespace-only, isn't a valid CommonMark link label and stays literal text,
    // so only the checked-box form was actually affected).
    let parser =
        Parser::new_with_broken_link_callback(body, Options::ENABLE_TASKLISTS, Some(callback));

    let ref_defs = build_ref_defs(&parser, &line_starts);
    let parsed = collect_parsed_links(parser, body, &line_starts);
    let local_anchors = heading_slugs(&parsed.headings);

    let mut findings = parsed.findings;
    findings.extend(unused_definition_findings(&ref_defs, &parsed.used_labels));
    findings.extend(dead_anchor_findings(&parsed.anchor_links, &local_anchors));
    findings.extend(dead_reference_target_findings(
        path,
        &ref_defs,
        &local_anchors,
    ));

    findings.sort_by_key(|f| f.line);
    findings.dedup();
    Ok(findings)
}

/// Every `[ref_id]: target` definition in the document, keyed by normalized label, with
/// its target and definition line — the lookup table [`unused_definition_findings`] and
/// [`dead_reference_target_findings`] both check reference-style uses/targets against.
fn build_ref_defs<'a, F: BrokenLinkCallback<'a>>(
    parser: &Parser<'a, F>,
    line_starts: &[usize],
) -> HashMap<String, (String, usize)> {
    parser
        .reference_definitions()
        .iter()
        .map(|(label, def)| {
            (
                normalize_label(label),
                (
                    def.dest.to_string(),
                    line_for_offset(line_starts, def.span.start),
                ),
            )
        })
        .collect()
}

/// What a single pass over `Parser` events collects: dangling-reference findings, every
/// reference label used (to know which definitions are dead), in-doc anchor targets, and
/// rendered heading text (to slugify into the anchor set those targets are checked against).
struct ParsedLinks {
    findings: Vec<Finding>,
    used_labels: HashSet<String>,
    anchor_links: Vec<(usize, String)>,
    headings: Vec<String>,
}

/// Walks every `Parser` event once: accumulates rendered heading text, and dispatches
/// link/image events to [`record_link_or_image`] (kept separate so its own nesting
/// doesn't stack on top of this event-dispatch loop's).
fn collect_parsed_links<'a, F: BrokenLinkCallback<'a>>(
    parser: Parser<'a, F>,
    body: &str,
    line_starts: &[usize],
) -> ParsedLinks {
    let mut parsed = ParsedLinks {
        findings: Vec::new(),
        used_labels: HashSet::new(),
        anchor_links: Vec::new(),
        headings: Vec::new(),
    };
    let mut current_heading: Option<String> = None;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { .. }) => current_heading = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => {
                if let Some(text) = current_heading.take() {
                    parsed.headings.push(text);
                }
            }
            Event::Text(ref text) | Event::Code(ref text) => {
                if let Some(heading) = current_heading.as_mut() {
                    heading.push_str(text);
                }
            }
            Event::Start(tag @ Tag::Link { .. }) | Event::Start(tag @ Tag::Image { .. }) => {
                record_link_or_image(&mut parsed, body, line_starts, tag, range);
            }
            _ => {}
        }
    }
    parsed
}

/// A `Tag::Link`/`Tag::Image` are structurally identical apart from the variant tag
/// itself, so every field this checker cares about is extracted the same way regardless
/// of which one `tag` is — the caller already tells them apart via `is_image`.
fn link_tag_fields(tag: Tag<'_>) -> (LinkType, CowStr<'_>, CowStr<'_>) {
    match tag {
        Tag::Link {
            link_type,
            dest_url,
            id,
            ..
        }
        | Tag::Image {
            link_type,
            dest_url,
            id,
            ..
        } => (link_type, dest_url, id),
        _ => unreachable!(),
    }
}

/// Whether `link_type` is reference-style at all (`[label][ref]`/`[label][]`/
/// `[label]`, as opposed to inline `[text](url)`), and — if so — whether it's dangling
/// (no matching `[ref]: target` definition exists anywhere in the document).
fn reference_style_flags(link_type: LinkType) -> (bool, bool) {
    let is_reference_style = matches!(
        link_type,
        LinkType::Reference
            | LinkType::ReferenceUnknown
            | LinkType::Collapsed
            | LinkType::CollapsedUnknown
            | LinkType::Shortcut
            | LinkType::ShortcutUnknown
    );
    let is_dangling = matches!(
        link_type,
        LinkType::ReferenceUnknown | LinkType::CollapsedUnknown | LinkType::ShortcutUnknown
    );
    (is_reference_style, is_dangling)
}

/// Handles one `Link`/`Image` start event: records a dangling reference-style use as a
/// finding, tracks every reference label used (regardless of whether it's dangling, so
/// [`unused_definition_findings`] doesn't flag a definition backing only an image), and
/// records an inline anchor link's target slug for [`dead_anchor_findings`] to check.
fn record_link_or_image(
    parsed: &mut ParsedLinks,
    body: &str,
    line_starts: &[usize],
    tag: Tag,
    range: std::ops::Range<usize>,
) {
    let is_image = matches!(tag, Tag::Image { .. });
    let (link_type, dest_url, id) = link_tag_fields(tag);
    let (is_reference_style, is_dangling) = reference_style_flags(link_type);
    if is_reference_style {
        let normalized = normalize_label(&id);
        parsed.used_labels.insert(normalized.clone());
        // A dangling *image* reference (missing asset via `![alt][ref]`) is a different,
        // unrelated failure mode from a broken doc cross-reference — matching the prior
        // doc_report.py-based checker, which never flagged these. It still counts as a
        // "use" above so a definition backing only an image isn't flagged unused.
        if is_dangling
            && !is_image
            && !normalized.starts_with('^')
            && !is_wiki_link_bracket(body, &range)
        {
            parsed.findings.push(Finding {
                line: line_for_offset(line_starts, range.start),
                message: format!("[{id}] used but never defined"),
            });
        }
    }
    // Only inline-style links/images (`[text](#slug)`) are checked here. A
    // reference-style link's `dest_url` resolves to its definition's target, which
    // `dead_reference_target_findings` already checks once at the definition's own
    // line — checking it again here would double-report the same dead anchor at both
    // the use site and the definition site.
    if let (LinkType::Inline, Some(slug)) = (link_type, dest_url.strip_prefix('#')) {
        parsed
            .anchor_links
            .push((line_for_offset(line_starts, range.start), slug.to_string()));
    }
}

/// CommonMark treats nested brackets as balanced text, so a Logseq-style wiki-link
/// `[[Page Name]]` parses as literal `[` + a shortcut reference link `[Page Name]` +
/// literal `]` — never as a single token. That inner shortcut is always dangling (no
/// `[Page Name]: url` definition exists), which would otherwise read as a broken
/// markdown reference. Detecting the surrounding literal brackets in the source text
/// is what tells the two apart.
fn is_wiki_link_bracket(body: &str, range: &std::ops::Range<usize>) -> bool {
    range.start > 0
        && body.as_bytes().get(range.start - 1) == Some(&b'[')
        && body.as_bytes().get(range.end) == Some(&b']')
}

/// A `[ref_id]: target` definition whose label matches no reference-style link/image
/// anywhere in the document.
fn unused_definition_findings(
    ref_defs: &HashMap<String, (String, usize)>,
    used_labels: &HashSet<String>,
) -> Vec<Finding> {
    let mut unused_ids: Vec<&String> = ref_defs
        .keys()
        .filter(|id| !used_labels.contains(*id))
        .collect();
    unused_ids.sort();
    unused_ids
        .into_iter()
        .map(|ref_id| Finding {
            line: ref_defs[ref_id].1,
            message: format!("[{ref_id}] defined but never used"),
        })
        .collect()
}

/// An inline anchor link (`[text](#slug)`) whose target slug matches no heading in this
/// document.
fn dead_anchor_findings(
    anchor_links: &[(usize, String)],
    local_anchors: &HashSet<String>,
) -> Vec<Finding> {
    anchor_links
        .iter()
        .filter(|(_, slug)| !local_anchors.contains(slug))
        .map(|(line, slug)| Finding {
            line: *line,
            message: format!("#{slug} -> no such heading in this doc"),
        })
        .collect()
}

/// Reference definitions pointing at a heading anchor (`[ref]: #slug` or
/// `[ref]: other.md#slug`) get the same dead-anchor check as an in-doc link, plus a
/// missing-target-file check for the cross-file case.
fn dead_reference_target_findings(
    path: &Path,
    ref_defs: &HashMap<String, (String, usize)>,
    local_anchors: &HashSet<String>,
) -> Vec<Finding> {
    let mut target_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    let mut def_entries: Vec<(&String, &(String, usize))> = ref_defs.iter().collect();
    def_entries.sort_by_key(|(id, _)| id.as_str());
    def_entries
        .into_iter()
        .filter_map(|(ref_id, (target, line))| {
            reference_target_finding(
                path,
                ref_id,
                (target, *line),
                local_anchors,
                &mut target_cache,
            )
        })
        .collect()
}

/// A `#frag` fragment finding, if `frag` is present, non-empty, and missing from
/// `anchors` — the shape both branches of [`reference_target_finding`] check, just
/// against a different anchor set and message.
fn fragment_finding(
    anchors: &HashSet<String>,
    frag: Option<&str>,
    line: usize,
    message: impl FnOnce(&str) -> String,
) -> Option<Finding> {
    match frag {
        Some(frag) if !frag.is_empty() && !anchors.contains(frag) => Some(Finding {
            line,
            message: message(frag),
        }),
        _ => None,
    }
}

/// One `[ref_id]: target` definition's dead-anchor/missing-file finding, if any — `None`
/// for a live target (or an `http(s)://` URL, never followed). `target_cache` memoizes a
/// target file's heading-anchor set across every definition pointing at it, so re-reading
/// and re-slugifying it isn't repeated per definition.
fn reference_target_finding(
    path: &Path,
    ref_id: &str,
    (target, line): (&str, usize),
    local_anchors: &HashSet<String>,
    target_cache: &mut HashMap<String, Option<HashSet<String>>>,
) -> Option<Finding> {
    if target.starts_with("http://") || target.starts_with("https://") {
        return None;
    }
    let (file_part, frag) = match target.split_once('#') {
        Some((f, fr)) => (f, Some(fr)),
        None => (target, None),
    };
    if file_part.is_empty() {
        return fragment_finding(local_anchors, frag, line, |frag| {
            format!("[{ref_id}]: #{frag} -> no such heading in this doc")
        });
    }

    let target_path = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(file_part);
    let target_anchors = target_cache
        .entry(file_part.to_string())
        .or_insert_with(|| {
            std::fs::read_to_string(&target_path)
                .ok()
                .map(|s| heading_slugs(&extract_headings(&s)))
        });
    match target_anchors {
        None => Some(Finding {
            line,
            message: format!("[{ref_id}]: {file_part} -> file does not exist"),
        }),
        Some(target_anchors) => fragment_finding(target_anchors, frag, line, |_frag| {
            format!("[{ref_id}]: {target} -> no such heading in {file_part}")
        }),
    }
}

/// CommonMark reference-label matching is case-insensitive and collapses internal
/// whitespace; pulldown-cmark applies this when resolving definitions, but the raw label
/// text it hands back through events/`reference_definitions()` is not folded, so
/// use/definition bookkeeping here re-normalizes it the same way.
fn normalize_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn line_start_offsets(src: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

fn line_for_offset(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(i) => i + 1,
        Err(i) => i,
    }
}

/// Rendered heading text for every heading in a document, extracted the same way
/// `check_source`'s main pass does — used for cross-file anchor targets, which only need
/// the anchor set of the *other* file, not its links/references.
fn extract_headings(body: &str) -> Vec<String> {
    let parser = Parser::new(body);
    let mut headings = Vec::new();
    let mut current_heading: Option<String> = None;
    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => current_heading = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => {
                if let Some(text) = current_heading.take() {
                    headings.push(text);
                }
            }
            Event::Text(ref text) | Event::Code(ref text) => {
                if let Some(heading) = current_heading.as_mut() {
                    heading.push_str(text);
                }
            }
            _ => {}
        }
    }
    headings
}

/// GitHub-style heading-anchor slugification, with GitHub's real duplicate-heading
/// suffixing: the first occurrence of a slug is unsuffixed, later ones get `-1`, `-2`,
/// ... Operates on rendered heading text (inline code/emphasis/links already resolved to
/// their visible text by the caller), not raw markup.
fn heading_slugs(headings: &[String]) -> HashSet<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut slugs = HashSet::new();
    for heading in headings {
        let base = github_slug(heading);
        let count = counts.entry(base.clone()).or_insert(0);
        let slug = if *count == 0 {
            base
        } else {
            format!("{base}-{count}")
        };
        *count += 1;
        slugs.insert(slug);
    }
    slugs
}

/// Lowercase, decode HTML entities, drop anything that isn't a letter/digit/space/
/// hyphen/underscore, then turn whitespace into hyphens without collapsing runs — a
/// stripped em-dash can leave two spaces that must anchor as `--` to match GitHub's real
/// slugger.
fn github_slug(heading: &str) -> String {
    let decoded = decode_html_entities(heading);
    let mut slug = String::with_capacity(decoded.len());
    for ch in decoded.chars() {
        if ch.is_whitespace() {
            slug.push('-');
        } else if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            slug.extend(ch.to_lowercase());
        }
        // all other punctuation is dropped entirely
    }
    slug
}

fn decode_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
#[path = "markdown_link_integrity_tests.rs"]
mod tests;
