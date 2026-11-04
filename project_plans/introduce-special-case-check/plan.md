# Implementation Plan: introduce-special-case-check

## Overview

Implement the `introduce-special-case` native checker following the existing
`duplicate-code-cross-file` persistent-index pattern. Go-only initial scope.
Estimated landing size: ~400 lines of Rust + fixtures + docs.

## Phase 1 — Persistent index and type extractor (foundation)

### Task 1.1 — Create `src/checkers/introduce_special_case.rs`

Implement a `SpecialCaseIndex` struct (mirrors `DuplicateIndex`):

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
struct SpecialCaseIndex {
    /// type_name → Vec<(file_path, line_1indexed)>
    occurrences: HashMap<String, Vec<(String, usize)>>,
    /// file_path → Vec<type_name> contributed
    files: HashMap<String, Vec<String>>,
}
```

Implement `load`, `save` (temp-file-then-rename), `remove_file`, `insert_file`,
`reindex_and_find`.

Index path: `~/.cache/kibitzer/special-case-index/<repo-hash>.json`.
Env-var override: `KIBITZER_SPECIAL_CASE_INDEX_DIR` (for backtest isolation).

### Task 1.2 — Go nil-check AST extractor

Write `extract_nil_checks(tree: &Tree, source: &[u8]) -> Vec<NilCheck>` where:

```rust
struct NilCheck {
    type_name: String,   // resolved type or "<unresolved>"
    line: usize,         // 1-indexed
}
```

Walk the tree for `if_statement` nodes whose condition is a `binary_expression` with
operator `==` or `!=` and one operand an `identifier` and the other `nil` (text `"nil"`).
For the identifier, resolve its type from the enclosing function's parameter list and
`var_declaration` / `short_var_declaration` nodes. Strip pointer prefixes (`*`) to get
the base type name. Skip `error`, `context.Context`, and other known high-noise builtins.

### Task 1.3 — `IntroduceSpecialCaseChecker` struct + `impl Checker`

```rust
pub struct IntroduceSpecialCaseChecker;

impl Checker for IntroduceSpecialCaseChecker {
    fn name(&self) -> &str { "introduce-special-case" }
    fn language(&self) -> Option<Language> { Some(Language::Go) }
    fn file_globs(&self) -> &[&str] { &["**/*.go"] }
    fn reads_cross_run_state(&self) -> bool { true }
    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> { … }
}
inventory::submit! { crate::checker::CheckerFactory(|| vec![Box::new(IntroduceSpecialCaseChecker)]) }
```

The `check` body:
1. Load index from `index_path_for_repo`.
2. `extract_nil_checks` from `ctx.tree` + `ctx.source`.
3. `index.reindex_and_find(file, nil_checks)` → findings.
4. Save index.
5. Return findings.

### Task 1.4 — Register in `config::default_checks()`

Add to the code-smell block:

```rust
native_check("introduce-special-case", Severity::Advisory, &["**/*.go"]),
```

Add `mod introduce_special_case;` to `main.rs`.

## Phase 2 — Configurability

### Task 2.1 — `min_occurrences` option via `configure()`

Override `Checker::configure` to parse `options.min_occurrences` (integer, default 3)
and return a configured instance. Affects the threshold in `reindex_and_find`.

## Phase 3 — Test fixtures

### Task 3.1 — `testdata/introduce-special-case/` corpus

Create three sub-directories, each with a small multi-file Go corpus:

- `fires/` — `a.go`, `b.go`, `c.go` each with a nil-check for `*OrderService` (3
  occurrences across 3 files) → checker must emit findings.
- `below-threshold/` — `a.go`, `b.go` with 2 occurrences → no finding.
- `distinct-types/` — `a.go` checks `*A`, `b.go` checks `*B`, `c.go` checks `*C`,
  each type only once → no finding.

### Task 3.2 — Unit tests in `introduce_special_case.rs`

Test `extract_nil_checks` on inline source strings covering:
- `==` form
- `!=` form (inverted branch order)
- type resolution from parameter list
- `error` exclusion
- `<unresolved>` fallback for unresolvable identifiers

Test `SpecialCaseIndex::reindex_and_find` for threshold boundary (2 → no finding,
3 → finding).

## Phase 4 — Backtest

### Task 4.1 — Transcript backtest

Run `kibitzer check backtest introduce-special-case` per `docs/backtesting.md`.
Record the false-positive rate in `docs/introduce-special-case-false-positives.md`.

### Task 4.2 — Public-corpus backtest

Run the checker against each repo in `docs/backtest-repos.md` via
`kibitzer check native introduce-special-case <file>` piped from `find`. Use
`scripts/backtest-triage.py` to track TP/FP. Record results.

## Phase 5 — Docs

### Task 5.1 — `docs/introduce-special-case-false-positives.md`

Document:
- Known FP patterns (`error`, `context.Context`, generated code)
- Suppression via `kibitzer:ignore introduce-special-case`
- Configuring `min_occurrences`

## Adversarial Review

**Risk 1 — Type resolution accuracy**: tree-sitter parses syntax, not semantics. The
extractor cannot follow type aliases or cross-file type definitions. Occurrences for
the same underlying type but reached under different alias names will be under-counted.
*Mitigation*: document as a known limitation; the check is a surfacer, not a prover.

**Risk 2 — Index staleness on file delete/rename**: if a file is deleted, its entries
are only removed when it is re-indexed (which never happens for deleted files). The
`remove_file` path only fires on write, not delete.
*Mitigation*: same as `duplicate-code-cross-file` — stale entries cause missed-removes,
not false positives (they lower the threshold, not raise it). Acceptable for a candidate
surfacer.

**Risk 3 — `error` exclusion list gaps**: `io.Reader`, `http.ResponseWriter`, and similar
heavily-nil-checked interfaces are not on the initial exclusion list.
*Mitigation*: start with just `error`; expand after first backtest against kubernetes.

**Risk 4 — Concurrent index writes**: same race as `duplicate_cross_file_checker` — two
daemon connections checking the same repo simultaneously can produce a lost update.
*Mitigation*: same accepted trade-off (last-write-wins, no corruption); note in code comment.

**Risk 5 — Test-file noise**: test helpers routinely nil-check the same service types.
*Mitigation*: exclude `_test.go` by default; make it configurable via `options.exclude_tests`.
