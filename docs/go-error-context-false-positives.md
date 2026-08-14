# `go-error-context` check — known false positives

Tracks confirmed false-positive firings of the `go-error-context` kibitzer
check (`src/go_error_context.rs`), wired up via `kibitzer check native
go-error-context {file}`. Advisory by default (`"severity": "advisory"` in
`.claude/inspect.json`) — this is a style nudge, not a correctness bug. Check
new occurrences against this list before re-investigating a firing from
scratch.

## Mechanism (confirmed by reading the source)

The check only fires in a file where `has_wrapping_convention` finds at least
one `fmt.Errorf(...)` call containing `%w` in a string literal argument —
without that signal, the file has no established wrapping convention to be
inconsistent with, so the check stays silent everywhere in it. Within such a
file, `collect_bare_passthroughs`/`is_bare_err_passthrough` matches only the
exact shape `if <id> != nil { return <id> }`: a `binary_expression` condition
comparing one identifier to `nil` with `!=`, whose consequence block's *only*
statement is a `return_statement` returning that same identifier unchanged.

## Documented scope gaps (deliberate, by design)

These are the four exclusions from criterion 4, each covered by a test in
`src/go_error_context.rs`'s `tests` module:

- **Sentinel comparisons** (`does_not_flag_sentinel_comparison`) — `if err ==
  io.EOF { return err }` is not `!= nil`, so it's a different comparison
  entirely and is never matched.
- **`errors.Is`/`errors.As` chains** (`does_not_flag_errors_is_chain`) — `if
  errors.Is(err, ErrNotFound) { return err }`'s condition is a `call_expression`,
  not a `binary_expression` against `nil`, so it falls through untouched.
- **Defer-based handling** (`does_not_flag_defer_based_handling`) — wrapping
  done inside a `defer func() { ... }()` closure (a common pattern for named
  return values) is invisible to this check: the bare `if err != nil { return
  err }` shape doesn't appear at the call site at all in that pattern.
- **Named returns with a bare `return`** (`does_not_flag_named_return_bare_return`)
  — `if err != nil { return }` (no expression) doesn't match: the consequence's
  only statement is a `return_statement` with no returned identifier, which
  `single_identifier_matches` rejects (it requires exactly one identifier in
  the returned expression list).

## Log

No confirmed false-positive firings logged yet beyond the deliberate exclusions
above. When one occurs, add an entry here in the same format as
`docs/go-primitive-obsession-false-positives.md`: repo, what changed, why it's
a false positive, and the mechanism (traced back to source, not guessed).
