# Research: fix-magic-literal-noise

**Date**: 2026-11-01

## Codebase state (verified by reading source)

### What is already implemented

All citations are to the current worktree.

**`normalize_numeric`** (`src/checkers/magic_literal_context.rs:180-196`):
- Strips digit separators (`_`), type suffixes (`u8`, `f32`, `L`, `f`, etc.)
- Strips trailing `.0` fraction: `1.0` → `"1"`, `2.0` → `"2"`, `0.0` → `"0"`
- Leaves radix-prefixed literals untouched (`0x1f` stays)
- Does NOT handle `e0`/`E0` zero-exponent: `1e0` stays as `"1e0"` (gap #3)

**`MAGIC_LITERAL_ALLOWLIST`** (`src/checkers/rules.rs:42`):
- `["0", "1", "-1", ""]` — compared after `normalize_literal_value`

**`MAGIC_LITERAL_NUMERIC_ALLOWLIST`** (`src/checkers/rules.rs:48-50`):
- `["2", "3", "4", "5", "6", "7", "8", "9", "10", "16", "100", "1000", "2026"]`
- Numerics only (verified: string `"100"` is not exempt — `is_allowlisted_literal` checks kind first)

**`is_allowlisted_literal`** (`src/checkers/rules.rs:1777-1786`):
- For numeric kinds: runs `normalize_numeric` then checks both allowlists
- For string kinds: checks `MAGIC_LITERAL_ALLOWLIST` and single-character exclusion

**`structural_exclusions`** (`src/checkers/magic_literal_context.rs:55-81`):
- `in_attribute`: skips Rust `#[..]`/`#![..]`, Java/Kotlin `@Annotation`, Python/TS `@decorator`
- `in_numeric_data_row`: skips numerics in arrays/tuples with ≥ 4 numeric elements
- `in_cfg_test_item` / `root_is_cfg_test`: skips **strings only** (`!numeric &&` guard at line 71) in `#[cfg(test)]` items — numerics are NOT excluded here

**`is_identifier_like`** (`src/checkers/rules.rs:1789-1794`):
- `^[\w\-./:#]{1,40}$` — braces `{` `}` are NOT in this set

**`skip_tests` / test threshold** (`src/checkers/rules.rs:1466-1502`):
- `skip_tests: true` (default), triggers `MAGIC_LITERAL_TEST_FILE_MIN_OCCURRENCES = 10`
- The bump is applied based on `file.test` (path-based `is_test_file`) — not on `#[cfg(test)]` AST detection
- A production `.rs` file (non-test path) with a `#[cfg(test)]` block still uses the production threshold (3) for numerics

**`is_test_file`** (referenced in docs):
- Path pattern: `*_test.go`, `test_*.py`, `*.test.ts`, `tests/` directories, etc.
- Does NOT recognize in-crate `#[cfg(test)]` modules as test code for the threshold bump

### Gap 1: Format-arg captures `"{p}"`

In `is_allowlisted_literal` (`rules.rs:1777`):
- The string `"{p}"` normalizes via `normalize_literal_value` to the inner value `{p}` (3 chars)
- `MAGIC_LITERAL_ALLOWLIST.contains(&"{p}")` → false
- Single-char check: `"{p}".chars().count() == 1` → false (`{p}` is 3 chars)
- `is_identifier_like("{p}")` → false (braces not in the charset)
- Result: `"{p}"` fires at 3+ occurrences ✗

Pattern to detect: a string whose unquoted content is entirely a Rust/Python format
placeholder. Rust format syntax: `{identifier}`, `{identifier:format_spec}`, `{:spec}`.
The minimal detector is: after stripping quotes, does the string start with `{` and end
with `}` with no nested braces or plain text outside them?

Proposed: `is_pure_format_capture(value: &str) -> bool` — checks that after stripping
the outer `{}`pair, the content is either empty, a word (`\w+`) optionally followed by
`:` and format spec characters, or just a format spec after `:`. Keep it conservative:
only exclude strings that are *entirely* a single format capture — multi-segment strings
like `"{p} items"` are NOT excluded.

### Gap 2: Numerics in `#[cfg(test)]` in production files

In `structural_exclusions` (`magic_literal_context.rs:55-81`):
```rust
let skip = in_attribute(node)
    || (numeric && in_numeric_data_row(node, numeric_kinds, &mut row_verdicts))
    || (!numeric          // <-- strings only
        && test_strings.is_some_and(|in_test_file| {
            in_test_file
                || *file_is_cfg_test.get_or_insert_with(|| root_is_cfg_test(node, src))
                || in_cfg_test_item(node, src)
        }));
```

Two approaches:
a) Extend `structural_exclusions` to mark numeric nodes in `#[cfg(test)]` items, return
   them in a separate `cfg_test_numeric_ids: HashSet<usize>` alongside `excluded`, then
   in `build_literal_finding` apply the test bump for such nodes.
b) Add a `cfg_test_numeric_bump` path in `LiteralFilter` that applies when all of a
   literal's remaining occurrences are in `#[cfg(test)]`.

Approach (a) is simpler but requires passing the set through `LiteralFilter`. Approach (b)
is more precise — only silences values that are *exclusively* in test code, not ones
that appear in both production and test sections. Approach (b) is preferred per AC 5.

Mechanics of approach (b): after filtering by `excluded` and `slog_keys` in
`build_literal_finding`, check if ALL remaining occurrences are in `#[cfg(test)]` items
using `in_cfg_test_item`. If so, apply `MAGIC_LITERAL_TEST_FILE_MIN_OCCURRENCES` as an
additional floor.

Cost: `in_cfg_test_item` walks ancestors per node — O(depth) per occurrence. For a file
with a `#[cfg(test)]` block, this is a small constant per numeric literal occurrence.
No caching needed unless the checker is applied to pathological files (e.g., 10k
occurrences), which is already handled by the data-row exclusion.

### Gap 3: Scientific notation `1e0` / `2e0`

`normalize_numeric("1e0")` currently returns `"1e0"` (no `.` to split on, no matching
suffix). The fix is to detect `eN` or `EN` where N is all-zero digits as a zero exponent:

```rust
if let Some((mantissa, exp)) = s.split_once(['e', 'E'])
    && exp.chars().all(|c| c == '0')
    && !mantissa.is_empty()
{
    return mantissa.to_string();
}
```

This handles `1e0` → `"1"`, `2E0` → `"2"`, `0e0` → `"0"`. Does not touch `1e-12`,
`2.5e3`, or `1e1` (non-zero exponent). Radix check must precede this (already does for
the radix-prefix guard at the top of `normalize_numeric`).

Edge case: `1.5e0` — splits on `e`, mantissa `1.5`, exp `0`; trailing `.0` strip then
gives `"1"` (but `1.5` doesn't end in `.0` unless `frac` is all-zeros). Actually the
correct behavior: `normalize_numeric("1.5e0")` → should remain `"1.5e0"` (it's a
non-trivial value 1.5). The zero-exponent strip should only reduce the exponent part,
leaving the mantissa for the existing `.0` check. So:
- After exp strip: `1.5e0` → `"1.5"` (remove `e0`)
- Then trailing `.0` check: `1.5` — frac is `5`, not all zeros → stays `"1.5"`
- Correct: `1.5e0` stays as `"1.5"` (non-trivial)

`1.0e0` → strip exp → `"1.0"` → strip trailing `.0` → `"1"` ✓

Order matters: the zero-exponent strip should happen BEFORE the trailing-`.0` strip in
`normalize_numeric` so both can compose correctly.

### Test infrastructure

Existing test helpers (`magic_literal_context.rs:219-232`, `rules.rs`):
- `count(lang, path, src)` returns the number of `replace-magic-literal` findings
- `repeated(lang, lit, n)` generates a file with `n` bindings of `lit`
- `normalize_numeric` is directly unit-tested in `magic_literal_context.rs`

New tests can reuse these helpers verbatim.

### Documentation update

`docs/syntax-rules.md:162-213` is the canonical false-positive reduction section.
New bullets to add under the second round:
- "**Format-arg captures**: a string whose entire content is a single Rust/Python format
  placeholder (`{ident}`, `{:spec}`, `{ident:spec}`) is not flagged."
- "**Numerics in `#[cfg(test)]` items within production `.rs` files**: treated with the
  test-file threshold (≥ 10) rather than the production threshold, matching how numerics
  in test-path files already behave."
- **Update** the normalize note to mention `1e0`→`"1"` alongside `1.0`→`"1"`.

## Risk assessment

- **Format-capture false-negative risk**: could we exclude a real magic literal `"{key}"`
  that appears in format strings but is actually a domain code? Unlikely — if `"{key}"`
  appeared 3+ times with no named constant, it almost certainly IS a template being
  repeated. The `is_pure_format_capture` check is conservative (single capture only).
- **Numeric `#[cfg(test)]` regression**: applying test threshold to numerics that are
  ONLY in `#[cfg(test)]` could miss real production issues, but AC 5 requires the check
  to still fire when the same value appears ≥ 3 times in production code regardless of
  its test-section uses. The "exclusively in test" guard is the key invariant.
- **`1e0` normalization false-negatives**: `1e0` is semantically `1`, so allowlisting it
  is correct. The only risk is if someone writes `1e0` to mean something else, which would
  be pathological.
