# ADR-001: Import block detection approach

**Date**: 2026-10-10
**Status**: Accepted

## Context

`qualifying_window`/`qualifying_windows` have no awareness that a window falls within a
file's import/use preamble. Three approaches were evaluated:

1. **Block-end sentinel** — compute `leading_import_end` once per file; skip windows
   whose `[start, start+MIN_BLOCK_LINES)` range falls entirely within it.
2. **Per-window import check** — for each window, check if all lines match a per-language
   import pattern; suppress regardless of position in the file.
3. **Regex on joined window text** — same as Option 2 but using a compiled regex.

## Decision

Option 1 (block-end sentinel).

## Rationale

The false positive is specifically about the **leading** import block — imports mid-file
(unusual but valid in Python, possible in TS) that happen to be duplicated represent
genuine refactoring signal and should not be suppressed. Options 2 and 3 suppress
import-pattern windows anywhere in the file, which is too broad.

Option 1 computes the block end once per file (O(n) single pass, negligible cost), then
the window filter is a single integer comparison per window.

`qualifying_window`/`qualifying_windows` signatures are unchanged; the filter is applied
at each call site with the pre-computed `import_end`. This keeps the two shared functions
as pure "is this window valid content?" utilities and separates the language-awareness
into a dedicated `leading_import_end` function that's independently testable.

## Consequences

- `index_cross_file_windows` gains a `paths: &[PathBuf]` parameter — minor internal API
  change, no public-surface impact.
- `file_windows` in `duplicate_cross_file_checker.rs` gains `file: &Path` — minor,
  caller already has it.
- `find_duplicate_blocks` gains `import_end: usize` parameter — `pub(crate)` already,
  called only from `DuplicateCodeChecker::check`.
- `duplicate_code_fuzzy.rs` is not covered by this fix (out of scope, no confirmed FP
  there yet).
