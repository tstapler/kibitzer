# Implementation Plan: long-function-flat-literal

**Feature**: Exempt `long-function` when the function body is a single `return` of a
flat composite/collection literal (no control-flow or call expressions in the literal's
subtree). Precision fix to `check_declaration` in `src/checkers/rules.rs`.
**Date**: 2026-10-11
**Status**: Ready for implementation
**ADRs**: None required — this is a precision fix with one clear implementation path
and no threshold/design trade-off requiring a formal decision record.

---

## Domain Glossary

| Term | Definition | Notes |
|------|-----------|-------|
| `is_flat_data_literal_body` | New predicate fn: `(body: Node, cfg: &LangRuleConfig) -> bool`. Returns `true` iff the body has exactly one statement, that statement is a return, the return value is a collection literal kind, and that literal contains no control-flow or call nodes. | Lives as a file-level private function alongside `check_declaration`. |
| `collection_literal_kinds` | New `LangRuleConfig` field: `&'static [&'static str]`. Per-language node kinds for pure data-container literals. Non-empty for Go, TS/TSX/JS, Python; `&[]` for Java, Kotlin, Rust. | Alternative: derived from `density_boundary_kinds - nesting_kinds` (Option A in architecture.md); explicit new field is preferred (Option B). |
| flat data literal body | A function body containing exactly one `return` of a collection literal whose entire subtree contains no `nesting_kinds`, `if_kind`, or `call_kinds` nodes. | The shape the exemption targets. |
| logic node | Any node whose kind is in `cfg.nesting_kinds`, equals `cfg.if_kind`, or is in `cfg.call_kinds`. | A literal containing a logic node is not flat. |

---

## Pattern Decisions

| Component | Pattern Chosen | Source | Alternative Rejected | Reason |
|-----------|---------------|--------|---------------------|--------|
| Change site | Guard expression inside existing `long-function` branch of `check_declaration` | Minimal-change principle | A new walk or a new `CATALOG` entry | The change is to an existing rule's precision, not a new rule; `check_declaration` is the single correct locus |
| `collection_literal_kinds` | New explicit `LangRuleConfig` field (Option B, `research/architecture.md §3`) | Self-documenting, independently verifiable | Derive from `density_boundary_kinds - nesting_kinds` | Implicit derivation fails silently when `density_boundary_kinds` is modified for unrelated reasons; same reasoning that chose `binding_finder` over a derived field in `replace-magic-literal` |
| Logic check in literal | Recursive descent stopping at `cfg.function_kinds` boundaries | Same recursion shape as `max_nesting_depth` | Walk all nodes without stopping | A closure inside a literal element is its own scope; logic there is not extracted by the caller's Extract Function anyway |
| Multi-language | `collection_literal_kinds = &[]` for Java/Kotlin/Rust | Correct, not a gap | Skip field for those languages | `vec![...]`/`listOf(...)` are call subtrees — the literal-only exemption genuinely does not apply; the `&[]` is intentionally correct |

---

## Tech Debt Disposition

| Area | Existing Issue | Disposition | Justification |
|------|----------------|-------------|---------------|
| `lang_config()` was a 200+ line function | Already refactored into 8 per-language constructors by `replace-magic-literal` work | Extend as-is | Adding one `collection_literal_kinds` line per constructor is proportionate; no new hotspot created |
| `src/checkers/rules.rs` file size | Pre-existing, ~2400+ lines after `replace-magic-literal` | Extend as-is | This feature adds ~30 lines net; not a meaningful worsening of a pre-existing condition |

---

## Epic 1: Predicate and field

### Story 1.1 — New `collection_literal_kinds` field in `LangRuleConfig`

**Goal**: Add `collection_literal_kinds: &'static [&'static str]` to `LangRuleConfig`
and populate all 8 language constructors.

**Tasks**:
- **1.1.1a**: Add `collection_literal_kinds: &'static [&'static str]` field to
  `LangRuleConfig` struct (`src/checkers/rules.rs:198`) with a doc comment explaining
  its purpose and why Java/Kotlin/Rust are `&[]`.
- **1.1.1b**: Populate `go_lang_config()`: `collection_literal_kinds: &["composite_literal"]`.
  Verify `"composite_literal"` is the correct tree-sitter node kind for Go by
  cross-checking against `density_boundary_kinds` (same file, line 914 — already has
  `"composite_literal"`, confirming the kind name).
- **1.1.1c**: Populate `typescript_lang_config()`:
  `collection_literal_kinds: &["array", "object"]`.
  Verify via `to_sexp()` on a TypeScript array-of-objects fixture that `"array"` and
  `"object"` are the correct node kinds.
- **1.1.1d**: Populate `tsx_lang_config()`: inherit from TypeScript via struct update or
  explicit copy. Same kinds as TypeScript.
- **1.1.1e**: Populate `javascript_lang_config()`: same as TypeScript (`["array", "object"]`).
- **1.1.1f**: Populate `python_lang_config()`: `collection_literal_kinds: &["list", "dictionary"]`.
  Verify via `to_sexp()`. Python's list and dict are the idiomatic flat-fixture containers.
- **1.1.1g**: Populate `java_lang_config()`: `collection_literal_kinds: &[]`. Document
  why in a comment: Java's array initializers are subtrees of an `array_creation_expression`,
  not standalone literal nodes.
- **1.1.1h**: Populate `kotlin_lang_config()`: `collection_literal_kinds: &[]`. Comment:
  `listOf(...)` is a call expression, not a literal.
- **1.1.1i**: Populate `rust_lang_config()`: `collection_literal_kinds: &[]`. Comment:
  `vec![...]` is a macro invocation; `&[...]` slice literals are `array_expression` but
  table tests using them are rare and often contain function calls.
- **1.1.1j**: Add `collection_literal_kinds` to `node_kind_literals_are_valid_for_their_grammar`
  (around line 1193) so the compile-time grammar verification test covers the new field.
  Run `cargo test node_kind_literals` to confirm it passes.
- **1.1.1k**: Run `cargo test --lib` to confirm all existing tests pass with the new field.

### Story 1.2 — `is_flat_data_literal_body` predicate

**Goal**: Implement the four-step predicate and wire it into `check_declaration`.

**Tasks**:
- **1.2.1a**: Write `fn is_flat_data_literal_body(body: Node, cfg: &LangRuleConfig) -> bool`
  in `src/checkers/rules.rs`, as a private file-level function alongside
  `check_declaration`:
  1. `let container = (cfg.statement_container)(body);`
  2. Collect `container.named_children()` into a `Vec<Node>`. If length != 1, return false.
  3. If `stmts[0].kind() != cfg.return_kind`, return false.
  4. Collect `stmts[0].named_children()` into a Vec. If length != 1, return false.
     (A bare `return;` has 0 named children; a multi-expression return is uncommon but
     handled by the length check.)
  5. If `!cfg.collection_literal_kinds.contains(&ret_val.kind())`, return false.
  6. Return `!has_logic_descendants(ret_val, cfg)`.
- **1.2.1b**: Write `fn has_logic_descendants(node: Node, cfg: &LangRuleConfig) -> bool`:
  - If `node.kind()` is in `cfg.nesting_kinds`, `cfg.call_kinds`, or equals `cfg.if_kind`,
    return true.
  - If `node.kind()` is in `cfg.function_kinds`, return false (don't descend into nested
    closures — they are their own scope).
  - Recurse into `node.named_children()`. Return true if any child returns true.
  - Otherwise return false.
- **1.2.1c**: In `check_declaration`, change the `long-function` push to:
  ```rust
  if body_lines > LONG_FUNCTION_LINES && !is_flat_data_literal_body(body, cfg) {
      findings.push(...);
  }
  ```
- **1.2.1d**: Run `cargo test --lib` to confirm all existing tests still pass (the guard
  must not suppress any existing `[long-function]` test cases, since those all have
  multi-statement bodies or non-literal returns).

---

## Epic 2: Tests

### Story 2.1 — Unit tests for `is_flat_data_literal_body`

All tests live in `src/checkers/rules.rs`'s `#[cfg(test)] mod tests` following the
existing convention.

**Tasks**:
- **2.1.1a** (`long_function_exempts_flat_go_composite_literal`): Build a Go source
  with a function whose body is one `return []tc{...}` with 45 entries (no calls, no
  control flow). Assert no `[long-function]` finding is emitted.
- **2.1.1b** (`long_function_does_not_exempt_literal_with_call`): Same shape but one
  entry contains `fmt.Sprintf(...)`. Assert `[long-function]` IS emitted.
- **2.1.1c** (`long_function_does_not_exempt_multi_statement_body`): Go function with
  a `var x = ...` assignment followed by `return []tc{...}`. Assert `[long-function]`
  IS emitted (two statements, not one).
- **2.1.1d** (`ts_long_function_exempts_flat_array_literal`): TypeScript function
  returning an array literal with 45 object entries, no calls. Assert no `[long-function]`.
- **2.1.1e** (`long_function_still_flags_real_logic`): Existing `flags_long_function`
  test is unchanged — it uses a multi-statement body (`println(...)` × 45) and must
  still fire.

---

## Epic 3: Backtesting

**Tasks**:
- **3.1.1a**: Run `kibitzer check backtest syntax-rules` against
  `~/.claude/projects/*/*.jsonl` transcript history (`docs/backtesting.md`). Note in the
  PR description: how many transcript edits contained a Go/TS function matching the
  flat-literal shape, and whether the now-suppressed findings were genuine noise or cases
  where Extract Function would have been actionable.
- **3.1.1b**: Sample the real-world corpus (`docs/backtest-repos.md`) — run
  `syntax-rules` on `kubernetes/kubernetes`'s test packages (most likely to have
  flat-literal fixtures). Confirm any newly-silenced `long-function` findings are
  table-test helpers, not cases where extraction would help.

---

## Epic 4: Documentation

**Tasks**:
- **4.1.1a**: Update `docs/syntax-rules.md`'s `long-function` row description (currently
  "Function/method body spans more lines than this — Fowler's *Extract Function*.") to
  add: "Functions whose body is a single `return` of a flat collection literal (no
  control-flow or call expressions) are exempt — there is nothing to extract."
- **4.1.1b**: Run kibitzer's own `markdown-link-integrity` check on the updated doc to
  confirm no broken links were introduced.

---

## Risk Controls

| Risk | Mitigation |
|------|-----------|
| Exemption silences a real Extract Function candidate | The four-step predicate is conservative; any call expression anywhere in the literal blocks the exemption. Backtest (Epic 3) confirms empirically. |
| New field adds compilation noise for Java/Kotlin/Rust | `&[]` entries compile fine; `node_kind_literals_are_valid_for_their_grammar` test (Task 1.1.1j) will not fail on an empty slice. |
| `has_logic_descendants` misses the `if_kind` guard | Explicit `== cfg.if_kind` check in addition to `nesting_kinds` (because `if_kind` is always excluded from `nesting_kinds` per the struct's comment). |
