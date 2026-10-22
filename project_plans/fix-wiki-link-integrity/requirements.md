# Requirements: fix-wiki-link-integrity

## Background

Backlog item b5c4ad94-8853-4d65-be19-7512570cef41.

The `markdown-link-integrity` checker (native Rust, `src/checkers/markdown_link_integrity.rs`)
uses `pulldown-cmark` to parse markdown. Without the `ENABLE_WIKILINKS` parser option,
Obsidian/Logseq-style `[[Page Name]]` double-bracket wiki-links parse as literal `[` + a
nested CommonMark shortcut reference link `[Page Name]` + literal `]`. Since no
`[Page Name]: url` in-document reference definition exists (wiki-links resolve cross-file
by filename, not in-document tables), every such link fires a "used but never defined" finding.

Two repos were affected (logged in `docs/markdown-link-integrity-false-positives.md`):
- A ~9,000-page Logseq personal wiki (essentially every page edit fires)
- The stapler-squad repo itself (GFM task-list checkbox `- [x]`/`- [ ]` variant of the same mechanism)

## Current State (triage finding: fix already landed)

**The core wiki-link bug is already fixed.** `src/checkers/markdown_link_integrity.rs` line 97:
```rust
let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_WIKILINKS;
```
`ENABLE_WIKILINKS` causes pulldown-cmark to emit `[[...]]` as `LinkType::WikiLink` events,
which are distinct from reference-style links and never reach the reference-matching logic.
`ENABLE_TASKLISTS` similarly fixes the GFM task-list checkbox variant.

Regression tests at `src/checkers/markdown_link_integrity_tests.rs`:
- `ignores_logseq_style_wiki_links` (line 221) — `[[Some Page]]`
- `ignores_piped_wiki_links` (line 227) — `[[Page|Text]]`
- `ignores_github_task_list_markers` (line 233) — `- [ ]` / `- [x]`

The false-positives doc (`docs/markdown-link-integrity-false-positives.md`, "Resolution" section
at ~line 397) confirms both entries were removed as fixed.

## Still-Open Related Issue

`[SEVERITY: High]`-style bracket tags used in headings or plain sentences are still
misread as `ShortcutUnknown` reference links and flagged as false positives. Unlike wiki-links
(which have dedicated syntax pulldown-cmark can detect), there is no CommonMark signal that
distinguishes `[TAG]` from a shortcut reference link — the spec itself is ambiguous.

False-positives doc (line 352–395, "2026-09-11 — stapler-squad") documents this as still open
and proposes a heuristic fix: only treat `ShortcutUnknown` as dangling if the label is
"slug-like" (no whitespace, no special characters like `:`) — `SEVERITY: High` contains
both a colon and a space, unlike legitimate reference labels (`appendix-live-conversations`).
The current `is_bracketed_checkbox_mark` guard is a point-patch for `[x]`/`[X]` only.

## Functional Requirements

### AC-0 (DONE — no implementation needed)
`[[Page Name]]` wiki-links in any `.md` file produce zero findings from
`markdown-link-integrity`, whether or not a file named `Page Name.md` exists in the repo.

### AC-1 (DONE — no implementation needed)
`[[Page|Display Text]]` piped wiki-links are silently ignored.

### AC-2 (DONE — no implementation needed)
GFM task-list checkboxes (`- [x]` / `- [ ]`) at list-item start produce zero findings.

### AC-3 (open)
A bracketed tag with internal whitespace or non-slug punctuation (e.g. `[SEVERITY: High]`,
`[BLOCKER]` used as a prose tag, not a reference label) does **not** produce a "used but
never defined" finding, unless the document actually uses it as a reference-style link
(i.e. the text also contains `[link text][BLOCKER]` or `[BLOCKER][]`).

### AC-4 (open)
Existing regression tests for AC-0, AC-1, AC-2 (already in
`src/checkers/markdown_link_integrity_tests.rs`) remain green.

### AC-5 (open)
New tests cover the `[SEVERITY: High]` and `[BLOCKER]`/`[CRITICAL]`/`[MAJOR]`/`[NIT]`
patterns from the `code:review` skill output format, confirming AC-3 is implemented
without breaking genuine dangling-reference detection.

### AC-6 (open)
The false-positives log (`docs/markdown-link-integrity-false-positives.md`) is updated
to mark the `[SEVERITY: High]`-style bracket-tag entry as resolved.

## Out of Scope

- A per-repo `wiki_links: true` config toggle: not needed since `ENABLE_WIKILINKS` is
  now always on (no false-positive risk; it only adds a distinct event type for `[[...]]`
  syntax).
- Cross-file wiki-link resolution (verifying the target `.md` file exists): the original
  bug report does not require it and `pulldown-cmark`'s WikiLink event type does not
  provide a file path to check against.
- Any changes to the other still-open false-positive entries (multi-edit grace-window
  issues, whole-file-vs-diff mismatch) — those are separate issues.
