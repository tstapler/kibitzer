# Plan: fix-magic-literal-noise

**Date**: 2026-11-01

## Objective

Reduce false positives in `replace-magic-literal` for the three remaining noise categories
identified in the bug report: format-arg capture strings (`"{p}"`), numeric literals in
`#[cfg(test)]` items inside production files, and zero-exponent scientific notation
(`1e0`). All changes are in `src/checkers/magic_literal_context.rs` and
`src/checkers/rules.rs`; no new files, no schema changes, no `LangRuleConfig` fields.

## Architecture fit

The existing two-phase collect/emit architecture:
1. `walk_literals` — collects all literal nodes into `LiteralCollector`
2. `structural_exclusions` — returns `excluded: HashSet<usize>` of node IDs to skip
3. `build_literal_finding` — applies allowlist + exclusions, emits one finding per value

The three gaps slot into different phases:
- Gap 1 (format captures) → `is_allowlisted_literal` (allowlist extension)
- Gap 2 (`#[cfg(test)]` numerics) → `build_literal_finding` (threshold logic)
- Gap 3 (`1e0` normalization) → `normalize_numeric` (pre-allowlist normalization)

## Task breakdown

### Task 1 — Extend `normalize_numeric` for zero-exponent scientific notation

**File**: `src/checkers/magic_literal_context.rs`

After the radix-prefix early-return and the suffix-strip, add:
```rust
// Zero exponent: 1e0 → "1", 2E0 → "2", 1.0e0 → "1.0" (then trailing .0 strip)
if let Some((mantissa, exp)) = s.split_once(['e', 'E'])
    && exp.chars().all(|c| c == '0')
    && !exp.is_empty()
    && !mantissa.is_empty()
{
    s = mantissa.to_string();
}
```

This must come BEFORE the trailing-`.0` strip so that `1.0e0` → `"1.0"` → `"1"`.

Add unit tests to the `normalize_numeric` test block:
```rust
assert_eq!(normalize_numeric("1e0"), "1");
assert_eq!(normalize_numeric("2E0"), "2");
assert_eq!(normalize_numeric("1.0e0"), "1");
assert_eq!(normalize_numeric("1e-12"), "1e-12");  // non-zero exponent: unchanged
assert_eq!(normalize_numeric("2.5e3"), "2.5e3");  // non-zero exponent: unchanged
assert_eq!(normalize_numeric("1e1"), "1e1");       // non-zero exponent: unchanged
```

Add one integration test in `magic_literal_context.rs:tests`:
```rust
#[test]
fn numerics_should_BeSkipped_When_ZeroExponentNormalizesToAllowlisted() {
    for (lang, lit) in [
        (Language::Rust, "1e0"),
        (Language::Rust, "2E0"),
        (Language::Python, "1e0"),
    ] {
        assert_eq!(count(lang, "a", &repeated(lang, lit, 3)), 0, "{lit}");
    }
    // Non-trivial scientific notation still fires.
    let nontrivial = repeated(Language::Rust, "1e-12", 3);
    assert_eq!(count(Language::Rust, "a.rs", &nontrivial), 1);
}
```

**Estimate**: 1 h

---

### Task 2 — Add `is_pure_format_capture` to allowlist

**File**: `src/checkers/magic_literal_context.rs`

Add a function that returns `true` when a string's unquoted value is a single Rust/Python
format-capture placeholder. Conservative definition: the entire value (after the surrounding
`{…}`) must be at most one `{…}` group with no literal text outside it.

```rust
/// Returns `true` when `value` (unquoted string content) is a single Rust/Python
/// format-capture placeholder: the whole value is `{…}` with no text outside the
/// braces. Examples: `{p}`, `{:.6}`, `{:?}`, `{}`. Multi-segment strings like
/// `"{p} items"` are NOT matched.
pub(super) fn is_pure_format_capture(value: &str) -> bool {
    value.starts_with('{')
        && value.ends_with('}')
        && value.len() >= 2
        && !value[1..value.len() - 1].contains('{')
        && !value[1..value.len() - 1].contains('}')
}
```

**File**: `src/checkers/rules.rs`

Import `is_pure_format_capture` alongside `normalize_numeric`:
```rust
use crate::checkers::magic_literal_context::{
    is_pure_format_capture, normalize_numeric, structural_exclusions,
};
```

Extend `is_allowlisted_literal` to call it for non-numeric kinds:
```rust
fn is_allowlisted_literal(raw: &str, kind: &str, cfg: &LangRuleConfig) -> bool {
    let value = normalize_literal_value(raw, kind);
    if cfg.numeric_literal_kinds.contains(&kind) {
        let number = normalize_numeric(value);
        return MAGIC_LITERAL_ALLOWLIST.contains(&number.as_str())
            || MAGIC_LITERAL_NUMERIC_ALLOWLIST.contains(&number.as_str());
    }
    MAGIC_LITERAL_ALLOWLIST.contains(&value)
        || value.chars().count() == 1
        || is_pure_format_capture(value)   // NEW
}
```

Add unit tests for `is_pure_format_capture` in `magic_literal_context.rs`:
```rust
#[test]
fn is_pure_format_capture_should_ReturnTrue_When_ContentIsSinglePlaceholder() {
    for v in ["{p}", "{value}", "{:.6}", "{:?}", "{}", "{0}"] {
        assert!(is_pure_format_capture(v), "{v}");
    }
    for v in ["{p} items", "no braces", "{a} {b}", "{{p}}"] {
        assert!(!is_pure_format_capture(v), "{v}");
    }
}
```

Add integration test in `magic_literal_context.rs:tests`:
```rust
#[test]
fn strings_should_BeSkipped_When_PureFormatCapture() {
    // "{p}" appears 3 times — should not fire
    let rust = repeated(Language::Rust, "\"{p}\"", 3);
    assert_eq!(count(Language::Rust, "src/a.rs", &rust), 0);
    // "{:.6}" appears 3 times — should not fire
    let fmt = repeated(Language::Rust, "\"{:.6}\"", 3);
    assert_eq!(count(Language::Rust, "src/a.rs", &fmt), 0);
    // A real message string still fires
    let msg = repeated(Language::Rust, "\"some message\"", 3);
    assert_eq!(count(Language::Rust, "src/a.rs", &msg), 1);
}
```

**Estimate**: 1.5 h

---

### Task 3 — Apply test threshold to numerics exclusively in `#[cfg(test)]` items

**File**: `src/checkers/rules.rs`

The mechanism: in `build_literal_finding`, after filtering `remaining` occurrences, check
whether ALL remaining occurrences are inside a `#[cfg(test)]` item. If so, apply
`MAGIC_LITERAL_TEST_FILE_MIN_OCCURRENCES` as a floor (same as the test-file path bump).

This requires importing `in_cfg_test_item` from `magic_literal_context`:
```rust
use crate::checkers::magic_literal_context::{
    in_cfg_test_item, is_pure_format_capture, normalize_numeric, structural_exclusions,
};
```

Make `in_cfg_test_item` `pub(super)` (it's currently `fn`, private to the module).

In `build_literal_finding`, after computing `remaining`:
```rust
// Numerics exclusively inside #[cfg(test)] items in a production file get the
// test-file threshold, not the production one.  Only applies when skip_tests is on
// and the file isn't already classified as a test file (which already sets test_bump).
if filter.test_bump == 0              // not already in a test file
    && !cfg.numeric_literal_kinds.is_empty()   // numeric-capable language
    && cfg.numeric_literal_kinds.contains(&first.kind)
    && !remaining.is_empty()
    && remaining
        .iter()
        .all(|occ| in_cfg_test_item(occ.node, src))
{
    bumps = bumps.max(MAGIC_LITERAL_TEST_FILE_MIN_OCCURRENCES);
}
```

Wait — `build_literal_finding` doesn't have `src` in scope. It receives `(text, occurrences, cfg, filter)`.
`filter` has no `src` field. Add `src: &[u8]` to `LiteralFilter` or pass it separately.

Preferred: add `src: &[u8]` to `LiteralFilter`. It's only used in `build_literal_finding`
for this new check, so memory cost is one `&[u8]` pointer.

Alternatively, pass it as a separate argument to `build_literal_finding`. Since
`emit_literal_findings` already takes both `collector` and `filter`, it's cleaner to
extend `LiteralFilter`:
```rust
struct LiteralFilter<'a> {
    // ... existing fields ...
    src: &'a [u8],   // NEW: needed for in_cfg_test_item
}
```

And in `SyntaxRulesChecker::check`:
```rust
let filter = self.literal_filter(kind, scopes.as_ref(), &excluded, &collector.slog_key_ids, src);
```

Add `src: &'a [u8]` parameter to `literal_filter` and thread it through.

Add integration test in `magic_literal_context.rs:tests`:
```rust
#[test]
fn numerics_should_UseCfgTestThreshold_When_InsideRustCfgTestItem() {
    // 5 occurrences in #[cfg(test)] only — below test threshold (10), above prod (3)
    let body = repeated(Language::Rust, "12345", 5);
    let in_cfg = format!("#[cfg(test)]\nmod t {{\n{body}}}\n");
    assert_eq!(count(Language::Rust, "src/a.rs", &in_cfg), 0);
    // 10 occurrences in #[cfg(test)] — at or above test threshold
    let many = repeated(Language::Rust, "12345", 10);
    let in_cfg_many = format!("#[cfg(test)]\nmod t {{\n{many}}}\n");
    assert_eq!(count(Language::Rust, "src/a.rs", &in_cfg_many), 1);
    // Same value 3x in production code + 2x in cfg(test) — fires at prod count
    let prod = repeated(Language::Rust, "12345", 3);
    let mixed = format!("{prod}\n#[cfg(test)]\nmod t {{\n{}\n}}\n",
                        repeated(Language::Rust, "12345", 2));
    assert_eq!(count(Language::Rust, "src/a.rs", &mixed), 1);
}
```

**Estimate**: 2 h

---

### Task 4 — Update `docs/syntax-rules.md`

Add three bullets to the "second round" node-context exclusions list
(`docs/syntax-rules.md:184-206`):

```
- **Format-arg captures**: a string whose entire content is a single Rust/Python format
  placeholder (`{ident}`, `{:spec}`, `{ident:spec}`, `{}`) is exempt regardless of
  occurrence count.
- **Numerics in `#[cfg(test)]` items within production `.rs` files**: treated with the
  test-file threshold (>= 10), not the production threshold (>= 3), when *all* remaining
  occurrences sit inside `#[cfg(test)]` items. A value that also appears in production
  code uses the production threshold for the production occurrences.
```

Update the normalize bullet to mention zero-exponent:
```
- **Type-suffixed or float spellings of allowlisted numbers**: ... `1e0` and `2e0` (zero
  exponent) are also treated as `1`/`2`.
```

**Estimate**: 0.5 h

---

### Task 5 — Backtest and record results

Run `kibitzer check backtest replace-magic-literal` against transcript history to verify:
- The format-capture exclusion does not misfire on legitimate repeated strings
- The `#[cfg(test)]` numeric adjustment shows reduction in noise without losing true positives

Record results in a new `docs/backtest-triage/<date>/fix-magic-literal-noise.md` file.

**Estimate**: 1 h

---

## Adversarial review

### Challenge 1: Is `in_cfg_test_item` correct for Task 3's "all occurrences in test" check?

`in_cfg_test_item` walks ancestors looking for a `#[cfg(test)]` sibling — it's O(depth)
per call and called per-occurrence. For a file with 100 occurrences of a value all in
`#[cfg(test)]`, that's 100 ancestor-walks. The existing code already calls it per-node in
`structural_exclusions` for strings, so this is not a new O(n) risk. Acceptable.

### Challenge 2: Could format-capture exclusion hide a real magic string?

Example: `format!("{p}")` repeated 3+ times where `p` is a protocol field name. Excluding
`"{p}"` means you don't notice that `p`'s format template is repeated. However, the
relevant smell is the CALL being repeated, not the format string — the `replace-magic-literal`
rule is about the *literal value*, not the call pattern. If `p` is really a named constant,
the fix is to name the variable `p` via a type or constant, not to name the format string.
This is an acceptable false-negative.

### Challenge 3: Does Task 3 cover the `root_is_cfg_test` case (whole file is test-only)?

When `#![cfg(test)]` makes the whole file test code, `root_is_cfg_test` returns true and
ALL strings are excluded. Numerics are NOT excluded in this path. After Task 3, numerics
in this case would still use the production threshold unless the "all occurrences in
cfg_test" check also fires for `root_is_cfg_test`. But since `root_is_cfg_test` means
the ENTIRE file is `#[cfg(test)]`, every occurrence satisfies `in_cfg_test_item`. The
Task 3 check would apply and give the test threshold. Correct behavior.

### Challenge 4: Task 3 adds `src` to `LiteralFilter` — is this a breaking change?

`LiteralFilter` is `pub(crate)` (no, actually it's `struct LiteralFilter<'a>` with no `pub`
— it's private to `rules.rs`). The only callsite is `SyntaxRulesChecker::check`. Adding
a field and a parameter to the constructor is a private API change with no external impact.

## Alternatives not chosen

- **Add `is_pure_format_capture` as a Bash slog-style predicate in `LangRuleConfig`**:
  over-engineered — format captures are a Rust/Python phenomenon not needing per-language
  dispatch. A simple `is_allowlisted_literal` check suffices.
- **Exclude ALL string literals in `#[cfg(test)]` from numeric threshold bump**:
  would be simpler but loses the distinction between "only in test" vs. "also in production"
  (AC 5 requires production occurrences to still fire).
- **Add `2.0`/`0.5` to the numeric allowlist**:
  `2.0` already handled by trailing-`.0` strip. `0.5` is a non-trivial value (a halfway
  threshold, a probability) and should flag when repeated — an accepted true positive cost.
