# Requirements: go-stdlib-idioms-fp

## Summary

Two Go checks fire on code whose shape is dictated by the standard library or by
well-established Go logging idioms. Both findings are non-actionable — the developer
cannot change the signatures without breaking the stdlib contract or making the code
less idiomatic — so they are false positives.

## Background

Filed from editing `server/user_themes.go` and `server/user_themes_test.go` in
`tstapler/stapler-squad` (branch `chore/sdd-go-git-fork-planning`) on 2026-10-09.

## Issue 1 — `go-primitive-obsession`: stdlib callback signatures

### Symptom

`sort.Slice(s, func(i, j int) bool { ... })` fires:
> "parameter declaration names 2 identifiers of primitive type `int`"

The `i, j int` parameter pair is not a user-designed API — it is the mandatory signature
of the comparator argument to `sort.Slice` (and related functions: `sort.SliceStable`,
`sort.Search`, `slices.SortFunc`, `slices.SortStableFunc`, `slices.BinarySearchFunc`,
`slices.IndexFunc`, `slices.ContainsFunc`, etc.). Extracting a named type for `i` and
`j` is not idiomatic and does not reduce the risk of argument-order confusion.

### Issue 1b — Test closures

`write := func(name, body string)` in a `_test.go` file also fired. Unexported function
literals assigned inside test functions are not public APIs and carry negligible
argument-order confusion risk compared to exported function declarations.

## Issue 2 — `syntax-rules-go replace-magic-literal`: slog attribute keys

### Symptom

`log.Warn("user-themes: ...", "file", name, "err", err)` fires:
> "string literal `"file"` appears N times (also lines ...) — consider extracting a named constant"

`"file"` and `"err"` are `slog`-style attribute keys in alternating key-value position.
The `log/slog` API (and the widely used `log/slog`-compatible wrappers) takes a message
followed by zero or more `(key string, value any)` pairs. Extracting string constants for
per-call keys obscures the log call and is not idiomatic Go.

## Acceptance Criteria

### AC-1 (primitive-obsession — stdlib callbacks)
`sort.Slice(s, func(i, j int) bool { ... })` produces **zero** `go-primitive-obsession`
findings when run against a Go file containing this pattern.

### AC-2 (primitive-obsession — broader sort/slices coverage)
The fix covers the full family of stdlib sort/slices callbacks:
`sort.Slice`, `sort.SliceStable`, `sort.Search`, `slices.SortFunc`,
`slices.SortStableFunc`, `slices.BinarySearchFunc`, `slices.IndexFunc`,
`slices.ContainsFunc`, `slices.EqualFunc`, `slices.CompareFunc`.

### AC-3 (primitive-obsession — test closures)
A `func_literal` with a same-typed multi-parameter list that appears in a `_test.go`
file produces **zero** `go-primitive-obsession` findings.

### AC-4 (primitive-obsession — true positives preserved)
A real multi-parameter pile-up on an exported function declaration (`func newUser(name,
email string) {}`) in a non-test file still fires.

### AC-5 (replace-magic-literal — slog key exclusion)
String literals in odd-indexed argument positions (position 1, 3, 5, … after the initial
message argument) of a call matching a slog-style logging function (`slog.Info`, `slog.Warn`,
`slog.Error`, `slog.Debug`, `log.Info`, `log.Warn`, `log.Error`, `log.Debug`, any
`*.Info`/`*.Warn`/`*.Error`/`*.Debug` selector call) do **not** appear in
`replace-magic-literal` findings.

### AC-6 (replace-magic-literal — true positives preserved)
A string literal that genuinely repeats 3+ times outside slog-key position still fires.

### AC-7 (backtest gate)
Both checkers must be run against the standard backtest corpus
(`kubernetes/kubernetes`, `BurntSushi/ripgrep`, `denoland/deno`, `stapler-squad`)
and produce no new true-positive regressions after the fix.

### AC-8 (regression tests)
New unit tests cover each suppression case and each true-positive guard.
