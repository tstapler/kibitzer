# Research: rust-check-parity

**Date:** 2026-11-19

---

## Current Rust check coverage (verified against `src/config.rs`)

Three checks are in `default_checks()` with `**/*.rs` globs today:

| Check | File | Notes |
|---|---|---|
| `rust-file-size` | `src/checkers/file_size.rs` | `LangConfig` arm at line 80 |
| `syntax-rules-rust` | `src/checkers/rules.rs` | `SyntaxRulesChecker` with full `rust_lang_config()` — includes `replace-magic-literal`, `long-function`, `bool-function-param`, `long-parameter-list`, `unreachable-code`, `redundant-else`, `slog-key-literal` |
| `comment-quality-rust` | `src/checkers/comment_quality.rs` | name dispatch at line 193 |

Additionally, `magic_literal_context.rs` has explicit Rust arms (lines 215, 257) but is exercised via `syntax-rules-rust` dispatch, not a separate default entry.

---

## Gaps — checks in other languages with no Rust equivalent

### test-no-assertion-rust

**Finding:** `src/checkers/test_no_assertion.rs:30`
```rust
const SUPPORTED: &[Language] = &[Language::Go, Language::Python, Language::Kotlin];
```
Rust is absent. `src/test_code/rust.rs` already provides a complete `TestCodeProfile` for Rust — `#[test]` function detection, assertion names (`assert!`, `assert_eq!`, `assert_ne!`, etc.), snapshot markers, and exemption markers (`#[ignore]`, `#[should_panic]`). Adding Rust to SUPPORTED is the only code change needed; the profile is already written.

**Config gap:** `config.rs` has `test-no-assertion-go` / `-python` / `-kotlin` entries; no `test-no-assertion-rust`.

**Backtest readiness:** `testdata/` likely has inline test corpus already (Kotlin/Python were added after Go); Rust inline tests `#[cfg(test)] mod tests` need a targeted fixture set.

---

### primitive-obsession (Rust)

**Finding:** `src/checkers/primitive_obsession.rs:32`
```rust
fn language_for_path(p: &Path) -> Option<Language> {
    // ... returns Some(Language::Go)
```
Only Go. The concept is language-agnostic: a struct/record type that uses multiple raw primitives (`String`, `u64`, `bool`) where a named type would be more expressive. Rust structs (`struct Foo { name: String, age: u64, active: bool }`) exhibit this smell identically.

**Implementation path:** The Go checker uses tree-sitter to find struct field type nodes. `RustKind` already enumerates Rust node kinds including `StructItem`, `FieldDeclarationList`, and `FieldDeclaration`. The port requires Rust-specific node kind arms.

---

### file-complexity (Rust)

**Finding:** `src/checkers/complexity.rs:32,43` — both `language_for_path` and `function_kinds` return Go only. The checker counts nesting depth / branch count using grammar `nesting_kinds`. Rust's `rust_lang_config()` already enumerates `nesting_kinds` (`for_expression`, `while_expression`, `loop_expression`, `match_expression`, `closure_expression`) — the port is a matter of routing `Language::Rust` through the same complexity calculation.

---

### sentinel-return-rust

**Finding:** `src/checkers/sentinel_return.rs` — no Rust arm. For Go the check detects `return nil` / `return ""` etc.; for Java `return null`; for TypeScript `return null | undefined | ""`. The Rust equivalent is a function whose return type is `Option<T>` or `Result<T, E>` explicitly returning `None` or `Err(SomeError::Fixed)` without contextual information — indicating a manual sentinel rather than natural error propagation via `?`.

**Complexity note:** Rust already has `?` as the idiomatic propagation operator. The check should flag explicit `return Err(...)` / `return None` in functions where every other exit is via `?`, not in functions where explicit error construction is intentional (e.g., constructors, boundary functions). False-positive risk is moderate; backtest against `ripgrep` and `servo/servo` mandatory.

---

### rust-ignored-error

No `rust-ignored-error` checker exists. The Go checker detects `_, err := f()` and swallowed `err`; the Java checker detects caught exceptions re-thrown as generic ones. The Rust equivalent has two shapes:

1. **`let _ = expr`** — compiler allows discarding `Result<_, _>` this way, silencing `#[must_use]`. Unlike `let _ = e.iter()` which is innocuous, `let _ = write!(...)` or `let _ = fs::remove_file(...)` discards an error.
2. **`.unwrap()` / `.expect()` in non-test code** — not a bug per se but a signal of "I'm explicitly refusing to propagate this error." `.expect()` with a good message is debatable; `.unwrap()` without context in production paths is clearer.

**Grammar:** Rust's `call_expression` with a callee of `method_call_expression` where the method name is `unwrap` or `expect`. Pattern 1 uses `let_declaration` with `_` as the pattern.

**False-positive risk:** `.unwrap()` in tests should be suppressed (via `#[cfg(test)]` context or file-path heuristic). `let _ =` on non-`Result` values should not fire.

---

### rust-error-context

No `rust-error-context` checker. `go-error-context` flags `return ..., err` without `fmt.Errorf("... %w", err)` wrapping. The Rust analogue: a `?` operator applied to a `Result` inside a function whose return type is `anyhow::Result` (or similar) without a preceding `.context()` / `.with_context()` call, when the original error type is not `anyhow::Error`. This is distinct from `rust-ignored-error` — it's about *propagation quality*, not suppression.

**Grammar:** `?` in Rust is a `try_expression` node (`result_expr?`). Check whether the sub-expression is `method_call_expression` with `.context()`/`.with_context()` chained.

**Scope limiter:** Only fire in functions declared `-> anyhow::Result<_>` or similar. Avoids false positives in code that uses custom error types with `From<_>` conversions where context is added at the boundary.

---

### split-loop-rust

`src/checkers/split_loop.rs` hardcodes `Language::Go` and uses `GoKind`. The checker algorithm (independent accumulator detection, dependency/guard linking) is grammar-parametric in concept. Rust `for` / `while` / `loop` bodies can exhibit the same smell.

**Implementation path:** The Go checker uses `GoKind::ForStatement`, `GoKind::RangeClause`, accumulator patterns `x op= e`, `x++`. The Rust equivalent uses `RustKind::ForExpression`, `RustKind::WhileExpression`, `RustKind::LoopExpression`, and accumulator patterns `x op= e` (same surface syntax in Rust's `binary_expression` with `+=` etc.) plus `x.push(e)` (method call with known accumulator semantics). The Go-specific accumulator kinds (`append`, `x[k] = e`) have Rust equivalents (`vec.push`, indexing via `index_expression`).

---

## Runtime signal for zero Rust findings

**Motivation:** The commit history records two separate instances where Rust was silently dropped from language dispatches (`arch_model.rs`, `arch_export.rs`) with no error, only a stat that was easy to miss. If all Rust checks run and produce zero findings, there is currently no output telling the user "kibitzer checked N .rs files and found nothing" — the output is simply empty. This is indistinguishable from "no Rust checks ran."

**Existing pattern:** `kibitzer status` reads `hook_log.rs` and prints per-check fired/failed counts. The run output itself (`run.rs`, `check.rs`) does not currently emit a per-language "X files checked" note.

**Proposed signal:** After running checks on a Rust-containing repo, if the total Rust findings count is 0 but N `.rs` files were scanned, emit a note line: `[rust] checked N files, 0 findings`. This can be gated behind `--verbose` or emitted only in hook/non-CI mode to avoid polluting CI logs.

---

## Parity table infrastructure

The description mentions `src/rust_parity.rs` as the source of truth for `docs/rust-parity.md`. The pattern to follow is a Rust module that defines a static table of `PariRow { check, status, notes }` entries, with a `kibitzer` subcommand (or a `codegen` script) that renders it to Markdown. The existing `codegen/` directory (node-type generation) suggests this codebase already uses code-driven doc generation.

---

## Backtest corpus readiness

The standard corpus (`kubernetes/kubernetes`, `apache/cassandra`, `servo/servo`, `BurntSushi/ripgrep`, `denoland/deno`, `microsoft/vscode`) plus `stapler-squad` covers Rust via `servo/servo` and `ripgrep`. Both are large, well-regarded Rust codebases with idiomatic error handling, inline tests, and complex loop bodies — good signal sources for all seven new checks. `scripts/clone-backtest-repos.sh` should include both; check that they're in the clone list before backtesting.

---

## Risk assessment

| Risk | Likelihood | Mitigation |
|---|---|---|
| `rust-error-context` high FP rate (custom error types with `From`) | High | Scope to `anyhow::Result` returns only; backtest gate |
| `sentinel-return-rust` FP on boundary/constructor functions | Medium | Suppress when function name is `new`/`try_new`/`from_*`; backtest gate |
| `split-loop-rust` FP on intentional multi-accumulator patterns | Medium | Min body size / accumulator count thresholds; backtest gate |
| `rust-ignored-error` FP on `.unwrap()` in test helpers | Low | `#[cfg(test)]` context suppression already established by `skip_test_strings` in `SyntaxRulesChecker` |
| Parity table drifts from code | Low | `rust_parity.rs` as single source of truth + CI codegen check |
