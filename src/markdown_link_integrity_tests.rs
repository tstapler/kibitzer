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
    let dir = std::env::temp_dir().join(format!("kibitzer-md-test-cross-{}", std::process::id()));
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
fn ignores_piped_wiki_links() {
    let body = "See [[Industrial Waste|the waste feature]] for details.\n";
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
