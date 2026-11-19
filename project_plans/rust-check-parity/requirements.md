# Requirements: rust-check-parity

**Date:** 2026-11-19  
**Branch:** `feat/rust-check-parity`  
**Item ID:** 5214a27b-3898-45af-b7c2-8bb97921180a

---

## Background

Kibitzer's default check catalog has grown language-asymmetrically: Go, Java, and TypeScript each have dedicated error-handling, test-quality, and structural-smell checks that `.rs` files never see. Rust was added as a `Language` variant and received `rust-file-size`, `syntax-rules-rust`, and `comment-quality-rust`, but the per-language gap is wider than those three. Additionally, the history of silently dropping Rust from internal language dispatches (`arch_model.rs`, `arch_export.rs`) means zero findings on a Rust file is currently ambiguous — it could mean clean code or simply that the checks didn't run.

This tracker captures the full set of work needed to close the gap.

---

## Goals

1. Every check category that makes semantic sense for Rust has a Rust-flavored implementation wired into `default_checks()`.
2. Checks that have no sound Rust analogue are recorded as N/A with an explicit rationale in `docs/rust-parity.md`.
3. Running kibitzer on a Rust file and seeing zero findings is not mistaken for "Rust checking skipped" — a runtime signal (coverage note or stats line) makes check activity visible.

---

## In Scope

### Gap: test-no-assertion-rust
`src/test_code/rust.rs` already has a Rust profile (assertion names, snapshot markers, `#[test]` detection). The `test-no-assertion` SUPPORTED list in `src/checkers/test_no_assertion.rs:30` is `[Go, Python, Kotlin]` — Rust is not there. Adding Rust is small.

### Gap: primitive-obsession (Rust)
`primitive_obsession.rs` currently reads `Some(Language::Go)` only. Rust structs carrying raw primitives instead of named types is an identical smell.

### Gap: file-complexity (Rust)
`complexity.rs` only handles Go. Cyclomatic/structural complexity for `fn` items in Rust is a direct port.

### Gap: sentinel-return-rust
`sentinel_return.rs` exists for Go, Java, TypeScript, Python, Kotlin. A Rust variant flags functions returning `Option<T>` or `Result<T, E>` via literal `None`/`Err(...)` sentinels instead of propagating with `?`.

### Gap: rust-ignored-error
No `rust-ignored-error` checker exists. Go and Java each have one. Rust equivalent: `let _ = expr_returning_result` and `.unwrap()` / `.expect()` calls in non-test code (which suppress the compiler's `#[must_use]` without providing context).

### Gap: rust-error-context
No `rust-error-context` checker. Go's `go-error-context` flags errors propagated without wrapping. Rust equivalent: `?` on a `Result` without an `.context()`/`.with_context()` call when the return type uses `anyhow::Error` or a similar rich-context type.

### Gap: split-loop-rust
`split_loop.rs` names `SplitLoopChecker` and `Language::Go` only. Rust `for` / `while` loops advancing independent accumulators is an identical smell — the checker's algorithm is grammar-parametric enough to extend.

### Runtime-signal for zero Rust findings
When kibitzer runs against a repo with `.rs` files and all Rust checks produce no findings, it should emit a coverage note (e.g., "checked N .rs files, 0 findings"). This mirrors the existing stat line pattern and prevents silent-miss confusion.

### Parity table: docs/rust-parity.md + src/rust_parity.rs
A generated Markdown table listing every check category, its Rust status (Shipped / Planned / N/A), and the rationale for N/A rows. Generated from a `src/rust_parity.rs` data module so the table stays in sync with the code.

---

## Out of Scope

- Declarations/hierarchy graph for Rust (`declarations.rs:118` explicitly notes this as a separate, not-yet-built feature requiring `symbol_extract.rs` work) — that is a larger architectural lift tracked separately.
- `go-type-switch-clustering` analogue for Rust `match` — deferred until a backtest against `servo/servo` or `ripgrep` confirms signal quality.
- `go-bulk-fetch-linear-scan` and `go-table-driven-test-candidate` — Go-idiom specific, no sound Rust analogue.
- `introduce-special-case` for Rust — requires the declarations graph (see above).
- `go-encapsulate-collection` / `go-encapsulate-variable` — Go package-level variable semantics don't translate to Rust.

---

## Acceptance Criteria

1. `test-no-assertion-rust` is wired into `default_checks()` and fires correctly on a `#[test]` function containing no assertion.
2. `primitive-obsession` fires on a Rust struct with multiple `String`/`u64`/`bool` fields in place of named types, and the check name appears in `default_checks()`.
3. `file-complexity` fires on a Rust function that exceeds the complexity threshold, and the check appears in `default_checks()`.
4. `sentinel-return-rust` fires on a Rust function that returns `Err(...)` or `None` as a sentinel in a pattern that should use `?`, and appears in `default_checks()`.
5. `rust-ignored-error` fires on `let _ = fallible_call()` and on `.unwrap()` / `.expect()` in non-test Rust code, and appears in `default_checks()`.
6. `rust-error-context` fires when a `?` propagation inside an `anyhow`-returning function lacks `.context()`, and appears in `default_checks()`.
7. `split-loop-rust` fires on a Rust `for` loop advancing two independent accumulators, and appears in `default_checks()`.
8. Running kibitzer on a `.rs`-only repo that produces zero findings emits a coverage note identifying the number of Rust files checked.
9. `docs/rust-parity.md` exists, is generated from `src/rust_parity.rs`, and every check in `default_checks()` with a Rust glob appears in the table with status "Shipped"; every intentional gap has an N/A entry with a rationale.
10. Every new Rust checker has been backtested against the standard corpus (`servo/servo`, `ripgrep`, `deno`) with before/after numbers recorded in its `docs/<name>-false-positives.md`.
11. `default_checks_ids_are_unique` test still passes; no regression in existing check counts.
