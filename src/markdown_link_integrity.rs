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
///   persistence (`Cache::save`) is only triggered when `changed_lines` is `None` — so
///   a diff-scoped per-edit hook call (the common case) never writes grace state back
///   to disk. In that configuration a still-failing violation stays Advisory on every
///   touch instead of escalating; it will never falsely block, but it also won't
///   self-escalate until the daemon is running or a non-diff-scoped (e.g. batch) check
///   runs against the file.
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

/// The three things a single pass over `Parser` events needs to collect: dangling-
/// reference findings discovered inline, every reference label actually used (to know
/// which definitions are dead), every in-doc anchor link's target slug, and rendered
/// heading text (to slugify into the anchor set those links are checked against).
struct ParsedLinks {
    findings: Vec<Finding>,
    used_labels: HashSet<String>,
    anchor_links: Vec<(usize, String)>,
    headings: Vec<String>,
}

/// Walks every `Parser` event once, dispatching to [`record_link_or_image`] for
/// link/image events and accumulating rendered heading text in between — split out of
/// `check_source` so that dispatch (a `match` per event, plus tracking the heading
/// currently being accumulated) doesn't nest inside the per-link/image logic that
/// [`record_link_or_image`] now owns.
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

/// Handles one `Link`/`Image` start event: records a dangling reference-style use as a
/// finding, tracks every reference label used (regardless of whether it's dangling, so
/// [`unused_definition_findings`] doesn't flag a definition backing only an image), and
/// records an inline anchor link's target slug for [`dead_anchor_findings`] to check.
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
/// nonexistent-target-file check for the cross-file case.
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
/// against a different anchor set (the local document's vs. a cross-file target's) and
/// with a different message.
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

/// One `[ref_id]: target` definition's dead-anchor/missing-file finding, if any —
/// `None` for a live target (or an `http(s)://` URL, which this checker never follows).
/// `target_cache` memoizes a target file's own heading-anchor set across every
/// definition pointing at it, since `dead_reference_target_findings` calls this once per
/// definition and re-reading/re-slugifying the same target file's headings on every one
/// pointing at it would be wasted work.
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
mod tests {
    use super::*;
    use std::fs;

    fn path() -> std::path::PathBuf {
        std::path::PathBuf::from("doc.md")
    }

    #[test]
    fn flags_used_but_never_defined() {
        let findings = check_source(&path(), "See [thing][missing] for details.\n").unwrap();
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("[missing] used but never defined")
        );
    }

    #[test]
    fn allows_defined_reference() {
        let body = "See [thing][ref] for details.\n\n[ref]: https://example.com\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_footnotes() {
        let body = "Note.[^1]\n\n[^1]: A footnote body.\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_image_references() {
        let body = "![alt][missing-image]\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_refs_inside_code_blocks() {
        let body = "```\n[thing][missing]\n```\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_refs_inside_inline_code() {
        let body = "Docs about markdown syntax:\n\n`[label][ref-id]` and `[ref-id]: target` are just examples.\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn image_ref_counts_as_used_for_unused_def_check() {
        let body = "![alt][pic]\n\n[pic]: https://example.com/img.png\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn shortcut_reference_counts_as_used_for_unused_def_check() {
        let body = "See [my-ref] for details.\n\n[my-ref]: https://example.com\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn collapsed_reference_counts_as_used() {
        let body = "Use [collapsed][].\n\n[collapsed]: https://example.com/b\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_dead_local_anchor() {
        let body = "# Real Heading\n\nSee [x][ref].\n\n[ref]: #no-such-heading\n";
        let findings = check_source(&path(), body).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("no such heading in this doc"));
    }

    #[test]
    fn allows_live_local_anchor() {
        let body = "# Real Heading\n\nSee [x][ref].\n\n[ref]: #real-heading\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_dead_inline_anchor_link() {
        let body = "# Real Heading\n\nSee [here](#nonexistent).\n";
        let findings = check_source(&path(), body).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("no such heading in this doc"));
    }

    #[test]
    fn flags_missing_target_file() {
        let dir = std::env::temp_dir().join(format!("kibitzer-md-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("doc.md");
        let body = "See [x][ref].\n\n[ref]: nonexistent.md#anchor\n";
        let findings = check_source(&doc, body).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("file does not exist"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flags_dead_cross_file_anchor() {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-md-test-cross-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.md");
        fs::write(&other, "# Other Heading\n").unwrap();
        let doc = dir.join("doc.md");
        let body = "See [x][ref].\n\n[ref]: other.md#missing\n";
        let findings = check_source(&doc, body).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("no such heading in other.md"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn allows_live_cross_file_anchor() {
        let dir =
            std::env::temp_dir().join(format!("kibitzer-md-test-cross-ok-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.md");
        fs::write(&other, "# Other Heading\n").unwrap();
        let doc = dir.join("doc.md");
        let body = "See [x][ref].\n\n[ref]: other.md#other-heading\n";
        let findings = check_source(&doc, body).unwrap();
        assert!(findings.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flags_unused_definition() {
        let body = "Nothing links here.\n\n[orphan]: https://example.com\n";
        let findings = check_source(&path(), body).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("[orphan] defined but never used")
        );
    }

    #[test]
    fn duplicate_headings_get_suffixed_anchors() {
        let body = "# Setup\n\n# Setup\n\nSee [x][ref].\n\n[ref]: #setup-1\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn reports_line_numbers() {
        let body = "line one\n\nSee [x][missing] here.\n";
        let findings = check_source(&path(), body).unwrap();
        assert_eq!(findings[0].line, 3);
    }

    // AC3: reference-label matching is case-insensitive and whitespace-normalized, on
    // both the use side and the definition side, regardless of which comes first.
    #[test]
    fn reference_matching_is_case_and_whitespace_insensitive_use_first() {
        let body = "See [it][Foo   Bar].\n\n[foo bar]: https://example.com\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    #[test]
    fn reference_matching_is_case_and_whitespace_insensitive_def_first() {
        let body = "[foo bar]: https://example.com\n\nSee [it][Foo   Bar].\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    // AC5: anchors are computed from rendered heading text, not raw markup — inline
    // code/emphasis/links inside a heading resolve to their visible text before
    // slugifying.
    #[test]
    fn anchor_computed_from_rendered_heading_text_with_inline_code() {
        let body = "## Using `fetch()`\n\n[link](#using-fetch)\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    #[test]
    fn anchor_computed_from_rendered_heading_text_with_emphasis_and_link() {
        let body = "## The *Bold* [Plan](https://example.com)\n\n[link](#the-bold-plan)\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    // GitHub's real slugger drops punctuation without collapsing the hyphen runs that
    // leaves behind — "A & B" removes `&` but keeps both surrounding spaces, so the
    // real GitHub anchor is `#a--b`, not `#a-b`.
    #[test]
    fn html_entity_headings_decode_before_slugifying() {
        let body = "## A &amp; B\n\n[link](#a--b)\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    #[test]
    fn headings_inside_details_blocks_are_recognized() {
        let body = "<details>\n<summary>More</summary>\n\n## Nested Heading\n\n</details>\n\n[link](#nested-heading)\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    #[test]
    fn empty_file_has_no_findings() {
        assert!(check_source(&path(), "").unwrap().is_empty());
    }

    #[test]
    fn malformed_reference_syntax_does_not_panic() {
        let body = "This has an [unclosed bracket and no matching close.\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_logseq_style_wiki_links() {
        let body = "See [[Some Page]] for details.\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    #[test]
    fn ignores_github_task_list_markers() {
        let body = "- [ ] todo item\n- [x] done item\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }

    #[test]
    fn fully_consistent_document_has_zero_findings() {
        let body = "# Heading One\n\nSee [the site][site-ref] and [Heading One](#heading-one).\n\n[site-ref]: https://example.com\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }
}
