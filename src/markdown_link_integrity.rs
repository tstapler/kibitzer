use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};

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
    let parser = Parser::new_with_broken_link_callback(body, Options::empty(), Some(callback));

    let ref_defs: HashMap<String, (String, usize)> = parser
        .reference_definitions()
        .iter()
        .map(|(label, def)| {
            (
                normalize_label(label),
                (
                    def.dest.to_string(),
                    line_for_offset(&line_starts, def.span.start),
                ),
            )
        })
        .collect();

    let mut used_labels: HashSet<String> = HashSet::new();
    let mut findings: Vec<Finding> = Vec::new();
    let mut anchor_links: Vec<(usize, String)> = Vec::new();
    let mut headings: Vec<String> = Vec::new();
    let mut current_heading: Option<String> = None;

    for (event, range) in parser.into_offset_iter() {
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
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                id,
                ..
            })
            | Event::Start(Tag::Image {
                link_type,
                dest_url,
                id,
                ..
            }) => {
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
                    LinkType::ReferenceUnknown
                        | LinkType::CollapsedUnknown
                        | LinkType::ShortcutUnknown
                );
                if is_reference_style {
                    let normalized = normalize_label(&id);
                    used_labels.insert(normalized.clone());
                    if is_dangling && !normalized.starts_with('^') {
                        findings.push(Finding {
                            line: line_for_offset(&line_starts, range.start),
                            message: format!("[{id}] used but never defined"),
                        });
                    }
                }
                // Only inline-style links/images (`[text](#slug)`) are checked here.
                // A reference-style link's `dest_url` resolves to its definition's
                // target, which the `def_entries` pass below already checks once at the
                // definition's own line — checking it again here would double-report
                // the same dead anchor at both the use site and the definition site.
                if let (LinkType::Inline, Some(slug)) = (link_type, dest_url.strip_prefix('#')) {
                    anchor_links
                        .push((line_for_offset(&line_starts, range.start), slug.to_string()));
                }
            }
            _ => {}
        }
    }

    let mut unused_ids: Vec<&String> = ref_defs
        .keys()
        .filter(|id| !used_labels.contains(*id))
        .collect();
    unused_ids.sort();
    for ref_id in unused_ids {
        findings.push(Finding {
            line: ref_defs[ref_id].1,
            message: format!("[{ref_id}] defined but never used"),
        });
    }

    let local_anchors = heading_slugs(&headings);

    for (line, slug) in &anchor_links {
        if !local_anchors.contains(slug) {
            findings.push(Finding {
                line: *line,
                message: format!("#{slug} -> no such heading in this doc"),
            });
        }
    }

    // Reference definitions pointing at a heading anchor (`[ref]: #slug` or
    // `[ref]: other.md#slug`) get the same dead-anchor check as an in-doc link, plus a
    // nonexistent-target-file check for the cross-file case.
    let mut target_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    let mut def_entries: Vec<(&String, &(String, usize))> = ref_defs.iter().collect();
    def_entries.sort_by_key(|(id, _)| id.as_str());
    for (ref_id, (target, line)) in def_entries {
        if target.starts_with("http://") || target.starts_with("https://") {
            continue;
        }
        let (file_part, frag) = match target.split_once('#') {
            Some((f, fr)) => (f, Some(fr)),
            None => (target.as_str(), None),
        };
        if file_part.is_empty() {
            if let Some(frag) = frag
                && !frag.is_empty()
                && !local_anchors.contains(frag)
            {
                findings.push(Finding {
                    line: *line,
                    message: format!("[{ref_id}]: #{frag} -> no such heading in this doc"),
                });
            }
            continue;
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
            None => findings.push(Finding {
                line: *line,
                message: format!("[{ref_id}]: {file_part} -> file does not exist"),
            }),
            Some(target_anchors) => {
                if let Some(frag) = frag
                    && !frag.is_empty()
                    && !target_anchors.contains(frag)
                {
                    findings.push(Finding {
                        line: *line,
                        message: format!("[{ref_id}]: {target} -> no such heading in {file_part}"),
                    });
                }
            }
        }
    }

    findings.sort_by_key(|f| f.line);
    findings.dedup();
    Ok(findings)
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
    fn flags_dangling_image_reference() {
        let body = "![alt][missing-image]\n";
        let findings = check_source(&path(), body).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("[missing-image] used but never defined")
        );
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
    fn fully_consistent_document_has_zero_findings() {
        let body = "# Heading One\n\nSee [the site][site-ref] and [Heading One](#heading-one).\n\n[site-ref]: https://example.com\n";
        assert!(check_source(&path(), body).unwrap().is_empty());
    }
}
