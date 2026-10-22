# Plan: fix-wiki-link-integrity

## Summary

The wiki-link false-positive bug (`[[Page Name]]` misread as an undefined CommonMark
reference link) is **already fixed** in the current codebase via
`Options::ENABLE_WIKILINKS` on the pulldown-cmark parser. The remaining open work is
a single still-open related false positive: `[TAG]`-style shortcut labels with internal
whitespace or colons (e.g. `[SEVERITY: High]`, `[BLOCKER]`, `[NIT]`) are still flagged
as dangling references.

## Work breakdown

### Phase 1 — Extend `is_bracketed_checkbox_mark` heuristic (AC-3)

**File**: `src/checkers/markdown_link_integrity.rs`
**Function**: `is_bracketed_checkbox_mark` (line 233)

Rename to `is_non_link_bracket_usage` (or keep the name and widen the body).
Add a second condition: suppress `ShortcutUnknown` if the normalized label contains `:`
or is alphabetic uppercase only (matching tag-like patterns such as `BLOCKER`, `CRITICAL`,
`MAJOR`, `NIT`, `SEVERITY: High`).

Proposed predicate (to replace/extend the existing body):
```rust
fn is_non_link_bracket_usage(link_type: LinkType, normalized_label: &str) -> bool {
    if !matches!(link_type, LinkType::Shortcut | LinkType::ShortcutUnknown) {
        return false;
    }
    // Single-char checkbox marks (`[x]` / `[X]`).
    if normalized_label == "x" || normalized_label == " " {
        return true;
    }
    // Tag-style labels: contain `:` (e.g. `[SEVERITY: High]`) or are ALL_CAPS
    // identifiers without hyphens/underscores (e.g. `[BLOCKER]`, `[NIT]`).
    // Real reference labels use kebab-case or snake_case by convention.
    if normalized_label.contains(':') {
        return true;
    }
    let all_alpha_upper = normalized_label
        .chars()
        .all(|c| c.is_ascii_alphabetic() && c.is_ascii_uppercase());
    all_alpha_upper && !normalized_label.is_empty()
}
```

**Trade-off**: a reference label written in ALL_CAPS (e.g. `[RFC]`) would be suppressed.
This is a known trade-off, same as suppressing `[x]`. A user who genuinely uses `[RFC]`
as a shortcut reference label can add an explicit `[RFC]: url` definition to suppress
the `"used but never defined"` finding (which would correctly go away with a definition).

**Update call site** in `record_link_or_image` (line 260):
```rust
&& !is_non_link_bracket_usage(link_type, &normalized)
```

### Phase 2 — Add regression tests (AC-5)

**File**: `src/checkers/markdown_link_integrity_tests.rs`

New tests:
```rust
#[test]
fn ignores_severity_tag_in_heading() {
    let body = "# BUG-105: service crash [SEVERITY: High]\n\nBody text.\n";
    assert!(check_source(&path(), body).unwrap().is_empty());
}

#[test]
fn ignores_code_review_severity_tags() {
    // Matches `code:review` skill output format.
    for tag in &["[BLOCKER]", "[CRITICAL]", "[MAJOR]", "[NIT]"] {
        let body = format!("{tag} Fix the null check.\n");
        assert!(check_source(&path(), &body).unwrap().is_empty(),
            "expected no findings for {tag}");
    }
}

#[test]
fn still_flags_a_dangling_shortcut_reference_with_a_colon_if_it_has_a_definition_elsewhere() {
    // A label like `[foo: bar]` that IS used as a real reference link elsewhere
    // in the doc must NOT be suppressed when another label with the same name is dangling.
    // (This test confirms the heuristic does not suppress a Reference/ReferenceUnknown,
    // only Shortcut/ShortcutUnknown.)
    let body = "See [link][foo: bar] for details.\n";
    let findings = check_source(&path(), body).unwrap();
    // `[foo: bar]` is a Reference (not Shortcut) here — not suppressed.
    assert_eq!(findings.len(), 1);
}
```

### Phase 3 — Update false-positives log (AC-6)

**File**: `docs/markdown-link-integrity-false-positives.md`

Update the "2026-09-11 — stapler-squad — `[SEVERITY: High]`-style bracket tag" entry
(currently marked as still open) to:
- Status: fixed in `<commit sha>`
- Note the function rename and new tests.

### Phase 4 — Backtest

Run the new `is_non_link_bracket_usage` heuristic against the backtest corpus
(`scripts/clone-backtest-repos.sh`) to confirm no regressions on real-world markdown files.
Focus on repos with heavy documentation (`kubernetes/kubernetes`, `microsoft/vscode`).

## Decision points

1. **Function rename**: `is_bracketed_checkbox_mark` → `is_non_link_bracket_usage`.
   Low risk — it's a private function used in one place.

2. **ALL_CAPS heuristic scope**: the plan suppresses only `ShortcutUnknown` (no definition
   anywhere in the doc). If a user writes `[RFC][]` (collapsed reference) or `[link][RFC]`
   (explicit reference), those use `CollapsedUnknown`/`ReferenceUnknown` — distinct
   `LinkType` values NOT in the `Shortcut | ShortcutUnknown` arm. So the heuristic only
   applies to standalone `[TAG]`, never to the forms that clearly signal "this is a link."

3. **Config toggle**: the bug description proposed a `wiki_links: true` config toggle.
   Research finds this unnecessary — `ENABLE_WIKILINKS` is always-on and has no
   false-positive risk. No config change is planned.

## Adversarial review

**Q: Will ALL_CAPS suppression cause false negatives on real reference labels?**
A: Yes — any shortcut reference link whose label is ALL_CAPS and has no matching definition
will be silently skipped. However, this is deliberate: the heuristic is identical in shape
to the existing `[x]` guard. A user who uses `[RFC]` as a real reference label should
define it. If they do, the "used but never defined" finding disappears regardless.

**Q: Does `[SEVERITY: High]` also parse as a Reference (not Shortcut) form anywhere?**
A: Only if it appears as `[text][SEVERITY: High]` or `[SEVERITY: High][]`. In those forms
`link_type` is `Reference`/`Collapsed`, not `Shortcut`, and the heuristic does not apply.
The false-positive only occurs with standalone `[SEVERITY: High]` (no following `[...]`
or `(url)`), which is the `Shortcut`/`ShortcutUnknown` case.

**Q: Is pulldown-cmark 0.13 guaranteed to have `ENABLE_WIKILINKS`?**
A: Yes — the option is used in the current codebase and already compiling. No dependency change needed.
