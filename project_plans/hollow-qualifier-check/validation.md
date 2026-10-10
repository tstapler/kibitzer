# Validation: hollow-qualifier-check

## Gate status

**HOLD — below implementation threshold.** Only 1 confirmed occurrence of the
hollow-qualifier pattern has been observed. This repo's methodology requires 2+ independent
occurrences before implementing a new check. The plan and requirements are complete so
work can begin immediately when the threshold is met.

## Acceptance criteria validation map

| AC | Validating test | Status |
|---|---|---|
| 1. Heading ending `, generically` produces finding | `flags_heading_ending_with_generically` | Pending (not implemented) |
| 2. Heading ending `, fundamentally` produces finding | `flags_heading_ending_with_fundamentally` | Pending |
| 3. Lead sentence ending `, essentially` produces finding | `flags_lead_sentence_ending_with_essentially` | Pending |
| 4. Normal heading produces no finding | `allows_heading_without_distancing_adverb` | Pending |
| 5. Non-lead sentence ending `, generically` produces no finding | `ignores_non_lead_sentence_ending_with_generically` | Pending |
| 6. List item ending `, generically` produces no finding | `ignores_list_item_ending_with_generically` | Pending |
| 7. Callable via `kibitzer check native hollow-qualifier-phrasing` | Integration test / manual verify | Pending |
| 8. Not in `default_checks()` — requires opt-in | Test: `default_checks_does_not_include_hollow_qualifier` | Pending |
| 9. Adverb list configurable via `Check.options` | `configure_overrides_adverb_list` + `configure_rejects_unknown_option_key` | Pending |
| 10. Unit tests pass | All unit tests in the module | Pending |
| 11. Backtested against standard corpus, FP rate documented | Backtest run + results in `docs/prose-checks.md` | Pending |
| 12. `docs/prose-checks.md` updated with opt-in table entry | Manual review of the doc | Pending |

## Open questions before implementation

1. **Additive vs. replacing `options.adverbs`?** `ai-vocabulary-density` has `words`
   replace the full list when set. For this check, an "additive" mode (append to
   defaults) might be more useful so projects can add house-style words without
   re-specifying the full list. Decision needed before implementing `configure()`.

2. **Lead-sentence boundary in complex paragraphs.** `split_sentences` splits on
   `.`/`!`/`?`. A heading or paragraph opening like "The primitive, generically. And
   then more context." would have the hollow qualifier in its first sentence — correctly
   caught. A lead sentence ending with `:` (before a code block) would NOT be split off.
   Edge case: rare enough to defer.

3. **Second occurrence confirmation.** Where and how should a second occurrence be
   tracked? Suggested: a corpus mining run against the design-docs repo where the first
   occurrence was found, plus the standard public-code corpus, to surface any existing
   occurrences. If zero additional occurrences are found in the corpus, the 2+ bar is not
   met from mining alone — a second user report is required.

## Risks confirmed acceptable

- Advisory severity + opt-in default adequately mitigates FP risk.
- The `<noun>, <adverb>` end-of-heading/sentence pattern is narrow enough to avoid
  common false-positive categories (adverb-first phrases, technical adverbs mid-sentence).
- `for_each_heading` is a natural extension of the existing `markdown_text` API.

## Recommended next action

Mine the design-docs repo and the standard backtest corpus for additional occurrences of
the `<noun>, <adverb>` heading/lead-sentence pattern before writing any implementation
code. If 1+ additional occurrence is found: proceed to implementation. If zero: hold and
re-evaluate the check idea's priority in `docs/check-ideas.md` as a candidate for the
"untested ideas" section.
