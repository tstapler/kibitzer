use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;

use crate::checker::{Checker, Finding};

/// Checks reference-style markdown links: `[label][ref-id]` uses with no matching
/// `[ref-id]: target` definition, and definitions whose target is a dead heading anchor
/// (same-doc `#frag` or `other.md#frag`) or a nonexistent file. Ported from
/// `doc_report.py`'s `check_references` — deliberately scoped to link *integrity* only
/// (a definition pointing nowhere is a factual error), not that script's advisory
/// structure/readability checks.
pub fn check_file(path: &Path) -> Result<Vec<Finding>> {
    let src =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    check_source(path, &src)
}

pub struct MarkdownLinkIntegrityChecker;

impl Checker for MarkdownLinkIntegrityChecker {
    fn check_file(&self, path: &Path) -> Result<Vec<Finding>> {
        check_file(path)
    }
}

pub fn check_source(path: &Path, body: &str) -> Result<Vec<Finding>> {
    let scan_body = strip_code(body);
    let mut findings = Vec::new();

    let ref_use = Regex::new(r"\[([^\]]+)\]\[([^\]]*)\]").unwrap();
    // Refs used by a non-image link — these are the only ones that can be reported
    // "used but never defined" (mirrors the Python original's negative-lookbehind-for-`!`
    // exclusion of image refs from that check).
    let mut used: HashMap<String, usize> = HashMap::new();
    // Every ref used at all, image or not — a definition referenced only by `![alt][ref]`
    // is still used, just not eligible for the undefined-ref check above.
    let mut used_any: HashMap<String, usize> = HashMap::new();
    for caps in ref_use.captures_iter(&scan_body) {
        let whole = caps.get(0).unwrap();
        // Negative-lookbehind-for-`!` (image refs) by hand: the `regex` crate has no
        // look-around support.
        let is_image = whole.start() > 0 && scan_body.as_bytes()[whole.start() - 1] == b'!';
        let label = caps[1].trim();
        let ref_id = caps[2].trim();
        let id = if ref_id.is_empty() { label } else { ref_id };
        if id.starts_with('^') {
            continue; // footnote, not a reference link
        }
        let line = line_of(&scan_body, whole.start());
        used_any.entry(id.to_string()).or_insert(line);
        if !is_image {
            used.entry(id.to_string()).or_insert(line);
        }
    }

    // A shortcut reference `[ref-id]` (label doubles as the id, no trailing `[...]`) is
    // valid CommonMark but invisible to `ref_use` above. It only matters for the
    // "defined but never used" check: the Python original never validates shortcut refs
    // either, so `used` (the undefined-ref check) stays as-is to match it.
    let shortcut_ref = Regex::new(r"\[([^\]\[]+)\]").unwrap();
    for caps in shortcut_ref.captures_iter(&scan_body) {
        let whole = caps.get(0).unwrap();
        let is_image = whole.start() > 0 && scan_body.as_bytes()[whole.start() - 1] == b'!';
        // A full reference `[text][ref]` or a definition `[ref]: target` both start with
        // `[...]` followed by `[` or `:` — skip those, they're handled elsewhere.
        let next_byte = scan_body.as_bytes().get(whole.end()).copied();
        if is_image || matches!(next_byte, Some(b'[') | Some(b'(') | Some(b':')) {
            continue;
        }
        let id = caps[1].trim();
        if id.is_empty() || id.starts_with('^') {
            continue;
        }
        let line = line_of(&scan_body, whole.start());
        used_any.entry(id.to_string()).or_insert(line);
    }

    let ref_def = Regex::new(r"(?m)^\[([^\]]+)\]:[ \t]*(\S+)").unwrap();
    let mut defs: HashMap<String, (String, usize)> = HashMap::new();
    for caps in ref_def.captures_iter(&scan_body) {
        let ref_id = caps[1].trim().to_string();
        if ref_id.starts_with('^') {
            continue;
        }
        let target = caps[2].trim().to_string();
        let line = line_of(&scan_body, caps.get(0).unwrap().start());
        defs.entry(ref_id).or_insert((target, line));
    }

    let mut used_ids: Vec<&String> = used.keys().collect();
    used_ids.sort();
    for ref_id in used_ids {
        if !defs.contains_key(ref_id) {
            findings.push(Finding {
                line: used[ref_id],
                message: format!("[{ref_id}] used but never defined"),
            });
        }
    }

    let mut unused_def_ids: Vec<&String> =
        defs.keys().filter(|id| !used_any.contains_key(*id)).collect();
    unused_def_ids.sort();
    for ref_id in unused_def_ids {
        findings.push(Finding {
            line: defs[ref_id].1,
            message: format!("[{ref_id}] defined but never used"),
        });
    }

    let local_anchors = anchors(body);
    let mut target_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();
    let mut def_entries: Vec<(&String, &(String, usize))> = defs.iter().collect();
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
        let target_path = path.parent().unwrap_or_else(|| Path::new(".")).join(file_part);
        let target_anchors = target_cache
            .entry(file_part.to_string())
            .or_insert_with(|| std::fs::read_to_string(&target_path).ok().map(|s| anchors(&s)));
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
    Ok(findings)
}

fn line_of(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].matches('\n').count() + 1
}

/// Blank out fenced (``` / ~~~) and inline (`...`) code so bracket syntax inside code
/// examples isn't mistaken for a real reference link. Preserves line structure so byte
/// offsets into the result still map to the same line numbers as the original body.
fn strip_code(body: &str) -> String {
    let inline_code = Regex::new(r"`[^`\n]*`").unwrap();
    // The marker that opened the current fence (``` or ~~~) — a fence only closes on a
    // matching marker, so e.g. a ```-fenced block demonstrating `~~~` syntax doesn't
    // prematurely "close" and leak real code into reference-link scanning.
    let mut fence_marker: Option<&'static str> = None;
    let mut out_lines: Vec<String> = Vec::new();
    for line in body.split('\n') {
        let trimmed = line.trim_start();
        let opens_or_closes = match fence_marker {
            Some(marker) => trimmed.starts_with(marker),
            None => trimmed.starts_with("```") || trimmed.starts_with("~~~"),
        };
        if opens_or_closes {
            fence_marker = match fence_marker {
                Some(_) => None,
                None if trimmed.starts_with("```") => Some("```"),
                None => Some("~~~"),
            };
            out_lines.push(String::new());
        } else if fence_marker.is_some() {
            out_lines.push(String::new());
        } else {
            out_lines.push(inline_code.replace_all(line, "").into_owned());
        }
    }
    out_lines.join("\n")
}

/// GitHub-style heading-anchor slugification: lowercase, drop emphasis markers and other
/// punctuation (keeping literal underscores/hyphens), and turn each whitespace character
/// into its own hyphen without collapsing runs — a stripped em-dash can leave two spaces
/// that must anchor as `--` to match GitHub's real slugger (and pass markdownlint MD051).
fn slugify(heading: &str) -> String {
    let mut out = String::new();
    for ch in heading.to_lowercase().chars() {
        if ch == '`' || ch == '*' {
            continue;
        } else if ch.is_whitespace() {
            out.push('-');
        } else if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        }
    }
    out
}

/// The full set of valid heading anchors in a doc, with GitHub's real duplicate-heading
/// suffixing: the first occurrence of a slug is unsuffixed, later ones get `-1`, `-2`, ...
fn anchors(body: &str) -> HashSet<String> {
    let heading = Regex::new(r"(?m)^(#{1,6})[ \t]+(.+?)[ \t]*$").unwrap();
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut result = HashSet::new();
    for caps in heading.captures_iter(body) {
        let slug = slugify(&caps[2]);
        let count = counts.entry(slug.clone()).or_insert(0);
        let anchor = if *count == 0 {
            slug
        } else {
            format!("{slug}-{count}")
        };
        *count += 1;
        result.insert(anchor);
    }
    result
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
        assert!(findings[0].message.contains("[missing] used but never defined"));
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
    fn mismatched_fence_marker_does_not_close_block() {
        // A ```-fenced block that itself demonstrates `~~~` syntax must not be treated
        // as closed by the `~~~` line — only a matching ``` closes it.
        let body = "```\n~~~\n[thing][missing]\n```\n";
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
    fn image_ref_still_excluded_from_undefined_check() {
        let body = "![alt][missing-image]\n";
        let findings = check_source(&path(), body).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn repeated_undefined_usage_reported_once() {
        let body = "[a][missing] and [b][missing] again.\n";
        let findings = check_source(&path(), body).unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn duplicate_definition_first_wins() {
        let body = "See [x][ref].\n\n[ref]: first.md\n[ref]: second.md\n";
        let findings = check_source(&path(), body).unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("first.md"));
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
    fn flags_missing_target_file() {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-md-test-{}",
            std::process::id()
        ));
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
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-md-test-cross-{}",
            std::process::id()
        ));
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
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-md-test-cross-ok-{}",
            std::process::id()
        ));
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
        assert!(findings[0].message.contains("[orphan] defined but never used"));
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
}
