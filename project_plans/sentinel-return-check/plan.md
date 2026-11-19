# Plan: sentinel-return-check

**Date:** 2026-11-10

## Overview

Implement a new advisory-severity native checker, `sentinel-return-<lang>`, for five
exception-based languages (Java, Python, Kotlin, TypeScript, JavaScript) that flags a
function returning a sentinel value (`null`/`None` or a negative integer literal)
specifically from inside a `catch`/`except` block — the structural signal for Fowler's
Replace Error Code with Exception. Following the existing `java-ignored-error` /
`java-lost-exception-cause` precedent: one struct per language, per-language tree-sitter
typed node kind enums, `inventory::submit!` registration, advisory severity, narrow
heuristic, backtest before shipping.

## Tasks

### T1: Java checker (`src/checkers/sentinel_return_java.rs`)

Implement `SentinelReturnJavaChecker`:
- Walk the tree; for each `catch_clause` node, recurse its body for `return_statement`
  nodes at direct depth (not nested in an inner function/lambda).
- Flag if the returned expression is a `null_literal` OR a unary `-` expression over a
  `decimal_integer_literal`.
- Suppress if the enclosing method name contains "index", "find", or "search"
  (case-insensitive) — the `indexOf` family of contract-negative-return conventions.
- Register as `sentinel-return-java` via `inventory::submit!`.
- Unit tests: true-positive (catch returns `-1`), true-positive (catch returns `null`),
  true-negative (catch throws new exception), true-negative (try body throws, no catch return).

### T2: Python checker (`src/checkers/sentinel_return_python.rs`)

Implement `SentinelReturnPythonChecker`:
- Walk; for each `except_clause`'s sibling `block`, find `return_statement` nodes.
- Flag if return value is `none` literal OR unary `-` over `integer` literal.
- Same method-name suppression heuristic.
- Register as `sentinel-return-python`.
- Unit tests: true-positive (except returns `None`), true-negative (raises).

### T3: Kotlin checker (`src/checkers/sentinel_return_kotlin.rs`)

Implement `SentinelReturnKotlinChecker`:
- Walk; for each `catch_block`, find `return_expression` nodes.
- Flag if return value is `null` literal OR unary `-` over `integer_literal`.
- Suppress if the enclosing function declares a nullable return type (`T?` — present as
  the `?` annotation on the return type node): returning `null` from a nullable-typed
  function is idiomatic Kotlin.
- Register as `sentinel-return-kotlin`.
- Unit tests: true-positive, true-negative (nullable return type), true-negative (throws).

### T4: TypeScript checker (`src/checkers/sentinel_return_typescript.rs`)

Implement `SentinelReturnTypeScriptChecker`:
- Walk; for each `catch_clause`, find `return_statement` in its body.
- Flag if return value is `null` literal OR unary `-` over `number` literal.
- Same method-name suppression.
- Register as `sentinel-return-typescript`.
- Share logic with JS via a free function `check_js_family_source(src, lang)` to avoid duplication.

### T5: JavaScript checker (`src/checkers/sentinel_return_javascript.rs`)

Implement `SentinelReturnJavaScriptChecker`:
- Delegates to the shared `check_js_family_source` from T4.
- Register as `sentinel-return-javascript`.
- Unit tests covering both `.js` and `.jsx` patterns.

### T6: Wire into `default_checks()` (`src/config.rs`)

Add a new `sentinel_return_checks()` function alongside `java_exception_checks()`:
```rust
fn sentinel_return_checks() -> Vec<Check> {
    vec![
        native_check("sentinel-return-java", Severity::Advisory,
            &["**/*.java", "!**/test/**"]),
        native_check("sentinel-return-python", Severity::Advisory, &["**/*.py"]),
        native_check("sentinel-return-kotlin", Severity::Advisory,
            &["**/*.kt", "**/*.kts"]),
        native_check("sentinel-return-typescript", Severity::Advisory,
            &["**/*.ts", "**/*.tsx"]),
        native_check("sentinel-return-javascript", Severity::Advisory,
            &["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"]),
    ]
}
```
Chain `sentinel_return_checks()` in `default_checks()`. Add all five checker modules to
`src/checkers/mod.rs`. Update `src/config.rs`'s `default_checks_ids_are_unique` test count.

### T7: False-positive documentation (`docs/sentinel-return-false-positives.md`)

Write the false-positive doc before the backtest numbers are known, documenting the
known structural patterns that produce false positives (see research.md):
- `indexOf`/`find`/`search` family (suppressed by name heuristic).
- Kotlin nullable return type (suppressed by T3's nullable-return-type check).
- Factory methods returning `null` for "not found" (not suppressed in v1 — advisory).
- Test code that uses catch-and-return in assertions.

Leave a placeholder section for backtest numbers to be filled in after T8.

### T8: Corpus backtest and false-positive triage

Run `kibitzer check backtest sentinel-return-java` against the transcript corpus.
Run `kibitzer run ~/code/github.com/apache/cassandra` and filter for `[sentinel-return-java]`.
Run against `microsoft/vscode` for TypeScript.
Run against `android/nowinandroid` for Kotlin.
Record findings and false-positive rate in `docs/sentinel-return-false-positives.md`.
If the false-positive rate exceeds 40% for any language, either narrow the heuristic
(e.g. require the function to have a non-void/non-nullable return type annotation) or
move that language to opt-in only (not in `default_checks()`).

## Adversarial review

**Risk: Kotlin nullable-return suppression is fragile.** The `T?` check requires
navigating the function return-type annotation node in the Kotlin grammar. If the
tree-sitter Kotlin grammar represents nullable types differently (e.g., as a
`nullable_type` wrapper rather than a `?` punctuation child), the suppression silently
fails and every `fun parse(): Int? { ... catch { return null } }` fires. Mitigation:
write an explicit unit test for the nullable-type suppression case and check the actual
tree-sitter node shape in `codegen/node-types/kotlin-node-types.json` before coding.

**Risk: `-` unary expression walking differs by grammar.** The `rules.rs` module already
handles this (`normalize_numeric`/`structural_exclusions` in `magic_literal_context.rs`),
confirming the unary-wrapper shape is consistent across grammars. Reuse or replicate that
logic rather than re-implementing it independently.

**Risk: False-positive rate may exceed 40% in Java without the `!**/test/**` exclusion.**
`java-ignored-error` needed this after backtesting; `sentinel-return-java` likely will too.
Pre-emptively include `!**/test/**` in T6 for Java and Kotlin; adjust after T8 for others.

**Risk: `apache/cassandra` doesn't have the checker corpus yet.** The check
`sentinel-return-java` is new; there's no cached backtest result. The first run will be
slow. Use `scripts/backtest-triage.py` to track results across runs rather than
re-reading raw output.

**Risk: Per-language checker count test.** `src/config.rs` has a test (`default_checks_ids_are_unique`) that asserts an exact count. Adding five new checks will break it. The fix is mechanical (increment the count), but it's easy to miss.

## Estimated effort

| Task | Category | Estimate |
|------|----------|----------|
| T1 Java checker + tests | backend | 3h |
| T2 Python checker + tests | backend | 2h |
| T3 Kotlin checker + tests (with nullable-return suppression) | backend | 3h |
| T4 TypeScript checker + shared JS/TS logic | backend | 2h |
| T5 JavaScript checker + tests | backend | 1h |
| T6 Wire into default_checks() + mod.rs + fix count test | backend | 1h |
| T7 False-positive docs (pre-backtest skeleton) | docs | 1h |
| T8 Corpus backtest + triage + fill in docs | test | 3h |

Total: ~16h
