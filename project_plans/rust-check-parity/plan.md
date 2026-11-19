# Plan: rust-check-parity

**Date:** 2026-11-19

---

## Implementation order

Checks are ordered by implementation complexity, ascending. Each check must pass its backtest gate before merging. The parity table and runtime signal are last because they depend on knowing what actually shipped.

---

### Phase 1 — Wire in already-ready check (no new grammar work)

#### Task 1.1 — `test-no-assertion-rust`
**Files:** `src/checkers/test_no_assertion.rs`, `src/config.rs`

`src/test_code/rust.rs` already provides the full Rust `TestCodeProfile`. The only change is:
1. Add `Language::Rust` to `SUPPORTED` in `test_no_assertion.rs:30`.
2. Add `native_check("test-no-assertion-rust", Severity::Advisory, &["**/*.rs"])` to `default_checks()` in `config.rs` (alongside the existing Go/Python/Kotlin entries).
3. Add a fixture test: a `#[test]` fn with no assert fires; a `#[test]` fn with `assert_eq!` does not.
4. Backtest against `servo/servo` and `ripgrep` — record pass/fail counts in `docs/test-no-assertion-rust-false-positives.md`.

**Estimate:** Small — the profile is written, the checker already dispatches on `TestCodeProfile`.

---

### Phase 2 — Port existing multi-language checks to Rust

#### Task 2.1 — `primitive-obsession-rust`
**Files:** `src/checkers/primitive_obsession.rs`, `src/config.rs`

Currently `language_for_path` returns `Some(Language::Go)`. Add a Rust arm:
1. Detect `struct_item` nodes with a `field_declaration_list` child.
2. For each field, extract its `type` node; map `RustKind` primitive type names (`String`, `u8`..`u64`, `i8`..`i64`, `f32`/`f64`, `bool`, `usize`, `isize`) to the same "primitive" classification.
3. Fire when N fields (threshold matching Go's) are all primitives.
4. Add `native_check("primitive-obsession-rust", ...)` to `default_checks()`.
5. Backtest; write `docs/primitive-obsession-rust-false-positives.md`.

**Note:** Rust's ownership model means some struct designs that would be "primitive obsession" in Go are actually idiomatic (e.g., `struct Point { x: f64, y: f64 }`). Threshold should be ≥ 3 primitive fields to avoid flagging small data structs.

---

#### Task 2.2 — `file-complexity-rust`
**Files:** `src/checkers/complexity.rs`, `src/config.rs`

Both `language_for_path` hooks in `complexity.rs` return `Some(Language::Go)`. Extend:
1. Add `Language::Rust` arm.
2. Use `rust_lang_config().nesting_kinds` already defined in `rules.rs` — no new grammar work.
3. Add `native_check("file-complexity-rust", ...)` to `default_checks()`.
4. Backtest; write `docs/file-complexity-rust-false-positives.md`.

**Note:** Rust closures (`closure_expression`) are already in `nesting_kinds` — ensure the complexity counter treats them like Go function literals (nesting increment, not separate function boundary) to avoid overcount.

---

#### Task 2.3 — `sentinel-return-rust`
**Files:** `src/checkers/sentinel_return.rs` (new Rust variant or extend existing), `src/config.rs`

The sentinel-return family uses per-language files (`sentinel_return_java.rs`, `sentinel_return_typescript.rs`, etc.). Add `sentinel_return_rust.rs`:
1. Target functions returning `Option<_>` or `Result<_, _>` (inspect return type node).
2. Flag explicit `return None` or `return Err(...)` in a function body where the dominant exit pattern is `?` propagation. Use a ratio: if at least one `?` is present and explicit `return Err/None` count > 0, flag those explicit returns.
3. Suppress in `new`/`try_new`/`from_*`/`default` functions (constructors by convention).
4. Add to `default_checks()` with `**/*.rs` glob.
5. Backtest; write `docs/sentinel-return-rust-false-positives.md`.

**False-positive note:** Boundary functions at error-domain edges legitimately use explicit `return Err(...)`. The `?`-presence ratio heuristic reduces but does not eliminate FPs; the false-positive doc should record the measured rate.

---

### Phase 3 — New Rust-specific checkers

#### Task 3.1 — `rust-ignored-error`
**Files:** `src/checkers/` (new `rust_ignored_error.rs`), `src/main.rs`, `src/config.rs`

New checker, two patterns:
1. **`let _ = <expr>`** where `<expr>` is a function or method call. Use tree-sitter: `let_declaration` with pattern `_` (wildcard). Only fire when the right-hand side is a `call_expression` or `method_call_expression` (not a bare value). Suppress in `#[cfg(test)]` items and `tests/` paths.
2. **`.unwrap()` / `.expect("")`** in non-test code. `method_call_expression` where `method` text is `unwrap` or `expect`. Suppress in test contexts.

Register via `inventory::submit!`, add to `config.rs`, write `docs/rust-ignored-error-false-positives.md`.

**Backtest note:** `servo/servo` and `ripgrep` both use `.expect()` with messages in production code. Pattern 2 may have high FP for well-messaged `.expect()` calls — consider suppressing `.expect(msg)` where `msg` is a non-empty string literal, flagging only bare `.unwrap()`.

---

#### Task 3.2 — `rust-error-context`
**Files:** `src/checkers/` (new `rust_error_context.rs`), `src/main.rs`, `src/config.rs`

New checker: a `try_expression` (`expr?`) inside a function returning `anyhow::Result<_>` (or `Result<_, _>` where the error type resolves to `anyhow::Error`) where the operand is not already a `method_call_expression` with `.context()`/`.with_context()`.

Grammar: `try_expression { expression: <inner_expr> }`. Fire when `<inner_expr>` is a bare `call_expression` or `identifier` / `field_expression`, not a chained `method_call_expression` ending in `context`/`with_context`.

Scope limiter: inspect the enclosing `function_item`'s return type for `anyhow` or a configurable type name list. Default: `anyhow::Result`, `anyhow::Error`. Make the type list configurable via `inspect.json` options.

Register, add to `config.rs`, write `docs/rust-error-context-false-positives.md`.

**Backtest note:** Code using typed domain errors (`MyError`) intentionally doesn't add `anyhow` context. Scoping to `anyhow::Result` returns only is the primary FP defense.

---

#### Task 3.3 — `split-loop-rust`
**Files:** `src/checkers/split_loop.rs` (extend or new file), `src/config.rs`

The existing `SplitLoopChecker` is tightly coupled to `GoKind`. Options:
- **Extend in-file:** add a `RustSplitLoopChecker` struct, `inventory::submit!`, reuse shared accumulator-detection logic via a trait or free function parameterized on node kind.
- **Extract shared logic:** refactor `split_loop.rs` into a `SplitLoopChecker<K: NodeKind>` generic or a function taking a `SplitLoopConfig { for_kinds, accumulator_kinds, ... }`. Cleaner but touches more code.

Recommended: add `RustSplitLoopChecker` in a new `split_loop_rust.rs` file, copy-adapting the Go checker's logic to Rust node kinds:
- Loop kinds: `ForExpression`, `WhileExpression`, `LoopExpression`
- Accumulator patterns: `assignment_expression` with `+=`/`-=` etc., `call_expression` on `push`/`insert`/`extend`
- Suppress closures (they appear inside loops but are not the loop body)

Add to `default_checks()`, write `docs/split-loop-rust-false-positives.md`.

---

### Phase 4 — Runtime signal for zero Rust findings

#### Task 4.1 — Zero-findings coverage note
**Files:** `src/check.rs` or `src/run.rs`, `src/hook_footer.rs`

After running all checks in a session, if at least one `.rs` file was checked and total Rust findings = 0, emit:

```
[rust] checked N .rs files — 0 findings (checks active: rust-file-size, syntax-rules-rust, comment-quality-rust, ...)
```

Design decisions:
- Output only when `--verbose` or in hook mode (not in `--json` mode, where it would break JSON consumers).
- Enumerate active Rust-scoped checks from the check list (those with `**/*.rs` globs) so the user can verify what ran.
- Track "files checked per language" in `run.rs`'s file loop.

This directly addresses the ambiguity documented in `checker.rs`'s `extensions()` comment about Rust being silently dropped.

---

### Phase 5 — Parity table

#### Task 5.1 — `src/rust_parity.rs` + `docs/rust-parity.md`

Define a data structure:
```rust
struct ParityRow {
    check: &'static str,
    status: Status, // Shipped | Planned | NA
    notes: &'static str,
}
enum Status { Shipped, Planned, NA }
```

Populate rows for every check in `default_checks()` that has a Rust glob (Shipped) and every Go/Java/TS check with a noted N/A (N/A + rationale). Add a `kibitzer parity` subcommand or a `codegen` script that renders `docs/rust-parity.md`.

Add a CI test: `docs/rust-parity.md` must match the rendered output of `rust_parity.rs` — same as how other docs are kept in sync.

---

## Adversarial review

**Concern 1 — Ordering:** Tasks 3.1 and 3.2 produce new checker files from scratch; they carry the highest false-positive risk. Placing them in Phase 3 (after the simpler Phase 1–2 wires) is correct. However, backtesting Phases 2 and 3 against `servo/servo` takes wall-clock time (clone + run). Plan for this: clone `servo/servo` and `ripgrep` early (can run while Phase 1 is being implemented).

**Concern 2 — `rust-error-context` scope creep:** Detecting return type as `anyhow::Result` requires parsing type annotation text, which is brittle (aliases, `use anyhow::Result` import). Mitigate: match on the type annotation string containing `anyhow` or `Error` as a substring, with an opt-out via `inspect.json`. Document the limitation.

**Concern 3 — `split-loop-rust` FP on iterator chains:** Rust code often uses chained iterators (`iter().map().collect()`) inside loops for accumulators. The accumulator-detection heuristic must distinguish `vec.push(x)` (loop-level accumulation) from iterator-chain expressions (not classic accumulator patterns). The Go checker's independence test (do the two accumulators share any variable reference?) is the right gate.

**Concern 4 — Parity table drift:** Requiring `docs/rust-parity.md` to match a Rust-compiled codegen output means CI must run the codegen step. If `rust_parity.rs` is not part of the main binary, it needs a separate codegen target or test. Use an `#[test]` that renders the table and compares against the committed file (like `rustfmt`'s output tests).

**Concern 5 — Zero-findings signal in JSON mode:** The coverage note must be suppressed in `--json` output mode and in `--check` / CI modes where unexpected stdout breaks parsers. Gate on output mode in `run.rs`.
