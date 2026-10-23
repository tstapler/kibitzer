# Research: fix-single-letter-bracket-labels

## Source Trace

### `is_non_link_bracket_usage` — current implementation

File: `src/checkers/markdown_link_integrity.rs`, lines 229–242

```rust
/// Severity tags emitted by the `code:review` skill, used as plain prose markers.
const SEVERITY_TAGS: [&str; 4] = ["BLOCKER", "CRITICAL", "MAJOR", "NIT"];

/// Bracketed text that is almost never a reference-link attempt: a `[x]`/`[X]` mark used
/// mid-sentence (outside `ENABLE_TASKLISTS`' list-item position), a `[LABEL: text]` tag, or
/// one of [`SEVERITY_TAGS`]. Scoped to `Shortcut`/`ShortcutUnknown` only —
/// `Reference`/`Collapsed` require an explicit `[label][ref]`/`[label][]`, still flagged.
/// Other all-caps labels (`[RFC]`) stay flagged: they are indistinguishable from real refs.
fn is_non_link_bracket_usage(link_type: LinkType, raw_label: &str) -> bool {
    if !matches!(link_type, LinkType::Shortcut | LinkType::ShortcutUnknown) {
        return false;
    }
    let label = raw_label.trim();
    label.eq_ignore_ascii_case("x") || label.contains(':') || SEVERITY_TAGS.contains(&label)
}
```

Call site (`record_link_or_image`, line 265):
```rust
if is_dangling
    && !is_image
    && !normalized.starts_with('^')
    && !is_non_link_bracket_usage(link_type, &id)
{
    parsed.findings.push(Finding { ... });
}
```

Note: the call passes `&id` (raw label), not `&normalized`. The function applies its own
`raw_label.trim()` but does not lowercase. `SEVERITY_TAGS` contains uppercase strings and
the comparison is case-sensitive — `"blocker"` would not match `"BLOCKER"`. This is
consistent with the existing test `ignores_code_review_severity_tags` which only tests the
uppercase form (the tags are always emitted uppercase by the `code:review` skill).

### Existing exemption for `[x]`

The check `label.eq_ignore_ascii_case("x")` uses case-insensitive comparison, catching
both `[x]` (task-list checkbox) and `[X]` (explicit uppercase). The design precedent
is: single-char labels that are clearly not reference-link identifiers get an exemption.

Tests already covering this:
- `ignores_a_checkbox_mark_mid_sentence_outside_a_task_list_item` (line 239)
- `ignores_an_uppercase_checkbox_mark_mid_sentence` (line 249)
- `ignores_a_checkbox_mark_inside_a_table_cell` (line 256)

### CommonMark spec — single-letter reference labels

CommonMark spec §6.6 (Link reference definitions) places no restriction on label
length. A single letter (`H`, `W`, `R`) is a fully valid reference label. However,
in real-world markdown authoring, single-letter shortcut references are never
intentionally used as hyperlinks — every observed case in the codebase history is
either `[x]` (checkbox) or an inspection-checklist label like `[H]/[W]/[R]`.

### Prior plan decision

`project_plans/fix-wiki-link-integrity/plan.md` proposed a broader ALL_CAPS heuristic
(suppress any label that is all uppercase letters). That proposal was declined at
landing time — the current comment explicitly says "Other all-caps labels (`[RFC]`) stay
flagged." This item's scope is narrower (single-character only) and avoids that
trade-off: single-character alphabetic labels are a genuinely distinct class with
essentially zero false-negative risk.

### False negative risk assessment

Extending `label.eq_ignore_ascii_case("x")` to any single ASCII alphabetic character:
- Suppresses `[A]`…`[Z]` and `[a]`…`[z]` shortcut-reference forms
- A user who genuinely uses `[A]` as a shortcut reference label almost certainly has
  a `[A]: url` definition — if they do, `is_dangling` is false anyway and the guard
  is never reached. The only risk is a user who writes `[A]` as a shortcut reference
  WITHOUT defining it, which is already a bug in their document.
- The `[x]` exemption exists for exactly this reason — the logic is identical.

### Test gap

No test currently covers `[H]`, `[W]`, `[R]` producing zero findings. The
`ignores_a_checkbox_mark_mid_sentence_outside_a_task_list_item` test is structurally
identical to what is needed but only parameterises over `[x]`.

### Docs false-positives log

File: `docs/markdown-link-integrity-false-positives.md` — not yet read in this session,
but the pattern from the prior fix (fix-wiki-link-integrity) is to add a new dated entry
documenting the symptom, root cause, and fix commit SHA.

## Decisions going into planning

1. **Scope**: single ASCII alphabetic character only (length 1, `is_ascii_alphabetic()`).
   Not multi-letter ALL_CAPS. This keeps the change minimal and consistent with the
   existing `[x]` exemption.

2. **Case sensitivity**: match with case-insensitive comparison (same as the `[x]` guard)
   so `[H]` and `[h]` are both suppressed.

3. **Call site argument**: the function currently receives `raw_label` (`&id`). The
   single-char check is straightforward on the raw label after `trim()` — no need to
   change the calling convention.

4. **No config toggle**: the heuristic is narrow enough that a global always-on
   suppression is appropriate, matching the existing `[x]` treatment.
