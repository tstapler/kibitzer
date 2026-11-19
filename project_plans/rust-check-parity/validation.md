# Validation: rust-check-parity

**Date:** 2026-11-19

---

## Requirements traceability

| AC | Requirement | Plan task | Validation method |
|---|---|---|---|
| AC1 | `test-no-assertion-rust` in `default_checks()`, fires on assertion-free test | Task 1.1 | Unit test fixture + `default_checks_ids_are_unique` |
| AC2 | `primitive-obsession` fires on Rust multi-primitive struct | Task 2.1 | Unit test fixture in `primitive_obsession.rs` |
| AC3 | `file-complexity` fires on complex Rust function | Task 2.2 | Unit test fixture in `complexity.rs` |
| AC4 | `sentinel-return-rust` fires on explicit Err/None return | Task 2.3 | Unit test fixture in `sentinel_return_rust.rs` |
| AC5 | `rust-ignored-error` fires on `let _ = fallible()` and `.unwrap()` | Task 3.1 | Unit test fixture in `rust_ignored_error.rs` |
| AC6 | `rust-error-context` fires on bare `?` in anyhow function | Task 3.2 | Unit test fixture in `rust_error_context.rs` |
| AC7 | `split-loop-rust` fires on independent-accumulator loop | Task 3.3 | Unit test fixture in `split_loop_rust.rs` |
| AC8 | Zero-findings coverage note emitted | Task 4.1 | Integration test: run against fixture Rust-only repo |
| AC9 | `docs/rust-parity.md` generated from `src/rust_parity.rs` | Task 5.1 | `#[test]` renders table and diffs against committed file |
| AC10 | All new checkers backtested against servo/servo, ripgrep | Tasks 1.1–3.3 | Backtest numbers in per-check FP docs |
| AC11 | Existing tests still pass | All tasks | `cargo test` gate on every PR |

---

## Validation test plan

### Unit test per new checker
Each new or extended checker file must add an inline `#[cfg(test)] mod tests` block covering:
- **True positive:** minimal Rust source that exhibits the smell; assert at least one finding.
- **True negative:** idiomatic Rust source that does not exhibit the smell; assert zero findings.
- **Edge cases specific to the check** (documented below).

#### test-no-assertion-rust edge cases
- `#[should_panic]` → no finding (exemption marker)
- `#[ignore]` → no finding
- `assert_snapshot!` (insta) → no finding (snapshot marker counts as assertion)
- Helper function called from test, not itself annotated `#[test]` → no finding

#### primitive-obsession-rust edge cases
- Struct with 2 primitive fields → no finding (below threshold)
- Struct with 3+ primitive fields, one of which is a named type → no finding
- Struct with 3+ primitive fields all raw → finding
- `#[derive(…)]` attributes on the struct → finding still fires
- Tuple struct (unnamed fields) → check whether to include or exclude; document decision

#### file-complexity-rust edge cases
- Closure inside a function → counts as nesting increment, not a new function boundary
- `match` expression → counted as nesting (already in `rust_lang_config().nesting_kinds`)
- `impl` block methods → each `function_item` is an independent complexity root

#### sentinel-return-rust edge cases
- Constructor function (`fn new(...) -> Option<Self>`) → no finding
- Function with zero `?` operators and explicit `return Err(...)` → no finding (not a mixed case)
- Function with both `?` and explicit `return Err(...)` → finding on the explicit return
- `return Ok(...)` → never a finding

#### rust-ignored-error edge cases
- `let _ = iter.next()` where `next()` returns `Option` not `Result` → no finding (only `Result` is `#[must_use]` by convention; use heuristic: flag if return type text contains `Result` or if it's a known-fallible API)
- `let _ = x` where `x` is a variable, not a call → no finding
- `.unwrap()` in `#[test]` function → no finding
- `.unwrap()` in `fn main()` → finding
- `.expect("message")` → configurable; default: flag only bare `.unwrap()`

#### rust-error-context edge cases
- `anyhow::bail!(...)` → not a `try_expression`, no finding
- `expr.context("msg")?` → no finding (context already added)
- `expr.map_err(|e| MyError::from(e))?` → no finding (error transformed, not bare `?`)
- `expr?` where function returns `Result<_, MySpecificError>` (not anyhow) → no finding

#### split-loop-rust edge cases
- Two accumulators that share a variable reference → no finding (linked)
- `for x in iter { a += x; b += x * 2; }` → finding (both accumulate from same input differently, but they don't feed each other)
- Iterator chain inside loop body (not an accumulator) → no finding
- Loop body > `MAX_LOOP_BODY_NODES` → skip (same guard as Go checker)

### Integration test for coverage note (AC8)
Create a minimal Rust-only test repo fixture (or use an in-memory temp dir in a test) containing a single clean `.rs` file. Run the check pipeline in verbose/hook mode. Assert the output contains a line matching `[rust] checked 1 .rs file` or similar. Assert the line is absent in `--json` output mode.

### Parity table consistency test (AC9)
In `src/rust_parity.rs`, add:
```rust
#[test]
fn parity_table_matches_committed_doc() {
    let rendered = render_parity_table();
    let committed = std::fs::read_to_string("docs/rust-parity.md").unwrap();
    assert_eq!(rendered.trim(), committed.trim(), "run `kibitzer parity` to regenerate");
}
```

### Regression guard (AC11)
Existing test `default_checks_ids_are_unique` in `config.rs` must continue to pass. Add entries to its count assertion for the new check IDs.

---

## Backtest gate (AC10)

Before each checker PR merges:
1. Run `kibitzer check backtest <name>` against Claude Code session transcripts.
2. Run the new checker against `servo/servo` and `BurntSushi/ripgrep` full trees.
3. Record: total files scanned, total findings, manually sampled findings (N=20 or all if <20), TP/FP classification, false-positive rate.
4. Write results to `docs/<name>-false-positives.md`. If FP rate > 20%, block the PR and re-tune the threshold.

---

## Open questions / risks

1. **`rust-error-context` type detection brittleness** — Detecting `anyhow::Result` from return type annotation text is a heuristic. If a project aliases `type Result<T> = anyhow::Result<T>`, the check won't fire. Acceptable for v1; document in FP doc.

2. **`.expect()` policy for `rust-ignored-error`** — `.expect("message")` is widely considered acceptable idiomatic Rust for precondition assertions. Flagging it would have high FP rate on well-maintained code. Recommendation: flag `.unwrap()` only by default; make `.expect()` opt-in via `inspect.json`.

3. **`split-loop-rust` generalization opportunity** — If the Go and Rust split-loop checkers share > 60% logic, refactor into a shared `split_loop_core.rs` module. Defer until both are written and the overlap is measured.

4. **Coverage note output format** — The coverage note must not break any existing test that asserts on exact check output. Audit the test suite for `assert_eq!(output, "")` patterns before shipping Task 4.1.
