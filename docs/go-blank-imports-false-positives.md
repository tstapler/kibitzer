# `go-blank-imports` check — known false positives

Tracks confirmed false-positive firings of the `go-blank-imports` kibitzer check
(`src/go_blank_imports.rs`), wired up via `kibitzer check native go-blank-imports
{file}`. Check new occurrences against this list before re-investigating a firing
from scratch.

## Mechanism (confirmed by reading the source)

The check walks the tree-sitter parse for every `import_spec` whose `name` field
is a `blank_identifier` (i.e. `import _ "pkg"`), and considers it justified via a
same-row trailing comment, or via `justified_by_leading_run` — a backward walk
through the import's preceding siblings (comments and other import specs, in
source order) that stops at either a leading comment (justified) or a blank-line
gap between siblings (not justified). This lets one header comment justify every
import in a contiguous, blank-line-free run below it — including past an
intervening named import — rather than only the single import immediately
below it (see `## Fixed` below for the fix history).

## Documented scope gaps

- **Comment two or more lines above the import.** A block comment separated
  from the import by a blank line is not detected as justification — only the
  immediately adjacent row counts. Move the comment to be adjacent, or expect a
  firing.
- **A doc comment on the `import (` block itself**, rather than on the specific
  blank-import line, does not justify any individual entry in the group.
- **Aliased non-blank imports** (`import f "fmt"`) and normal named imports are
  never flagged — only `name: (blank_identifier)` triggers the check.

## Known limitation: unbounded recursion on pathological input

`collect_comment_rows`/`collect_import_spec_rows`/`collect_blank_imports`
recurse over the tree-sitter AST with no depth guard. A Go source file with
extreme nesting depth can exhaust the stack and abort the `kibitzer` process
rather than degrading to a failed check result. Pre-existing pattern shared
with `src/primitive_obsession.rs`'s AST walk — no depth guard exists anywhere
in the codebase yet.

## Fixed

### 2026-09-12 — kubernetes/kubernetes + stapler-squad corpus backtest — group-leading comment only credited the very next import

- **Symptom**: `providers.go`/`build/tools.go` (kubernetes/kubernetes) each had one
  header comment followed by several blank imports in the same `import (...)` block;
  only the import immediately below the comment was justified, the rest were flagged.
  `features_test.go` (stapler-squad) had a real justification comment separated from
  its blank import by a named import that `goimports` had sorted in between.
- **Mechanism**: `leading_comment_rows.contains(&(row - 1))` (old
  `collect_blank_imports`) only ever checked exactly one row above the flagged
  import — no concept of "this comment justifies the whole contiguous run below
  it," and no way to see past an intervening sibling import.
- **Fixed by**: replacing the single-row lookback with `justified_by_leading_run`
  (`src/go_blank_imports.rs`) — a backward walk through preceding siblings that
  stops at a leading comment (justified) or a blank-line gap (not justified),
  passing through intervening import specs either way. Regression-guarded by
  `leading_comment_justifies_whole_contiguous_run_of_blank_imports`,
  `leading_comment_justifies_blank_import_past_intervening_named_import`,
  `blank_line_breaks_the_run_so_earlier_comment_does_not_justify_later_import`, and
  `flags_blank_import_with_no_comment_anywhere_nearby` (`src/go_blank_imports.rs`'s
  test module). Verified directly against the real corpus files: `providers.go`,
  `build/tools.go`, and `features_test.go` now produce no findings, while a genuine
  true positive (`cmd/genfeaturegates/genfeaturegates.go`, kubernetes/kubernetes,
  commentless blank imports) still flags.

## Log

No open entries.
