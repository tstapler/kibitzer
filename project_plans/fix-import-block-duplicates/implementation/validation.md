# Validation Checklist: fix-import-block-duplicates

**Date**: 2026-10-10

## Pre-implementation

- [ ] Confirm `leading_import_end` returns 0 when `ext` is unrecognised and the caller
      treats 0 as "no preamble" (no windows suppressed).
- [ ] Confirm `find_duplicate_blocks`'s signature change from `(source)` to
      `(source, import_end)` only affects its one call site in `DuplicateCodeChecker::check`.
- [ ] Confirm `index_cross_file_windows`'s `paths` parameter can be derived from
      `find_cross_file_duplicates`'s existing `files: &[(PathBuf, String)]` input without
      re-cloning (can use `files.iter().map(|(p,_)| p.as_ref())`).

## Post-implementation

- [ ] `cargo test` passes with zero regressions.
- [ ] New test `does_not_flag_kotlin_import_block_cross_file` passes (exact stelekit repro).
- [ ] New test `still_flags_duplicate_after_import_block` passes (genuine duplicate
      following the import block is still reported).
- [ ] `kibitzer check backtest duplicate-code-cross-file` run against local transcript
      corpus shows no new false positives vs. pre-fix baseline.
- [ ] Run kibitzer against `stapler-squad` repo with the patched binary: the
      `config/config.go:745` true positive from the 2026-09-12 log entry still fires.
- [ ] `docs/duplicate-code-cross-file-false-positives.md` 2026-09-23 entry is in `## Fixed`.
- [ ] `docs/duplicate-code-false-positives.md` has a new fixed entry for import-block scope.

## Edge cases to verify manually or via test

- A Kotlin file with `@file:JvmName("...")` annotation before `package` — should be treated
  as preamble (import_end scan must handle it).
- A Go file with a single-line `import "fmt"` (not grouped) — `import ` prefix match covers it.
- A Rust file starting with `#![allow(dead_code)]` attribute before `use` — `#![` prefix
  marks it as preamble.
- A Python file starting with a module docstring — the docstring lines start with `"""`,
  not matched by any import pattern, so `leading_import_end` correctly returns 0 (no
  preamble detected). Import lines below the docstring are NOT in the "leading" block.
  This is acceptable: Python files with top-level docstrings before imports are unusual,
  and the false positive only fires if 3+ such files share the same 6-line import window
  even below a docstring — still unlikely.

## Requirement → test coverage (validate pass, 2026-10-11)

| AC | Covered by |
|----|-----------|
| 1 (.kt/.java cross-file) | `does_not_flag_kotlin_import_block_cross_file`; add a `.java` variant |
| 2 (.rs single-file) | `does_not_flag_rust_use_block` |
| 3 (.go grouped, quoted paths) | `leading_import_end_go_grouped_import` + add a Go no-flag test |
| 4 (.py/.js/.ts/.tsx/.jsx) | `leading_import_end_python` + add a JS-family no-flag test |
| 5 (after-preamble not suppressed) | `does_not_suppress_mid_file_duplicate_that_happens_to_use_import_syntax` |
| 6 (existing tests unmodified) | `cargo test`; no existing test references the changed signatures |
| 7 (real duplicate still flagged) | `still_flags_duplicate_after_import_block` |
| 8 (docs) | Task 7 |

Added edge case: file consisting only of imports → `leading_import_end == lines.len()`.

## Pre-mortem

- Over-suppression of real code: `#`/`#[` count as preamble for every supported ext, so a
  run of 6+ attribute/comment lines is treated as preamble. Impact is a missed finding on
  a pathological block, not a false positive; accepted.
- Hook path and batch path diverge: mitigated by both calling `leading_import_end`.
- Silent Go regression if a `const (` block precedes imports: not valid Go; accepted.

## Readiness gate: PASS

Every AC maps to a test or task, the plan's two defects are fixed above, no BLOCKED
items remain, and no code has been written yet.
