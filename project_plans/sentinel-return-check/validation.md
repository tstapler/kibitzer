# Validation: sentinel-return-check

**Date:** 2026-11-10

## Validation against requirements

### AC1 — `SentinelReturnChecker` fires on sentinel return in catch/except across five languages

**Validation method:** Unit tests (T1–T5) using `crate::test_support::check_<lang>_source`. Each language checker gets at least one true-positive test case with a `return -1` or `return null`/`return None` inside a catch/except block, and one true-negative (exception thrown instead of sentinel return).

**Risk:** Python's `except_clause` node and its sibling `block` may not be directly parent/child — depends on exact grammar shape. Must inspect `codegen/node-types/python-node-types.json` before writing the walker.

### AC2 — Checker does NOT fire on Go, Rust, or Bash files

**Validation method:** Each checker declares `fn language() -> Option<Language>` returning the specific language, and `fn file_globs()` restricting to that language's extensions. The existing `checker.rs` machinery ensures `check()` is only invoked for matching files. Unit tests for the Go and Rust non-fire case are low value (the platform guarantees it); the language/glob restriction is the structural guarantee.

### AC3 — Registered and reachable by `kibitzer check native sentinel-return-<lang>`

**Validation method:** `inventory::submit!` wires each checker into the global registry (confirmed by existing checker pattern: `java_ignored_error.rs` line 54–56). After `cargo build`, `kibitzer check list` should show all five. No dedicated test needed beyond the registry unit test pattern already in `src/check/native_check_tests.rs`.

### AC4 — Per-language unit tests (true-positive and true-negative)

**Validation method:** Standard pattern — a `#[cfg(test)]` module in each checker file, using `crate::test_support::check_java_source` / `check_python_source` / etc., covering:
- True positive: catch/except block returns sentinel.
- True negative: catch/except block throws/raises instead.
- True negative (Kotlin only): function with nullable return type `T?` returns `null` from catch.
- True negative: sentinel returned in a non-error branch (e.g., a normal `if x < 0 { return -1; }` outside a catch).

### AC5 — Wired into `default_checks()` as Advisory

**Validation method:** `src/config.rs` has `default_checks_ids_are_unique` and a count assertion test. After T6, run `cargo test` and confirm the count test passes (it must be incremented by 5). Check that `kibitzer run` on a test file triggers the new checks.

### AC6 — `docs/sentinel-return-false-positives.md` documents known false-positive scenarios

**Validation method:** File exists, covers at least: indexOf/find/search name heuristic suppression, Kotlin nullable return type suppression, factory-method null pattern, and test-code exclusion.

### AC7 — Backtest false-positive rate ≤40%

**Validation method:** Run T8 backtest procedure. Record before/after in the false-positive doc. If rate exceeds 40% for a language, either add suppressions or move to opt-in. This is the binding gate before wiring into `default_checks()` — it's acceptable to land the checker registered (via `inventory::submit!`) but not yet in `default_checks()` pending this validation.

### AC8 — `make ready` passes

**Validation method:** Run `make ready` after all source changes. This runs `ansible-lint` and `shellcheck` over the bootstrap (unchanged by this PR), and `cargo test` for the Rust crate.

## Open questions / risks

1. **Grammar shape for Python `except` body**: Does tree-sitter-python represent `except ValueError: return None` as `except_clause → (identifier, block)` or differently? Must inspect the grammar before writing the Python walker. If the grammar wraps the except body in a `try_statement`'s `except` branch rather than a direct `except_clause`, the walker logic differs.

2. **Kotlin nullable return type node**: Does tree-sitter-kotlin expose the `?` nullability marker as a typed node kind (e.g. `KotlinKind::NullableType`)? If the grammar doesn't name it cleanly, the suppression heuristic must fall back to scanning the return-type text for `?`.

3. **TypeScript/JS shared logic**: The TS and JS grammars in kibitzer use separate tree-sitter parsers (`tree-sitter-typescript` vs. `tree-sitter-javascript`). The node kind enums are separate (`TypeScriptKind`, `JavaScriptKind`) but share the same underlying node names. A free function parameterized by the language kind would work but requires generic dispatch or duplicated match arms. The cleaner option is duplicating the small walker per language (already done by existing checkers like `duplicate_code.rs`).

4. **`sentinel-return-python` language scope**: The `Language::Python` variant already exists in `checker.rs`. Python file globs (`**/*.py`) are already used by `primitive-obsession`'s Go-only exclusion and `syntax-rules-python`. No new grammar variant is needed.

5. **`-1` vs. small-integer generalization**: The proposal flags negative-integer literals. Should `0` also be flagged? In many APIs, `0` is a valid success return (e.g. `int exitCode() { ... catch { return 0; }`), so `0` is too ambiguous for v1. Restrict to strictly negative integers (the unary-minus shape) for now.

## Verdict

This plan is implementable with the existing checker infrastructure. Five-language scope is the main size risk — if false-positive rates prove high for JS/TS after backtesting, those languages can be deferred to v2 (move to opt-in). Kotlin's nullable-type suppression is the single most fragile heuristic and must be validated with an explicit grammar-node inspection before coding.

**Recommendation:** Proceed with Java first (most backtest corpus coverage), then Python, then Kotlin+TS+JS in order of risk. Each can land as its own PR following kibitzer's merge-before-tag release convention.
