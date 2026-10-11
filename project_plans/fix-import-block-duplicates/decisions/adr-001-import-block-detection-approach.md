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
through one helper, `non_preamble_windows(lines, ext)` in `src/checkers/import_preamble.rs`,
which computes `leading_import_end` and filters; every call site uses it so none can forget
the filter. This keeps `qualifying_window(s)` as pure "is this window valid content?"
utilities and the language-awareness independently testable.

## Consequences

- `index_cross_file_windows` takes `(path, lines)` pairs (`&[(&Path, Vec<String>)]`) so the
  extension travels with the lines — minor internal API change, no public-surface impact.
- `file_windows` in `duplicate_cross_file_checker.rs` gains `file: &Path` — minor,
  caller already has it.
- `find_duplicate_blocks` gains an `ext: &str` parameter (not a precomputed `import_end`) —
  called only from `DuplicateCodeChecker::check`.
- `duplicate_code_fuzzy.rs` IS covered: it calls `non_preamble_windows` too (its literal
  normalization makes a run of distinct quoted import paths collapse to one key).
