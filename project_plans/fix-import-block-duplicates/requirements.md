# Requirements: fix-import-block-duplicates

**Date**: 2026-10-10
**Type**: bugfix
**Complexity**: 2 — focused scope change with clear root cause; affects two checkers and their shared window function

## Problem Statement

`duplicate-code` and `duplicate-code-cross-file` both false-positive on files whose
leading import/use block happens to be identical across 3+ files. The checkers window-hash
raw trimmed lines with no awareness that a window falls entirely inside a file's `import`
or `use` preamble. Import statements are mechanically dictated by what the file references
and cannot be "extracted into a shared function" — the check's prescribed remedy is
nonsensical for them. The false positive fires on the `PostToolUse` hook, so it surfaces
during normal editing, not just on CI.

Confirmed occurrence: three Kotlin test files in `tstapler/stelekit`
(`feat/graph-creation-name-description`) sharing six `import arrow.core.*` /
`dev.stapler.stelekit.*` lines; logged in
`docs/duplicate-code-cross-file-false-positives.md` (2026-09-23 entry) and noted as open
issue #102.

Root cause: `qualifying_window` / `qualifying_windows` in
`src/checkers/duplicate_code.rs` filter windows only on blank lines and `MIN_BLOCK_CHARS`
— no exclusion for windows whose lines are entirely import/use declarations.
`find_duplicate_blocks` (single-file) and `file_windows` (cross-file, via
`qualifying_windows`) both inherit the gap.

## Acceptance Criteria

1. **Kotlin/Java import blocks suppressed**: `duplicate-code-cross-file` does not flag a
   window that consists entirely of `import ...` lines at the top of a `.kt` or `.java`
   file, even when that window appears in 3+ files.

2. **Rust use blocks suppressed**: `duplicate-code` and `duplicate-code-cross-file` do
   not flag a window consisting entirely of `use ...` lines at the top of a `.rs` file.

3. **Go import blocks suppressed**: both checkers suppress a window that falls entirely
   within the leading `import (...)` block of a `.go` file, including quoted import paths
   (e.g. `"fmt"`, `"os"`).

4. **Python/JS/TS import blocks suppressed**: same suppression for `from X import Y` /
   `import X` lines at the top of `.py`, `.js`, `.ts`, `.tsx`, `.jsx` files.

5. **Mid-file import windows still flagged**: a window starting *after* the file's first
   non-blank, non-comment, non-import/use line is NOT suppressed, so genuinely duplicated
   code that happens to use import-style syntax mid-file (unusual but real) is still
   caught.

6. **Genuine duplicates still flagged**: existing `flags_duplicated_block` and
   `find_cross_file_duplicates_flags_duplication` tests still pass unmodified.

7. **Backtest does not regress**: `kibitzer check backtest duplicate-code-cross-file`
   against the Claude Code session transcript corpus produces no additional false positives
   relative to the pre-fix baseline.

8. **False-positive log updated**: `docs/duplicate-code-cross-file-false-positives.md`'s
   open 2026-09-23 entry moves to a `## Fixed` section; `docs/duplicate-code-false-positives.md`
   gets a corresponding new fixed entry noting the import-block scope gap is now closed.

## Out of Scope

- Excluding test files (`*_test.go`, `*Test.kt`) from the duplicate checkers entirely —
  this is a separate open question noted in `docs/duplicate-code-false-positives.md`'s
  "Needs discussion" section.
- Handling "package" declarations (Go's `package foo`, Kotlin's `package ...`) as part of
  the suppressed block — the issue is specifically about import lines, not the whole
  file-top preamble. Package declarations are a single line and cannot fill a 6-line
  window on their own.
- (Originally out of scope, now covered:) the fuzzy duplicate checker
  (`src/checkers/duplicate_code_fuzzy.rs`) applies the same preamble filter, because its
  literal normalization collapses distinct quoted import paths into one key.

## Constraints

- The fix must live in `qualifying_window`/`qualifying_windows` or in their call sites —
  not in a per-checker workaround — so both the hook-based `duplicate-code-cross-file`
  and the batch `kibitzer check duplicates` command benefit from the same fix.
- No new config option or suppression comment should be required; this is a
  correctness fix, not a configurable behavior change.
- `pub(crate)` API surface of `qualifying_window`/`qualifying_windows` must remain
  callable from `duplicate_cross_file_checker.rs`.
