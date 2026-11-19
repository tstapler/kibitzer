# Validation: introduce-special-case-check

## Validation checklist

### AC 1 — Checker registered and in default_checks

- [ ] `src/checkers/introduce_special_case.rs` exists with `inventory::submit!` block
- [ ] `mod introduce_special_case;` added to `main.rs`
- [ ] `native_check("introduce-special-case", Severity::Advisory, &["**/*.go"])` added
  to `config::default_checks()` (verify with `grep -r introduce-special-case src/config.rs`)
- [ ] `kibitzer check list` output includes `introduce-special-case`

### AC 2 — Tree-sitter nil-check extraction (both `==` and `!=` forms)

- [ ] Unit test: `extract_nil_checks` on `if svc == nil { return "" } else { svc.Name() }` → 1 hit, type resolved
- [ ] Unit test: `if svc != nil { svc.Name() } else { return "" }` → same result
- [ ] Unit test: `if err == nil { … }` → 0 hits (`error` excluded)
- [ ] Unit test: unresolvable identifier → recorded under `<unresolved>`, not flagged

### AC 3 — Persistent index correct structure

- [ ] Index file is JSON at the expected cache path after checking a file
- [ ] Re-checking a modified file replaces its entries, not appends
- [ ] `KIBITZER_SPECIAL_CASE_INDEX_DIR` env var redirects the index (test with a tmpdir)

### AC 4 — Finding emitted at ≥3 occurrences across ≥2 files

- [ ] `testdata/introduce-special-case/fires/` fixture: running checker against
  `a.go`, `b.go`, `c.go` in sequence emits findings on the third file
- [ ] Finding message contains the type name and occurrence count

### AC 5 — Threshold configurable via `options.min_occurrences`

- [ ] A `.kibitzer/inspect.json` with `{"checks": [{"name": "introduce-special-case", "options": {"min_occurrences": 5}}]}` raises the threshold: fixture that fires at 3 does not fire at threshold 5

### AC 6 — `reads_cross_run_state` returns true

- [ ] `kibitzer::checker::reads_cross_run_state("introduce-special-case")` returns `true`
  (covered by `src/checker.rs`'s existing test for this function)

### AC 7 — Fixture corpus complete

- [ ] `testdata/introduce-special-case/fires/` — 3 files, same type, fires
- [ ] `testdata/introduce-special-case/below-threshold/` — 2 files, same type, no finding
- [ ] `testdata/introduce-special-case/distinct-types/` — 3 files, 3 different types, no finding

### AC 8 — `cargo test` passes

- [ ] `cargo test` green with no regressions (run before PR is opened)

### AC 9 — Backtest run and results documented

- [ ] `kibitzer check backtest introduce-special-case` run; output recorded in
  `docs/introduce-special-case-false-positives.md` or PR body
- [ ] Public-corpus run (`kubernetes/kubernetes` minimum) with TP/FP count recorded

### AC 10 — False-positives doc exists

- [ ] `docs/introduce-special-case-false-positives.md` created with:
  - Known FP patterns (error, context.Context, test files)
  - `kibitzer:ignore introduce-special-case` suppression example
  - `min_occurrences` configuration example

## Open questions / risks to resolve before implementation starts

1. **`go_encapsulate_collection.rs` type-extraction precedent** — does that checker
   already have a utility for resolving parameter types from tree-sitter nodes? If so,
   the type extractor in AC 2 should reuse it rather than duplicate the logic.
   *Action*: read `src/checkers/go_encapsulate_collection.rs` before writing
   `extract_nil_checks`.

2. **Index namespace collision** — the duplicate-code checker uses
   `~/.cache/kibitzer/duplicate-index/`. The new checker should use
   `~/.cache/kibitzer/special-case-index/` (a separate sub-directory). Confirm the
   cache-path helper in `src/cache.rs` is flexible enough for a different directory name,
   or whether a new helper is needed.

3. **`<unresolved>` bucket policy** — the requirements exclude `<unresolved>` from
   firing. Verify this is the right call: a file full of `if p == nil` where `p` is
   always an interface parameter whose concrete type is from another file would silently
   miss a valid refactoring candidate. Consider: if `<unresolved>` accumulates ≥5
   occurrences, emit a lower-confidence advisory noting unresolved types are excluded.
   *Defer* to post-backtest decision based on observed false-negative rate.

4. **Index compatibility with backtest.rs** — `backtest.rs` currently sets
   `KIBITZER_DUPLICATE_INDEX_DIR`. It likely needs to also set
   `KIBITZER_SPECIAL_CASE_INDEX_DIR` so transcript-replay backtests for the new checker
   do not corrupt the live index. Confirm `backtest.rs` handles this.
