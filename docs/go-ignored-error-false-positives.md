# `go-ignored-error` check — known false positives

Tracks confirmed false-positive firings of the `go-ignored-error` kibitzer check
(`src/go_ignored_error.rs`), wired up via `kibitzer check native go-ignored-error
{file}`. Check new occurrences against this list before re-investigating a
firing from scratch.

## Mechanism (confirmed by reading the source)

The check walks every `short_var_declaration`'s `left` expression list and
flags it only when there are two or more names, the *last* one is the
identifier `_`, and (as of the fix below) the `right` field is a single
`call_expression` — a comma-ok type assertion or map index on the RHS is
excluded via `rhs_is_call`, since those are syntactically distinct from a
function call and the discarded value is never an error by convention. It
deliberately does not flag a blank identifier in any other position —
`_, err := f()` keeps the error and is never flagged, since a blank in the
first slot discards a different (non-error, by convention) value. Beyond the
RHS-is-a-call check, this remains a syntactic heuristic: it has no type
information and cannot confirm a real call's discarded value is actually an
`error` (see the remaining open gap in `## Log` below).

## Documented scope gaps

- **Non-error last return value from a method call on a local variable, or any
  call into stdlib/vendor/another module.** `sync.Map.LoadOrStore` (`m.LoadOrStore(...)`
  where `m sync.Map`), `singleflight.Group.Do` (`g.Do(...)` where `g singleflight.Group`),
  and any call whose callee isn't a same-module package-qualified free function are
  still flagged when the real last return isn't an error — resolving these would need
  either local type-inference (to know `m`'s/`g`'s declared type) or reading
  stdlib/vendor source, both out of scope for the fix below. (A same-module
  package-qualified free function whose real last return isn't an error, e.g.
  `pkiutil.PathsForCertAndKey`, is fixed — see `## Fixed` below.)
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

## Fixed

### 2026-09-12 — kubernetes/kubernetes corpus backtest — comma-ok type assertion/map index (non-call RHS) misread as an ignored error

- **Symptom**: `prefix, _ := v[1].(string)` (kubernetes/kubernetes
  `vendor/k8s.io/klog/v2/klogr.go:80`) flagged — this is a type assertion, not a function call, discarding the
  comma-ok bool, not an error.
- **Mechanism**: the check never inspected the RHS at all — any `short_var_declaration` with ≥2 names and a
  trailing `_` fired regardless of what produced the values.
- **Fixed by**: `rhs_is_call(node)` requires the `short_var_declaration`'s `right` field to be a single
  `call_expression` before flagging — a type assertion (`type_assertion_expression`) or map index
  (`index_expression`) on the RHS no longer qualifies. Confirmed via a tree-sitter-go 0.25 grammar dump that
  `right` is an `expression_list` whose sole child is one of these distinct node kinds. Regression-guarded by
  `does_not_flag_comma_ok_type_assertion` and `does_not_flag_comma_ok_map_index` (`src/go_ignored_error.rs`'s test
  module), plus a true-positive guard `flags_ignored_error_from_real_call`. Verified directly: `klogr.go:80` is
  gone, while the doc's cited true positive (`cmd/kubeadm/app/cmd/util/join_test.go:44`, kubernetes/kubernetes)
  still flags. Note: a plain `result, _ = f()` re-assignment (not `:=`) remains a separate, already-documented,
  unfixed scope gap — a different tree-sitter node kind this fix doesn't touch.

### 2026-09-12 — kubernetes/kubernetes corpus backtest — same-module cross-package free function whose real last return isn't an error

- **Symptom**: `pkiutil.PathsForCertAndKey` (kubernetes/kubernetes
  `cmd/kubeadm/app/phases/certs/renewal/readwriter.go:64`) flagged — the real declaration returns
  `(string, string)`, so the discarded value is a plain path, never an error.
- **Mechanism**: the check has no type information and can't know a given callee's actual return types — it can
  only see that the shape (a real call, ≥2 names, trailing `_`) matches.
- **Fixed by**: `src/go_call_resolution.rs`, a new module that — for a package-qualified free-function call whose
  import alias resolves (via the current file's own `import_spec`s) to an import path under this file's own Go
  module (per `go.mod`'s `module` directive) — reads that package's actual `.go` source off disk, finds the
  matching top-level `function_declaration`, and checks whether its real last return type is literally `error`.
  Wired into `go_ignored_error.rs::safe_to_suppress`: resolved-and-not-`error` suppresses the finding; anything
  unresolvable (not a package-qualified selector, no `go.mod` found, import not under this module, target
  file/function not found) falls back to flagging exactly as before — resolution only ever *suppresses*, never
  adds a new finding, so failure modes stay conservative. Deliberately does not attempt method calls on local
  variables (`m.LoadOrStore(...)`) or calls into stdlib/vendor/another module — see the scope gap above.
  Regression-guarded by `does_not_flag_same_module_call_whose_real_last_return_is_not_error`,
  `still_flags_same_module_call_whose_real_last_return_is_error` (true-positive guard), and
  `still_flags_when_the_import_is_not_under_this_module` (conservative-fallback guard) in
  `src/go_ignored_error.rs`'s test module, built against a real temp Go module on disk (not synthetic
  string-matching). Verified directly: `readwriter.go:64` is gone (and the file now produces zero findings at
  all), while `config.go:1026` (`sync.Map`) and `github/client.go:159` (`singleflight`) in stapler-squad — both
  method calls, out of scope per the updated gap above — still flag, unchanged. Aggregate `go-ignored-error`
  findings on kubernetes/kubernetes dropped from 4,401 to 3,893 across the whole corpus from this one mechanism.

## Log

No open entries beyond the documented scope gaps above.
