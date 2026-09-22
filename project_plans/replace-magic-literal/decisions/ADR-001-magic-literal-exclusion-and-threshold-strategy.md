# ADR-001: Magic-literal exclusion scope and threshold-gating strategy

**Status**: Accepted
**Date**: 2026-09-22

## Context

`replace-magic-literal` is kibitzer's first default-on check that inspects literal
*values*, and the first to do any name-binding/reference resolution at all
(`research/pitfalls.md` §3, `research/architecture.md` §4). Three related design
choices materially affect its false-positive/false-negative rate for every kibitzer
install, and none of them are fully pinned down by the requirements text as written:

1. Requirements AC2 specifies the repetition threshold as `>=2`. But this repo's own
   `duplicate-code` checker shipped at "any repeat past the first" and had to be walked
   back to `>=3` after a real backtest showed two-copy repetition is routinely benign,
   dominated by table-driven test fixtures (`docs/refactoring-catalog-analysis.md:130-136`,
   `docs/duplicate-code-false-positives.md:47-55`). `research/pitfalls.md` §1 argues the
   risk is *structurally worse* here: a single literal is a far smaller, more common unit
   than `duplicate-code`'s 6-line/60-character window, so the base rate of incidental
   2-occurrence matches is inherently higher.
2. The requirements say the exclusion applies to a literal that's "the direct initializer
   of a `const`/`let`/`val`-style binding... referenced elsewhere by name" — but
   `research/pitfalls.md` §4 shows that reading "any `let`" literally would let a trivial,
   once-referenced local variable exempt a literal without representing Fowler's actual
   refactoring (promotion to a *meaningful named constant*, not just one extra level of
   indirection).
3. No checker in `src/rules.rs` has previously needed a generated-code guard
   (`grep -n "is_generated" src/rules.rs` returns nothing), but every other native checker
   that eventually needed one discovered the gap only after a noisy backtest
   (`docs/duplicate-code-false-positives.md`, `docs/file-complexity-false-positives.md`).

## Decision

1. **Threshold**: Ship coded at `>=2` (`MAGIC_LITERAL_MIN_OCCURRENCES = 2`), matching
   AC2 literally, but treat the mandatory corpus backtest (plan.md Phase 3, Story 3.2.2)
   as a real decision gate, not a formality: if the corpus triage shows table-driven-test
   noise dominating false positives the way it did for `duplicate-code` (~60% of sampled
   findings per `docs/duplicate-code-false-positives.md:47-55`), the threshold is bumped
   to `3` in the same PR, before merge — not shipped at 2 and "discovered" broken later.
2. **Binding scope**: Restrict the named-constant exclusion to bindings that are
   idiomatically "promoted to a constant," not merely "immutable": Go `const`, TS/JS
   `lexical_declaration` with `kind == "const"` (excluding `let`), Java `final`, Kotlin
   `val` (kept — Kotlin has no separate const keyword usable at local scope, and the
   requirements name it explicitly), Rust `const_item`/`static_item` (excluding
   `let_declaration`), Python a casing-convention heuristic (`^[A-Z][A-Z0-9_]*$`, since
   Python has no `const` keyword at all).
3. **Generated-file guard**: Proactively call `file_size::is_generated`, scoped to only
   the new literal-collection pass — not retrofitted onto the existing 4 rules, which have
   no backtest evidence of needing it.

## Consequences

- The threshold may change from 2 to 3 as a follow-up commit inside the same PR once
  backtest evidence is in — this is expected, not a plan failure; `plan.md`'s Dependency
  Visualization shows Story 3.2.2 looping back to Task 1.2.1a for exactly this reason.
- Restricting `let`/non-`const` bindings from the exclusion is a deliberate, documented
  deviation from the requirements' looser literal wording, in the direction of *fewer*
  false negatives (a `let`-bound trivial variable will still correctly get flagged as
  magic, rather than being silently exempted). This trade-off should be called out
  explicitly in code review, since it narrows AC4's literal scope.
- Python's constant exclusion is fundamentally weaker than the other 7 languages'
  (a naming convention, not a language-enforced guarantee) — an accepted, documented
  asymmetry, not an oversight.
- No lexical scope resolution is implemented; a same-named binding shadowed in a
  different function scope is a known, accepted false negative (matches this file's
  existing `collect_condition_identifiers` precedent, `src/rules.rs:878-880`).

## Alternatives Considered

- **Ship `>=3` preemptively** (skip validating `>=2` at all): rejected — would silently
  override AC2 without evidence, and the entire point of the mandatory backtest is to
  decide this empirically rather than guess from `duplicate-code`'s unrelated (larger-unit)
  precedent.
- **Honor "let" literally in the exclusion**: rejected — `research/pitfalls.md` §4 shows
  this risks under-flagging real magic literals sitting one hop behind a locally-scoped
  variable, which is the opposite failure mode from what a default-on advisory check
  should optimize for (a missed real smell is worse than an occasional over-cautious flag,
  for a check whose entire job is surfacing smells).
- **True lexical scope resolution for the "referenced elsewhere" check**: rejected as
  disproportionate to this feature's "Small" appetite; `research/build-vs-buy.md` §3
  recommends the same same-file/non-scope-resolved heuristic explicitly.
