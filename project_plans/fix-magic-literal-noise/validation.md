# Validation: fix-magic-literal-noise

**Date**: 2026-11-01

## Pre-implementation validation

### Claim 1: `normalize_numeric("1.0") == "1"` (already implemented)
VERIFIED — `src/checkers/magic_literal_context.rs:423-430` has a live unit test:
```rust
assert_eq!(normalize_numeric("2.0"), "2");
assert_eq!(normalize_numeric("0.0"), "0");
```
And the integration test at line 400-419 confirms `(Language::Rust, "1.0")` produces 0
findings at 3 occurrences.

### Claim 2: Strings in `#[cfg(test)]` are already excluded
VERIFIED — `src/checkers/magic_literal_context.rs:297-308` has a passing test
`strings_should_BeSkipped_When_InsideRustCfgTestModule`. The `!numeric &&` guard
at line 71 explicitly limits this exclusion to strings.

### Claim 3: `"{p}"` currently fires
CONFIRMED by code reading:
- `normalize_literal_value("\"{p}\"", "string_literal")` → strips quotes → `{p}`
- `is_identifier_like("{p}")`: `{` is NOT in `is_alphanumeric() || matches!('_'|'-'|'.'|'/'|':'|'#')` → returns `false`
- Single-char check: `"{p}".chars().count()` is 3, not 1
- Result: no exemption → fires at 3+ occurrences

No existing test covers this case → confirmed gap.

### Claim 4: Numerics in `#[cfg(test)]` items use production threshold
CONFIRMED by code reading:
- `structural_exclusions` line 71: `!numeric &&` means numerics skip the `in_cfg_test_item` check
- `LiteralFilter.test_bump` is set from `file.test` (path-based), not from AST
- A `.rs` file at `src/stats.rs` with a `#[cfg(test)] mod tests { ... }` would have
  `kind.test == false`, so `test_bump == 0` and numerics use `MAGIC_LITERAL_MIN_OCCURRENCES = 3`

### Claim 5: `normalize_numeric("1e0")` returns `"1e0"` (not yet handled)
CONFIRMED by code reading — `normalize_numeric` has:
1. Radix check: `1e0` doesn't start with `0x/0b/0o` → pass
2. Suffix check: `1e0` ends with... checking `NUMERIC_SUFFIXES`:
   `"usize", "isize", "u128"... "f64"...` — `e0` is not a suffix (verified: `s.ends_with("f")` would match `1e0` only if the last char is `f`, which it's not — `0` is the last char)
3. Trailing-`.0`: `"1e0".split_once('.')` → `None` (no dot) → skip
4. Returns `"1e0"` as-is

No existing test covers `1e0` → confirmed gap.

## Risks

### Risk 1: `is_pure_format_capture` over-matches
**Mitigation**: the check requires:
- Starts with `{`, ends with `}`
- No nested braces in content
- At least 2 chars (`{}` length)

This excludes `{{p}}` (escaped brace, Rust syntax for literal `{p}` output), `{a} {b}`
(two captures), and strings with literal text. The only false-negative risk is a domain
key like `"{status}"` that happens to look like a format capture — but a key appearing
3+ times in a file IS a candidate for a named constant regardless of the brace notation.
The risk is low.

### Risk 2: `in_cfg_test_item` performance in Task 3
**Mitigation**: `in_cfg_test_item` is O(depth) per node, already called for every string
node in `structural_exclusions`. Adding it for numeric nodes in `build_literal_finding`
means O(depth × remaining_occurrences) per value — called only when `filter.test_bump == 0`
(production files). For a typical production `.rs` file with a `#[cfg(test)]` block,
depth is ~5 and occurrences are in the tens. Acceptable.

### Risk 3: Task 3 changes the `LiteralFilter` struct
**Mitigation**: `LiteralFilter` is private to `rules.rs` (no `pub`). The change is
internal, with no downstream consumers. The only callsite (`SyntaxRulesChecker::check`)
is in the same file.

## Validation criteria at completion

1. `cargo test` passes with all existing tests green
2. New tests for each of AC 1–6 pass
3. `kibitzer check native syntax-rules-rust <file-with-format-captures>` produces 0
   `replace-magic-literal` findings for the format-capture strings
4. Backtest results are recorded (Task 5)
