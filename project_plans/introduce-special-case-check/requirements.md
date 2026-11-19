# Requirements: introduce-special-case-check

## Item

**Backlog item 8e945331-aa3d-4ac4-8734-7dab09843c30**
Cross-file pattern check: Introduce Special Case (repeated nil-check-then-branch for the same type)

## Problem Statement

When the same `if x == nil { <fallback> } else { x.Method(...) }` pattern repeats across
several call sites for values of the same declared type, that is a candidate for Fowler's
**Introduce Special Case** refactoring (née *Introduce Null Object*, MFRD 2nd ed.).
Currently kibitzer has no detector for this. Engineers accumulate these patterns
unknowingly across files, missing the signal that the type itself should encode the
special-case behaviour.

## Goal

A cross-file, diff-aware kibitzer checker — `introduce-special-case` — that surfaces
repeated nil-check-then-branch patterns grouped by the static type of the checked
variable, flagging types that accumulate ≥3 occurrences across the repo. The check is a
*candidate surfacer*, not a rewrite engine: its finding message names the type and
occurrence count; designing the actual null-object implementation remains a human/LLM
task.

## Dependency

PR #28 added `duplicate-code-cross-file` and the persistent-index infrastructure it
uses. That code is already merged (`src/checkers/duplicate_cross_file_checker.rs`,
`src/checker.rs::reads_cross_run_state`). This feature reuses those same patterns —
it does not rebuild them.

## Acceptance Criteria

1. A new native checker named `introduce-special-case` is registered via
   `inventory::submit!` and listed in `config::default_checks()` for Go source files
   (`**/*.go`) at `Severity::Advisory`.

2. For a `.go` file being checked, the checker uses tree-sitter to find every occurrence
   of the pattern `if <ident> == nil { … } else { <ident>.Method(…) }` (and the
   logically equivalent `if <ident> != nil { <ident>.Method(…) } else { … }` form),
   extracting the declared type of `<ident>` where resolvable from the same file's
   function/method signatures and variable declarations.

3. The checker maintains a persistent JSON index (same cache-dir / repo-root-hash scheme
   as `duplicate_cross_file_checker.rs`) mapping each type name to the set of
   `(file, line)` occurrences found so far, incrementally updated for the file being
   checked.

4. When a type's occurrence count across the repo reaches ≥3 distinct call sites
   (across ≥2 distinct files), the checker emits one `Finding` per occurrence in the
   current file, with a message such as:
   `introduce-special-case: *MyType nil-check repeated 4 times across 3 files — consider Fowler's Introduce Special Case`

5. The threshold (default 3) is configurable per-repo via `.kibitzer/inspect.json`
   `options.min_occurrences`.

6. The checker implements `reads_cross_run_state() -> bool { true }` so the check
   harness correctly treats its results as order-dependent.

7. A `testdata/introduce-special-case/` fixture directory contains at least:
   - a multi-file Go corpus with ≥3 nil-check occurrences for one type (should fire)
   - a corpus with only 2 occurrences (should not fire)
   - a corpus with nil-checks for unrelated types (should not fire for either)

8. All existing `cargo test` suites pass with the new checker registered.

9. The checker is backtested per `docs/backtesting.md` before merging:
   `kibitzer check backtest introduce-special-case` run against Claude Code session
   transcripts, and the checker run against the standard backtest corpus
   (`docs/backtest-repos.md`) with before/after false-positive rate recorded.

10. A `docs/introduce-special-case-false-positives.md` file is created documenting
    known false-positive patterns (e.g. error interface checks, generated code, test files)
    with suppression guidance.

## Out of Scope

- Generating the null-object implementation — the check only surfaces the pattern.
- Languages other than Go in the initial landing (TypeScript/Java support deferred).
- Resolving types that cross file boundaries (only types visible in the current file's
  scope are attributed; unresolved identifiers are recorded under a synthetic
  `<unresolved>` bucket that is not flagged).
