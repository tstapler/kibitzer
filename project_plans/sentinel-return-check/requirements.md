# Requirements: sentinel-return-check

**Item:** Native check: Replace Error Code with Exception (sentinel-return pattern, non-Go)
**Date:** 2026-11-10
**Project name:** `sentinel-return-check`

## Problem

In exception-based languages (Java, Python, Kotlin, JS/TS — explicitly **not** Go), functions that return a sentinel value on failure (`-1`, `null`, a magic error code) instead of raising an exception force every caller to remember to check for it. This is Fowler's **Replace Error Code with Exception** refactoring. The pattern leaks error handling policy into every call site, often silently dropped or checked inconsistently, while exception-based languages provide a structured mechanism specifically for this purpose.

## Goal

Add a new per-file native checker (`replace-error-code`) to kibitzer's `src/checkers/` that flags functions in non-Go files whose body contains an explicit early-return of a sentinel value (a small integer constant or `null`/`None`) specifically in an error-handling-shaped branch (`catch`/`except` block, or a branch containing an error-keyed comment/identifier), surfacing them as Replace Error Code with Exception candidates.

## Scope

### In scope
- **v1**: Flag the sentinel-return pattern alone — a function whose body contains an early return of a sentinel value inside an error-handling branch — without requiring call-site cross-referencing.
- Target languages: Java (`*.java`), Python (`*.py`), Kotlin (`*.kt`), TypeScript (`*.ts`, `*.tsx`), JavaScript (`*.js`, `*.jsx`).
- Advisory severity (style nudge, not a correctness bug — matches `java-error-context` precedent).
- Register in `default_checks()` via a new `non_go_sentinel_return_checks()` function in `src/config.rs`, matching the existing `java_exception_checks()` pattern.
- Backtest against `apache/cassandra` (Java), `android/nowinandroid` (Kotlin), `microsoft/vscode` (TypeScript), and the transcript corpus before shipping.
- Document false positives in `docs/sentinel-return-false-positives.md`.

### Out of scope
- Go — Go's `(value, error)` return convention is idiomatic; applying this rule to Go contradicts `go-ignored-error`/`go-error-context`.
- Cross-file call-site verification (nice-to-have v2 refinement, explicitly not a blocker per the item description).
- Rust — Rust's `Result<T, E>` return convention is idiomatic (same reasoning as Go).

## Acceptance criteria

1. A new `SentinelReturnChecker` struct in `src/checkers/sentinel_return.rs` implements the `Checker` trait and fires on a function that returns a sentinel value from inside a `catch`/`except` block (or error-keyed branch) in Java, Python, Kotlin, TypeScript, and JavaScript source files.
2. The checker does **not** fire on Go, Rust, or Bash files.
3. The checker is registered via `inventory::submit!` and reachable by `kibitzer check native sentinel-return-<lang>` (or a single multi-language checker name, whichever design the implementation settles on).
4. At least one per-language unit test using `crate::test_support::check_<lang>_source` covers the true-positive case (sentinel return inside error branch) and the true-negative case (exception thrown instead).
5. The checker is wired into `default_checks()` in `src/config.rs` with `Severity::Advisory` for all five target languages, excluding test files (following `java-ignored-error`'s `!**/test/**` pattern where backtest noise warrants it).
6. A `docs/sentinel-return-false-positives.md` documents known false-positive scenarios (e.g. legitimate factory methods returning `null`/`-1` by convention, `Optional.empty()` equivalents).
7. A corpus backtest against `apache/cassandra` and at least one TypeScript/JS repo (e.g. `microsoft/vscode`) produces a false-positive rate under 40% (kibitzer's existing bar per ADR-004 backtest discipline), with findings and triage numbers recorded in `docs/sentinel-return-false-positives.md`.
8. `make ready` passes (ansible-lint + shellcheck; `cargo test` passes for the Rust crate).

## Non-goals

- Automatically inserting `throw`/`raise` statements — this is an advisory hint only.
- Tracking which call sites fail to check the return value (a v2 enhancement).
- Flagging Rust's `Option::None` / `Err(...)` returns (idiomatic Rust, same exclusion logic as Go).
