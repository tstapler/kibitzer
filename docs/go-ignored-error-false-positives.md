# `go-ignored-error` check — known false positives

Tracks confirmed false-positive firings of the `go-ignored-error` kibitzer check
(`src/go_ignored_error.rs`), wired up via `kibitzer check native go-ignored-error
{file}`. Check new occurrences against this list before re-investigating a
firing from scratch.

## Mechanism (confirmed by reading the source)

The check walks every `short_var_declaration`'s `left` expression list and
flags it only when there are two or more names AND the *last* one is the
identifier `_`. It deliberately does not flag a blank identifier in any other
position — `_, err := f()` keeps the error and is never flagged, since a blank
in the first slot discards a different (non-error, by convention) value. This
is a syntactic, convention-based heuristic: it has no type information and
cannot confirm the discarded value is actually an `error`.

## Documented scope gaps

- **Non-error last return values.** `x, _ := someMapLookup()`-shaped code where
  the last return isn't actually an error (e.g. the `ok` from a two-value map
  index or type assertion, `v, ok := m[k]`) is flagged even though nothing
  error-shaped is being discarded — the check has no type information and
  can't distinguish `(T, bool)` from `(T, error)`.
- **Plain assignment, not declaration.** Only `:=` (`short_var_declaration`) is
  checked; a plain `result, _ = f()` re-assignment is a different tree-sitter
  node kind and is not covered.
- **Function calls without a trailing blank at all** (`result, err := f()`) are
  never flagged, by design — the check only fires when the last slot is
  explicitly discarded.

## Known limitation: unbounded recursion on pathological input

The AST walk that finds `short_var_declaration` nodes recurses with no depth
guard. A Go source file with extreme nesting depth can exhaust the stack and
abort the `kibitzer` process rather than degrading to a failed check result.
Pre-existing pattern shared with `src/primitive_obsession.rs`'s AST walk — no
depth guard exists anywhere in the codebase yet.

## Log

No confirmed false-positive firings logged yet. When one occurs — most likely
the `v, ok := ...` shape above — add an entry here in the same format as
`docs/go-primitive-obsession-false-positives.md`: repo, what changed, why it's
a false positive, and the mechanism (traced back to source, not guessed).
