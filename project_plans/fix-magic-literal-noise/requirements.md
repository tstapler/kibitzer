# Requirements: fix-magic-literal-noise

**Date**: 2026-11-01
**Type**: bugfix — false-positive reduction in the existing `replace-magic-literal` rule
**Complexity**: 2 — targeted additions to `src/checkers/magic_literal_context.rs` and
`src/checkers/rules.rs`, all following the existing structural-exclusion / allowlist pattern

## Problem Statement

Writing a 170-line Rust statistics module (Mann-Whitney U test, median, erfc approximation)
with unit tests produced 15 `[replace-magic-literal]` findings in one `PostToolUse` run.
Three gap categories remain after the prior false-positive-reduction rounds:

1. **Format-arg captures**: `"{p}"` in `assert!(cond, "{p}")` is an implicit Rust/Python
   format-capture string (captures the local variable `p`), not a meaningful domain
   constant. Its unquoted content `{p}` is not identifier-like (braces are outside the
   identifier charset), so neither the identifier-string bump nor the single-char exclusion
   catches it. It fires whenever the same capture pattern appears ≥ 3 times across a file.

2. **Numerics inside `#[cfg(test)]` items in production `.rs` files**: the existing
   `in_cfg_test_item` exclusion only applies to *string* literals (because of the
   `!numeric &&` guard in `structural_exclusions`). Numeric literals in a `#[cfg(test)]`
   module inside a non-test-path `.rs` file count toward the production threshold (3),
   not the test threshold (10). The docs (`docs/syntax-rules.md:181`) already flag this
   as the dominant remaining noise: "test-like code `is_test_file` does not recognize
   (`test/e2e/`, in-crate `#[cfg(test)]`)."

3. **Scientific-notation literals that cannot normalize to an allowlisted integer**:
   `1e0` should normalize to `"1"` (same semantic as `1.0`, already handled), but
   `normalize_numeric` only strips trailing `.0` — it doesn't handle the `e0`/`E0`
   exponent form. More specific polynomial coefficients (`1e-12`) are not allowlisted
   and should not be — they are genuine magic numbers — but `1e0` and `2e0` are the
   same trivial values already allowlisted in their decimal-point form.

### What is already fixed (do not re-implement)

The prior round (ADR-001 + second-round context exclusions) already handles:
- `0.0` → `"0"`, `1.0` → `"1"`, `2.0` → `"2"` via `normalize_numeric` (trailing `.0`)
- `2`–`10`, `16`, `100`, `1000` via `MAGIC_LITERAL_NUMERIC_ALLOWLIST`
- String literals inside any `#[cfg(test)]` item (via `in_cfg_test_item`)
- Numeric literals in test *files* (path-based, threshold bumped to 10)
- `"B/op"`, `"pkg"`, `"ns/op"` etc. as identifier-like strings (≥ 6 occurrences)
- Numeric data-table rows with ≥ 4 elements

## Acceptance Criteria

1. A string literal whose entire unquoted content matches the pattern of a Rust/Python
   format-capture expression (e.g. `{ident}`, `{ident:spec}`, or any content that
   `is_pure_format_capture` classifies as a format template) is excluded from
   `replace-magic-literal` regardless of occurrence count.
2. `"{p}"`, `"{value}"`, `"{:.6}"`, and `"{:?}"` must **not** fire on 3+ occurrences
   in a production `.rs` file. A non-format string like `"some message"` with 3+
   occurrences must still fire.
3. Numeric literals inside a Rust `#[cfg(test)]` item in a production `.rs` file (one
   whose path does **not** satisfy `is_test_file`) are treated with the test-file
   threshold (≥ 10 occurrences), not the production threshold (≥ 3).
4. A production `.rs` file with the same numeric value repeated ≥ 3 times in
   *non-test* code but only inside `#[cfg(test)]` in the test section must **not**
   fire when all occurrences are in the test section and total < 10.
5. A production `.rs` file with a numeric value appearing ≥ 3 times in production code
   **and** in `#[cfg(test)]` continues to fire, anchored at the production-code occurrence.
6. `normalize_numeric("1e0")` returns `"1"` and `normalize_numeric("2e0")` returns `"2"`,
   so `1e0`/`2e0`/`1E0` literals are treated as allowlisted. Non-trivial scientific
   notation (`1e-12`, `2.5e3`) is left unchanged and continues to fire.
7. All existing tests in `src/checkers/magic_literal_context.rs` and `src/checkers/rules.rs`
   continue to pass.
8. New unit tests cover each of AC 1–6 following the existing `count(lang, path, src)`
   helper convention.
9. `docs/syntax-rules.md`'s "false-positive reductions" section is updated to document
   the three new exclusions.
10. A backtest (`kibitzer check backtest replace-magic-literal`) has been run against
    transcript history and shows the format-capture exclusion does not misfire on
    non-format strings; results are recorded in the PR or a `docs/backtest-triage/` file.

## Out of Scope

- Excluding genuinely non-trivial scientific-notation polynomial coefficients like
  `1e-12` from `replace-magic-literal` — these ARE magic numbers and the rule is correct
  to flag them. The fix is to name them (`const ERFC_TOLERANCE: f64 = 1e-12`).
- Extending format-capture detection to Go/Python/Java format strings — Rust's `{ident}`
  capture syntax is the narrow source of noise; other languages use `%s`/`{}` patterns
  that don't produce false positives in practice at this threshold.
- Changes to `long-function` or `deep-nesting` (mentioned in the same session report but
  described as genuine hits).

## Appetite

Small. Three focused additions to existing functions, each following an already-established
code pattern:
- `is_pure_format_capture(value: &str) -> bool` in `magic_literal_context.rs`
- One additional `|| is_pure_format_capture(...)` arm in `is_allowlisted_literal`
- Extension of `structural_exclusions` or `build_literal_finding` to apply the test bump
  to numerics inside `#[cfg(test)]` items
- `normalize_numeric` extended to strip `e0`/`E0` zero-exponent suffix

All three share the existing two-phase collect/emit architecture and need no new fields in
`LangRuleConfig` or `SyntaxRulesChecker`.
