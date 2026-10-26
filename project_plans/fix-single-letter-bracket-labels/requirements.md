# Requirements: fix-single-letter-bracket-labels

## Background

Backlog item 4e00e9e0-a042-41ab-b59f-a9969fb0084d.

The `markdown-link-integrity` checker (`src/checkers/markdown_link_integrity.rs`) uses
pulldown-cmark to parse markdown. Single-letter bracketed labels at the start of a list
item — inspection hold-point designations like `[H] Hold`, `[W] Witness`, `[R] Review`
— are parsed as CommonMark shortcut reference links with no matching `[H]: url`
definition in the document. The checker reports them as "used but never defined."

These labels are plain-text markers, not reference links. They appear in planning and
quality-assurance templates (e.g. ISO/ASME inspection checklists). In a Logseq page
they repeat across many bullet lines, so the finding fires on every edit and the
PostToolUse Stop hook re-reports it each turn.

## Current State

`is_non_link_bracket_usage` (line 236) already suppresses:
- `[x]`/`[X]` checkbox marks (`label.eq_ignore_ascii_case("x")`)
- Labels containing `:` like `[SEVERITY: High]`
- Explicit severity tags: `[BLOCKER]`, `[CRITICAL]`, `[MAJOR]`, `[NIT]` (via `SEVERITY_TAGS`)

The function comment (line 235) explicitly notes that "other all-caps labels (`[RFC]`)
stay flagged: they are indistinguishable from real refs." The `[x]` guard was already
generalised in the fix-wiki-link-integrity work but was not extended to other
single-letter labels.

Repro (issue, run 2026-10-22):
```bash
printf -- "- [H] Sash supported before the last screw is removed\n- [W] Even reveal after rehang\n" > a.md
kibitzer check native markdown-link-integrity a.md
# a.md:1: [H] used but never defined
# a.md:2: [W] used but never defined
# exit 2
```

## Mechanism (VERIFIED via source)

In `record_link_or_image` (line 265), a dangling shortcut reference is suppressed only
if `is_non_link_bracket_usage` returns true. For `[H]`:
- `label.eq_ignore_ascii_case("x")` → false (`H ≠ x`)
- `label.contains(':')` → false
- `SEVERITY_TAGS.contains(&label)` → false (`H` not in the set)

Result: `is_non_link_bracket_usage` returns false, finding is emitted. Same for `W`, `R`.

## Functional Requirements

### AC-0 (done — regression insurance)
`[x]`/`[X]` at any position in a line produces zero findings (already tested; must stay
green).

### AC-1 (new)
A single ASCII letter in a shortcut bracket, used as a bullet-item label (e.g. `[H]`,
`[W]`, `[R]`, `[A]`…`[Z]`, `[a]`…`[z]`), produces zero findings from
`markdown-link-integrity` when no matching reference definition exists.

### AC-2 (new)
Single-letter label suppression only applies to `Shortcut`/`ShortcutUnknown` link
types. An explicit reference form `[text][H]` or collapsed form `[H][]` is still
flagged when no definition exists (these forms clearly signal intent to create a
hyperlink).

### AC-3 (new)
A single-letter label that actually has a matching `[H]: url` definition in the same
document produces zero findings — suppression is irrelevant because the reference is
fully defined.

### AC-4 (new)
New unit tests cover `[H]`, `[W]`, `[R]` shortcut references producing zero findings,
and verify that `[text][H]` explicit-reference form still fires when no definition
exists.

### AC-5 (new)
The false-positives log (`docs/markdown-link-integrity-false-positives.md`) is updated
with a new entry documenting this fix.

## Out of Scope

- A broader ALL_CAPS heuristic (e.g. suppressing `[RFC]`): the prior plan
  (`project_plans/fix-wiki-link-integrity/plan.md`) proposed this but the landing
  decision (comment at line 235) explicitly declined it. This item is narrower:
  single-character labels only.
- Multi-letter arbitrary labels like `[QA]`, `[ITP]`: not part of the reported repro.
  Those can be added via SEVERITY_TAGS-style explicit listing if needed.
- Per-repo configuration toggles: not needed for a targeted single-char heuristic with
  very low false-negative risk.
