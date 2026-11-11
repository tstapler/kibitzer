# Research: sentinel-return-check

**Date:** 2026-11-10

## Fowler's Replace Error Code with Exception

Fowler's *Refactoring* (2nd ed.) catalogs "Replace Error Code with Exception" as a refactoring for the case where a function returns a special value to signal an error when it should throw an exception instead. The core smell: the function communicates both a result and an error signal through its return type, forcing every caller to inspect and branch on a sentinel. This is distinct from Go's `(value, error)` pattern (idiomatic multi-return) and from Rust's `Result<T,E>` (idiomatic sum type) — both give callers a typed, compiler-enforced contract, whereas a raw `int` return where `-1` means "error" provides no such contract.

## Prior art in existing kibitzer checks

The repo already has a family of exception-related Java checks that share the structural approach this new checker should follow:

- **`java-ignored-error`** (`src/checkers/java_ignored_error.rs`): flags empty `catch` blocks. Advisory, excludes `**/test/**` after backtest found 86% of raw hits were JUnit try/catch fixtures.
- **`java-error-context`** (`src/checkers/java_error_context.rs`): flags bare `catch (X e) { throw e; }` rethrows. Requires a same-file wrapping convention signal to avoid false positives.
- **`java-lost-exception-cause`** (`src/checkers/java_lost_exception_cause.rs`): flags `throw new SomeException("msg")` inside catch that drops the original cause.
- **`java-swallowed-interrupt`** (`src/checkers/java_swallowed_interrupt.rs`): flags `catch (InterruptedException ...)` that neither restores interrupt status nor rethrows.

The existing pattern is: single-language struct, implements `Checker`, uses tree-sitter typed node kinds (`JavaKind`, etc.), `inventory::submit!` for registration, advisory severity, narrow heuristic to stay below the 40% false-positive bar established in ADR-004.

## SonarQube and PMD prior art

SonarQube's `java:S1193` ("Exception types should not be tested using instanceof") and `java:S1166` ("Exception handlers should preserve the original exception") are adjacent. The direct analogue is SonarQube `S2272` and PMD's `ReturnFromFinallyBlock` family. For the sentinel-return pattern specifically, SonarQube's `java:S2272` flags methods that return `-1` or `null` from a catch block; it uses a simple structural check (catch block → return of a negative literal or null). This validates the structural approach proposed in the item description.

## Tree-sitter grammar coverage

Kibitzer already has tree-sitter grammars compiled for all five target languages: `tree-sitter-java`, `tree-sitter-python`, `tree-sitter-kotlin`, `tree-sitter-typescript` (TS + TSX), `tree-sitter-javascript`. The node kind enums (`JavaKind`, `PythonKind`, `KotlinKind`, `TypeScriptKind`, `JavaScriptKind`) are generated at build time from `codegen/node-types/*.json` and available as typed enums for compile-time safety.

Relevant node types for the sentinel-return detection:
- **Java**: `catch_clause` (wraps `catch` body), `return_statement`, integer literals (`decimal_integer_literal`), `null_literal`.
- **Python**: `except_clause` (Python `except` block), `return_statement`, integer literals (`integer`), `none` literal.
- **Kotlin**: `catch_block`, `return_expression`, integer literals (`integer_literal`), `null` literal.
- **TypeScript/JS**: `catch_clause`, `return_statement`, `number` literal, `null` literal.

A common sentinel-return pattern to detect:
```java
// Java
int parse(String s) {
    try {
        return Integer.parseInt(s);
    } catch (NumberFormatException e) {
        return -1;  // ← flag this
    }
}
```
```python
# Python
def parse(s):
    try:
        return int(s)
    except ValueError:
        return None  # ← flag this
```

## False-positive landscape

The hardest false positives, based on analogous checks in the codebase and SonarQube's experience with `java:S2272`:

1. **`indexOf`/`search` conventions** — Java's `String.indexOf()`, Python's `str.find()`, JS's `Array.prototype.indexOf()` return `-1` by contract. A wrapper that delegates to these and also returns `-1` from its own catch is a true false positive. Mitigation: check if the function name contains "index"/"find"/"search" and suppress.
2. **`Optional`/nullable factory methods** — a `static Foo fromString(String s)` that returns `null` from a catch is a legitimate "returns-null-for-invalid" factory. These are hard to suppress without type resolution.
3. **Kotlin's nullable return type `T?`** — `fun parse(s: String): Int?` returning `null` from a catch is idiomatic Kotlin for nullable return. Mitigation: check whether the declared return type is nullable (`T?`) and suppress if so.
4. **Test code** — JUnit-style tests that deliberately catch exceptions and return sentinel codes. Mitigation: exclude `**/test/**` as `java-ignored-error` does.
5. **Legacy code patterns that callers depend on** — the checker is advisory only, not blocking, so this is acceptable.

## Implementation strategy

**v1 scope: catch/except block → return sentinel**

The narrowest version that avoids call-site analysis: flag a `return` statement of a sentinel value (small negative integer literal or `null`/`None`) that appears *directly inside* a `catch`/`except` block. This covers the most common and least ambiguous shape. It does not require finding call sites.

**Signal needed:**
- The containing function exists (to avoid flagging top-level returns in Python scripts).
- The `return` is inside a `catch`/`except` node (not a general error-keyed branch).
- The return value is a small negative integer literal (e.g., `-1`, `-2`, literal `0` only if the function already throws/raises elsewhere — but `0` is too ambiguous for v1) or `null`/`None`.

**v1 simplification:** Only flag `null`/`None` returns and negative-integer returns (via `unary_expression` wrapping as confirmed in the `rules.rs` comment: "every one of the 8 grammars tokenizes a negative numeric literal as a unary_expression/`-`-prefixed wrapper"). This matches the SonarQube approach and avoids the `-1` allowlist headache by only flagging unambiguously sentinel-shaped values.

## Multi-language vs. single checker

Two options:
1. Five separate `SentinelReturnJavaChecker`, `SentinelReturnPythonChecker`, etc. — matches `java-ignored-error`'s single-language-per-struct precedent, allows per-language tuning of sentinel values and node kinds.
2. One `SentinelReturnChecker` with a `language()` returning `None` and a `file_globs()` covering all five — simpler, but can't tune per language.

Option 1 is preferred: per-language structs follow the established codebase pattern, allow each language's checker to be suppressed or scoped independently in `.kibitzer/inspect.json`, and let per-language false-positive patterns be addressed independently. The check names follow the pattern `sentinel-return-java`, `sentinel-return-python`, etc.

## Backtest strategy

Required per CLAUDE.md:
1. `kibitzer check backtest sentinel-return-java` against transcript corpus after implementation.
2. Corpus runs against `apache/cassandra` (Java), `android/nowinandroid` (Kotlin), `microsoft/vscode` (TypeScript). Python and JS have no dedicated corpus entries yet — accept as known gap, run against a subset of `denoland/deno`'s TypeScript.
3. Record before/after false-positive rates in `docs/sentinel-return-false-positives.md`.
4. Must achieve ≤40% false-positive rate (ADR-004 bar) or restrict scope further.
