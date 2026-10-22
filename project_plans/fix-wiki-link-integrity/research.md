# Research: fix-wiki-link-integrity

## Status: already-fixed core + one open related false positive

### Core wiki-link bug — FIXED

`src/checkers/markdown_link_integrity.rs:97`:
```rust
let opts = Options::ENABLE_TASKLISTS | Options::ENABLE_WIKILINKS;
```

`pulldown-cmark`'s `Options::ENABLE_WIKILINKS` makes `[[...]]` emit a
`LinkType::WikiLink` event instead of being parsed as `[` + shortcut reference link + `]`.
The reference-matching logic in `reference_style_flags` only acts on
`Reference`/`ReferenceUnknown`/`Collapsed`/`CollapsedUnknown`/`Shortcut`/`ShortcutUnknown`
— `WikiLink` is not in this set, so the event is silently ignored.

Tests:
- `ignores_logseq_style_wiki_links` — `[[Some Page]]` → 0 findings ✓
- `ignores_piped_wiki_links` — `[[Industrial Waste|the waste feature]]` → 0 findings ✓

`docs/markdown-link-integrity-false-positives.md` "Resolution" section (lines 397–427)
confirms both wiki-link and task-list-checkbox entries were removed as fixed.

### Open false positive: `[TAG]`-style shortcut reference links

**Mechanism**: CommonMark has no dedicated syntax for "plain bracketed text that is
definitely not a link attempt." `[SEVERITY: High]` in a heading or sentence is
syntactically identical to a shortcut reference link. When no `[SEVERITY: High]: url`
definition exists, pulldown-cmark emits `LinkType::ShortcutUnknown`. The checker's
`reference_style_flags` classifies all `*Unknown` types as dangling.

**Only partial guard exists**: `is_bracketed_checkbox_mark` (line 233) suppresses
`ShortcutUnknown` with normalized label `"x"` — a point-patch for a single label.
The false-positives doc explicitly marks this as open (line 391–395).

**Affected patterns in this codebase**:
- `code:review` skill output: `[BLOCKER]`, `[CRITICAL]`, `[MAJOR]`, `[NIT]`
- stapler-squad bug-report convention: `[SEVERITY: High]` in `# BUG-NNN: ...` headings

**Proposed fix** (from false-positives doc, line 380–384):
> Only treat a `ShortcutUnknown` as dangling if the bracketed text looks like a plausible
> reference label — e.g. a slug-like `[a-z0-9-]+` pattern, or no internal whitespace.
> `SEVERITY: High` contains both a colon and a space.

**Heuristic design**:
A reference label that a human actually intends to use as a CommonMark reference will:
1. Not contain whitespace (reference labels can technically contain whitespace by spec,
   but convention strongly prefers `kebab-case` or `under_score` identifiers)
2. Not contain `:` (colons are path/URL separators, not label characters)
3. Not be ALL_CAPS with spaces (a code-review tag convention, not a reference label)

Candidate rule: suppress `ShortcutUnknown` if the normalized label contains `:` or has
any character outside `[a-z0-9_.-]` after normalization (lowercasing + whitespace collapse).
This is narrower than "no whitespace" — a label like `my ref` with a space would still be
flagged — but covers the `:` case that `[SEVERITY: High]` and `[BLOCKER: foo]` represent.

Alternatively, "contains whitespace OR contains colon OR is ALL_CAPS_WITH_UNDERSCORES"
would catch `[SEVERITY: High]` and `[BLOCKER]`/`[CRITICAL]`/`[MAJOR]`/`[NIT]`.

**Risk**: A genuine reference label with a `:` exists in the CommonMark spec (e.g.
`[foo: bar]` is syntactically valid as a label). Suppressing it would be a false negative.
The heuristic is a trade-off; the existing `is_bracketed_checkbox_mark` guard already
accepts the same trade-off for `[x]`.

### pulldown-cmark API surface

- `Options::ENABLE_WIKILINKS` — already in `pulldown-cmark` as of at least 0.10.x
  (used in the codebase). `[[...]]` → `LinkType::WikiLink`. No version bump needed.
- `Options::ENABLE_TASKLISTS` — same crate, already on.
- Shortcut reference matching: `is_bracketed_checkbox_mark` at line 233 is the correct
  extension point for additional label-shape heuristics — already reads `link_type` and
  `normalized_label`, just returns `false` for everything except `"x"`.

### Cargo.toml dependency

```bash
grep pulldown /home/tstapler/.stapler-squad/workspaces/d685c4b1a423cca3/worktrees/triage-b5c4ad94-8853-4d65-be19-7512570cef41_18df943a50974b8c/Cargo.toml
```
