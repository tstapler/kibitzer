# Plan: hollow-qualifier-check

## Approach

Implement `hollow-qualifier-phrasing` as a new native prose checker for kibitzer,
following the shape established by `ai_vocabulary_density` and `repetitive_sentences`.
The check scans markdown headings and paragraph lead sentences for the `<noun>, <adverb>`
hollow-qualifier anti-pattern using pulldown-cmark event parsing and a configurable
adverb list.

**Pre-condition for implementation start:** a second independent occurrence confirmed in
the wild (transcript, corpus mining, or user report). This plan documents the full
implementation path so work can begin immediately once the threshold is met.

## Implementation tasks

### Task 1 — Add `for_each_heading` helper to `markdown_text.rs`

Add a public `for_each_heading(body: &str, mut on_heading: impl FnMut(usize, &str))`
function alongside `for_each_paragraph`. Walk pulldown-cmark events for `Tag::Heading`,
collect `Text`/`Code` events as the heading's text (same flattening as paragraphs), call
`on_heading(line, &text)` at `TagEnd::Heading`. Also add a `check_headings` wrapper
(analogous to `check_paragraphs`) that collects `Finding`s.

**Why shared helper:** the module doc comment notes that `check_paragraphs` was extracted
when a 7th checker independently duplicated the same wrapper — a heading-aware helper
follows the same discipline.

### Task 2 — Implement `src/checkers/hollow_qualifier_phrasing.rs`

```
pub struct HollowQualifierPhrasingChecker {
    adverbs: Vec<String>,
}
```

Core logic:
1. Build a regex from `self.adverbs`:
   `r"(?i),\s*(?:<adverb1>|<adverb2>|…)\s*$"` — case-insensitive, anchored at end.
2. In `check()`: scan headings via `for_each_heading`; scan paragraph lead sentences
   via `for_each_paragraph` + `split_sentences` (first sentence only).
3. Report `Finding { line, message: format!("heading/lead sentence reads as a category
   label — \"{text}\" names the kind of thing, not the thing itself; rewrite to say
   what it actually covers") }`.

Default adverb list (11 words): `generically`, `fundamentally`, `essentially`,
`basically`, `broadly`, `generally`, `abstractly`, `conceptually`, `theoretically`,
`ultimately`, `nominally`.

`configure()` follows `ai_vocabulary_density`'s pattern: `Options { adverbs:
Option<Vec<String>> }` replaces the whole list when set.

Registration: `inventory::submit! { crate::checker::CheckerFactory(…) }` — NOT added to
`prose_checks()` (opt-in only).

### Task 3 — Wire `hollow_qualifier_phrasing` into `src/checkers/mod.rs`

Add `pub mod hollow_qualifier_phrasing;` to `src/checkers/mod.rs`. The
`inventory::submit!` call in the module registers the checker automatically.

### Task 4 — Unit tests in `hollow_qualifier_phrasing.rs`

Tests required (map directly to ACs 1-6, 9-10):
- `flags_heading_ending_with_generically`
- `flags_heading_ending_with_fundamentally`
- `flags_lead_sentence_ending_with_essentially`
- `allows_heading_without_distancing_adverb`
- `ignores_non_lead_sentence_ending_with_generically`
- `ignores_list_item_ending_with_generically`
- `configure_overrides_adverb_list`
- `configure_rejects_unknown_option_key`

### Task 5 — Update `docs/prose-checks.md`

Add a row to the "Opt-in" table:

| Check | Flags | Why it's opt-in |
|---|---|---|
| `hollow-qualifier-phrasing` | A heading or paragraph lead sentence ending with `, <adverb>` (noun phrase + distancing adverb) | Single observed occurrence so far; backtest FP rate not yet known |

Add a one-paragraph note in the configuring section for the `adverbs` option.

### Task 6 — Backtest

Run `kibitzer check backtest hollow-qualifier-phrasing` against transcripts and the
standard corpus. Document FP rate. If FP rate is acceptable (comparable to other default
prose checks), file a follow-up to graduate the check to `default_checks()`.

## File inventory

| File | Change |
|---|---|
| `src/markdown_text.rs` | Add `for_each_heading` + `check_headings` helpers |
| `src/checkers/hollow_qualifier_phrasing.rs` | New file — checker struct + tests |
| `src/checkers/mod.rs` | Add `pub mod hollow_qualifier_phrasing;` |
| `docs/prose-checks.md` | Update opt-in table + options example |

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| False positives from domain-specific adverbs ("architecturally", "technically") | Conservative default list; fully configurable via `options`; advisory only |
| Headings in reference docs using `, adverb` as a deliberate annotation | Advisory severity; opt-in default; users can suppress with `kibitzer:ignore` |
| `for_each_heading` is a new shared API surface | Low risk: closely mirrors `for_each_paragraph`; tested in the new checker's tests |
| Missing setext headings | pulldown-cmark parses setext and ATX headings the same — no extra work needed |

## Adversarial review

**Concern: Is the one-occurrence bar appropriately flagged?**
Yes — requirements.md explicitly states the 2+ occurrence threshold is not yet met and
implementation should not start until it is. The plan documents the full path so no
planning work is repeated when the second occurrence arrives.

**Concern: Is the regex pattern too narrow (misses variants)?**
The `<noun>, <adverb>` end-of-heading pattern is deliberately narrow to keep FP rate
low. Variants like "fundamentally speaking, the primitive" (adverb at start) are NOT
caught — that's intentional. The pattern targets the specific form observed in the wild.

**Concern: Should this be Vale-based instead of native?**
Vale is recommended in `docs/prose-checks.md` for configurable/pluggable style rules.
This check's pattern is narrow and mechanical enough for a native regex implementation
(no tokenization, no style ruleset needed). Vale would be appropriate if we wanted
project-specific house-style words — not for this fixed mechanical anti-pattern.

**Concern: `for_each_heading` in `markdown_text.rs` adds API surface.**
The module already has `for_each_paragraph` and `check_paragraphs`. A parallel
`for_each_heading`/`check_headings` pair is consistent with the module's design. The
risk is low.

## Implementation deviations

- No `check_headings` wrapper: the one caller needs a per-kind label and the lead-sentence
  path in the same pass, so the wrapper would have a single caller. Instead
  `markdown_text::for_each_block` (one walker, `BlockKind::{Paragraph, Heading}`, document
  order) backs `for_each_paragraph` and the checker; no separate `for_each_heading`.
- An empty or all-blank `adverbs` list is a config-load error, not a silent no-op.
