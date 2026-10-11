# Requirements: long-function-flat-literal

**Date**: 2026-10-11
**Type**: precision improvement to existing native checker (`src/checkers/rules.rs`)
**Complexity**: 2 — targeted exemption added to an existing per-declaration rule;
no new walk, no new `LangRuleConfig` fields (or at most one), no two-pass aggregation.
This is the same complexity class as a node-kind exclusion added to `deep-nesting`'s
nesting walk.

## Problem Statement

`long-function` in `SyntaxRulesChecker` flags any function body exceeding 40 lines
regardless of what the body contains. When the body is a single `return` of a flat
composite/collection literal — the standard Go table-driven-test pattern, a TypeScript
`const cases = () => [...]` fixture, a Python `def cases() -> list[Case]: return [...]`
helper — there is nothing to extract: the body is already one flat unit. The only
actionable responses are to scatter the cases (worse) or extract them to a second
function that immediately trips the same rule again (verified by the filer on the exact
reproducer). The finding is noise, not signal.

This is classified as a precision enhancement, not a false positive: the rule fires
correctly per its current specification (body spans > 40 lines). The specification needs
a carve-out for this well-understood shape.

`docs/syntax-rules.md:160–167` already documents that `replace-magic-literal` hits
table-test noise and points users at `scope` exclusion globs or `.kibitzer/accepted/`.
That workaround exists for per-repo tolerance, but it silences the entire
`syntax-rules-*` checker for a path — `long-function` loses coverage for every other
function in that file. A narrower in-checker exemption is the proportionate fix.

## Mechanism (from the filed item)

`check_declaration` in `src/checkers/rules.rs` (around line 1534 on `master`) computes
`body.end_position().row - body.start_position().row + 1` and flags when it exceeds
`LONG_FUNCTION_LINES` (40). It inspects only line count. The sibling `deep-nesting` rule
does walk control flow, but `long-function` does not. No test-file special-casing exists
in the syntax-rules path — `_test.go` handling lives only in `complexity.rs`,
`duplicate_code_fuzzy.rs`, and `go_table_driven_test.rs`.

## Options Considered (from the filed item)

| # | Approach | Scope | Risk |
|---|----------|-------|------|
| 1 | Skip when body is one `return` of a composite literal with no control-flow or call children | Narrowest | Low FP risk; requires one small tree walk |
| 2 | Count only statement lines, not literal-element lines | Moderate | Harder to bound; changes the metric for all functions |
| 3 | Skip `_test.go` files for `long-function` only | Broad | Silences legitimate long test setup functions |

**Selected: Option 1.** Narrowest, most principled. Options 2 and 3 would suppress
findings in cases where Extract Function is genuinely applicable.

## Baseline

- `check_declaration` (`src/checkers/rules.rs:1534`) is the single entry point for
  `long-function`. Adding the exemption there, before the `body_lines > LONG_FUNCTION_LINES`
  branch, keeps the change local.
- `LangRuleConfig` already includes `density_boundary_kinds` (`src/checkers/rules.rs:914`
  for Go: `&["func_literal", "composite_literal"]`), which overlaps with the "data
  container" kinds this exemption needs. Reusing it avoids a new field; see
  `research/architecture.md` for the trade-off.
- The flat-literal detection function needs to: (a) confirm the body has exactly one
  statement, (b) confirm that statement is `return_kind`, (c) confirm the return value's
  kind is a collection literal and not a function literal, (d) confirm no control-flow
  (`nesting_kinds`, `if_kind`) or call (`call_kinds`) descendants are present in the
  literal's body.
- For Go, the block nesting is `block > statement_list > return_statement`, accessed via
  `go_statement_container` (`src/checkers/rules.rs:601`), so the "one statement" check
  must go through `cfg.statement_container`, not iterate block children directly.
- `lang_config()` was already refactored into 8 per-language constructor functions
  (`go_lang_config`, etc.) as part of the `replace-magic-literal` feature, so any new
  `LangRuleConfig` field goes into those 8 small functions — not into a 200-line monolith.

## Users / Consumers

All kibitzer users running the default check catalog on codebases that have table-test
helper functions or fixture-returning functions. The exemption is conservative enough
(one-statement body, one collection literal, no logic) that its FP risk is near zero —
a function meeting all these criteria has nothing Extract Function can improve.

## Success Metrics / Acceptance Criteria

1. A function whose body is exactly one `return` of a flat composite/collection literal —
   no control-flow (`if`, `for`, `switch`, etc.) and no call expressions anywhere in the
   literal's subtree — does **not** produce a `[long-function]` finding, regardless of
   how many entries the literal contains.
2. A function whose single-`return` body contains a composite literal that **does** include
   a call expression inside one of its entries (e.g. `{name: fmt.Sprintf("case %d", i)}`)
   still produces a `[long-function]` finding when the body exceeds 40 lines. ("Logic
   present" → exemption does not apply.)
3. A function whose single-`return` body contains a composite literal that **does** include
   a nested `if` or `for` (e.g. a factory function with conditional entries) still produces
   a `[long-function]` finding when the body exceeds 40 lines.
4. A function with a body that has more than one statement is not affected by this change —
   `long-function` still fires for it normally.
5. The exemption applies to at least Go. Other languages where the pattern is idiomatic
   (TypeScript array-of-objects, Python list-of-dicts) are addressed if the
   `LangRuleConfig` approach can cover them without per-language special-casing; Go-only
   is acceptable if the multi-language path requires disproportionate complexity.
6. Unit tests cover: (a) a Go flat composite literal over 40 lines that is exempted,
   (b) a Go flat composite literal over 40 lines whose entries contain a call expression
   (not exempted), (c) a normal Go function over 40 lines with real logic (still flagged),
   (d) a representative test for at least one other language (TypeScript or Python).
7. The exemption is documented in `docs/syntax-rules.md`'s `long-function` row/section —
   the same place `replace-magic-literal`'s table-test note lives (line 160).
8. `kibitzer check backtest syntax-rules` has been run against real transcript history
   (`docs/backtesting.md`) to confirm the exemption does not suppress real `long-function`
   findings on edited files, and the result is noted in the PR description.
9. The real-world corpus (`docs/backtest-repos.md`) has been sampled for this exemption:
   any function in the corpus matching the flat-literal shape is reviewed to confirm it
   was genuinely noise and not an Extract Function candidate.

## Appetite

Small — one new helper function (`is_flat_data_literal_body`) called from one point in
`check_declaration`, with 4–6 unit tests, and a one-paragraph doc update. Smaller than
`replace-magic-literal` by roughly a factor of five (no new `LangRuleConfig` fields
required if reusing `density_boundary_kinds`, no two-pass aggregation, no ADR needed
for a threshold).
