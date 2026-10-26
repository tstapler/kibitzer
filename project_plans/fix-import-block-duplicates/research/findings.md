# Research Findings: fix-import-block-duplicates

**Date**: 2026-10-10

## Code paths involved

### `src/checkers/duplicate_code.rs`

- `qualifying_window(lines, start)` — returns the 6-line slice starting at `start`,
  or `None` if it spans a blank line or totals < 60 chars. **No import awareness.**
  `pub(crate)` — shared with cross-file checker.
- `qualifying_windows(lines)` — iterator over all valid windows in a file, built on
  `qualifying_window`. `pub(crate)` — shared.
- `find_duplicate_blocks(source)` — called by `DuplicateCodeChecker::check()`.
  Runs its own inline windowing loop (not via `qualifying_windows`) but applies
  the same blank-line and char-count filters. Does **not** use `qualifying_windows`.
- `find_cross_file_duplicates(files)` — batch command entry point. Calls
  `index_cross_file_windows(&normalized)`, which calls `qualifying_windows` per file.
- `DuplicateCodeChecker::check(file, ctx)` — calls `is_generated(ctx.source)` for
  the generated-code exclusion, then `find_duplicate_blocks(ctx.source)`. The file
  path is available here as `file: &Path`.

### `src/checkers/duplicate_cross_file_checker.rs`

- `file_windows(source)` — normalizes lines, calls `qualifying_windows`, joins each
  window to a string key for the index. No file-path awareness.
- `DuplicateIndex::reindex_file_and_find_duplicates(file, source)` — calls
  `file_windows(source)`. The file path is available here as `file: &Path`.

## Call chain summary

```
DuplicateCodeChecker::check(file, ctx)
  → find_duplicate_blocks(ctx.source)
      → inline loop (same filters as qualifying_window, but not calling it)

CrossFileDuplicateChecker::check(file, ctx)
  → DuplicateIndex::reindex_file_and_find_duplicates(file, ctx.source)
      → file_windows(ctx.source)
          → qualifying_windows(&normalized)
              → qualifying_window(lines, start)

find_cross_file_duplicates(files)  [batch command]
  → index_cross_file_windows(&normalized)
      → qualifying_windows(lines) per file
```

## Precedent: generated-code exclusion

The 2026-09-12 fix for generated-code false positives (same pattern of "scope problem, not
matching-logic problem") was implemented as an early return in `DuplicateCodeChecker::check`
via `is_generated(ctx.source)`. That approach works for whole-file exclusions but not for
the import-block case, where we want to skip specific windows rather than entire files.

## Design options considered

### Option A — Block-end sentinel (chosen)

Add `pub(crate) fn leading_import_end(lines: &[String], ext: &str) -> usize` that scans
from the top to find the first line that is not blank, not a comment (`//`, `#`, `*`, `/*`),
and not a per-language import/use/package declaration. The returned index is the exclusive
end of the import preamble. A window at `[start, start+MIN_BLOCK_LINES)` is suppressed if
`start + MIN_BLOCK_LINES <= import_end` (the window is entirely within the preamble).

Callers compute `import_end` once per file and pass it alongside the normalized lines.
The filter is applied at each call site (not inside `qualifying_window`/`qualifying_windows`,
keeping their signatures stable for now).

Go multi-line import blocks (`import ( ... )`) require state-machine awareness:
`import (` opens a block, `)` alone closes it; lines inside start with a quoted path
(`"`), an alias + quoted path, or `_`/`.` + quoted path. A simple prefix check handles
all cases if we treat any line starting with `"`, `_`, `.`, or a known alias pattern
inside the Go import context as an import line.

**Pros**: precise (only leading block suppressed, not mid-file import windows); one
computation per file, not per window; no `qualifying_window` signature change needed for
callers that already have lines + extension.

**Cons**: `index_cross_file_windows` must receive file paths alongside normalized lines
(minor API change to an internal function); Go detection requires tracking `import (`
state across lines.

### Option B — Per-window import check

Skip each window where every line matches the per-language import pattern (regardless of
position). Simpler to implement, but suppresses any import-like window anywhere in the
file, not just the leading block. Unusual but real case (conditional/deferred imports
mid-file in Python, type-only imports added mid-file in TS) would be silently skipped.

**Rejected**: less precise than Option A; issue description explicitly says "leading
contiguous import block."

### Option C — Regex on joined window text

Detect if a window's joined text matches a regex like `^(import|use|from)\s+`. Same
imprecision risk as Option B, plus adds a runtime regex compile.

**Rejected**: unnecessary complexity relative to Option A.

## Language import line patterns

| Extension | Patterns (trimmed line starts with…) | Notes |
|-----------|--------------------------------------|-------|
| `.go` | `package `, `import `, `import(`, `"`, `\``, `_`, `)` | Inside `import (...)` block, lines start with quoted paths or aliases; `)` alone closes the block |
| `.kt`, `.java` | `package `, `import `, `@file:` | `@file:` annotations appear before `package` in Kotlin |
| `.rs` | `use `, `extern crate `, `#![`, `#[` | Crate-level attributes appear before `use` |
| `.py` | `import `, `from ` | `from X import Y` form |
| `.js`, `.ts`, `.tsx`, `.jsx` | `import ` | Includes `import type` |

Comment prefixes that don't end the block: `//`, `#` (Python/shell), `*` (inside `/* */`),
`/*`.

## Files to change

1. `src/checkers/duplicate_code.rs` — add `leading_import_end`, update
   `find_duplicate_blocks(source, import_end)`, update `DuplicateCodeChecker::check`
   to compute and pass `import_end`, update `index_cross_file_windows` signature.
2. `src/checkers/duplicate_cross_file_checker.rs` — update `file_windows(file, source)`
   to accept path, compute `import_end`, filter windows; update
   `reindex_file_and_find_duplicates` to pass `file` to `file_windows`.
3. `docs/duplicate-code-false-positives.md` — add a `## Fixed` entry for this change.
4. `docs/duplicate-code-cross-file-false-positives.md` — promote the open 2026-09-23
   entry to `## Fixed`.
5. Tests — new unit tests for `leading_import_end` (Go, Kotlin, Rust, Python, TS) and
   for the no-false-positive behaviors in both checkers.

## Test fixtures to add

- `does_not_flag_kotlin_import_block_cross_file` — three Kotlin files sharing 6 import
  lines (the exact stelekit repro).
- `does_not_flag_rust_use_block` — three `.rs` files sharing 6 `use` lines, single-file
  variant.
- `does_not_flag_go_import_block` — `import (...)` block spanning 6+ lines shared across
  three Go files.
- `still_flags_duplicate_after_import_block` — six identical real-code lines after the
  import block, to confirm AC #6 (genuine duplicates still flagged).
- `does_not_suppress_mid_file_import_window` — import lines appearing mid-file (not in
  leading block) are still flagged when duplicated.
