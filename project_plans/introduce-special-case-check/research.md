# Research: introduce-special-case-check

## Cross-file infrastructure (PR #28 — already merged)

`src/checkers/duplicate_cross_file_checker.rs` implements the persistent-index pattern:
- `DuplicateIndex` — a `HashMap<String, Vec<(String, usize)>>` (window text → occurrences)
  plus `HashMap<String, Vec<String>>` (file → window keys it contributed), serialized as
  JSON in `~/.cache/kibitzer/duplicate-index/<repo-hash>.json`.
- `index_path_for_repo` — walks up to find `.git`, hashes the canonical path, names the file.
- `reads_cross_run_state() -> bool { true }` — signals the check harness that results
  are order-dependent (backtest.rs routes such checkers to a scratch index via
  `KIBITZER_DUPLICATE_INDEX_DIR`).

The new `introduce-special-case` checker can adopt the same shape verbatim, replacing the
duplicate-window key with a `TypeName` key and replacing window text with
`(file, line, pattern_text)` occurrence records.

## Checker registration pattern

Every checker uses `inventory::submit! { crate::checker::CheckerFactory(|| vec![Box::new(MyChecker)]) }`.
`config::default_checks()` is split into families (`code_smell_checks`,
`go_exception_checks`, etc.) — the new checker belongs in a new call or in the existing
`code_smell_checks` block since `introduce-special-case` is a cross-language smell
(Go-only initially).

## Tree-sitter Go grammar — nil-check AST shape

The Go tree-sitter grammar represents:

```
if x == nil {
    return ""
} else {
    x.Method()
}
```

as an `if_statement` node with:
- `condition`: `binary_expression` with `operator: "=="`, left: `identifier`, right:
  `nil` (`nil` is an `identifier` with text "nil" in tree-sitter Go)
- `consequence`: `block`
- `alternative`: `else` → `block`

The `!=` / inverted form has the same shape with `operator: "!="`.

Type resolution scope (single-file): scan the enclosing function's `parameter_list` and
`var_declaration`/`short_var_declaration` assignments to find the declared type for the
identifier. If the type cannot be resolved from the current file, skip attribution (record
under `<unresolved>`).

## Existing nil-check checkers in the Go ecosystem

- `go vet`, `staticcheck`, `nilcheck` (golangci-lint): these report *potential* nil
  dereferences (crashes), not *repeated* nil-guard patterns. No existing linter surfaces
  the Introduce Special Case refactoring signal.
- The `go-encapsulate-collection` checker in this codebase (`src/checkers/
  go_encapsulate_collection.rs`) shows the pattern of walking a tree-sitter tree for a
  structural shape and emitting findings — a close model.

## Threshold calibration

The backtest-repos corpus (`docs/backtest-repos.md`) includes `kubernetes/kubernetes` and
`BurntSushi/ripgrep`. Both codebases have nil-guard patterns, but `error` interface checks
(`if err == nil`) are extremely common and should be excluded: `error` is a builtin
interface, not a user-defined type, so its occurrences would drown out signal. The
`<unresolved>` bucket approach plus an explicit exclusion list (`error`, `context.Context`,
pointer-to-builtin) is the right false-positive mitigation.

Threshold of 3 occurrences across ≥2 files:
- Low enough to catch real accumulation (a type used in 3 services)
- High enough to avoid firing on a type that appears twice in one file and once in a test

## Backtest environment variable

`KIBITZER_DUPLICATE_INDEX_DIR` is already defined in `duplicate_cross_file_checker.rs`
for the duplicate-code checker's scratch index. The new checker needs its own env var
(`KIBITZER_SPECIAL_CASE_INDEX_DIR`) so backtest runs for each checker can be redirected
independently.

## False-positive risk inventory

| Pattern | Why it fires | Mitigation |
|---------|-------------|------------|
| `if err == nil` | `error` appears everywhere | Exclude `error` and other stdlib interfaces from type attribution |
| `if ctx == nil` | `context.Context` is passed everywhere | Exclude `context.Context` |
| Generated code | May repeat patterns mechanically | Honor `.kibitzer/inspect.json` globs exclusion |
| Test files (`_test.go`) | Test helpers often mirror nil-guards | Option: exclude `_test.go` by default (configurable) |
| Pointer receivers `if p == nil` where `p *T` and T has many implementations | May fire for unrelated types | Group by pointer base type, not raw pointer expression |

## Key files to read/model

- `src/checkers/duplicate_cross_file_checker.rs` — persistent index model
- `src/checkers/primitive_obsession.rs` — simple tree-sitter walker model
- `src/checkers/go_encapsulate_collection.rs` — structural Go AST check model
- `src/config.rs` lines 840–876 — where to add the default_checks entry
- `src/checker.rs` lines 220–231 — `reads_cross_run_state` interface
- `docs/backtesting.md` — required pre-merge backtest procedure
- `docs/backtest-repos.md` — corpus list
