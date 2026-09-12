# `go-error-context` check — known false positives

Tracks confirmed false-positive firings of the `go-error-context` kibitzer
check (`src/go_error_context.rs`), wired up via `kibitzer check native
go-error-context {file}`. Advisory by default (`"severity": "advisory"` in
`.claude/inspect.json`) — this is a style nudge, not a correctness bug. Check
new occurrences against this list before re-investigating a firing from
scratch.

## Mechanism (confirmed by reading the source)

The check only fires in a file where `has_wrapping_convention` finds at least
one `fmt.Errorf(...)` call containing `%w` in an interpreted (`"..."`) or raw
(`` `...` ``) string literal argument — without that signal, the file has no
established wrapping convention to be inconsistent with, so the check stays
silent everywhere in it. Within such a file, `collect_bare_passthroughs`/
`is_bare_err_passthrough` matches the shape `if <id> != nil { return ...,
<id> }` (parenthesized conditions are unwrapped first): a `binary_expression`
condition comparing one identifier to `nil` with `!=`, whose consequence
block's *only* statement is a `return_statement` whose returned expression
list contains exactly one identifier, matching that same name — a plain
single-value `return err` or a multi-value `return 0, err` both match, since
only the identifiers in the list are considered.

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

## Known limitation: unbounded recursion on pathological input

`has_wrapping_convention` and `collect_bare_passthroughs` recurse over the
tree-sitter AST with no depth guard. A Go source file with extreme nesting
depth (thousands of nested blocks — implausible from a human but possible
from generated code) can exhaust the stack and abort the `kibitzer` process
rather than degrading to a failed check result. This is a pre-existing
pattern shared with `src/primitive_obsession.rs`'s AST walk, not something
new to this check; no depth guard exists anywhere in the codebase yet.

## Fixed

### 2026-09-12 — kubernetes/kubernetes + stapler-squad corpus backtest — single stray `%w` in a large/vendor/generated file implied a false file-wide convention

- **Symptom**: `vendor/golang.org/x/net/http2/transport.go` (kubernetes/kubernetes, a 3036-line vendored file with
  exactly one `%w` call among dozens of bare passthroughs) and `session/ent/claudesession_query.go`
  (stapler-squad, an ent-codegen file whose single `%w` helper is boilerplate the generator always emits) both
  flagged — neither file's authors chose a wrapping convention; one incidental `%w` call is noise in code nobody
  hand-edits for wrapping style.
- **Mechanism**: `has_wrapping_convention` (`src/go_error_context.rs`) treated a single `%w` occurrence anywhere
  in the file as binary proof of a file-wide convention, with no frequency/density threshold and no exclusion for
  generated files (`file_globs` is a bare `**/*.go`).
- **Fixed by**: two changes. (a) An early return for generated files, reusing `file_size::is_generated`. (b)
  `has_wrapping_convention` replaced by `count_wrapping_occurrences`/`wrapping_convention_established(wrap_count,
  bare_count) -> bool { wrap_count >= 2 || bare_count <= wrap_count }` — two or more independent wraps count
  regardless of file size, and a single wrap only counts as a convention if bare passthroughs don't outnumber it
  (still catches the existing locked-in small-file case of one wrap, one bare passthrough). No `vendor/`
  path-based exclusion was added — `src/check.rs`'s `SKIP_DIRS` already excludes `vendor/` in normal `kibitzer
  run` usage; the `transport.go` false positive only reproduced via direct `check native` invocation, which the
  generated-file-header check doesn't cover for non-generated vendor code, but the density fix (b) independently
  handles it regardless of path. Regression-guarded by `does_not_flag_generated_file`,
  `does_not_flag_lone_wrap_among_many_bare_passthroughs`, and `flags_genuine_inconsistency_with_multiple_wrap_sites`
  (`src/go_error_context.rs`'s test module). Verified directly: both corpus files now produce no findings, while
  the doc's cited true positive (`cmd/kube-controller-manager/app/certificates.go:382`, kubernetes/kubernetes)
  still flags.

## Log

### 2026-09-12 — kubernetes/kubernetes + stapler-squad corpus backtest — no data-flow awareness of an already-wrapped error

- **Repo**: `kubernetes/kubernetes`, file
  [`pkg/kubelet/cm/dra/manager.go:492`](https://github.com/kubernetes/kubernetes/blob/0ba07e0866bb1e3d67464b2d3e01f6bde6fe9963/pkg/kubelet/cm/dra/manager.go#L492);
  `tstapler/stapler-squad`, file
  [`session/workspace_peers.go:91`](https://github.com/tstapler/stapler-squad/blob/d320c038ceea6eb1ec6e193f9611f1540a49f25f/session/workspace_peers.go#L91).
- **What the code looks like**: in `manager.go`, `err` was already wrapped one line above inside the same
  closure (`fmt.Errorf("checkpoint ResourceClaim state: %w", err)`), with an inline comment confirming it ("No
  wrapping, this is the error above."). In `workspace_peers.go`, the returned `err` comes from
  `sessionGoalsByUUIDs`, which already wraps every failure it returns.
- **Why it's a false positive**: the error already carries context by the time it reaches the flagged
  `return ..., err` — wrapping it again would double-wrap, not fix a real gap.
- **Mechanism**: `bare_err_passthrough_name` (`src/go_error_context.rs`, renamed from `is_bare_err_passthrough`
  by the 2026-09-12 fix below) is pure local AST pattern matching within one `if` statement — it has no data-flow
  or cross-call visibility into whether the specific `err` being returned was already wrapped a line above (same
  function) or by the callee that produced it (a different function entirely). It can't distinguish "this file
  wraps errors, and this specific one still needs it" from "this file wraps errors, and this specific one already
  has it."
- **Attempted, does not close this entry**: `wrapped_earlier_in_enclosing_function` (added by the same 2026-09-12
  fix) does a same-function backward scan for a textual `err = fmt.Errorf(..., "%w", ..., err, ...)`
  reassignment anywhere before the flagged `if`, not just the immediately preceding statement. It does **not**
  help here: `manager.go`'s wrap happens *inside a closure passed to another call*
  (`err = m.cache.withLock(logger, func() error { ... return fmt.Errorf(...) ... })`) — the outer assignment is
  just `err = <call>`, with no `fmt.Errorf` visible at the assignment site at all — and `workspace_peers.go`'s
  wrap happens in a different function entirely. Both need real interprocedural reasoning (does this specific
  call's result already carry wrapped context?) that no amount of same-function textual scanning can provide.
  Confirmed by re-running the checker against both files after the fix: both still flag, unchanged.
