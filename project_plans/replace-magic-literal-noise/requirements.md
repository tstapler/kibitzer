# replace-magic-literal: numeric Rust noise

Source: backlog item efe0c3c7 (no acceptance criteria listed; derived below).

## Reproduction against the current build (kibitzer 0.1.28)

A Rust sample (erfc/median/midpoint + `#[cfg(test)]` asserts) still yields two findings:
`0.5` (7x, halving) and `1e-12` (5x, mostly test tolerances). Already fixed since the
report: `0.0`/`1.0`/`2.0`..`9.0` (`normalize_numeric` + `MAGIC_LITERAL_NUMERIC_ALLOWLIST`),
and `"{p}"` inside `#[cfg(test)]` (string skip).

## Requirements

1. `0.5` is allowlisted for numeric kinds (halving, same neutrality as `2`).
2. Numeric literals inside Rust `#[cfg(test)]` items / `#![cfg(test)]` files are excluded
   from counting (as strings already are), so test fixtures cannot push a value over the
   threshold or inflate production counts. Gated by the existing `skip_test_strings`
   option; test *files* keep today's raised floor, unchanged.
3. A string literal that is only a format-args placeholder (`"{p}"`, `"{}"`, `"{:?}"`)
   is never flagged, in production code too.
4. Backtest per CLAUDE.md: before/after finding counts on the corpus, recorded in
   `docs/syntax-rules.md`.
