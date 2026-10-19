# Implementation Plan: fix-import-block-duplicates

**Date**: 2026-10-10

## Overview

Add a `leading_import_end` function that computes the exclusive end index of a file's
leading import/use preamble, then filter qualifying windows in both `find_duplicate_blocks`
and `file_windows` to skip windows that fall entirely within that preamble. The fix lands
in two source files plus two false-positive log docs, and requires new unit tests.

## Task Breakdown

### Task 1 — Add `leading_import_end` to `duplicate_code.rs`

**File**: `src/checkers/duplicate_code.rs`

Add a `pub(crate)` function:

```rust
/// Returns the 0-indexed exclusive end of the file's leading import/use preamble:
/// the first line index that is not blank, not a comment, and not an import/package/use
/// declaration for the file's language (inferred from `ext`).  Any qualifying window
/// that starts before this index and ends at or before it falls entirely inside the
/// import preamble and is not worth flagging as a duplication candidate.
/// Returns 0 when `ext` is unrecognised or the file starts with a non-import line.
pub(crate) fn leading_import_end(lines: &[String], ext: &str) -> usize {
    if !matches!(ext, "go" | "kt" | "java" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx") {
        return 0;
    }
    // A file that is entirely preamble yields `lines.len()`, not 0, so it is fully suppressed.
    lines.iter().position(|l| !is_preamble_line(l, ext)).unwrap_or(lines.len())
}

fn is_preamble_line(line: &str, ext: &str) -> bool {
    if line.is_empty()
        || line.starts_with("//")
        || line.starts_with('#')
        || line.starts_with('*')
        || line.starts_with("/*")
    {
        return true;
    }
    match ext {
        "go" => {
            line.starts_with("package ")
                || line.starts_with("import ")
                || line.starts_with("import(")
                || line == ")"
                || line.starts_with('"')
                || line.starts_with('`')
                || line.starts_with("_ ")
                || line.starts_with(". ")
        }
        "kt" | "java" => {
            line.starts_with("package ") || line.starts_with("import ") || line.starts_with("@file:")
        }
        "rs" => {
            line.starts_with("use ")
                || line.starts_with("extern crate ")
                || line.starts_with("#![")
                || line.starts_with("#[")
        }
        "py" => line.starts_with("import ") || line.starts_with("from "),
        "js" | "ts" | "tsx" | "jsx" => line.starts_with("import "),
        _ => false,
    }
}
```

**Note**: `is_preamble_line` is private. `leading_import_end` is `pub(crate)` so
`duplicate_cross_file_checker.rs` can call it.

### Task 2 — Thread `import_end` into `find_duplicate_blocks`

**File**: `src/checkers/duplicate_code.rs`

Change signature to `find_duplicate_blocks(source: &str, ext: &str)` (the extension, not a
precomputed `import_end`). It normalizes lines itself and iterates
`non_preamble_windows(&normalized, ext)`, which skips windows whose last line falls within
the leading import/use preamble. In `DuplicateCodeChecker::check(file, ctx)`:

```rust
Ok(find_duplicate_blocks(ctx.source, lang_ext(file)))
```

(Implemented: the preamble logic lives in `src/checkers/import_preamble.rs`.)

### Task 3 — Thread `import_end` into `file_windows` (cross-file checker)

**File**: `src/checkers/duplicate_cross_file_checker.rs`

Change `file_windows(source: &str)` to `file_windows(file: &Path, source: &str)`:

```rust
fn file_windows(file: &Path, source: &str) -> Vec<(usize, String)> {
    let normalized: Vec<String> = source.lines().map(|l| l.trim().to_string()).collect();
    non_preamble_windows(&normalized, lang_ext(file))
        .map(|(start, window)| (start, window.join("\n")))
        .collect()
}
```

Update `DuplicateIndex::reindex_file_and_find_duplicates(file, source)` — it already
receives `file: &Path`, so pass it through: `file_windows(file, source)`.

### Task 4 — Thread `import_end` into `index_cross_file_windows` (batch command)

**File**: `src/checkers/duplicate_code.rs`

`find_cross_file_duplicates(files: &[(PathBuf, String)])` calls
`index_cross_file_windows(&normalized)`. Change `index_cross_file_windows` to accept
file paths alongside normalized lines:

```rust
fn index_cross_file_windows(
    normalized: &[(&Path, Vec<String>)],
) -> HashMap<&[String], Vec<(usize, usize)>> {
    let mut occurrences = HashMap::new();
    for (file_idx, (path, lines)) in normalized.iter().enumerate() {
        for (start, window) in non_preamble_windows(lines, lang_ext(path)) {
            occurrences.entry(window).or_default().push((file_idx, start));
        }
    }
    occurrences
}
```

In `find_cross_file_duplicates`, build `(path, lines)` pairs from `files` and pass them to
`index_cross_file_windows`.

### Task 5 — Unit tests for `leading_import_end`

**File**: `src/checkers/import_preamble.rs`, `#[cfg(test)] mod tests`

Add tests:
- `leading_import_end_go_grouped_import` — Go file with `package main` + `import (` block, asserts the returned index equals the first `func` line
- `leading_import_end_kotlin` — Kotlin file with package + import block, asserts index at first `class` line
- `leading_import_end_rust` — Rust file with `use` declarations, asserts index at first `fn` or `struct` line
- `leading_import_end_python` — Python file with `import` + `from` lines, asserts index at first non-import line
- `leading_import_end_unknown_ext` — returns 0 for unknown extension

### Task 6 — No-false-positive regression tests

**File**: `src/checkers/duplicate_code.rs` (single-file) and `src/checkers/duplicate_cross_file_checker.rs` (cross-file)

Add:
- `does_not_flag_kotlin_import_block_cross_file` — the exact stelekit repro: three source
  strings each starting with 6 identical `import ...` lines; `find_cross_file_duplicates`
  must return no findings.
- `does_not_flag_rust_use_block` — three `.rs` source strings sharing 6 `use` lines;
  `DuplicateCodeChecker::check` must return no findings.
- `still_flags_duplicate_after_import_block` — confirms AC #6: 6+ identical real code
  lines **after** the import block are still flagged.
- `does_not_suppress_mid_file_duplicate_that_happens_to_use_import_syntax` — verifies
  Option A's precision: a window after the first substantive line is still flagged even
  if its lines look like imports.

### Task 7 — Update false-positive log docs

**File**: `docs/duplicate-code-cross-file-false-positives.md`

Move the 2026-09-23 entry from `## Log` to a new `## Fixed` section. Add:

```markdown
### 2026-09-23 → Fixed 2026-10-10 — leading import-block suppression added

- **Fixed by**: `leading_import_end` (`src/checkers/duplicate_code.rs`) — computes the
  end of the file's leading preamble (blank lines, comments, `import`/`use`/`package`
  declarations); windows entirely within that preamble are filtered in both
  `file_windows` (hook checker) and `index_cross_file_windows` (batch command).
```

**File**: `docs/duplicate-code-false-positives.md`

Add a new `## Fixed` entry noting the import-block scope gap is now closed for the
`duplicate-code` single-file checker as well.

## Sequencing

1. Task 1 (new function) — no callers yet, no breakage.
2. Tasks 2 + 4 together — both edit `duplicate_code.rs`; batch them.
3. Task 3 — edits `duplicate_cross_file_checker.rs`.
4. Task 5 — unit tests for `leading_import_end`.
5. Task 6 — regression tests (confirm the fix works end to end).
6. Task 7 — docs update.

Run `cargo test` after Task 6 to verify all existing tests still pass.

## Adversarial review notes

**Concern**: `leading_import_end` returns 0 for unknown extensions but the callers don't
handle that specially — a 0 means "no preamble," which is safe (no windows are suppressed
for unknown types). Verified: `start + MIN_BLOCK_LINES <= 0` is always false when
`import_end = 0` and `MIN_BLOCK_LINES = 6`. No bug.

**Concern**: Go's grouped import block contains lines starting with `"` — stripping
those as "import lines" is an approximation. A file that truly starts with a `const` or
`var` block using string literals could be false-negated. However, `const` and `var`
blocks don't appear before `import` in well-formed Go, and the check is specifically for
the *leading* preamble — if the file starts with `"foo"` it's not valid Go anyway. Low
risk.

**Concern**: `leading_import_end` normalizes lines independently from the caller.
`find_duplicate_blocks` normalizes again internally. This double-allocates, but since
`leading_import_end` is `O(n)` and called once per file, the overhead is negligible.

**Concern**: `duplicate_code_fuzzy.rs` has its own window logic. Now covered: it calls
`non_preamble_windows` as well, since literal normalization would otherwise collapse a run
of distinct quoted import paths into one key.

**Resolved in validate pass (2026-10-11)**:
- `leading_import_end` used `unwrap_or(0)`, so a file that is entirely imports returned
  "no preamble" and stayed flaggable. Now returns `lines.len()`; unknown extensions
  return 0 via an explicit early guard so that change cannot suppress them.
- Task 4 cloned every `PathBuf`; now borrows `&[&Path]`.
- Existing tests never call `find_duplicate_blocks`/`index_cross_file_windows` directly
  (grep of `src/`), so the signature changes keep AC6 (tests unmodified) satisfiable.
